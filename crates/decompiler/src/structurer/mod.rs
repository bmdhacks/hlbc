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
pub mod region_dominance;
pub mod region_graph;
pub mod sese;
pub mod stmts;

use petgraph::graph::NodeIndex;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, Reg, RefFun, RefString, RefType, Type};
use hlbc::{Bytecode, Resolve, Str};

use crate::analyzer::CfgAnalysis;
use crate::ast::Expr;
use crate::lifter::Cfg;
use crate::ssa::{SsaCfg, SsaVar, UseDefInfo};
use crate::type_prop::TypeInfo;

use crate::closure_analysis::ClosureAnalysis;
use crate::exception_analysis::ExceptionAnalysis;

// Re-exports for new reducer-based structuring (used by lib.rs)
pub use lower::{lower_region, LoweringContext};
pub use patterns::PatternContext;
pub use reducer::{reduce_to_region, reduce_to_region_with_string_switches, reduce_to_region_with_exceptions};

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

/// An expression stored for potential inlining, along with its context.
/// Used to track where expressions were defined and what memory they depend on,
/// enabling escape analysis to flush expressions before conditional branches.
#[derive(Clone, Debug)]
pub(crate) struct InlineExpr {
    /// The expression to be inlined
    pub expr: Expr,
    /// Memory dependency - what memory this expression reads
    pub mem_dep: MemoryDep,
    /// The CFG block where this expression was defined (tracked for potential future use)
    #[allow(dead_code)]
    pub def_block: NodeIndex,
    /// The scope depth when this expression was defined
    pub def_scope: u32,
}

/// A detected string switch case (used for string switch detection)
#[derive(Debug, Clone)]
pub(crate) struct StringSwitchCase {
    /// The string literal for this case
    pub(crate) string_ref: RefString,
    /// Opcode index of the handler (where JEq jumps to)
    pub(crate) handler_op: usize,
}

/// A detected string switch region in bytecode (used for string switch detection, to be implemented in new path)
#[derive(Debug, Clone)]
pub(crate) struct StringSwitchRegion {
    /// First opcode of the switch (the first JNull)
    pub(crate) start_op: usize,
    /// Last opcode of the switch pattern (before handlers/default)
    pub(crate) end_op: usize,
    /// The register holding the string being switched on
    pub(crate) switch_arg_reg: Reg,
    /// All detected cases with their string values and handler targets
    pub(crate) cases: Vec<StringSwitchCase>,
    /// Opcode index of the default case handler
    pub(crate) default_op: usize,
}

/// CFG-level mapping for an inline expansion (addByte, addInt32, downcast).
/// Maps the opcode range to the set of CFG nodes it spans, so the reducer
/// can pre-collapse them before the main reduction loop.
#[derive(Debug, Clone)]
pub struct InlineExpansionCfgMapping {
    /// All CFG nodes that contain opcodes from this expansion
    pub nodes: HashSet<NodeIndex>,
}

/// CFG-level mapping for a string switch region
#[derive(Debug, Clone)]
pub struct StringSwitchCfgMapping {
    /// CFG nodes that contain string switch pattern opcodes (the 9-opcode checks per case)
    pub pattern_nodes: HashSet<NodeIndex>,
    /// Case handlers: (string literal ref, handler CFG node)
    pub handler_nodes: Vec<(RefString, NodeIndex)>,
    /// Default case CFG node
    pub default_node: NodeIndex,
    /// Register holding the string being switched on
    pub switch_arg_reg: Reg,
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

