//! Pass 5: Structurer - Convert SSA-CFG to structured AST
//!
//! This module transforms an SSA-annotated CFG back into structured code:
//! - Uses loop info from Analyzer to emit `while` loops
//! - Uses dominator tree to structure if/else
//! - Converts φ-functions to variable assignments at branch ends

pub mod expression;
pub mod idioms;
pub mod lower;
pub mod patterns;
pub mod reducer;
pub mod region;
pub mod region_graph;
pub mod sese;
pub mod stmts;

use petgraph::graph::NodeIndex;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, Reg, RefFun, RefString, RefType, Type};
use hlbc::{Bytecode, Str};

use crate::analyzer::{CfgAnalysis, NaturalLoop};
use crate::ast::{Constant, Expr, Operation, Statement};
use crate::lifter::Cfg;
use crate::ssa::{SsaCfg, SsaInstr, SsaVar};
use crate::type_prop::TypeInfo;

use crate::ssa::UseDefInfo;

use crate::closure_analysis::ClosureAnalysis;
use crate::exception_analysis::{ExceptionAnalysis, TryRegion};

// Re-exports for new reducer-based structuring (used by lib.rs)
pub use lower::{lower_region, LoweringContext};
pub use patterns::PatternContext;
pub use reducer::reduce_to_region;

pub use stmts::simplify_statements; // external export

/// Tracks memory dependencies for SSA inline expressions
/// Used to determine when an inline expression must be invalidated
/// because its source memory has been modified.
#[derive(Clone, Debug)]
pub(crate) enum MemoryDep {
    /// No memory dependency - constants, arithmetic results.
    /// Always safe to inline.
    None,
    /// Depends on a specific field of an object.
    /// Invalidated when SetField writes to the same (obj_reg, field_idx).
    Field { obj: Reg, field: usize },
    /// Depends on a specific global variable.
    /// Invalidated when SetGlobal writes to the same global.
    Global { global: hlbc::types::RefGlobal },
    /// Conservative dependency - could read any memory.
    /// Invalidated by any memory write or call.
    AnyMemory,
}

/// A detected string switch case
#[derive(Debug, Clone)]
struct StringSwitchCase {
    /// The string literal for this case
    string_ref: RefString,
    /// Opcode index of the handler (where JEq jumps to)
    handler_op: usize,
}

/// A detected string switch region in bytecode
#[derive(Debug, Clone)]
pub(crate) struct StringSwitchRegion {
    /// First opcode of the switch (the first JNull)
    pub(crate) start_op: usize,
    /// Last opcode of the switch pattern (before handlers/default)
    pub(crate) end_op: usize,
    /// The register holding the string being switched on
    pub(crate) switch_arg_reg: Reg,
    /// All detected cases with their string values and handler targets
    cases: Vec<StringSwitchCase>,
    /// Opcode index of the default case handler
    pub(crate) default_op: usize,
}

pub struct Structurer<'a> {
    pub(crate) code: &'a Bytecode,
    pub(crate) func: &'a Function,
    pub(crate) cfg: &'a Cfg,
    pub(crate) analysis: &'a CfgAnalysis,
    pub(crate) ssa: &'a SsaCfg,
    pub(crate) _type_info: &'a TypeInfo,

    pub(crate) closure_analysis: Option<&'a ClosureAnalysis>,

    /// True if this is a `this`-bound closure (from InstanceClosure opcode)
    /// where reg0 is implicitly bound to `this`
    pub(crate) is_this_bound_closure: bool,

    /// Processed blocks (to avoid re-processing)
    pub(crate) processed: HashSet<NodeIndex>,
    /// Use-def info for inlining decisions (ILSpy-style)
    pub(crate) use_info: HashMap<SsaVar, UseDefInfo>,
    /// Variable names that have been declared (for declaration tracking)
    pub(crate) declared_vars: HashSet<Str>,
    /// Current opcode index being processed (for debug name lookup)
    pub(crate) current_op: usize,
    /// Method info: maps function references to (owner_type, method_name)
    /// Used to convert f(obj, args) to obj.f(args) syntax
    pub(crate) method_info: HashMap<RefFun, (RefType, Str)>,
    /// Current scope depth (0 = function level, >0 = inside loop/if/switch)
    pub(crate) scope_depth: u32,
    /// Variables that need hoisting to function scope (declared inside nested scope)
    pub(crate) hoisted_vars: HashSet<Str>,
    /// Hoisted vars that need :Dynamic type (assigned empty anonymous objects)
    pub(crate) needs_dynamic_type: HashSet<Str>,
    /// Hoisted vars -> their types (for type hints in declarations)
    pub(crate) hoisted_var_types: HashMap<Str, RefType>,
    /// Array bytes tracking: maps bytes register -> array expression
    /// Used to reconstruct arr[i] from bytes[shifted_i] pattern
    /// We store the Expr (not Reg) to capture the correct SSA version at field access time
    pub(crate) array_bytes_source: HashMap<Reg, Expr>,
    /// Shifted index tracking: maps shifted reg -> (original index expression, shift amount)
    /// Used to reverse index * 4 back to original index for array access
    /// We store the Expr (not Reg) to capture the correct SSA version at shift time
    pub(crate) shifted_indices: HashMap<Reg, (Expr, i32)>,
    /// Exception region analysis for try/catch structuring
    pub(crate) exception_analysis: ExceptionAnalysis,
    /// Enum global -> (enum type, constructor index) mapping
    /// Built by analyzing the entry point function's enum initialization pattern
    pub(crate) enum_global_map: HashMap<hlbc::types::RefGlobal, (RefType, usize)>,
    /// String conversion sources: maps length output reg -> original value expression
    /// Used to track ftos/itos/dtos(value, ref_out) where ref_out points to length_reg
    /// When __alloc__(bytes, length_reg) is seen, we can use the original value expression
    /// We store Expr (not Reg) to capture the correct SSA-versioned name at conversion time
    pub(crate) string_conversion_source: HashMap<Reg, Expr>,
    /// Ref targets: maps ref reg -> target reg (for Ref dst = &src)
    pub(crate) ref_targets: HashMap<Reg, Reg>,
    /// Registers that should use raw names (rN) instead of debug names
    /// to avoid type conflicts (e.g., iterator vs iteration value)
    pub(crate) use_raw_name_regs: HashSet<Reg>,
    /// Registers that hold iterators (from .keys() or .iterator() calls)
    /// Used to detect when these are later reassigned to non-iterator values
    pub(crate) iterator_regs: HashSet<Reg>,
    /// Current SSA destination variable (set when processing each opcode)
    /// Used for SSA-versioned naming
    pub(crate) current_ssa_dst: Option<SsaVar>,
    /// Current SSA source variables (set when processing each opcode)
    /// Used for SSA-versioned naming of source operands
    pub(crate) current_ssa_uses: Vec<SsaVar>,
    /// Detected string switch regions (from bytecode pattern analysis)
    pub(crate) string_switches: Vec<StringSwitchRegion>,
    /// Current loop header (if any) - used to distinguish continue from switch fall-through
    pub(crate) current_loop_header: Option<NodeIndex>,
    /// Expressions available for inlining (SSA var -> (expression, memory dependency))
    /// Single-use, pure expressions are stored here instead of emitting a statement.
    /// When the variable is referenced, the stored expression is inlined at the use site.
    /// The memory dependency tracks what memory the expression reads, allowing smart
    /// invalidation when conflicting writes occur.
    /// Uses RefCell for interior mutability so we can remove entries when inlined.
    pub(crate) inline_exprs: RefCell<HashMap<SsaVar, (Expr, MemoryDep)>>,
    /// Opcodes to suppress (not emit as statements)
    /// Used when an opcode's result is consumed by another construct (e.g., EnumIndex for switch)
    pub(crate) suppressed_ops: HashSet<usize>,
    /// Enum pattern bindings: maps (switch_reg, construct_idx, field_idx) -> bound param name
    /// When set, EnumField opcodes matching these keys emit the param name
    /// instead of Type.enumParameters(...). Used to generate cleaner switch case patterns.
    pub(crate) enum_param_bindings: HashMap<(Reg, usize, usize), String>,
}

impl<'a> Structurer<'a> {
    pub fn new(
        code: &'a Bytecode,
        func: &'a Function,
        cfg: &'a Cfg,
        analysis: &'a CfgAnalysis,
        ssa: &'a SsaCfg,
        type_info: &'a TypeInfo,
    ) -> Self {
        Self::new_with_closures(code, func, cfg, analysis, ssa, type_info, None)
    }

    pub fn new_with_closures(
        code: &'a Bytecode,
        func: &'a Function,
        cfg: &'a Cfg,
        analysis: &'a CfgAnalysis,
        ssa: &'a SsaCfg,
        type_info: &'a TypeInfo,
        closure_analysis: Option<&'a ClosureAnalysis>,
    ) -> Self {
        Self::new_with_options(code, func, cfg, analysis, ssa, type_info, closure_analysis, false)
    }

    pub fn new_with_options(
        code: &'a Bytecode,
        func: &'a Function,
        cfg: &'a Cfg,
        analysis: &'a CfgAnalysis,
        ssa: &'a SsaCfg,
        type_info: &'a TypeInfo,
        closure_analysis: Option<&'a ClosureAnalysis>,
        is_this_bound_closure: bool,
    ) -> Self {
        // (needs func for purity info)
        let use_info = ssa.compute_use_counts(func);

        let method_info = Self::build_method_info(code);

        // Pre-populate declared_vars so we don't hoist them
        let mut declared_vars = HashSet::new();
        if let Some(Type::Fun(fun_type) | Type::Method(fun_type)) = code.types.get(func.t.0) {
            let num_args = fun_type.args.len();
            for i in 0..num_args {
                if let Some(param_name) = func.arg_name(code, i) {
                    declared_vars.insert(param_name.into());
                }
            }
        }

        // used for try/catch structuring
        let exception_analysis = ExceptionAnalysis::analyze(func);

        // Determine if reg0 is `this`:
        // - Explicitly passed for this-bound closures (InstanceClosure)
        // - Or detected as instance method (constructor or first arg matches parent type)
        let is_this_bound_closure = is_this_bound_closure || {
            if let Some(Type::Fun(fun_type) | Type::Method(fun_type)) = code.types.get(func.t.0) {
                let func_name = code.strings.get(func.name.0)
                    .map(|s| s.as_ref())
                    .unwrap_or("");
                let is_constructor = func_name.starts_with("__constructor__");
                let first_arg_is_self = if let Some(parent_type) = func.parent {
                    !fun_type.args.is_empty() && fun_type.args[0] == parent_type
                } else {
                    false
                };
                is_constructor || first_arg_is_self
            } else {
                false
            }
        };

        Structurer {
            code,
            func,
            cfg,
            analysis,
            ssa,
            _type_info: type_info,
            closure_analysis,
            is_this_bound_closure,
            processed: HashSet::new(),
            use_info,
            exception_analysis,
            declared_vars,
            current_op: 0,
            method_info,
            scope_depth: 0,
            hoisted_vars: HashSet::new(),
            needs_dynamic_type: HashSet::new(),
            hoisted_var_types: HashMap::new(),
            array_bytes_source: HashMap::new(),
            shifted_indices: HashMap::new(),
            enum_global_map: Self::build_enum_global_map(code),
            string_conversion_source: HashMap::new(),
            ref_targets: HashMap::new(),
            use_raw_name_regs: HashSet::new(),
            iterator_regs: HashSet::new(),
            current_ssa_dst: None,
            current_ssa_uses: Vec::new(),
            string_switches: Self::detect_string_switches(code, func),
            current_loop_header: None,
            inline_exprs: RefCell::new(HashMap::new()),
            suppressed_ops: HashSet::new(),
            enum_param_bindings: HashMap::new(),
        }
    }

    /// This is used to convert f(obj, args) to obj.f(args) syntax.
    fn build_method_info(code: &Bytecode) -> HashMap<RefFun, (RefType, Str)> {
        let mut info = HashMap::new();

        for (type_idx, ty) in code.types.iter().enumerate() {
            if let Type::Obj(obj) = ty {
                for proto in &obj.protos {
                    let method_name: Str = code.strings.get(proto.name.0)
                        .cloned()
                        .unwrap_or_else(|| format!("method_{}", proto.findex.0).into());
                    info.insert(proto.findex, (RefType(type_idx), method_name));
                }
            }
        }

        info
    }

    pub(crate) fn get_current_class_name(&self) -> Option<Str> {
        self.func.parent.map(|parent_ref| {
            self.code[parent_ref].get_type_obj()
                .map(|obj| obj.name(self.code))
                .unwrap_or_else(|| Str::from(""))
        })
    }

    /// Structure the entire function into statements
    pub fn structure(&mut self) -> Vec<Statement> {

        // preprosess: detect patterns for suppression
        self.detect_enum_switch_patterns();
        self.detect_internal_function_calls();

        let stmts = if self.exception_analysis.has_exceptions() {
            // use opcode-range-based structuring which handles nested Trap/EndTrap
            // correctly without CFG edge interference.
            self.structure_block_range(0, self.func.ops.len())
        } else {
            // use CFG-based structuring.
            self.structure_from(self.cfg.entry, None)
        };

        // Prepend hoisted variable declarations (for vars first assigned inside scopes)
        let mut result = Vec::new();
        let current_class = self.get_current_class_name();
        let current_class_str = current_class.as_ref().map(|s| s.as_ref());
        for name in &self.hoisted_vars {
            let type_hint = if self.needs_dynamic_type.contains(name) {
                // for vars that will hold empty anonymous objects
                Some("Dynamic".into())
            } else if let Some(type_ref) = self.hoisted_var_types.get(name) {
                let ty = &self.code.types[type_ref.0];
                // Use context-aware type formatting to simplify nested types
                let type_str = crate::fmt::to_haxe_type_in_context(ty, self.code, current_class_str);
                // Don't emit Void type hints - use Dynamic instead
                // (Void variables are not valid in Haxe)
                // Also use Dynamic for haxe.Exception since catch blocks
                // can catch any type, not just Exception
                if type_str == "Void" || type_str == "haxe.Exception" {
                    Some("Dynamic".into())
                } else {
                    Some(type_str)
                }
            } else {
                None // no type available
            };
            result.push(Statement::VarDecl { name: name.clone(), type_hint });
        }
        result.extend(stmts);

        simplify_statements(result)
    }

    /// Structure code starting from a given block
    fn structure_from(&mut self, start: NodeIndex, stop_at: Option<NodeIndex>) -> Vec<Statement> {
        if Some(start) == stop_at || self.processed.contains(&start) {
            return vec![];
        }

        self.structure_from_inner(start, stop_at)
    }

    /// Uses iterative processing for sequential blocks to avoid deep recursion
    fn structure_from_inner(&mut self, start: NodeIndex, stop_at: Option<NodeIndex>) -> Vec<Statement> {
        let mut all_stmts = Vec::new();
        let mut current = Some(start);

        while let Some(block) = current {
            // Check stop conditions
            if Some(block) == stop_at || self.processed.contains(&block) {
                break;
            }

            let block_start_op = self.cfg.graph[block].start;
            let block_end_op = self.cfg.graph[block].end;

            // Check if this block CONTAINS a string switch (may start mid-block after setup ops)
            if let Some(switch_region) = self.string_switches.iter()
                .find(|s| s.start_op >= block_start_op && s.start_op <= block_end_op)
                .cloned()
            {
                // Structure any ops before the switch starts (e.g., NullCheck, Field ops)
                if switch_region.start_op > block_start_op {
                    for op_idx in block_start_op..switch_region.start_op {
                        self.current_op = op_idx;
                        let new_stmts = self.opcode_to_statements(op_idx);
                        all_stmts.extend(new_stmts);
                    }
                }
                all_stmts.extend(self.structure_string_switch(&switch_region, stop_at));
                break; // String switch handles its own continuation
            }

            // Check if this is a loop header
            if let Some(loop_info) = self.analysis.loops.iter().find(|l| l.header == block).cloned() {
                all_stmts.extend(self.structure_loop(&loop_info, stop_at));
                break; // Loop handles its own continuation
            }

            self.processed.insert(block);
            let stmts = self.structure_block(block);
            all_stmts.extend(stmts);

            // Get successors
            let succs: Vec<NodeIndex> = self.cfg.successors(block);

            match succs.len() {
                0 => {
                    // Terminal block - done
                    break;
                }
                1 => {
                    // Single successor - check for continue/break, then iterate
                    let is_in_loop = self.current_loop_header.is_some()
                        && Some(succs[0]) == self.current_loop_header;
                    if is_in_loop && Some(succs[0]) == stop_at {
                        let block_data = &self.cfg.graph[block];
                        let is_pure_jump = block_data.start == block_data.end
                            && matches!(self.func.ops.get(block_data.start), Some(Opcode::JAlways { .. } | Opcode::Label));
                        if is_pure_jump || block_data.end == block_data.start {
                            all_stmts.push(Statement::Continue);
                            break;
                        }
                    }

                    if let Some(header) = self.current_loop_header {
                        if let Some(loop_info) = self.analysis.loops.iter().find(|l| l.header == header) {
                            if !loop_info.body.contains(&succs[0]) {
                                let block_data = &self.cfg.graph[block];
                                let is_pure_jump = block_data.start == block_data.end
                                    && matches!(self.func.ops.get(block_data.start), Some(Opcode::JAlways { .. } | Opcode::Label));
                                if is_pure_jump || block_data.end == block_data.start {
                                    all_stmts.push(Statement::Break);
                                    break;
                                }
                            }
                        }
                    }

                    // Continue to next block iteratively
                    current = Some(succs[0]);
                }
                2 => {
                    // Conditional - structure it and get the continuation point
                    let (cond_stmts, continuation) = self.structure_conditional(block, &succs, stop_at);
                    all_stmts.extend(cond_stmts);
                    current = continuation;
                }
                _ => {
                    // Switch statement - structure it and get the continuation point
                    let (switch_stmts, continuation) = self.structure_switch_with_continuation(block, &succs, stop_at);
                    all_stmts.extend(switch_stmts);
                    current = continuation;
                }
            }
        }

        all_stmts
    }