    /// Processed blocks (to avoid re-processing) - used by legacy path, kept for future use
    #[allow(dead_code)]
    pub(crate) processed: HashSet<NodeIndex>,
    /// Use-def info for inlining decisions (ILSpy-style)
    pub(crate) use_info: HashMap<SsaVar, UseDefInfo>,
    /// Dead phi destinations - phis whose results are never used
    /// Used to exclude dead phis from "same-register phi source" checks
    pub(crate) dead_phis: HashSet<SsaVar>,
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
    /// Debug name -> type mapping to detect type conflicts
    /// When the same debug name (e.g., "v") is used for registers of different types,
    /// we force raw names to avoid Haxe type errors
    /// Uses RefCell for interior mutability (updated during structuring)
    pub(crate) debug_name_types: RefCell<HashMap<String, RefType>>,
    /// Array bytes tracking: maps bytes register -> array expression
    /// Used to reconstruct arr[i] from bytes[shifted_i] pattern
    /// We store the Expr (not Reg) to capture the correct SSA version at field access time
    pub(crate) array_bytes_source: HashMap<Reg, Expr>,
    /// Shifted index tracking: maps shifted reg -> (original index expression, shift amount)
    /// Used to reverse index * 4 back to original index for array access
    /// We store the Expr (not Reg) to capture the correct SSA version at shift time
    pub(crate) shifted_indices: HashMap<Reg, (Expr, i32)>,
    /// Exception region analysis for try/catch structuring - to be implemented in new path
    #[allow(dead_code)]
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
    /// CFG-level mappings for string switches
    pub(crate) string_switch_cfg_mappings: Vec<StringSwitchCfgMapping>,
    /// Opcodes that are part of string switch patterns (to suppress during block lowering)
    pub(crate) string_switch_opcodes: HashSet<usize>,
    /// Current loop header (if any) - used to distinguish continue from switch fall-through
    pub(crate) current_loop_header: Option<NodeIndex>,
    /// Expressions available for inlining (SSA var -> InlineExpr)
    /// Single-use, pure expressions are stored here instead of emitting a statement.
    /// When the variable is referenced, the stored expression is inlined at the use site.
    /// The InlineExpr tracks memory dependency, definition block, and scope depth,
    /// enabling escape analysis to flush expressions before conditional branches.
    /// Uses RefCell for interior mutability so we can remove entries when inlined.
    pub(crate) inline_exprs: RefCell<HashMap<SsaVar, InlineExpr>>,
    /// Opcodes to suppress (not emit as statements)
    /// Used when an opcode's result is consumed by another construct (e.g., EnumIndex for switch)
    pub(crate) suppressed_ops: HashSet<usize>,
    /// Enum pattern bindings: maps (switch_reg, construct_idx, field_idx) -> bound param name
    /// When set, EnumField opcodes matching these keys emit the param name
    /// instead of Type.enumParameters(...). Used to generate cleaner switch case patterns.
    pub(crate) enum_param_bindings: HashMap<(Reg, usize, usize), String>,
    /// Variables that actually had assignments emitted (for filtering hoisted VarDecls).
    /// This allows us to skip emitting VarDecls for variables that were hoisted but never
    /// actually assigned (e.g., because the assignment was dead and skipped).
    pub(crate) actually_used_vars: HashSet<Str>,
    /// Pending constructors: maps New dst register -> (type_ref, new_op_idx)
    /// When New is seen, we store the info here instead of emitting.
    /// When the constructor Call is seen, we emit the full new Type(args) statement.
    pub(crate) pending_constructors: HashMap<Reg, (RefType, usize)>,
    /// Detected inline expansion patterns (addByte, addInt32, downcast, etc.)
    pub(crate) inline_expansions: Vec<idioms::InlineExpansion>,
    /// Opcodes that are part of inline expansions (to suppress during block lowering)
    pub(crate) inline_expansion_opcodes: HashSet<usize>,
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
        let (use_info, dead_phis) = ssa.compute_use_counts(func);

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

        let inline_expansions = idioms::detect_inline_expansions(code, func);
        let inline_expansion_opcodes: HashSet<usize> = inline_expansions.iter()
            .flat_map(|e| e.start_op..=e.end_op)
            .collect();

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
            dead_phis,
            exception_analysis,
            declared_vars,
            current_op: 0,
            method_info,
            scope_depth: 0,
            hoisted_vars: HashSet::new(),
            needs_dynamic_type: HashSet::new(),
            hoisted_var_types: HashMap::new(),
            debug_name_types: RefCell::new(HashMap::new()),
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
            string_switch_cfg_mappings: Vec::new(),
            string_switch_opcodes: HashSet::new(),
            current_loop_header: None,
            inline_exprs: RefCell::new(HashMap::new()),
            suppressed_ops: HashSet::new(),
            enum_param_bindings: HashMap::new(),
            actually_used_vars: HashSet::new(),
            pending_constructors: HashMap::new(),
            inline_expansions,
            inline_expansion_opcodes,
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
}