    /// Structure a detected string switch pattern
    fn structure_string_switch(
        &mut self,
        region: &StringSwitchRegion,
        _stop_at: Option<NodeIndex>,
    ) -> Vec<Statement> {
        // Mark all blocks that contain the switch pattern as processed
        // But NOT the handler blocks (which might share a block with pattern ops)
        let handler_ops: std::collections::HashSet<usize> = region.cases.iter()
            .map(|c| c.handler_op)
            .chain(std::iter::once(region.default_op))
            .collect();

        for op_idx in region.start_op..=region.end_op {
            if let Some(&block) = self.cfg.op_to_block.get(&op_idx) {
                let block_start = self.cfg.graph[block].start;
                // Only mark as processed if this block doesn't start a handler
                if !handler_ops.contains(&block_start) {
                    self.processed.insert(block);
                }
            }
        }

        // Build the switch argument expression (the string being compared)
        let switch_arg = self.reg_to_expr(region.switch_arg_reg);

        // Build cases
        let mut cases = Vec::new();
        for case in &region.cases {
            let case_value = Expr::Constant(Constant::String(case.string_ref));

            // Structure the case handler
            // Find the block that contains the handler opcode
            let handler_body = if let Some(&handler_block) = self.cfg.op_to_block.get(&case.handler_op) {
                if !self.processed.contains(&handler_block) {
                    // Note: structure_from will mark the block as processed
                    self.structure_from(handler_block, None)
                } else {
                    vec![]
                }
            } else {
                // Fallback: structure the handler as a block range
                self.structure_block_range(case.handler_op, self.func.ops.len())
            };

            cases.push((vec![case_value], handler_body));
        }

        // Structure the default case
        let default_body = if let Some(&default_block) = self.cfg.op_to_block.get(&region.default_op) {
            if !self.processed.contains(&default_block) {
                // Note: structure_from will mark the block as processed
                self.structure_from(default_block, None)
            } else {
                vec![]
            }
        } else {
            self.structure_block_range(region.default_op, self.func.ops.len())
        };

        vec![Statement::Switch {
            arg: switch_arg,
            default: default_body,
            cases,
            enum_type: None,
        }]
    }

    /// Structure a loop
    fn structure_loop(&mut self, loop_info: &NaturalLoop, stop_at: Option<NodeIndex>) -> Vec<Statement> {
        let header = loop_info.header;
        self.processed.insert(header);

        // IMPORTANT: Process header block statements FIRST before extracting condition.
        // This populates inline_exprs so that the loop condition can use inlined expressions.
        // Otherwise, variables marked for inlining won't have their declarations emitted,
        // but the condition extraction won't find them in inline_exprs.
        let header_block = &self.cfg.graph[header];
        self.scope_depth += 1;
        let header_stmts = self.structure_block_range(header_block.start, header_block.end);
        self.scope_depth -= 1;

        // Extract condition and body start (now inline_exprs is populated)
        let (condition, body_start, exit_target) = self.extract_loop_condition(loop_info);

        // Structure body (increment scope depth to avoid declaring vars inside loop)
        let body = if let Some(body_node) = body_start {
            // Temporarily allow processing body nodes
            let old_processed = self.processed.clone();
            for &node in &loop_info.body {
                if node != header {
                    self.processed.remove(&node);
                }
            }
            // NOTE: We no longer mark exit targets as processed here.
            // Instead, we rely on break detection in structure_from to handle
            // blocks whose successor is outside the loop.
            // This allows break blocks to be properly structured and emit Break statements.
            self.scope_depth += 1;
            let old_loop_header = self.current_loop_header;
            self.current_loop_header = Some(header);
            let body_stmts = self.structure_from(body_node, Some(header));
            self.current_loop_header = old_loop_header;
            self.scope_depth -= 1;
            self.processed = old_processed;
            for &node in &loop_info.body {
                self.processed.insert(node);
            }
            body_stmts
        } else {
            vec![]
        };

        // Build the loop structure
        let loop_stmt = if header_stmts.is_empty() {
            // No header statements - simple while(condition)
            Statement::While {
                cond: condition,
                stmts: body,
            }
        } else {
            // Header has statements that compute the condition
            // Use while(true) { header_stmts; if (!cond) break; body; }
            // The condition is the "continue condition" so we break when it's false
            let break_cond = Expr::Op(Operation::Not(Box::new(condition)));
            let break_stmt = Statement::IfElse {
                cond: break_cond,
                if_: vec![Statement::Break],
                else_: vec![],
            };
            let mut loop_body = header_stmts;
            loop_body.push(break_stmt);
            loop_body.extend(body);
            Statement::While {
                cond: Expr::Constant(Constant::Bool(true)),
                stmts: loop_body,
            }
        };

        let mut result = vec![loop_stmt];

        // Continue after loop
        if let Some(exit) = exit_target {
            if Some(exit) != stop_at && !self.processed.contains(&exit) {
                result.extend(self.structure_from(exit, stop_at));
            }
        }

        result
    }

    /// Structure a range of opcodes into statements (for header blocks)
    pub(crate) fn structure_block_range(&mut self, start: usize, end: usize) -> Vec<Statement> {
        let mut stmts = Vec::new();
        let mut op_idx = start;

        while op_idx < end {
            self.current_op = op_idx;

            // Check if this opcode starts an exception region
            // Clone the region to avoid borrow checker issues
            if let Some(region) = self.exception_analysis.region_starting_at(op_idx).cloned() {
                // Compute catch_end: find next sequential region or use end
                let catch_end = self.find_catch_end(region.handler_op, end);

                // Structure the try/catch and skip past the entire region
                let try_catch = self.structure_exception_region(&region, catch_end);
                stmts.push(try_catch);
                // Skip past the catch body to continue after the try/catch
                op_idx = catch_end;
                continue;
            }

            // Check if this is a Switch opcode - needs special handling
            if let Opcode::Switch { .. } = &self.func.ops[op_idx] {
                if let Some(block) = self.cfg.op_to_block.get(&op_idx).copied() {
                    // Use CFG-based switch structuring
                    let succs = self.cfg.successors(block);
                    let switch_stmts = self.structure_switch(block, &succs, None);
                    stmts.extend(switch_stmts);
                    // Skip to merge point or continue after switch
                    if let Some(merge) = self.find_switch_merge_point(block) {
                        let merge_block = &self.cfg.graph[merge];
                        op_idx = merge_block.start;
                    } else {
                        // No merge point found, just continue past the switch opcode
                        op_idx += 1;
                    }
                    continue;
                } else {
                    // No CFG block for this switch - emit a comment and continue
                    stmts.push(Statement::Comment(format!(
                        "switch at {} (no CFG block)",
                        op_idx
                    )));
                    op_idx += 1;
                    continue;
                }
            }

            let new_stmts = self.opcode_to_statements(op_idx);
            stmts.extend(new_stmts);
            op_idx += 1;
        }
        stmts
    }

    /// Find where a catch body ends
    /// Returns the first opcode after the catch body
    fn find_catch_end(&self, handler_start: usize, outer_end: usize) -> usize {
        // Look for the next top-level try region that starts after handler_start
        // That would be the start of the next sequential try/catch
        let mut catch_end = outer_end;

        for region in self.exception_analysis.top_level_regions() {
            if region.trap_op > handler_start && region.trap_op < catch_end {
                catch_end = region.trap_op;
            }
        }

        catch_end
    }

    /// Structure a try/catch region using exception analysis
    fn structure_exception_region(&mut self, region: &TryRegion, outer_end: usize) -> Statement {
        // Get the exception variable name from the Trap opcode
        let catch_var = self.reg_name(region.exc_reg).to_string();

        // Try body: from trap+1 to end_trap (excluding EndTrap itself which is control flow)
        // The EndTrap marks the end of the try body's normal exit path
        self.scope_depth += 1;
        let try_stmts = self.structure_opcode_range(
            region.trap_op + 1,
            region.end_trap_op, // End before EndTrap
            &region.nested,
        );
        self.scope_depth -= 1;

        // Catch body: from handler to computed end
        // Catch body ends at: outer_end (for top-level), or function end
        // For nested regions, catch body is inside outer region's try body
        let catch_end = outer_end;

        // Mark the exception register to use raw names in the catch body
        // This ensures uses of the caught exception match the catch parameter name
        // (prevents SSA versioning from creating mismatched names like `catch (r0)` vs `r0_1`)
        self.use_raw_name_regs.insert(region.exc_reg);

        self.scope_depth += 1;
        let catch_stmts = self.structure_opcode_range(
            region.handler_op,
            catch_end,
            &[], // Nested regions in catch body would need separate tracking
        );
        self.scope_depth -= 1;

        Statement::TryCatch {
            try_stmts,
            catch_var,
            catch_stmts,
        }
    }

    /// Structure a range of opcodes, handling nested exception regions
    fn structure_opcode_range(
        &mut self,
        start: usize,
        end: usize,
        nested_regions: &[TryRegion],
    ) -> Vec<Statement> {
        let mut stmts = Vec::new();
        let mut op_idx = start;

        while op_idx < end {
            self.current_op = op_idx;

            // Check if this opcode starts a nested exception region
            if let Some(region) = nested_regions.iter().find(|r| r.trap_op == op_idx) {
                let try_catch = self.structure_exception_region(region, end);
                stmts.push(try_catch);
                // Skip past the entire nested try/catch including its catch body
                // The catch body was processed from handler_op to 'end', so skip there
                op_idx = end;
                continue;
            }

            // Check if this is a Switch opcode - needs special handling
            if let Opcode::Switch { .. } = &self.func.ops[op_idx] {
                if let Some(block) = self.cfg.op_to_block.get(&op_idx).copied() {
                    // Use CFG-based switch structuring
                    let succs = self.cfg.successors(block);
                    let switch_stmts = self.structure_switch(block, &succs, None);
                    stmts.extend(switch_stmts);
                    // Skip to merge point or continue after switch
                    if let Some(merge) = self.find_switch_merge_point(block) {
                        let merge_block = &self.cfg.graph[merge];
                        op_idx = merge_block.start;
                    } else {
                        op_idx += 1;
                    }
                    continue;
                } else {
                    stmts.push(Statement::Comment(format!(
                        "switch at {} (no CFG block)",
                        op_idx
                    )));
                    op_idx += 1;
                    continue;
                }
            }

            let new_stmts = self.opcode_to_statements(op_idx);
            stmts.extend(new_stmts);
            op_idx += 1;
        }
        stmts
    }