// Legacy structurer methods have been removed.
// The new path uses reduce_to_region() + lower_region() from reducer.rs and lower.rs.

// The following legacy methods were removed:
// - structure(), structure_from(), structure_from_inner()
// - structure_string_switch(), structure_loop(), structure_block_range()
// - structure_exception_region(), structure_opcode_range()
// - extract_loop_condition(), structure_conditional(), structure_switch(), etc.
// - structure_block(), compute_target()

#[cfg(test)]
mod legacy_removed {
    // Legacy unit tests removed - they called structurer.structure() which no longer exists.
    // Integration tests in tests/roundtrip/ cover the new reducer path.
}

// Helper methods - may be used by lower.rs or future implementations
impl<'a> Structurer<'a> {
    #[allow(dead_code)]
    pub(crate) fn compute_target(&self, op_idx: usize, offset: i32) -> Option<NodeIndex> {
        let target_idx = (op_idx as i64 + offset as i64 + 1) as usize;
        self.cfg.block_for_op(target_idx)
    }

    /// Build CFG-level mappings for detected string switch regions.
    /// This converts opcode-based StringSwitchRegion to CFG node indices.
    /// Must be called after Structurer has access to the CFG.
    pub fn build_string_switch_cfg_mappings(&mut self) {
        let mut mappings = Vec::new();
        let mut all_pattern_opcodes = HashSet::new();

        for ss in &self.string_switches {
            if let Some(mapping) = self.map_string_switch_to_cfg(ss) {
                // Collect all pattern opcodes for suppression
                for op_idx in ss.start_op..=ss.end_op {
                    all_pattern_opcodes.insert(op_idx);
                }
                mappings.push(mapping);
            }
        }

        self.string_switch_cfg_mappings = mappings;
        self.string_switch_opcodes = all_pattern_opcodes;
    }

    /// Map a single StringSwitchRegion to CFG nodes.
    fn map_string_switch_to_cfg(&self, ss: &StringSwitchRegion) -> Option<StringSwitchCfgMapping> {
        // Find all CFG nodes whose opcodes fall within the pattern range (start_op..=end_op)
        let mut pattern_nodes = HashSet::new();
        for op_idx in ss.start_op..=ss.end_op {
            if let Some(&cfg_node) = self.cfg.op_to_block.get(&op_idx) {
                pattern_nodes.insert(cfg_node);
            }
        }

        // Map each case handler_op to its CFG node
        let mut handler_nodes = Vec::new();
        for case in &ss.cases {
            if let Some(&handler_node) = self.cfg.op_to_block.get(&case.handler_op) {
                handler_nodes.push((case.string_ref, handler_node));
            }
        }

        // Map default_op to its CFG node
        let default_node = self.cfg.op_to_block.get(&ss.default_op).copied()?;

        Some(StringSwitchCfgMapping {
            pattern_nodes,
            handler_nodes,
            default_node,
            switch_arg_reg: ss.switch_arg_reg,
        })
    }

    /// Build CFG-level mappings for detected inline expansions.
    /// Only includes CFG blocks that are FULLY contained within the expansion range.
    /// Blocks that partially overlap (e.g., a GetThis before the NullCheck) are excluded
    /// to prevent absorbing non-expansion code.
    pub fn build_inline_expansion_cfg_mappings(&self) -> Vec<InlineExpansionCfgMapping> {
        self.inline_expansions.iter().map(|exp| {
            let mut candidate_nodes = HashSet::new();
            for op_idx in exp.start_op..=exp.end_op {
                if let Some(&cfg_node) = self.cfg.op_to_block.get(&op_idx) {
                    candidate_nodes.insert(cfg_node);
                }
            }

            // Filter: only keep nodes whose ALL ops are within the expansion range
            let nodes = candidate_nodes.into_iter().filter(|&cfg_node| {
                if let Some(block) = self.cfg.graph.node_weight(cfg_node) {
                    block.start >= exp.start_op && block.end <= exp.end_op
                } else {
                    false
                }
            }).collect();

            InlineExpansionCfgMapping { nodes }
        }).collect()
    }