    /// Extract loop condition
    fn extract_loop_condition(
        &self,
        loop_info: &NaturalLoop,
    ) -> (Expr, Option<NodeIndex>, Option<NodeIndex>) {
        let header = loop_info.header;
        let block = &self.cfg.graph[header];
        let last_op = &self.func.ops[block.end];

        match last_op {
            Opcode::JNull { reg, offset } => {
                let target = self.compute_target(block.end, *offset);
                let cond_var = self.reg_to_expr_in_block(*reg, header);
                let null_expr = Expr::Constant(Constant::Null);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop, so continue while != null
                    (
                        Expr::Op(Operation::NotEq(Box::new(cond_var), Box::new(null_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        Expr::Op(Operation::Eq(Box::new(cond_var), Box::new(null_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JSLt { a, b, offset } => {
                let target = self.compute_target(block.end, *offset);
                let a_expr = self.reg_to_expr_in_block(*a, header);
                let b_expr = self.reg_to_expr_in_block(*b, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    (
                        Expr::Op(Operation::Gte(Box::new(a_expr), Box::new(b_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        Expr::Op(Operation::Lt(Box::new(a_expr), Box::new(b_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JSGte { a, b, offset } => {
                let target = self.compute_target(block.end, *offset);
                let a_expr = self.reg_to_expr_in_block(*a, header);
                let b_expr = self.reg_to_expr_in_block(*b, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when a >= b, so continue while a < b
                    (
                        Expr::Op(Operation::Lt(Box::new(a_expr), Box::new(b_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        Expr::Op(Operation::Gte(Box::new(a_expr), Box::new(b_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JNotNull { reg, offset } => {
                let target = self.compute_target(block.end, *offset);
                let cond_var = self.reg_to_expr_in_block(*reg, header);
                let null_expr = Expr::Constant(Constant::Null);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when not null, so continue while null
                    (
                        Expr::Op(Operation::Eq(Box::new(cond_var), Box::new(null_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        Expr::Op(Operation::NotEq(Box::new(cond_var), Box::new(null_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JTrue { cond, offset } => {
                let target = self.compute_target(block.end, *offset);
                let cond_expr = self.reg_to_expr_in_block(*cond, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when true, so continue while not true
                    (
                        Expr::Op(Operation::Not(Box::new(cond_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        cond_expr,
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JFalse { cond, offset } => {
                let target = self.compute_target(block.end, *offset);
                let cond_expr = self.reg_to_expr_in_block(*cond, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when false, so continue while true
                    (
                        cond_expr,
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        Expr::Op(Operation::Not(Box::new(cond_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JNotLt { a, b, offset } => {
                // JNotLt: jump if NOT (a < b), i.e., jump if a >= b
                let target = self.compute_target(block.end, *offset);
                let a_expr = self.reg_to_expr_in_block(*a, header);
                let b_expr = self.reg_to_expr_in_block(*b, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when a >= b, so continue while a < b
                    (
                        Expr::Op(Operation::Lt(Box::new(a_expr), Box::new(b_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    // Jump stays in loop when a >= b
                    (
                        Expr::Op(Operation::Gte(Box::new(a_expr), Box::new(b_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JNotGte { a, b, offset } => {
                // JNotGte: jump if NOT (a >= b), i.e., jump if a < b
                let target = self.compute_target(block.end, *offset);
                let a_expr = self.reg_to_expr_in_block(*a, header);
                let b_expr = self.reg_to_expr_in_block(*b, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when a < b, so continue while a >= b
                    (
                        Expr::Op(Operation::Gte(Box::new(a_expr), Box::new(b_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    // Jump stays in loop when a < b
                    (
                        Expr::Op(Operation::Lt(Box::new(a_expr), Box::new(b_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            _ => {
                // Default: while(true)
                let exit = loop_info.exit_nodes.first().and_then(|&n| {
                    self.cfg.successors(n).into_iter().find(|s| !loop_info.body.contains(s))
                });
                (
                    Expr::Constant(Constant::Bool(true)),
                    self.cfg.block_for_op(block.end + 1),
                    exit,
                )
            }
        }
    }

    /// Check if a block is a conditional that can be collected into a chain.
    /// We allow blocks with preambles - they'll be handled when building the result.
    fn is_chain_candidate(&self, block: NodeIndex) -> bool {
        let succs = self.cfg.successors(block);
        if succs.len() != 2 {
            return false;
        }
        // Avoid loop headers - they need special handling
        if self.analysis.loops.iter().any(|l| l.header == block) {
            return false;
        }
        !self.processed.contains(&block)
    }

    /// Structure a branch target, returning statements
    fn structure_branch(&mut self, target: Option<NodeIndex>, stop_at: Option<NodeIndex>) -> Vec<Statement> {
        if let Some(t) = target {
            if Some(t) != stop_at && !self.processed.contains(&t) {
                self.structure_from(t, stop_at)
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    }

    /// Structure a conditional and return (statements, continuation_point)
    /// Caller handles the continuation iteratively to avoid deep recursion
    fn structure_conditional(
        &mut self,
        block: NodeIndex,
        _succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
    ) -> (Vec<Statement>, Option<NodeIndex>) {
        // Collect the if-else-if chain iteratively
        let mut chain: Vec<(Vec<Statement>, Expr, Vec<Statement>)> = vec![];
        let mut current_block = Some(block);
        let mut final_merge: Option<NodeIndex> = None;
        let mut is_first = true;
        let mut has_preambles = false;

        self.scope_depth += 1;

        while let Some(blk) = current_block {
            if !is_first && self.processed.contains(&blk) {
                break;
            }

            let preamble = if is_first {
                is_first = false;
                vec![]
            } else {
                self.processed.insert(blk);
                let p = self.structure_block(blk);
                if !p.is_empty() {
                    has_preambles = true;
                }
                p
            };

            let block_data = &self.cfg.graph[blk];
            let last_op = &self.func.ops[block_data.end];

            self.current_op = block_data.end;
            if let Some((ssa_dst, ssa_uses)) = self.ssa.get_instr_for_op(block_data.end) {
                self.current_ssa_dst = ssa_dst;
                self.current_ssa_uses = ssa_uses.clone();
            } else {
                self.current_ssa_dst = None;
                self.current_ssa_uses.clear();
            }

            let (condition, then_target, else_target) = self.extract_condition(block_data.end, last_op);

            let merge = self.find_merge_point(then_target, else_target);
            if merge.is_some() {
                final_merge = merge;
            }

            self.processed.insert(blk);

            let is_not_eq = matches!(&condition, Expr::Op(Operation::NotEq(_, _)));

            if is_not_eq {
                let eq_condition = if let Expr::Op(Operation::NotEq(a, b)) = &condition {
                    Expr::Op(Operation::Eq(a.clone(), b.clone()))
                } else {
                    condition.clone()
                };

                let chain_merge = if let Some(e) = else_target {
                    let case_succs = self.cfg.successors(e);
                    if case_succs.len() == 1 { Some(case_succs[0]) } else { merge }
                } else {
                    merge
                };

                if chain_merge.is_some() {
                    final_merge = chain_merge;
                }

                let effective_stop = chain_merge.or(stop_at);
                let case_stmts = self.structure_branch(else_target, effective_stop);
                chain.push((preamble, eq_condition, case_stmts));

                if let Some(t) = then_target {
                    if Some(t) != chain_merge && self.is_chain_candidate(t) {
                        current_block = Some(t);
                        continue;
                    }
                }

                let else_stmts = self.structure_branch(then_target, effective_stop);
                let stmts = self.build_conditional_result(chain, else_stmts, has_preambles);
                return (stmts, self.get_unprocessed_continuation(final_merge, stop_at));
            }

            let effective_stop = merge.or(stop_at);
            let then_stmts = self.structure_branch(then_target, effective_stop);
            chain.push((preamble, condition, then_stmts));

            if let Some(e) = else_target {
                if Some(e) != merge && self.is_chain_candidate(e) {
                    current_block = Some(e);
                    continue;
                }
            }

            let else_stmts = self.structure_branch(else_target, effective_stop);
            let stmts = self.build_conditional_result(chain, else_stmts, has_preambles);
            return (stmts, self.get_unprocessed_continuation(final_merge, stop_at));
        }

        if chain.is_empty() {
            self.scope_depth -= 1;
            return (vec![], None);
        }

        // Note: build_conditional_result decrements scope_depth
        let stmts = self.build_conditional_result(chain, vec![], has_preambles);
        (stmts, self.get_unprocessed_continuation(final_merge, stop_at))
    }

    /// Build conditional result without processing continuation
    /// Includes optimizations: switch conversion, ternary condensation
    fn build_conditional_result(
        &mut self,
        chain: Vec<(Vec<Statement>, Expr, Vec<Statement>)>,
        else_stmts: Vec<Statement>,
        has_preambles: bool,
    ) -> Vec<Statement> {
        self.scope_depth -= 1;

        if has_preambles {
            self.build_nested_if_else(chain, else_stmts)
        } else {
            let flat_chain: Vec<(Expr, Vec<Statement>)> = chain.into_iter()
                .map(|(_, cond, body)| (cond, body))
                .collect();

            // Try to convert if-else-if chain to switch statement
            if let Some((switch_arg, cases)) = self.analyze_for_switch(&flat_chain) {
                vec![Statement::Switch {
                    arg: switch_arg,
                    default: else_stmts,
                    cases,
                    enum_type: None,
                }]
            } else if flat_chain.len() > 1 {
                vec![Statement::IfElseChain {
                    branches: flat_chain,
                    else_: else_stmts,
                }]
            } else if flat_chain.len() == 1 {
                let (cond, then_stmts) = flat_chain.into_iter().next().unwrap();
                if then_stmts.is_empty() && else_stmts.is_empty() {
                    vec![]
                }
                // Try ternary condensation: both branches return directly
                else if let Some(ternary) = self.try_condense_ternary_return(&cond, &then_stmts, &else_stmts) {
                    vec![ternary]
                }
                // Try ternary assignment: both branches assign to same variable
                else if let Some(ternary) = self.try_condense_ternary_assign(&cond, &then_stmts, &else_stmts) {
                    vec![ternary]
                } else {
                    vec![Statement::IfElse {
                        cond,
                        if_: then_stmts,
                        else_: else_stmts,
                    }]
                }
            } else {
                else_stmts
            }
        }
    }

    /// Get the continuation point if it's unprocessed and not at stop_at
    fn get_unprocessed_continuation(&self, merge: Option<NodeIndex>, stop_at: Option<NodeIndex>) -> Option<NodeIndex> {
        if let Some(m) = merge {
            if Some(m) != stop_at && !self.processed.contains(&m) {
                return Some(m);
            }
        }
        None
    }

    /// Build nested IfElse structure iteratively from a chain with preambles
    /// This avoids recursion by building from last to first
    fn build_nested_if_else(
        &self,
        chain: Vec<(Vec<Statement>, Expr, Vec<Statement>)>,
        final_else: Vec<Statement>,
    ) -> Vec<Statement> {
        // Build from last to first: each iteration wraps the previous result as the else branch
        let mut current_else = final_else;

        for (preamble, cond, then_body) in chain.into_iter().rev() {
            let if_else = Statement::IfElse {
                cond,
                if_: then_body,
                else_: current_else,
            };
            // Preamble goes before the if-else
            let mut block = preamble;
            block.push(if_else);
            current_else = block;
        }

        current_else
    }

    /// Extract condition from a conditional jump
    fn extract_condition(
        &self,
        op_idx: usize,
        op: &Opcode,
    ) -> (Expr, Option<NodeIndex>, Option<NodeIndex>) {
        let target = |offset: i32| self.compute_target(op_idx, offset);
        let fall = self.cfg.block_for_op(op_idx + 1);

        match op {
            Opcode::JNull { reg, offset } => {
                let var = self.reg_to_expr(*reg);
                let null = Expr::Constant(Constant::Null);
                (Expr::Op(Operation::Eq(Box::new(var), Box::new(null))), target(*offset), fall)
            }
            Opcode::JNotNull { reg, offset } => {
                let var = self.reg_to_expr(*reg);
                let null = Expr::Constant(Constant::Null);
                (Expr::Op(Operation::NotEq(Box::new(var), Box::new(null))), target(*offset), fall)
            }
            Opcode::JTrue { cond, offset } => {
                (self.reg_to_expr(*cond), target(*offset), fall)
            }
            Opcode::JFalse { cond, offset } => {
                let var = self.reg_to_expr(*cond);
                (Expr::Op(Operation::Not(Box::new(var))), target(*offset), fall)
            }
            Opcode::JSLt { a, b, offset } => {
                let cond = Expr::Op(Operation::Lt(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JSGte { a, b, offset } => {
                let cond = Expr::Op(Operation::Gte(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JEq { a, b, offset } => {
                let cond = Expr::Op(Operation::Eq(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JNotEq { a, b, offset } => {
                let cond = Expr::Op(Operation::NotEq(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            // Unsigned comparisons (used for array bounds checking)
            Opcode::JULt { a, b, offset } => {
                let cond = Expr::Op(Operation::Lt(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JUGte { a, b, offset } => {
                let cond = Expr::Op(Operation::Gte(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JSLte { a, b, offset } => {
                let cond = Expr::Op(Operation::Lte(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JNotLt { a, b, offset } => {
                // not(a < b) is equivalent to a >= b
                let cond = Expr::Op(Operation::Gte(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JNotGte { a, b, offset } => {
                // not(a >= b) is equivalent to a < b
                let cond = Expr::Op(Operation::Lt(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            _ => (Expr::Constant(Constant::Bool(true)), fall, None),
        }
    }

    /// Check if a block terminates (ends with Ret, Throw, or Rethrow)
    fn block_terminates(&self, block: NodeIndex) -> bool {
        let block_data = &self.cfg.graph[block];
        matches!(
            &self.func.ops[block_data.end],
            Opcode::Ret { .. } | Opcode::Throw { .. } | Opcode::Rethrow { .. }
        )
    }

    /// Check if a branch terminates (the target block ends in return/throw)
    fn branch_terminates(&self, target: Option<NodeIndex>) -> bool {
        match target {
            Some(block) => {
                // Check if the block itself terminates
                self.block_terminates(block) || self.cfg.successors(block).is_empty()
            }
            None => true, // No target = terminates
        }
    }

    /// Find merge point of two branches.
    /// If one branch terminates (return/throw) and the other branch doesn't eventually
    /// reach the same terminator, the non-terminating branch IS the merge point.
    fn find_merge_point(&self, a: Option<NodeIndex>, b: Option<NodeIndex>) -> Option<NodeIndex> {
        let a = a?;
        let b = b?;

        // Standard merge point detection: find common successor
        let a_succs: HashSet<_> = self.cfg.successors(a).into_iter().collect();
        let b_succs: HashSet<_> = self.cfg.successors(b).into_iter().collect();

        for s in &a_succs {
            if b_succs.contains(s) {
                return Some(*s);
            }
        }
        if a_succs.contains(&b) { return Some(b); }
        if b_succs.contains(&a) { return Some(a); }

        // If one branch terminates AND the other doesn't reach it,
        // the non-terminating branch is where control "continues" after the if.
        // But only apply this if the other branch doesn't eventually flow into
        // the terminating branch (which would mean they share a merge point).
        let a_terminates = self.branch_terminates(Some(a));
        let b_terminates = self.branch_terminates(Some(b));

        // If BOTH branches terminate, there is no merge point
        if a_terminates && b_terminates {
            return None;
        }

        if a_terminates && !b_succs.contains(&a) {
            // Branch 'a' terminates, branch 'b' doesn't flow into 'a'
            // If b is a simple block with one successor, that successor is the true
            // continuation point. Otherwise, b itself is the continuation (for if-chains
            // where b is another conditional block).
            if b_succs.len() == 1 {
                return b_succs.into_iter().next();
            }
            return Some(b);
        }
        if b_terminates && !a_succs.contains(&b) {
            // Branch 'b' terminates, branch 'a' doesn't flow into 'b'
            if a_succs.len() == 1 {
                return a_succs.into_iter().next();
            }
            return Some(a);
        }

        None
    }

    /// Analyze an if-else-if chain to see if it can be converted to a switch statement.
    /// Returns Some((switch_arg, cases)) if the chain matches the switch pattern.
    ///
    /// Switch pattern criteria:
    /// - All conditions are equality tests: Eq(X, C) or Eq(C, X)
    /// - X is the same expression in all conditions (the switch argument)
    /// - C is a constant (Int, String, Bool, or enum constructor)
    fn analyze_for_switch(
        &self,
        chain: &[(Expr, Vec<Statement>)],
    ) -> Option<(Expr, Vec<(Vec<Expr>, Vec<Statement>)>)> {
        // Need at least 3 conditions to make a switch worthwhile
        if chain.len() < 3 {
            return None;
        }

        let mut switch_arg: Option<Expr> = None;
        let mut cases = Vec::new();

        for (cond, body) in chain {
            let (arg, case_val) = self.extract_equality_pattern(cond)?;

            if let Some(ref existing) = switch_arg {
                // Check if the argument matches the existing switch argument
                if !self.exprs_structurally_equal(existing, &arg) {
                    return None;
                }
            } else {
                switch_arg = Some(arg);
            }

            cases.push((vec![case_val], body.clone()));
        }

        Some((switch_arg?, cases))
    }

    /// Extract an equality pattern from a condition expression.
    /// Returns Some((arg, case_value)) where case_value is a constant.
    /// Note: NotEq patterns are converted to Eq during chain collection.
    fn extract_equality_pattern(&self, cond: &Expr) -> Option<(Expr, Expr)> {
        match cond {
            Expr::Op(Operation::Eq(a, b)) => {
                // First, check if one side is a simple constant (integer, string literal, etc.)
                if self.is_switch_case_constant(b) {
                    return Some((*a.clone(), *b.clone()));
                } else if self.is_switch_case_constant(a) {
                    return Some((*b.clone(), *a.clone()));
                }

                // If not a simple constant, try string comparison pattern:
                // Eq(Variable(result), Variable(zero))
                // where result = string_compare(bytes, string_literal, len) and zero = 0
                if let (Expr::Variable(reg_a, _), Expr::Variable(reg_b, _)) = (a.as_ref(), b.as_ref()) {
                    if let Some(result) = self.try_extract_string_compare_pattern(*reg_a, *reg_b) {
                        return Some(result);
                    }
                    if let Some(result) = self.try_extract_string_compare_pattern(*reg_b, *reg_a) {
                        return Some(result);
                    }
                }

                None
            }
            _ => None,
        }
    }

    /// Check if an expression is a valid switch case constant
    fn is_switch_case_constant(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Constant(c) => matches!(
                c,
                Constant::Int(_)
                | Constant::InlineInt(_)
                | Constant::String(_)
                | Constant::Bool(_)
            ),
            _ => false,
        }
    }

    /// Check if two expressions are structurally equal (for switch arg comparison)
    fn exprs_structurally_equal(&self, a: &Expr, b: &Expr) -> bool {
        match (a, b) {
            (Expr::Variable(ra, _), Expr::Variable(rb, _)) => ra == rb,
            (Expr::Constant(ca), Expr::Constant(cb)) => self.constants_equal(ca, cb),
            (Expr::Field(obj_a, field_a), Expr::Field(obj_b, field_b)) => {
                field_a == field_b && self.exprs_structurally_equal(obj_a, obj_b)
            }
            (Expr::Call(call_a), Expr::Call(call_b)) => {
                // For calls, compare function and arguments
                if !self.exprs_structurally_equal(&call_a.fun, &call_b.fun)
                    || call_a.args.len() != call_b.args.len()
                {
                    return false;
                }
                call_a.args.iter().zip(call_b.args.iter())
                    .all(|(a, b)| self.exprs_structurally_equal(a, b))
            }
            (Expr::Op(op_a), Expr::Op(op_b)) => {
                // Simple operation comparison - just check if they're the same variant
                // This is a conservative check
                std::mem::discriminant(op_a) == std::mem::discriminant(op_b)
            }
            _ => false,
        }
    }

    /// Check if two constants are equal
    fn constants_equal(&self, a: &Constant, b: &Constant) -> bool {
        match (a, b) {
            (Constant::Int(ia), Constant::Int(ib)) => ia == ib,
            (Constant::InlineInt(ia), Constant::InlineInt(ib)) => ia == ib,
            (Constant::Float(fa), Constant::Float(fb)) => fa == fb,
            (Constant::String(sa), Constant::String(sb)) => sa == sb,
            (Constant::Bool(ba), Constant::Bool(bb)) => ba == bb,
            (Constant::Null, Constant::Null) => true,
            (Constant::This, Constant::This) => true,
            _ => false,
        }
    }

    /// Try to condense an if/else with returns into a ternary return.
    /// Returns Some(statement) if the pattern matches, None otherwise.
    ///
    /// Supported patterns:
    /// - Pattern 1: if (cond) { return a; } else { return b; } → return cond ? a : b
    /// - Pattern 2: if (cond) { r = a; return r; } else { r = b; return r; } → return cond ? a : b
    /// - Pattern 3: if (cond) { r = a; return r; } else { return b; } → return cond ? a : b
    /// - Pattern 4: if (cond) { return a; } else { r = b; return r; } → return cond ? a : b
    fn try_condense_ternary_return(
        &self,
        cond: &Expr,
        then_stmts: &[Statement],
        else_stmts: &[Statement],
    ) -> Option<Statement> {
        // Pattern 1: Simple returns in both branches
        // if (cond) { return a; } else { return b; }
        if then_stmts.len() == 1 && else_stmts.len() == 1 {
            if let (Statement::Return(Some(if_expr)), Statement::Return(Some(else_expr))) =
                (&then_stmts[0], &else_stmts[0])
            {
                return Some(Statement::Return(Some(Expr::IfElse {
                    cond: Box::new(cond.clone()),
                    if_: vec![Statement::ExprStatement(if_expr.clone())],
                    else_: vec![Statement::ExprStatement(else_expr.clone())],
                })));
            }
        }

        // Pattern 2: Assignment + return in both branches
        // if (cond) { r = a; return r; } else { r = b; return r; }
        if then_stmts.len() == 2 && else_stmts.len() == 2 {
            if let (
                Statement::Assign { variable: if_var, assign: if_assign, .. },
                Statement::Return(Some(if_ret)),
            ) = (&then_stmts[0], &then_stmts[1])
            {
                if let (
                    Statement::Assign { variable: else_var, assign: else_assign, .. },
                    Statement::Return(Some(else_ret)),
                ) = (&else_stmts[0], &else_stmts[1])
                {
                    // Check that both assign to the same register and return that register
                    if let (Expr::Variable(if_reg, _), Expr::Variable(else_reg, _)) =
                        (if_var, else_var)
                    {
                        if if_reg == else_reg {
                            if let (
                                Expr::Variable(if_ret_reg, _),
                                Expr::Variable(else_ret_reg, _),
                            ) = (if_ret, else_ret)
                            {
                                if if_ret_reg == if_reg && else_ret_reg == else_reg {
                                    return Some(Statement::Return(Some(Expr::IfElse {
                                        cond: Box::new(cond.clone()),
                                        if_: vec![Statement::ExprStatement(if_assign.clone())],
                                        else_: vec![Statement::ExprStatement(else_assign.clone())],
                                    })));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Pattern 3: Asymmetric - if has assign+return, else has direct return
        // if (cond) { r = a; return r; } else { return b; }
        if then_stmts.len() == 2 && else_stmts.len() == 1 {
            if let (
                Statement::Assign { variable: if_var, assign: if_assign, .. },
                Statement::Return(Some(if_ret)),
            ) = (&then_stmts[0], &then_stmts[1])
            {
                if let Statement::Return(Some(else_expr)) = &else_stmts[0] {
                    if let (Expr::Variable(if_reg, _), Expr::Variable(if_ret_reg, _)) =
                        (if_var, if_ret)
                    {
                        if if_reg == if_ret_reg {
                            return Some(Statement::Return(Some(Expr::IfElse {
                                cond: Box::new(cond.clone()),
                                if_: vec![Statement::ExprStatement(if_assign.clone())],
                                else_: vec![Statement::ExprStatement(else_expr.clone())],
                            })));
                        }
                    }
                }
            }
        }

        // Pattern 4: Asymmetric (reversed) - if has direct return, else has assign+return
        // if (cond) { return a; } else { r = b; return r; }
        if then_stmts.len() == 1 && else_stmts.len() == 2 {
            if let Statement::Return(Some(if_expr)) = &then_stmts[0] {
                if let (
                    Statement::Assign { variable: else_var, assign: else_assign, .. },
                    Statement::Return(Some(else_ret)),
                ) = (&else_stmts[0], &else_stmts[1])
                {
                    if let (Expr::Variable(else_reg, _), Expr::Variable(else_ret_reg, _)) =
                        (else_var, else_ret)
                    {
                        if else_reg == else_ret_reg {
                            return Some(Statement::Return(Some(Expr::IfElse {
                                cond: Box::new(cond.clone()),
                                if_: vec![Statement::ExprStatement(if_expr.clone())],
                                else_: vec![Statement::ExprStatement(else_assign.clone())],
                            })));
                        }
                    }
                }
            }
        }

        None
    }

    /// Try to condense if/else assignment pattern into ternary assignment.
    /// Pattern: if (cond) { x = a; } else { x = b; } → x = cond ? a : b
    fn try_condense_ternary_assign(
        &self,
        cond: &Expr,
        then_stmts: &[Statement],
        else_stmts: &[Statement],
    ) -> Option<Statement> {
        // Both branches must have exactly one statement
        if then_stmts.len() != 1 || else_stmts.len() != 1 {
            return None;
        }

        // Both must be assignments
        let (if_decl, if_var, if_assign) = match &then_stmts[0] {
            Statement::Assign { declaration, variable, assign, .. } => {
                (declaration, variable, assign)
            }
            _ => return None,
        };

        let (else_decl, else_var, else_assign) = match &else_stmts[0] {
            Statement::Assign { declaration, variable, assign, .. } => {
                (declaration, variable, assign)
            }
            _ => return None,
        };

        // Both must assign to the same register
        let (if_reg, if_name) = match if_var {
            Expr::Variable(reg, name) => (reg, name),
            _ => return None,
        };

        let else_reg = match else_var {
            Expr::Variable(reg, _) => reg,
            _ => return None,
        };

        if if_reg != else_reg {
            return None;
        }

        // Create ternary assignment: x = cond ? a : b
        // Use declaration from first branch (the one that declares the variable)
        Some(Statement::Assign {
            declaration: *if_decl || *else_decl,
            variable: Expr::Variable(*if_reg, if_name.clone()),
            assign: Expr::IfElse {
                cond: Box::new(cond.clone()),
                if_: vec![Statement::ExprStatement(if_assign.clone())],
                else_: vec![Statement::ExprStatement(else_assign.clone())],
            },
        })
    }

    /// Structure a switch statement and return (statements, continuation_point)
    /// This version doesn't process the continuation - caller handles it iteratively
    fn structure_switch_with_continuation(
        &mut self,
        block: NodeIndex,
        succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
    ) -> (Vec<Statement>, Option<NodeIndex>) {
        // Use the core switch logic but capture the merge point
        let merge_point = self.find_switch_merge_point(block);

        // Structure the switch without continuation
        let stmts = self.structure_switch_core(block, succs, stop_at, false);

        // Return unprocessed continuation
        let continuation = if let Some(m) = merge_point {
            if Some(m) != stop_at && !self.processed.contains(&m) {
                Some(m)
            } else {
                None
            }
        } else {
            None
        };

        (stmts, continuation)
    }

    /// Structure a switch statement
    fn structure_switch(
        &mut self,
        block: NodeIndex,
        _succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
    ) -> Vec<Statement> {
        self.structure_switch_core(block, _succs, stop_at, true)
    }

    /// Core switch structuring logic
    fn structure_switch_core(
        &mut self,
        block: NodeIndex,
        _succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
        do_continuation: bool,
    ) -> Vec<Statement> {
        use hlbc::types::RefEnumConstruct;

        let block_data = &self.cfg.graph[block];
        let last_op = &self.func.ops[block_data.end];

        // Extract switch info
        let (switch_reg, offsets) = match last_op {
            Opcode::Switch { reg, offsets, .. } => (*reg, offsets.clone()),
            _ => {
                // Fallback: emit comment
                return vec![Statement::Comment("switch statement".to_string())];
            }
        };

        let switch_op_idx = block_data.end;
        // Set SSA context for the switch opcode so reg_to_expr uses SSA-versioned names
        self.current_op = switch_op_idx;
        if let Some((ssa_dst, ssa_uses)) = self.ssa.get_instr_for_op(switch_op_idx) {
            self.current_ssa_dst = ssa_dst;
            self.current_ssa_uses = ssa_uses.clone();
        } else {
            self.current_ssa_dst = None;
            self.current_ssa_uses.clear();
        }
        let switch_arg = self.reg_to_expr(switch_reg);

        // Check if switch_arg is Type.enumIndex(x) and unwrap to just x
        // Also extract the enum type for proper case pattern formatting
        // enum_value_reg is the register holding the actual enum value (for pattern binding)
        let (switch_arg, enum_type, enum_value_reg) = self.unwrap_enum_index_switch(switch_arg, switch_reg);

        // Group cases by their target opcode
        // offsets[i] = offset for case value i, target = switch_op_idx + 1 + offset
        let mut target_to_cases: HashMap<usize, Vec<usize>> = HashMap::new();
        for (case_val, &offset) in offsets.iter().enumerate() {
            let target_op = (switch_op_idx as i64 + 1 + offset as i64) as usize;
            target_to_cases
                .entry(target_op)
                .or_default()
                .push(case_val);
        }

        // Default case is fall-through (switch_op_idx + 1)
        let default_op = switch_op_idx + 1;

        // Find the merge point - look for where all branches converge
        // This is typically the op after the last JAlways in any case
        let merge_point = self.find_switch_merge_point(block);

        // Structure default case (fall-through) - increment scope depth for case bodies
        self.scope_depth += 1;
        let default_stmts = if let Some(default_block) = self.cfg.block_for_op(default_op) {
            if !target_to_cases.contains_key(&default_op) && !self.processed.contains(&default_block) {
                self.structure_from(default_block, merge_point)
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        // Structure each case target
        let mut cases: Vec<(Vec<Expr>, Vec<Statement>)> = Vec::new();
        let mut processed_targets: HashSet<usize> = HashSet::new();

        // Sort case values for deterministic output
        let mut sorted_targets: Vec<_> = target_to_cases.into_iter().collect();
        sorted_targets.sort_by_key(|(_, vals)| vals.iter().min().copied().unwrap_or(0));

        // Determine the merge_op for scanning case body ranges
        let merge_op = merge_point.and_then(|mp| {
            let mp_block = &self.cfg.graph[mp];
            Some(mp_block.start)
        }).unwrap_or(self.func.ops.len());

        // Collect sorted target ops for determining case body end points
        let all_target_ops: Vec<usize> = sorted_targets.iter().map(|(op, _)| *op).collect();

        for (idx, (target_op, case_vals)) in sorted_targets.iter().enumerate() {
            if processed_targets.contains(target_op) {
                continue;
            }
            processed_targets.insert(*target_op);

            // Determine the end of this case's body (next case target or merge)
            let case_end_op = all_target_ops.get(idx + 1).copied().unwrap_or(merge_op);

            // Convert case values to expressions
            // If we have an enum type, use constructor names instead of integers
            let case_exprs: Vec<Expr> = case_vals
                .iter()
                .map(|&v| {
                    if let Some(ref_type) = enum_type {
                        // Try to get enum constructor name for this index
                        if let Type::Enum { constructs, .. } = &self.code[ref_type] {
                            if let Some(construct) = constructs.get(v) {
                                // If constructor has parameters, check if we can bind them
                                if !construct.params.is_empty() {
                                    // Scan the case body for EnumField accesses on this construct
                                    let accessed_fields = if let Some(enum_reg) = enum_value_reg {
                                        self.scan_enum_field_accesses(enum_reg, *target_op, case_end_op)
                                    } else {
                                        HashSet::new()
                                    };

                                    // Generate bindings for accessed fields, wildcards for others
                                    let bindings: Vec<Expr> = construct.params.iter().enumerate()
                                        .map(|(i, _)| {
                                            if accessed_fields.contains(&(v, i)) {
                                                let param_name = format!("param{}", i);
                                                // Register this binding for EnumField handler
                                                if let Some(enum_reg) = enum_value_reg {
                                                    self.enum_param_bindings.insert(
                                                        (enum_reg, v, i),
                                                        param_name.clone()
                                                    );
                                                }
                                                Expr::Ident(param_name.into())
                                            } else {
                                                Expr::Ident("_".into())
                                            }
                                        })
                                        .collect();
                                    return Expr::EnumConstr(ref_type, RefEnumConstruct(v), bindings);
                                }
                                // Use EnumConstr for fully qualified name (e.g., ImageType.NoImage)
                                // This ensures the enum type is always included in the output
                                return Expr::EnumConstr(ref_type, RefEnumConstruct(v), vec![]);
                            }
                        }
                    }
                    // Fallback to integer
                    Expr::Constant(Constant::InlineInt(v))
                })
                .collect();

            // Skip default case target if it's also a case target
            if *target_op == default_op {
                // Add to cases instead of default
                let stmts = if let Some(target_block) = self.cfg.block_for_op(*target_op) {
                    if !self.processed.contains(&target_block) {
                        self.structure_from(target_block, merge_point)
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                };
                cases.push((case_exprs, stmts));
                continue;
            }

            let stmts = if let Some(target_block) = self.cfg.block_for_op(*target_op) {
                if !self.processed.contains(&target_block) {
                    self.structure_from(target_block, merge_point)
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            cases.push((case_exprs, stmts));
        }
        self.scope_depth -= 1;

        // Hoist variable declarations from cases to before the switch
        // This is needed because Haxe switch cases share scope but VarDecl
        // in one case doesn't make the variable visible in other cases
        let mut hoisted_decls: Vec<Statement> = Vec::new();
        let mut hoisted_names: HashSet<Str> = HashSet::new(); // Track what we've hoisted

        // Helper to extract var name from an expression
        fn extract_var_name(expr: &Expr) -> Option<Str> {
            match expr {
                Expr::Variable(_, Some(name)) => Some(name.clone()),
                _ => None
            }
        }

        // Process case statements: hoist var declarations and convert to assignments
        fn process_case_stmts(
            stmts: Vec<Statement>,
            hoisted: &mut HashSet<Str>,
            decls: &mut Vec<Statement>,
        ) -> Vec<Statement> {
            stmts.into_iter().map(|stmt| {
                match stmt {
                    Statement::Assign { declaration: true, variable, assign } => {
                        // This is a "var x = ..." statement
                        // Hoist the declaration and convert to simple assignment
                        if let Some(name) = extract_var_name(&variable) {
                            if !hoisted.contains(&name) {
                                hoisted.insert(name.clone());
                                // Check if assign is empty anonymous object - needs :Dynamic
                                let type_hint = if matches!(&assign, Expr::Anonymous(_, fields) if fields.is_empty()) {
                                    Some("Dynamic".into())
                                } else {
                                    None
                                };
                                // Create a declaration without initialization: "var x;" or "var x:Dynamic;"
                                decls.push(Statement::VarDecl { name, type_hint });
                            }
                        }
                        // Convert to non-declaration assignment
                        Statement::Assign { declaration: false, variable, assign }
                    }
                    other => other
                }
            }).collect()
        }

        // Process all case statements (process default FIRST since it has the declarations)
        let default_stmts = process_case_stmts(default_stmts, &mut hoisted_names, &mut hoisted_decls);

        let cases: Vec<(Vec<Expr>, Vec<Statement>)> = cases
            .into_iter()
            .map(|(vals, stmts)| (vals, process_case_stmts(stmts, &mut hoisted_names, &mut hoisted_decls)))
            .collect();

        // Add hoisted names to declared_vars so merge point won't re-declare them
        for name in &hoisted_names {
            self.declared_vars.insert(name.clone());
        }

        // Build result: hoisted declarations, then switch
        let mut result = hoisted_decls;
        result.push(Statement::Switch {
            arg: switch_arg,
            default: default_stmts,
            cases,
            enum_type,
        });

        // Continue after merge (only if requested)
        if do_continuation {
            if let Some(m) = merge_point {
                if Some(m) != stop_at && !self.processed.contains(&m) {
                    result.extend(self.structure_from(m, stop_at));
                }
            }
        }

        // Clear enum param bindings after processing switch
        self.enum_param_bindings.clear();

        result
    }

    /// Find the merge point for a switch statement
    pub(crate) fn find_switch_merge_point(&self, switch_block: NodeIndex) -> Option<NodeIndex> {
        // Get all successors of the switch block
        let succs = self.cfg.successors(switch_block);

        if succs.is_empty() {
            return None;
        }

        // Look for a common successor among all case branches
        // Each case typically ends with JAlways to the merge point
        let mut candidate_merges: HashMap<NodeIndex, usize> = HashMap::new();

        for succ in &succs {
            // Follow each case to find where it jumps to
            let case_succs = self.cfg.successors(*succ);
            for case_succ in case_succs {
                *candidate_merges.entry(case_succ).or_default() += 1;
            }
        }

        // The merge point must be reachable from at least 2 cases to be valid.
        // If only one case reaches a node, it's part of that case's control flow,
        // not a merge point. This prevents incorrectly skipping branches.
        candidate_merges
            .into_iter()
            .filter(|(_, count)| *count >= 2)
            .max_by_key(|(_, count)| *count)
            .map(|(node, _)| node)
    }

    /// Structure a single basic block into statements
    pub(crate) fn structure_block(&mut self, node: NodeIndex) -> Vec<Statement> {
        let block = &self.cfg.graph[node];
        let mut stmts = Vec::new();

        // Build mapping from op_idx to SSA destination for this block
        let op_to_ssa: HashMap<usize, SsaVar> = if let Some(ssa_block) = self.ssa.blocks.get(&node) {
            ssa_block.ops.iter().filter_map(|op| {
                if let SsaInstr::Op { op_idx, dst: Some(dst), .. } = op {
                    Some((*op_idx, *dst))
                } else {
                    None
                }
            }).collect()
        } else {
            HashMap::new()
        };

        // Note: φ-functions are handled via φ-elimination in structure_conditional
        // We don't emit separate variable declarations for φ destinations here because:
        // 1. Same-register phis (different versions of same reg) don't need declarations
        // 2. Different-register phis get assignments emitted by structure_conditional
        // 3. The register will be declared when first assigned in regular code

        // Process opcodes
        // Skip control flow ops at block.end - they're handled by structuring logic
        let end = if self.is_control_flow_op(block.end) {
            // If block.end is a control flow op, process up to (but not including) it
            // Special case: if block is just one control flow op (start == end), skip entirely
            if block.start >= block.end {
                // No non-control-flow ops to process
                return stmts;
            }
            block.end - 1
        } else {
            block.end
        };

        let func_end = self.func.ops.len();
        let mut op_idx = block.start;
        while op_idx <= end {
            // Track current opcode for debug name lookup
            self.current_op = op_idx;

            // Check if this opcode starts an exception region
            if let Some(region) = self.exception_analysis.region_starting_at(op_idx).cloned() {
                // Compute catch_end: find next sequential region or use function end
                let catch_end = self.find_catch_end(region.handler_op, func_end);

                // Structure the try/catch and skip past the entire region
                let try_catch = self.structure_exception_region(&region, catch_end);
                stmts.push(try_catch);
                // Skip past the catch body to continue after the try/catch
                op_idx = catch_end;
                continue;
            }

            // Check if this op has an SSA destination we can analyze
            let is_dead = if let Some(&ssa_dst) = op_to_ssa.get(&op_idx) {
                self.is_dead_var(ssa_dst)
            } else {
                false
            };

            // Only skip dead variables for opcodes that have no side effects.
            // Function calls always have side effects and should be emitted.
            let op = &self.func.ops[op_idx];
            let has_side_effects = matches!(
                op,
                Opcode::Call0 { .. }
                    | Opcode::Call1 { .. }
                    | Opcode::Call2 { .. }
                    | Opcode::Call3 { .. }
                    | Opcode::Call4 { .. }
                    | Opcode::CallN { .. }
                    | Opcode::CallMethod { .. }
                    | Opcode::CallThis { .. }
                    | Opcode::CallClosure { .. }
                    | Opcode::SetField { .. }
                    | Opcode::SetArray { .. }
                    | Opcode::SetMem { .. }
                    | Opcode::SetI8 { .. }
                    | Opcode::SetI16 { .. }
                    | Opcode::SetGlobal { .. }
                    | Opcode::Throw { .. }
            );

            if is_dead && !has_side_effects {
                op_idx += 1;
                continue;
            }

            // Emit statement for this opcode
            let new_stmts = self.opcode_to_statements(op_idx);
            stmts.extend(new_stmts);
            op_idx += 1;
        }

        // Check for return/throw
        self.current_op = block.end;
        if let Some(ret) = self.check_terminal(block.end) {
            stmts.push(ret);
        }

        stmts
    }

    pub(crate) fn compute_target(&self, op_idx: usize, offset: i32) -> Option<NodeIndex> {
        let target_idx = (op_idx as i64 + offset as i64 + 1) as usize;
        self.cfg.block_for_op(target_idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::CfgAnalysis;
    use crate::lifter::Cfg;
    use crate::ssa::SsaCfg;
    use crate::type_prop::TypePropagator;
    use hlbc::opcodes::Opcode;
    use hlbc::types::{RefFun, RefInt, RefString, RefType};

    fn create_mock_bytecode() -> Bytecode {
        let mut code = Bytecode::default();
        code.ints = vec![0, 1, 10, 42];
        code.strings = vec!["test".into()];
        code.types = vec![hlbc::types::Type::Void, hlbc::types::Type::I32];
        code
    }

    fn create_mock_function(ops: &[Opcode], num_regs: usize) -> Function {
        Function {
            name: RefString(0),
            t: RefType(0),
            findex: RefFun(0),
            regs: vec![RefType(1); num_regs],
            ops: ops.to_vec(),
            debug_info: None,
            assigns: None,
            parent: None,
        }
    }

    #[test]
    fn test_simple_linear() {
        let code = create_mock_bytecode();
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(3) },
            Opcode::Ret { ret: Reg(0) },
        ];

        let func = create_mock_function(&ops, 1);
        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        let ssa = SsaCfg::build(&func, &cfg, &analysis);
        let type_info = TypePropagator::new(&code, &func, &cfg, &ssa).propagate();

        let mut structurer = Structurer::new(&code, &func, &cfg, &analysis, &ssa, &type_info);
        let stmts = structurer.structure();

        assert!(!stmts.is_empty());
    }

    #[test]
    fn test_conditional() {
        let code = create_mock_bytecode();
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JAlways { offset: 1 },
            Opcode::Int { dst: Reg(1), ptr: RefInt(2) },
            Opcode::Ret { ret: Reg(1) },
        ];

        let func = create_mock_function(&ops, 2);
        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        let ssa = SsaCfg::build(&func, &cfg, &analysis);
        let type_info = TypePropagator::new(&code, &func, &cfg, &ssa).propagate();

        let mut structurer = Structurer::new(&code, &func, &cfg, &analysis, &ssa, &type_info);
        let stmts = structurer.structure();

        assert!(!stmts.is_empty());
    }
}