    /// Emit a synthetic method call for a detected inline expansion.
    ///
    /// Replaces the inlined bytecode with a clean method call like `buf.addByte(value)`.
    pub(crate) fn emit_inline_expansion_call(&mut self, exp: &idioms::InlineExpansion) -> Vec<crate::ast::Statement> {
        use crate::ast::{Call, Expr, Statement};

        match &exp.kind {
            idioms::InlineExpansionKind::AddByte | idioms::InlineExpansionKind::AddInt32 => {
                let method_name = match &exp.kind {
                    idioms::InlineExpansionKind::AddByte => "addByte",
                    idioms::InlineExpansionKind::AddInt32 => "addInt32",
                    _ => unreachable!(),
                };
                // Resolve obj_reg from the first Field op in the expansion (which uses buf as obj)
                // and value_reg from the end (where the written value is in scope).
                // Skip NullCheck at start since it may not have SSA uses for the reg.
                let saved_op = self.current_op;

                // Find the first Field op that uses obj_reg
                let field_op = (exp.start_op..=exp.end_op).find(|&op_idx| {
                    matches!(self.func.ops.get(op_idx), Some(Opcode::Field { obj, .. }) if *obj == exp.obj_reg)
                }).unwrap_or(exp.start_op);
                self.current_op = field_op;
                if let Some((dst, uses)) = self.ssa.get_instr_for_op(field_op) {
                    self.current_ssa_dst = dst;
                    self.current_ssa_uses = uses.to_vec();
                }
                let obj = self.reg_to_expr(exp.obj_reg);

                self.current_op = exp.end_op;
                if let Some((dst, uses)) = self.ssa.get_instr_for_op(exp.end_op) {
                    self.current_ssa_dst = dst;
                    self.current_ssa_uses = uses.to_vec();
                }
                let val = if let Some(ref_int) = exp.value_const {
                    Expr::Constant(crate::ast::Constant::Int(ref_int))
                } else {
                    self.reg_to_expr(exp.value_reg)
                };

                self.current_op = saved_op;
                let call = Call {
                    fun: Expr::Field(Box::new(obj), method_name.into()),
                    args: vec![val],
                };
                vec![Statement::ExprStatement(Expr::Call(Box::new(call)))]
            }
            idioms::InlineExpansionKind::Downcast { class_global, result_reg } => {
                let saved_op = self.current_op;

                // Resolve value_reg from the Call2 op (exp.start_op + 1)
                self.current_op = exp.start_op + 1;
                if let Some((dst, uses)) = self.ssa.get_instr_for_op(self.current_op) {
                    self.current_ssa_dst = dst;
                    self.current_ssa_uses = uses.to_vec();
                }
                let val = self.reg_to_expr(exp.value_reg);

                // Resolve result_reg from the cast op (exp.start_op + 3)
                // where the result register is defined with its debug name
                self.current_op = exp.start_op + 3;
                if let Some((dst, uses)) = self.ssa.get_instr_for_op(self.current_op) {
                    self.current_ssa_dst = dst;
                    self.current_ssa_uses = uses.to_vec();
                }
                let var = self.reg_to_expr_dst(*result_reg);

                self.current_op = saved_op;

                // Get the class name from the global's type
                let class_name = if let Some(type_ref) = self.code.globals.get(class_global.0) {
                    if let Some(Type::Obj(obj)) = self.code.types.get(type_ref.0) {
                        let name = self.code.get(obj.name);
                        // Strip $ prefix from companion type name
                        let clean = name.replace(".$", ".").replace('$', "");
                        clean
                    } else {
                        "Dynamic".to_string()
                    }
                } else {
                    "Dynamic".to_string()
                };
                let call = Call {
                    fun: Expr::Field(
                        Box::new(Expr::Ident("Std".into())),
                        "downcast".into(),
                    ),
                    args: vec![val, Expr::Ident(class_name.into())],
                };
                vec![self.make_assign(var, Expr::Call(Box::new(call)))]
            }
        }
    }
}

