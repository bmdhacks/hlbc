//! Pass 5: Structurer - Convert SSA-CFG to structured AST
//!
//! This module transforms an SSA-annotated CFG back into structured code:
//! - Uses loop info from Analyzer to emit `while` loops
//! - Uses dominator tree to structure if/else
//! - Converts φ-functions to variable assignments at branch ends
//!
//! This is a simplified implementation focused on correctness over optimization.

use petgraph::graph::NodeIndex;
use std::collections::{HashMap, HashSet};

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, Reg, RefFun, Type};
use hlbc::{Bytecode, Str};

use crate::analyzer::{CfgAnalysis, NaturalLoop};
use crate::ast::{Call, Constant, ConstructorCall, Expr, Operation, Statement};
use crate::lifter::Cfg;
use hlbc::types::RefType;
use crate::ssa::{SsaCfg, SsaInstr, SsaVar, get_dst_reg as get_opcode_dst};
use crate::type_prop::TypeInfo;

use crate::ssa::UseDefInfo;

/// Context for structuring
pub struct Structurer<'a> {
    code: &'a Bytecode,
    func: &'a Function,
    cfg: &'a Cfg,
    analysis: &'a CfgAnalysis,
    ssa: &'a SsaCfg,
    _type_info: &'a TypeInfo,

    /// Processed blocks (to avoid re-processing)
    processed: HashSet<NodeIndex>,
    /// Variable names for SSA variables
    var_names: HashMap<SsaVar, Str>,
    /// Counter for generating variable names
    var_counter: u32,
    /// Known constant values for registers (for constant propagation)
    constants: HashMap<Reg, Constant>,
    /// Use-def info for inlining decisions (ILSpy-style)
    use_info: HashMap<SsaVar, UseDefInfo>,
    /// Expressions to inline (single-use variables)
    inline_exprs: HashMap<SsaVar, Expr>,
    /// Variable names that have been declared (for declaration tracking)
    declared_vars: HashSet<Str>,
    /// Current opcode index being processed (for debug name lookup)
    current_op: usize,
    /// Method info: maps function references to (owner_type, method_name)
    /// Used to convert f(obj, args) to obj.f(args) syntax
    method_info: HashMap<RefFun, (RefType, Str)>,
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
        // Compute use counts for inlining decisions (needs func for purity info)
        let use_info = ssa.compute_use_counts(func);

        // Build method info map from all types' protos
        let method_info = Self::build_method_info(code);

        Structurer {
            code,
            func,
            cfg,
            analysis,
            ssa,
            _type_info: type_info,
            processed: HashSet::new(),
            var_names: HashMap::new(),
            var_counter: 0,
            constants: HashMap::new(),
            use_info,
            inline_exprs: HashMap::new(),
            declared_vars: HashSet::new(),
            current_op: 0,
            method_info,
        }
    }

    /// Build a map from function references to their owner types and method names.
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

    /// Create an assignment statement, tracking declaration status.
    /// Returns a Statement::Assign with declaration=true if this is the first
    /// assignment to this variable name.
    fn make_assign(&mut self, variable: Expr, assign: Expr) -> Statement {
        // Extract variable name to check if it's been declared
        // ONLY simple variables can have declarations (var x = ...)
        // Field access, array index, etc. are NEVER declarations
        let is_declaration = match &variable {
            Expr::Variable(_, Some(name)) => {
                if self.declared_vars.contains(name) {
                    false
                } else {
                    self.declared_vars.insert(name.clone());
                    true
                }
            }
            Expr::Ident(name) => {
                if self.declared_vars.contains(name) {
                    false
                } else {
                    self.declared_vars.insert(name.clone());
                    true
                }
            }
            // Field access (obj.field) - never a declaration
            Expr::Field(_, _) => false,
            // Array access (arr[i]) - never a declaration
            Expr::Array(_, _) => false,
            // Any other expression type - never a declaration
            _ => false,
        };

        Statement::Assign {
            declaration: is_declaration,
            variable,
            assign,
        }
    }

    /// Create a call statement, handling void return types correctly.
    /// For void functions, we emit just the call as an expression statement.
    /// For non-void functions, we assign the result to a variable.
    fn make_call_stmt(&mut self, dst: Reg, call: Call) -> Statement {
        if self.is_void_type(dst) {
            Statement::ExprStatement(Expr::Call(Box::new(call)))
        } else {
            let var = self.reg_to_expr(dst);
            self.make_assign(var, Expr::Call(Box::new(call)))
        }
    }

    /// Structure the entire function into statements
    pub fn structure(&mut self) -> Vec<Statement> {
        // Pre-scan all blocks for constant assignments
        self.scan_all_constants();

        let stmts = self.structure_from(self.cfg.entry, None);
        // Post-process to simplify
        simplify_statements(stmts)
    }

    /// Check if an SSA variable can be inlined.
    /// Implements ILSpy-style safety guards:
    /// - Guard 1 (Side-Effect): Pure ops only (checked in UseDefInfo::can_inline)
    /// - Guard 2 (Debug Name): Don't inline user-named variables
    /// - Guard 3 (Phi): No cross-block inlining (checked in UseDefInfo::can_inline)
    fn can_inline_var(&self, var: SsaVar) -> bool {
        // Guard 2: Don't inline variables with user-defined debug names
        // These are meaningful names the programmer chose, preserve them
        if self.has_user_debug_name(var.reg) {
            return false;
        }

        // Check SSA-based criteria (purity, use count, phi uses)
        self.use_info.get(&var).map_or(false, |info| info.can_inline())
    }

    /// Check if a register has a user-defined debug name (not synthetic)
    fn has_user_debug_name(&self, reg: Reg) -> bool {
        // Check if the function has assigns (debug variable names)
        // assigns is Vec<(RefString name, usize op_idx)>
        // IMPORTANT: op_idx points to the opcode AFTER the definition.
        // The actual definition is at op_idx - 1.
        if let Some(assigns) = &self.func.assigns {
            for (_, op_idx) in assigns {
                let def_idx = op_idx.saturating_sub(1);
                if def_idx < self.func.ops.len() {
                    if let Some(dst_reg) = get_opcode_dst(&self.func.ops[def_idx]) {
                        if dst_reg == reg {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Check if an SSA variable is dead (defined but never used)
    fn is_dead_var(&self, var: SsaVar) -> bool {
        self.use_info.get(&var).map_or(false, |info| info.is_dead())
    }

    /// Store an expression for later inlining
    fn store_for_inline(&mut self, var: SsaVar, expr: Expr) {
        self.inline_exprs.insert(var, expr);
    }

    /// Try to get an inlined expression for a register
    fn get_inlined_expr(&mut self, reg: Reg) -> Option<Expr> {
        // Find the most recent SSA version for this register that can be inlined
        // This is a simplification - in practice we'd need to track the current version
        for (var, expr) in self.inline_exprs.iter() {
            if var.reg == reg {
                return Some(expr.clone());
            }
        }
        None
    }

    /// Scan all blocks for constant assignments to build the constants map.
    /// Only keeps constants for registers assigned exactly once with a constant value.
    fn scan_all_constants(&mut self) {
        // First pass: count assignments per register and track constant values
        let mut assign_counts: HashMap<Reg, usize> = HashMap::new();
        let mut constant_values: HashMap<Reg, Constant> = HashMap::new();

        for node in self.cfg.graph.node_indices() {
            let block = &self.cfg.graph[node];
            for op_idx in block.start..=block.end {
                match &self.func.ops[op_idx] {
                    Opcode::Int { dst, ptr } => {
                        *assign_counts.entry(*dst).or_insert(0) += 1;
                        constant_values.insert(*dst, Constant::Int(*ptr));
                    }
                    Opcode::Float { dst, ptr } => {
                        *assign_counts.entry(*dst).or_insert(0) += 1;
                        constant_values.insert(*dst, Constant::Float(*ptr));
                    }
                    Opcode::Bool { dst, value } => {
                        *assign_counts.entry(*dst).or_insert(0) += 1;
                        constant_values.insert(*dst, Constant::Bool(*value));
                    }
                    Opcode::String { dst, ptr } => {
                        *assign_counts.entry(*dst).or_insert(0) += 1;
                        constant_values.insert(*dst, Constant::String(*ptr));
                    }
                    Opcode::Null { dst } => {
                        *assign_counts.entry(*dst).or_insert(0) += 1;
                        constant_values.insert(*dst, Constant::Null);
                    }
                    // Any non-constant assignment invalidates the register
                    Opcode::Incr { dst } | Opcode::Decr { dst } => {
                        *assign_counts.entry(*dst).or_insert(0) += 1;
                        constant_values.remove(dst);
                    }
                    Opcode::Mov { dst, .. } |
                    Opcode::Add { dst, .. } |
                    Opcode::Sub { dst, .. } |
                    Opcode::Mul { dst, .. } |
                    Opcode::Field { dst, .. } |
                    Opcode::Call0 { dst, .. } |
                    Opcode::Call1 { dst, .. } |
                    Opcode::Call2 { dst, .. } |
                    Opcode::Call3 { dst, .. } |
                    Opcode::Call4 { dst, .. } |
                    Opcode::CallN { dst, .. } |
                    Opcode::CallMethod { dst, .. } |
                    Opcode::CallThis { dst, .. } |
                    Opcode::CallClosure { dst, .. } |
                    Opcode::GetGlobal { dst, .. } |
                    Opcode::New { dst, .. } |
                    Opcode::GetArray { dst, .. } |
                    Opcode::GetThis { dst, .. } |
                    Opcode::SDiv { dst, .. } |
                    Opcode::UDiv { dst, .. } |
                    Opcode::SMod { dst, .. } |
                    Opcode::UMod { dst, .. } |
                    Opcode::And { dst, .. } |
                    Opcode::Or { dst, .. } |
                    Opcode::Xor { dst, .. } |
                    Opcode::Shl { dst, .. } |
                    Opcode::SShr { dst, .. } |
                    Opcode::UShr { dst, .. } |
                    Opcode::Neg { dst, .. } |
                    Opcode::Not { dst, .. } |
                    Opcode::ToVirtual { dst, .. } |
                    Opcode::ToSFloat { dst, .. } |
                    Opcode::ToUFloat { dst, .. } |
                    Opcode::ToInt { dst, .. } |
                    Opcode::ToDyn { dst, .. } |
                    Opcode::SafeCast { dst, .. } |
                    Opcode::UnsafeCast { dst, .. } |
                    Opcode::Ref { dst, .. } |
                    Opcode::Unref { dst, .. } |
                    Opcode::Type { dst, .. } |
                    Opcode::DynGet { dst, .. } => {
                        *assign_counts.entry(*dst).or_insert(0) += 1;
                        constant_values.remove(dst);
                    }
                    _ => {}
                }
            }
        }

        // Second pass: only keep constants for registers assigned exactly once
        for (reg, constant) in constant_values {
            if assign_counts.get(&reg) == Some(&1) {
                self.constants.insert(reg, constant);
            }
        }
    }

    /// Structure code starting from a given block
    fn structure_from(&mut self, start: NodeIndex, stop_at: Option<NodeIndex>) -> Vec<Statement> {
        if Some(start) == stop_at || self.processed.contains(&start) {
            return vec![];
        }

        // Check if this is a loop header
        if let Some(loop_info) = self.analysis.loops.iter().find(|l| l.header == start).cloned() {
            return self.structure_loop(&loop_info, stop_at);
        }

        self.processed.insert(start);
        let mut stmts = self.structure_block(start);

        // Get successors
        let succs: Vec<NodeIndex> = self.cfg.successors(start);

        match succs.len() {
            0 => stmts, // Terminal
            1 => {
                stmts.extend(self.structure_from(succs[0], stop_at));
                stmts
            }
            2 => {
                stmts.extend(self.structure_conditional(start, &succs, stop_at));
                stmts
            }
            _ => {
                // Switch statement - multiple successors means multiple cases
                stmts.extend(self.structure_switch(start, &succs, stop_at));
                stmts
            }
        }
    }

    /// Structure a loop
    fn structure_loop(&mut self, loop_info: &NaturalLoop, stop_at: Option<NodeIndex>) -> Vec<Statement> {
        let header = loop_info.header;
        self.processed.insert(header);

        // Extract condition and body start
        let (condition, body_start, exit_target) = self.extract_loop_condition(loop_info);

        // Structure body
        let body = if let Some(body_node) = body_start {
            // Temporarily allow processing body nodes
            let old_processed = self.processed.clone();
            for &node in &loop_info.body {
                if node != header {
                    self.processed.remove(&node);
                }
            }
            let body_stmts = self.structure_from(body_node, Some(header));
            self.processed = old_processed;
            for &node in &loop_info.body {
                self.processed.insert(node);
            }
            body_stmts
        } else {
            vec![]
        };

        let mut result = vec![Statement::While {
            cond: condition,
            stmts: body,
        }];

        // Continue after loop
        if let Some(exit) = exit_target {
            if Some(exit) != stop_at && !self.processed.contains(&exit) {
                result.extend(self.structure_from(exit, stop_at));
            }
        }

        result
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

    /// Structure a conditional
    fn structure_conditional(
        &mut self,
        block: NodeIndex,
        _succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
    ) -> Vec<Statement> {
        let block_data = &self.cfg.graph[block];
        let last_op = &self.func.ops[block_data.end];

        let (condition, then_target, else_target) = self.extract_condition(block_data.end, last_op);

        // Find merge point
        let merge = self.find_merge_point(then_target, else_target);

        // Structure branches
        let mut then_stmts = if let Some(t) = then_target {
            if Some(t) != merge && !self.processed.contains(&t) {
                self.structure_from(t, merge)
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        let mut else_stmts = if let Some(e) = else_target {
            if Some(e) != merge && !self.processed.contains(&e) {
                self.structure_from(e, merge)
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        // φ-elimination: insert assignments at branch ends for merge point φ-functions
        if let Some(merge_node) = merge {
            let phi_assignments = self.get_phi_assignments_for_merge(merge_node, then_target, else_target);
            then_stmts.extend(phi_assignments.0);
            else_stmts.extend(phi_assignments.1);
        }

        let mut result = vec![Statement::IfElse {
            cond: condition,
            if_: then_stmts,
            else_: else_stmts,
        }];

        // Continue after merge
        if let Some(m) = merge {
            if Some(m) != stop_at && !self.processed.contains(&m) {
                result.extend(self.structure_from(m, stop_at));
            }
        }

        result
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
            _ => (Expr::Constant(Constant::Bool(true)), fall, None),
        }
    }

    /// Find merge point of two branches
    fn find_merge_point(&self, a: Option<NodeIndex>, b: Option<NodeIndex>) -> Option<NodeIndex> {
        let a = a?;
        let b = b?;
        let a_succs: HashSet<_> = self.cfg.successors(a).into_iter().collect();
        let b_succs: HashSet<_> = self.cfg.successors(b).into_iter().collect();

        for s in &a_succs {
            if b_succs.contains(s) {
                return Some(*s);
            }
        }
        if a_succs.contains(&b) { Some(b) }
        else if b_succs.contains(&a) { Some(a) }
        else { None }
    }

    /// Get φ-elimination assignments for a merge point
    /// Returns (assignments for then branch, assignments for else branch)
    fn get_phi_assignments_for_merge(
        &mut self,
        merge: NodeIndex,
        then_pred: Option<NodeIndex>,
        else_pred: Option<NodeIndex>,
    ) -> (Vec<Statement>, Vec<Statement>) {
        let mut then_assigns = Vec::new();
        let mut else_assigns = Vec::new();

        if let Some(ssa_block) = self.ssa.blocks.get(&merge) {
            for phi in &ssa_block.phis {
                if let SsaInstr::Phi { dst, sources } = phi {
                    let dst_name = self.get_var_name(*dst);
                    let dst_expr = Expr::Variable(dst.reg, Some(dst_name.clone()));

                    // Find source for then branch
                    if let Some(then_node) = then_pred {
                        if let Some((_, src_var)) = sources.iter().find(|(pred, _)| *pred == then_node) {
                            let src_expr = Expr::Variable(src_var.reg, Some(self.get_var_name(*src_var)));
                            then_assigns.push(self.make_assign(dst_expr.clone(), src_expr));
                        }
                    }

                    // Find source for else branch
                    if let Some(else_node) = else_pred {
                        if let Some((_, src_var)) = sources.iter().find(|(pred, _)| *pred == else_node) {
                            let src_expr = Expr::Variable(src_var.reg, Some(self.get_var_name(*src_var)));
                            else_assigns.push(self.make_assign(dst_expr, src_expr));
                        }
                    }
                }
            }
        }

        (then_assigns, else_assigns)
    }

    /// Structure a switch statement
    fn structure_switch(
        &mut self,
        block: NodeIndex,
        _succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
    ) -> Vec<Statement> {
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
        let switch_arg = self.reg_to_expr(switch_reg);

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

        // Structure default case (fall-through)
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
        let mut cases: Vec<(Vec<usize>, Vec<Statement>)> = Vec::new();
        let mut processed_targets: HashSet<usize> = HashSet::new();

        // Sort case values for deterministic output
        let mut sorted_targets: Vec<_> = target_to_cases.into_iter().collect();
        sorted_targets.sort_by_key(|(_, vals)| vals.iter().min().copied().unwrap_or(0));

        for (target_op, case_vals) in sorted_targets {
            if processed_targets.contains(&target_op) {
                continue;
            }
            processed_targets.insert(target_op);

            // Skip default case target if it's also a case target
            if target_op == default_op {
                // Add to cases instead of default
                let stmts = if let Some(target_block) = self.cfg.block_for_op(target_op) {
                    if !self.processed.contains(&target_block) {
                        self.structure_from(target_block, merge_point)
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                };
                cases.push((case_vals, stmts));
                continue;
            }

            let stmts = if let Some(target_block) = self.cfg.block_for_op(target_op) {
                if !self.processed.contains(&target_block) {
                    self.structure_from(target_block, merge_point)
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            cases.push((case_vals, stmts));
        }

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
                                // Create a declaration without initialization: "var x;"
                                decls.push(Statement::VarDecl { name });
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

        let cases: Vec<(Vec<usize>, Vec<Statement>)> = cases
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
            enum_type: None,
        });

        // Continue after merge
        if let Some(m) = merge_point {
            if Some(m) != stop_at && !self.processed.contains(&m) {
                result.extend(self.structure_from(m, stop_at));
            }
        }

        result
    }

    /// Find the merge point for a switch statement
    fn find_switch_merge_point(&self, switch_block: NodeIndex) -> Option<NodeIndex> {
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

        // The merge point should be reachable from all/most cases
        candidate_merges
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(node, _)| node)
    }

    /// Structure a single basic block into statements
    fn structure_block(&mut self, node: NodeIndex) -> Vec<Statement> {
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
        // We emit variable declarations here for any φ destinations that need them
        if let Some(ssa_block) = self.ssa.blocks.get(&node) {
            for phi in &ssa_block.phis {
                if let SsaInstr::Phi { dst, .. } = phi {
                    // Skip dead φ variables
                    if !self.is_dead_var(*dst) {
                        let name = self.get_var_name(*dst);
                        // Skip if already declared (e.g., hoisted from switch)
                        if !self.declared_vars.contains(&name) {
                            self.declared_vars.insert(name.clone());
                            stmts.push(Statement::VarDecl { name });
                        }
                    }
                }
            }
        }

        // Process opcodes
        let end = if self.is_control_flow_op(block.end) {
            block.end.saturating_sub(1)
        } else {
            block.end
        };

        for op_idx in block.start..=end {
            // Track current opcode for debug name lookup
            self.current_op = op_idx;

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
                    | Opcode::SetGlobal { .. }
                    | Opcode::Throw { .. }
            );

            if is_dead && !has_side_effects {
                continue;
            }

            // Emit statement for this opcode
            if let Some(stmt) = self.opcode_to_statement(op_idx) {
                stmts.push(stmt);
            }
        }

        // Check for return/throw
        self.current_op = block.end;
        if let Some(ret) = self.check_terminal(block.end) {
            stmts.push(ret);
        }

        stmts
    }

    /// Convert an opcode to just the expression (for inlining)
    fn opcode_to_expr(&self, op_idx: usize) -> Option<Expr> {
        let op = &self.func.ops[op_idx];
        match op {
            Opcode::Int { ptr, .. } => Some(Expr::Constant(Constant::Int(*ptr))),
            Opcode::Float { ptr, .. } => Some(Expr::Constant(Constant::Float(*ptr))),
            Opcode::Bool { value, .. } => Some(Expr::Constant(Constant::Bool(*value))),
            Opcode::String { ptr, .. } => Some(Expr::Constant(Constant::String(*ptr))),
            Opcode::Null { .. } => Some(Expr::Constant(Constant::Null)),
            Opcode::Mov { src, .. } => Some(self.reg_to_expr(*src)),
            Opcode::Add { a, b, .. } => Some(Expr::Op(Operation::Add(
                Box::new(self.reg_to_expr(*a)),
                Box::new(self.reg_to_expr(*b)),
            ))),
            Opcode::Sub { a, b, .. } => Some(Expr::Op(Operation::Sub(
                Box::new(self.reg_to_expr(*a)),
                Box::new(self.reg_to_expr(*b)),
            ))),
            Opcode::Mul { a, b, .. } => Some(Expr::Op(Operation::Mul(
                Box::new(self.reg_to_expr(*a)),
                Box::new(self.reg_to_expr(*b)),
            ))),
            Opcode::Field { obj, field, .. } => {
                let obj_expr = self.reg_to_expr(*obj);
                let field_name = self.get_field_name(*obj, *field);
                Some(Expr::Field(Box::new(obj_expr), field_name))
            }
            Opcode::GetGlobal { global, .. } => {
                // Check if this is a string constant global
                if let Some(string_ref) = self.get_global_string_value(*global) {
                    Some(Expr::Constant(Constant::String(string_ref)))
                } else {
                    let name = self.get_global_name(*global);
                    Some(Expr::Ident(name))
                }
            }
            _ => None, // Complex expressions not handled for inlining
        }
    }

    fn is_control_flow_op(&self, op_idx: usize) -> bool {
        matches!(
            &self.func.ops[op_idx],
            Opcode::JTrue { .. }
                | Opcode::JFalse { .. }
                | Opcode::JNull { .. }
                | Opcode::JNotNull { .. }
                | Opcode::JSLt { .. }
                | Opcode::JSGte { .. }
                | Opcode::JEq { .. }
                | Opcode::JNotEq { .. }
                | Opcode::JAlways { .. }
                | Opcode::Switch { .. }
        )
    }

    fn check_terminal(&self, op_idx: usize) -> Option<Statement> {
        match &self.func.ops[op_idx] {
            Opcode::Ret { ret } => {
                // Don't return void-typed values
                if self.is_void_type(*ret) {
                    Some(Statement::Return(None))
                } else {
                    Some(Statement::Return(Some(self.reg_to_expr(*ret))))
                }
            }
            Opcode::Throw { exc } => Some(Statement::Throw(self.reg_to_expr(*exc))),
            _ => None,
        }
    }

    /// Convert opcode to statement
    fn opcode_to_statement(&mut self, op_idx: usize) -> Option<Statement> {
        let op = &self.func.ops[op_idx];

        match op {
            Opcode::Label | Opcode::Nop => None,
            Opcode::JTrue { .. } | Opcode::JFalse { .. } | Opcode::JNull { .. }
            | Opcode::JNotNull { .. } | Opcode::JAlways { .. } | Opcode::Ret { .. }
            | Opcode::Throw { .. } => None,

            Opcode::Mov { dst, src } => {
                let var = self.reg_to_expr(*dst);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(var, expr))
            }

            Opcode::Int { dst, ptr } => {
                let var = self.reg_to_expr(*dst);
                let val = Expr::Constant(Constant::Int(*ptr));
                Some(self.make_assign(var, val))
            }

            Opcode::Float { dst, ptr } => {
                let var = self.reg_to_expr(*dst);
                let val = Expr::Constant(Constant::Float(*ptr));
                Some(self.make_assign(var, val))
            }

            Opcode::Bool { dst, value } => {
                let var = self.reg_to_expr(*dst);
                let val = Expr::Constant(Constant::Bool(*value));
                Some(self.make_assign(var, val))
            }

            Opcode::String { dst, ptr } => {
                let var = self.reg_to_expr(*dst);
                let val = Expr::Constant(Constant::String(*ptr));
                Some(self.make_assign(var, val))
            }

            Opcode::Null { dst } => {
                let var = self.reg_to_expr(*dst);
                let val = Expr::Constant(Constant::Null);
                Some(self.make_assign(var, val))
            }

            Opcode::Add { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Add(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::Sub { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Sub(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::Mul { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Mul(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::Incr { dst } => {
                let var = self.reg_to_expr(*dst);
                Some(Statement::ExprStatement(Expr::Op(Operation::Incr(Box::new(var)))))
            }

            Opcode::Decr { dst } => {
                let var = self.reg_to_expr(*dst);
                Some(Statement::ExprStatement(Expr::Op(Operation::Decr(Box::new(var)))))
            }

            Opcode::Field { dst, obj, field } => {
                let var = self.reg_to_expr(*dst);
                let obj_expr = self.reg_to_expr(*obj);
                let field_name = self.get_field_name(*obj, *field);
                let expr = Expr::Field(Box::new(obj_expr), field_name);
                Some(self.make_assign(var, expr))
            }

            Opcode::Call0 { dst, fun } => {
                let call = Call::new_fun(*fun, vec![]);
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::Call1 { dst, fun, arg0 } => {
                // Check for super method call: calling parent's method with same name, this as arg
                if *arg0 == Reg(0) && self.is_super_method_call(*fun) {
                    if let Some(method_name) = self.get_function_name(*fun) {
                        let call = Call::new_super_method(method_name, vec![]);
                        return Some(self.make_call_stmt(*dst, call));
                    }
                }

                let args = [*arg0];
                let call = self.try_make_method_call(*fun, &args)
                    .unwrap_or_else(|| Call::new_fun(*fun, vec![self.reg_to_expr(*arg0)]));
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::Call2 { dst, fun, arg0, arg1 } => {
                let args = [*arg0, *arg1];
                let call = self.try_make_method_call(*fun, &args)
                    .unwrap_or_else(|| Call::new_fun(*fun, vec![self.reg_to_expr(*arg0), self.reg_to_expr(*arg1)]));
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::Call3 { dst, fun, arg0, arg1, arg2 } => {
                let args = [*arg0, *arg1, *arg2];
                let call = self.try_make_method_call(*fun, &args)
                    .unwrap_or_else(|| Call::new_fun(*fun, vec![
                        self.reg_to_expr(*arg0),
                        self.reg_to_expr(*arg1),
                        self.reg_to_expr(*arg2),
                    ]));
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::Call4 { dst, fun, arg0, arg1, arg2, arg3 } => {
                let args = [*arg0, *arg1, *arg2, *arg3];
                let call = self.try_make_method_call(*fun, &args)
                    .unwrap_or_else(|| Call::new_fun(*fun, vec![
                        self.reg_to_expr(*arg0),
                        self.reg_to_expr(*arg1),
                        self.reg_to_expr(*arg2),
                        self.reg_to_expr(*arg3),
                    ]));
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::CallN { dst, fun, args } => {
                // Check for super constructor call: inside constructor, calling parent's __constructor__
                if self.is_current_function_constructor() && self.is_constructor_function(*fun) {
                    if !args.is_empty() && args[0] == Reg(0) {
                        // This is super(args...) - skip first arg (this)
                        let super_args: Vec<_> = args[1..].iter().map(|r| self.reg_to_expr(*r)).collect();
                        let call = Call::new_super(super_args);
                        return Some(self.make_call_stmt(*dst, call));
                    }
                }

                let call = self.try_make_method_call(*fun, args)
                    .unwrap_or_else(|| {
                        let arg_exprs: Vec<_> = args.iter().map(|r| self.reg_to_expr(*r)).collect();
                        Call::new_fun(*fun, arg_exprs)
                    });
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::CallMethod { dst, field, args } => {
                if args.is_empty() {
                    return Some(Statement::Comment("callmethod with no args".into()));
                }
                let obj = self.reg_to_expr(args[0]);
                // For CallMethod, 'field' is a pindex (proto index), not a field index
                let method_name = self.get_proto_name(args[0], *field);
                let method = Expr::Field(Box::new(obj), method_name);
                let arg_exprs: Vec<_> = args[1..].iter().map(|r| self.reg_to_expr(*r)).collect();
                let call = Call { fun: method, args: arg_exprs };
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::CallThis { dst, field, args } => {
                let this = Expr::Variable(Reg(0), Some("this".into()));
                let field_name = self.get_field_name(Reg(0), *field);
                let method = Expr::Field(Box::new(this), field_name);
                let arg_exprs: Vec<_> = args.iter().map(|r| self.reg_to_expr(*r)).collect();
                let call = Call { fun: method, args: arg_exprs };
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::CallClosure { dst, fun, args } => {
                let fun_expr = self.reg_to_expr(*fun);
                let arg_exprs: Vec<_> = args.iter().map(|r| self.reg_to_expr(*r)).collect();
                let call = Call { fun: fun_expr, args: arg_exprs };
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::GetGlobal { dst, global } => {
                let var = self.reg_to_expr(*dst);
                // Check if this is a string constant global
                if let Some(string_ref) = self.get_global_string_value(*global) {
                    Some(self.make_assign(var, Expr::Constant(Constant::String(string_ref))))
                } else {
                    let global_name = self.get_global_name(*global);
                    Some(self.make_assign(var, Expr::Ident(global_name)))
                }
            }

            Opcode::SetGlobal { global, src } => {
                let global_name = self.get_global_name(*global);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(Expr::Ident(global_name), expr))
            }

            Opcode::SetField { obj, field, src } => {
                let obj_expr = self.reg_to_expr(*obj);
                let field_name = self.get_field_name(*obj, *field);
                let target = Expr::Field(Box::new(obj_expr), field_name);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(target, expr))
            }

            Opcode::New { dst } => {
                let var = self.reg_to_expr(*dst);
                let type_ref = self.get_type_ref(*dst);
                // For Virtual types (anonymous objects), use empty object literal
                if matches!(&self.code.types[type_ref.0], hlbc::types::Type::Virtual { .. }) {
                    Some(self.make_assign(var, Expr::Anonymous(type_ref, HashMap::new())))
                } else {
                    // Look ahead for __constructor__ call to get constructor arguments
                    let ctor_args = self.find_constructor_args(*dst, op_idx);
                    let ctor = ConstructorCall::new(type_ref, ctor_args);
                    Some(self.make_assign(var, Expr::Constructor(ctor)))
                }
            }

            Opcode::NullCheck { reg } => {
                let var = self.reg_to_expr(*reg);
                Some(Statement::Comment(format!("nullcheck {}", self.reg_name(*reg))))
            }

            Opcode::ToVirtual { dst, src } => {
                // ToVirtual is often just a cast, emit as assignment
                let var = self.reg_to_expr(*dst);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(var, expr))
            }

            Opcode::ToSFloat { dst, src } | Opcode::ToUFloat { dst, src } => {
                // Convert int to float - emit as assignment (implicit cast in Haxe)
                let var = self.reg_to_expr(*dst);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(var, expr))
            }

            Opcode::ToInt { dst, src } => {
                // Convert float to int - emit as Std.int(src) call
                let var = self.reg_to_expr(*dst);
                let src_expr = self.reg_to_expr(*src);
                let call = Expr::Call(Box::new(Call {
                    fun: Expr::Field(Box::new(Expr::Ident("Std".into())), "int".into()),
                    args: vec![src_expr],
                }));
                Some(self.make_assign(var, call))
            }

            Opcode::ToDyn { dst, src } => {
                // Convert to Dynamic - emit as simple assignment
                let var = self.reg_to_expr(*dst);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(var, expr))
            }

            Opcode::SafeCast { dst, src } | Opcode::UnsafeCast { dst, src } => {
                // Cast to destination type - emit as simple assignment for now
                let var = self.reg_to_expr(*dst);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(var, expr))
            }

            Opcode::Rethrow { exc } => {
                Some(Statement::Throw(self.reg_to_expr(*exc)))
            }

            Opcode::SDiv { dst, a, b } | Opcode::UDiv { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Div(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::SMod { dst, a, b } | Opcode::UMod { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Mod(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::And { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::And(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::Or { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Or(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::Xor { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Xor(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::Shl { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Shl(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::SShr { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Shr(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::UShr { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                // UShr is unsigned shift right, displayed as >>> in Haxe
                let expr = Expr::Op(Operation::Shr(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::Neg { dst, src } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Neg(Box::new(self.reg_to_expr(*src))));
                Some(self.make_assign(var, expr))
            }

            Opcode::Not { dst, src } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Not(Box::new(self.reg_to_expr(*src))));
                Some(self.make_assign(var, expr))
            }

            Opcode::GetArray { dst, array, index } => {
                let var = self.reg_to_expr(*dst);
                let arr = self.reg_to_expr(*array);
                let idx = self.reg_to_expr(*index);
                let expr = Expr::Array(Box::new(arr), Box::new(idx));
                Some(self.make_assign(var, expr))
            }

            Opcode::SetArray { array, index, src } => {
                let arr = self.reg_to_expr(*array);
                let idx = self.reg_to_expr(*index);
                let target = Expr::Array(Box::new(arr), Box::new(idx));
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(target, expr))
            }

            Opcode::ArraySize { dst, array } => {
                let var = self.reg_to_expr(*dst);
                let arr = self.reg_to_expr(*array);
                let expr = Expr::Field(Box::new(arr), "length".into());
                Some(self.make_assign(var, expr))
            }

            Opcode::GetThis { dst, field } => {
                let var = self.reg_to_expr(*dst);
                let this = Expr::Variable(Reg(0), Some("this".into()));
                let field_name = self.get_field_name(Reg(0), *field);
                let expr = Expr::Field(Box::new(this), field_name);
                Some(self.make_assign(var, expr))
            }

            Opcode::SetThis { field, src } => {
                let this = Expr::Variable(Reg(0), Some("this".into()));
                let field_name = self.get_field_name(Reg(0), *field);
                let target = Expr::Field(Box::new(this), field_name);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(target, expr))
            }

            Opcode::Bytes { dst, ptr } => {
                let var = self.reg_to_expr(*dst);
                // Bytes constants are stored separately, emit as Unknown for now
                let val = Expr::Unknown(format!("bytes@{}", ptr.0));
                Some(self.make_assign(var, val))
            }

            Opcode::Ref { dst, src } => {
                // Reference - creates a pointer to a value
                let var = self.reg_to_expr(*dst);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(var, expr))
            }

            Opcode::Unref { dst, src } => {
                // Dereference - reads from a pointer
                let var = self.reg_to_expr(*dst);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(var, expr))
            }

            Opcode::Type { dst, ty } => {
                let var = self.reg_to_expr(*dst);
                let val = Expr::Constant(Constant::TypeRef(*ty));
                Some(self.make_assign(var, val))
            }

            Opcode::DynGet { dst, obj, field } => {
                let var = self.reg_to_expr(*dst);
                let obj_expr = self.reg_to_expr(*obj);
                let field_name = self.code.strings.get(field.0)
                    .cloned()
                    .unwrap_or_else(|| format!("dyn_{}", field.0).into());
                let expr = Expr::Field(Box::new(obj_expr), field_name);
                Some(self.make_assign(var, expr))
            }

            Opcode::DynSet { obj, field, src } => {
                let obj_expr = self.reg_to_expr(*obj);
                let field_name = self.code.strings.get(field.0)
                    .cloned()
                    .unwrap_or_else(|| format!("dyn_{}", field.0).into());
                let target = Expr::Field(Box::new(obj_expr), field_name);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(target, expr))
            }

            _ => Some(Statement::Comment(format!("// unhandled: {:?}", op))),
        }
    }

    fn get_global_name(&self, global: hlbc::types::RefGlobal) -> Str {
        if let Some(ty) = self.code.globals.get(global.0) {
            match &self.code.types[ty.0] {
                hlbc::types::Type::Obj(obj) => {
                    let raw_name = self.code.strings.get(obj.name.0)
                        .cloned()
                        .unwrap_or_else(|| format!("global_{}", global.0).into());
                    // Strip internal $ from type names like "haxe.$Log" -> "haxe.Log"
                    // or "$Counter" -> "Counter"
                    return self.clean_internal_name(&raw_name);
                }
                hlbc::types::Type::Struct(obj) => {
                    let raw_name = self.code.strings.get(obj.name.0)
                        .cloned()
                        .unwrap_or_else(|| format!("global_{}", global.0).into());
                    return self.clean_internal_name(&raw_name);
                }
                _ => {}
            }
        }
        format!("global_{}", global.0).into()
    }

    /// Clean internal names by stripping $ prefix from components.
    /// e.g., "haxe.$Log" -> "haxe.Log", "$Counter" -> "Counter"
    fn clean_internal_name(&self, name: &str) -> Str {
        if name.contains(".$") {
            // Replace .$X with .X
            name.replace(".$", ".").into()
        } else if name.starts_with('$') {
            // Strip leading $
            name[1..].into()
        } else {
            name.into()
        }
    }

    /// Check if a global is a string constant and return its string reference
    fn get_global_string_value(&self, global: hlbc::types::RefGlobal) -> Option<hlbc::types::RefString> {
        // Check if global has a constant initializer
        let &const_idx = self.code.globals_initializers.get(&global)?;
        let constants = self.code.constants.as_ref()?;
        let constant_def = constants.get(const_idx)?;

        // Check if the global's type is String (an Obj type named "String")
        let type_ref = self.code.globals.get(global.0)?;
        if let hlbc::types::Type::Obj(obj) = &self.code.types[type_ref.0] {
            let name = self.code.strings.get(obj.name.0)?;
            if name.as_ref() == "String" {
                // For String type, fields[0] is the string pool index
                let string_idx = constant_def.fields.first()?;
                return Some(hlbc::types::RefString(*string_idx));
            }
        }
        None
    }

    /// Convert a global reference to an expression.
    /// Checks for string constants first, otherwise returns an identifier.
    fn global_to_expr(&self, global: hlbc::types::RefGlobal) -> Expr {
        if let Some(string_ref) = self.get_global_string_value(global) {
            Expr::Constant(Constant::String(string_ref))
        } else {
            Expr::Ident(self.get_global_name(global))
        }
    }

    fn get_type_ref(&self, reg: Reg) -> RefType {
        let reg_idx = reg.0 as usize;
        if reg_idx < self.func.regs.len() {
            self.func.regs[reg_idx]
        } else {
            RefType(0) // Fallback to void type
        }
    }

    /// Try to create a method call from a function reference and arguments.
    /// If the function is a method (first arg is `this` of the owner type),
    /// returns a Call with obj.method(rest_args) syntax.
    /// Otherwise returns None and the caller should use normal function call syntax.
    fn try_make_method_call(&self, fun: RefFun, args: &[Reg]) -> Option<Call> {
        // Check if this function is a method
        let (owner_type, method_name) = self.method_info.get(&fun)?;

        // Must have at least one argument (the object)
        if args.is_empty() {
            return None;
        }

        // Check if the first argument's type matches the owner type
        let first_arg_type = self.get_type_ref(args[0]);
        if first_arg_type != *owner_type {
            return None;
        }

        // Create method call: obj.method(rest_args)
        let obj = self.reg_to_expr(args[0]);
        let method = Expr::Field(Box::new(obj), method_name.clone());
        let arg_exprs: Vec<_> = args[1..].iter().map(|r| self.reg_to_expr(*r)).collect();

        Some(Call { fun: method, args: arg_exprs })
    }

    /// Check if a register has void type (used to skip assignments of void-returning calls)
    fn is_void_type(&self, reg: Reg) -> bool {
        let type_ref = self.get_type_ref(reg);
        matches!(&self.code.types[type_ref.0], hlbc::types::Type::Void)
    }

    fn get_type_name(&self, reg: Reg) -> Str {
        let reg_idx = reg.0 as usize;
        if reg_idx < self.func.regs.len() {
            let type_ref = self.func.regs[reg_idx];
            match &self.code.types[type_ref.0] {
                hlbc::types::Type::Obj(obj) => {
                    return self.code.strings.get(obj.name.0)
                        .cloned()
                        .unwrap_or_else(|| format!("Type_{}", type_ref.0).into());
                }
                hlbc::types::Type::Struct(obj) => {
                    return self.code.strings.get(obj.name.0)
                        .cloned()
                        .unwrap_or_else(|| format!("Type_{}", type_ref.0).into());
                }
                _ => {}
            }
        }
        "Object".into()
    }

    fn get_var_name(&mut self, var: SsaVar) -> Str {
        if let Some(name) = self.var_names.get(&var) {
            return name.clone();
        }
        let name: Str = self.get_debug_name(var.reg)
            .unwrap_or_else(|| format!("v{}", { self.var_counter += 1; self.var_counter - 1 }))
            .into();
        self.var_names.insert(var, name.clone());
        name
    }

    fn reg_name(&self, reg: Reg) -> Str {
        self.get_debug_name(reg)
            .unwrap_or_else(|| format!("r{}", reg.0))
            .into()
    }

    fn get_debug_name(&self, reg: Reg) -> Option<String> {
        let reg_idx = reg.0 as usize;

        // First, check if this is a function parameter
        // Parameters are the first N registers where N = number of function args
        if let Some(Type::Fun(fun_type) | Type::Method(fun_type)) = self.code.types.get(self.func.t.0) {
            let num_args = fun_type.args.len();
            if reg_idx < num_args {
                // For instance methods, reg0 is 'this' and doesn't have an assign entry.
                // The assigns at op_idx 0 start from the first explicit parameter (reg1).
                // So we need to adjust: arg_name_pos = reg_idx - 1 for instance methods.
                //
                // Instance methods are detected by:
                // 1. Constructors (name starts with "__constructor__")
                // 2. Methods where first arg type matches parent type
                //
                // Note: Just having parent.is_some() is NOT enough - static methods
                // also have parent set (they belong to a class).
                let func_name = self.code.strings.get(self.func.name.0)
                    .map(|s| s.as_ref())
                    .unwrap_or("");
                let is_constructor = func_name.starts_with("__constructor__");

                // Check if first arg type matches parent type (indicates instance method)
                let first_arg_is_self = if let Some(parent_type) = self.func.parent {
                    !fun_type.args.is_empty() && fun_type.args[0] == parent_type
                } else {
                    false
                };

                let is_instance_method = is_constructor || first_arg_is_self;

                if reg_idx == 0 && is_instance_method {
                    // reg0 is 'this' for instance methods
                    return Some("this".to_string());
                }

                // For explicit parameters, adjust the position for arg_name
                let arg_name_pos = if is_instance_method { reg_idx - 1 } else { reg_idx };
                if let Some(name) = self.func.arg_name(self.code, arg_name_pos) {
                    if self.is_valid_identifier(&name) {
                        return Some(name.to_string());
                    }
                }
            }
        }

        // assigns is Vec<(RefString name, usize op_idx)>
        // IMPORTANT: op_idx points to the opcode AFTER the definition.
        // The actual definition is at op_idx - 1.
        // This is a HashLink convention where assigns mark scope boundaries.
        //
        // We need to find the MOST RECENT assignment to this register that happened
        // AT OR BEFORE the current opcode. This ensures we get the right variable name
        // when a register is reused for different variables.
        if let Some(assigns) = &self.func.assigns {
            let mut best_name: Option<String> = None;
            let mut best_def_idx: usize = 0;

            for (str_ref, op_idx) in assigns {
                // Skip assigns at op_idx 0 - these are parameter names, handled above
                if *op_idx == 0 {
                    continue;
                }

                // The definition is at the previous opcode
                let def_idx = op_idx.saturating_sub(1);

                // Only consider assignments at or before current_op
                if def_idx > self.current_op {
                    continue;
                }

                if def_idx < self.func.ops.len() {
                    if let Some(dst_reg) = get_opcode_dst(&self.func.ops[def_idx]) {
                        if dst_reg == reg {
                            if let Some(name) = self.code.strings.get(str_ref.0) {
                                // Validate that this looks like a real identifier
                                // (not a string constant value like "Hello.hx")
                                if self.is_valid_identifier(name) {
                                    // Keep the most recent (highest def_idx) name
                                    if best_name.is_none() || def_idx >= best_def_idx {
                                        best_name = Some(name.to_string());
                                        best_def_idx = def_idx;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            return best_name;
        }
        None
    }

    /// Check if a string is a valid Haxe identifier (not a constant value)
    fn is_valid_identifier(&self, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        // Valid identifiers: start with letter or underscore, contain only alphanumeric/underscore
        let first = name.chars().next().unwrap();
        if !first.is_alphabetic() && first != '_' {
            return false;
        }
        // Reject if contains dots, spaces, or looks like a file path
        if name.contains('.') || name.contains(' ') || name.contains('/') {
            return false;
        }
        // Must be all alphanumeric/underscore
        name.chars().all(|c| c.is_alphanumeric() || c == '_')
    }

    fn get_field_name(&self, obj_reg: Reg, field: hlbc::types::RefField) -> Str {
        // Try to look up field name from type
        let reg_idx = obj_reg.0 as usize;
        if reg_idx < self.func.regs.len() {
            let type_ref = self.func.regs[reg_idx];
            if let Some(hlbc::types::Type::Obj(obj)) = self.code.types.get(type_ref.0) {
                if let Some(f) = obj.fields.get(field.0) {
                    if let Some(name) = self.code.strings.get(f.name.0) {
                        // Empty names are interface implementation cache fields
                        if !name.is_empty() {
                            return name.clone();
                        }
                    }
                }
            }
        }
        // Fallback for unknown or unnamed fields
        format!("__field_{}", field.0).into()
    }

    /// Get method name from a proto index (pindex).
    /// Used for CallMethod where 'field' is actually a pindex into the vtable.
    fn get_proto_name(&self, obj_reg: Reg, pindex: hlbc::types::RefField) -> Str {
        let reg_idx = obj_reg.0 as usize;
        if reg_idx < self.func.regs.len() {
            let type_ref = self.func.regs[reg_idx];
            if let Some(hlbc::types::Type::Obj(obj)) = self.code.types.get(type_ref.0) {
                // Search protos for one with matching pindex
                for proto in &obj.protos {
                    if proto.pindex == pindex.0 as i32 {
                        if let Some(name) = self.code.strings.get(proto.name.0) {
                            return name.clone();
                        }
                    }
                }
            }
        }
        // Fallback
        format!("method_{}", pindex.0).into()
    }

    /// Look ahead from a New opcode to find the __constructor__ call arguments.
    /// In HashLink bytecode, object construction is split:
    ///   New reg0 = new Type
    ///   GetGlobal reg1 = global@5  // "Hello World"
    ///   Call2 __constructor__(reg0, reg1)
    /// We need to combine these into: new Type("Hello World")
    fn find_constructor_args(&self, new_dst: Reg, new_op_idx: usize) -> Vec<Expr> {
        // Search forward within the same basic block for a constructor call
        let block = self.cfg.op_to_block.get(&new_op_idx);
        let search_end = block
            .and_then(|b| Some(self.cfg.graph[*b].end))
            .unwrap_or(self.func.ops.len().saturating_sub(1));

        // Track register values from intermediate opcodes
        let mut reg_values: HashMap<Reg, Expr> = HashMap::new();

        for idx in (new_op_idx + 1)..=search_end.min(new_op_idx + 10) {
            // Search a bit further for constructor args
            if idx >= self.func.ops.len() {
                break;
            }

            let op = &self.func.ops[idx];

            // Track constant/global assignments
            match op {
                Opcode::String { dst, ptr } => {
                    reg_values.insert(*dst, Expr::Constant(Constant::String(*ptr)));
                }
                Opcode::Int { dst, ptr } => {
                    reg_values.insert(*dst, Expr::Constant(Constant::Int(*ptr)));
                }
                Opcode::Float { dst, ptr } => {
                    reg_values.insert(*dst, Expr::Constant(Constant::Float(*ptr)));
                }
                Opcode::Bool { dst, value } => {
                    reg_values.insert(*dst, Expr::Constant(Constant::Bool(*value)));
                }
                Opcode::Null { dst } => {
                    reg_values.insert(*dst, Expr::Constant(Constant::Null));
                }
                Opcode::GetGlobal { dst, global } => {
                    reg_values.insert(*dst, self.global_to_expr(*global));
                }
                _ => {}
            }

            // Helper to get expression for a register
            let get_arg_expr = |reg: Reg| -> Expr {
                reg_values.get(&reg).cloned().unwrap_or_else(|| self.reg_to_expr(reg))
            };

            match op {
                // Call2 __constructor__(obj, arg1)
                Opcode::Call2 { fun, arg0, arg1, .. } if *arg0 == new_dst => {
                    if self.is_constructor_function(*fun) {
                        return vec![get_arg_expr(*arg1)];
                    }
                }
                // Call3 __constructor__(obj, arg1, arg2)
                Opcode::Call3 { fun, arg0, arg1, arg2, .. } if *arg0 == new_dst => {
                    if self.is_constructor_function(*fun) {
                        return vec![get_arg_expr(*arg1), get_arg_expr(*arg2)];
                    }
                }
                // Call4 __constructor__(obj, arg1, arg2, arg3)
                Opcode::Call4 { fun, arg0, arg1, arg2, arg3, .. } if *arg0 == new_dst => {
                    if self.is_constructor_function(*fun) {
                        return vec![
                            get_arg_expr(*arg1),
                            get_arg_expr(*arg2),
                            get_arg_expr(*arg3),
                        ];
                    }
                }
                // CallN __constructor__(obj, args...)
                Opcode::CallN { fun, args, .. } if !args.is_empty() && args[0] == new_dst => {
                    if self.is_constructor_function(*fun) {
                        return args[1..].iter().map(|r| get_arg_expr(*r)).collect();
                    }
                }
                _ => {}
            }
        }
        vec![] // No constructor call found
    }

    /// Check if a function is a __constructor__
    fn is_constructor_function(&self, fun: RefFun) -> bool {
        if let Some(func) = fun.as_fn(self.code) {
            self.code.strings.get(func.name.0)
                .map(|s| s.starts_with("__constructor__"))
                .unwrap_or(false)
        } else {
            false
        }
    }

    /// Check if the current function being decompiled is a constructor
    fn is_current_function_constructor(&self) -> bool {
        self.code.strings.get(self.func.name.0)
            .map(|s| s.starts_with("__constructor__"))
            .unwrap_or(false)
    }

    /// Check if calling a function with this as first arg is a super method call.
    /// This is true when:
    /// 1. Current function has the same name as the target function
    /// 2. Current function is an override (has parent)
    /// 3. Target function belongs to the parent class
    fn is_super_method_call(&self, fun: RefFun) -> bool {
        // Get current function name
        let current_name = match self.code.strings.get(self.func.name.0) {
            Some(n) => n.clone(),
            None => return false,
        };

        // Get target function
        let target_func = match fun.as_fn(self.code) {
            Some(f) => f,
            None => return false,
        };

        // Get target function name
        let target_name = match self.code.strings.get(target_func.name.0) {
            Some(n) => n.clone(),
            None => return false,
        };

        // Names must match
        if current_name != target_name {
            return false;
        }

        // Current function must have a parent (be a method)
        let current_parent = match self.func.parent {
            Some(p) => p,
            None => return false,
        };

        // Target function must also have a parent
        let target_parent = match target_func.parent {
            Some(p) => p,
            None => return false,
        };

        // Target parent must be different from current parent (i.e., it's the base class)
        current_parent != target_parent
    }

    /// Get the name of a function
    fn get_function_name(&self, fun: RefFun) -> Option<Str> {
        fun.as_fn(self.code)
            .and_then(|f| self.code.strings.get(f.name.0))
            .cloned()
    }

    fn reg_to_expr(&self, reg: Reg) -> Expr {
        // NOTE: We previously checked inline_exprs here, but the lookup was broken:
        // it found ANY SSA version with matching reg, not the correct version.
        // For now, we just return a variable reference. Proper SSA-aware inlining
        // would require tracking which version is "current" at each use site.
        //
        // TODO: Implement proper SSA version tracking for expression inlining
        let name = self.reg_name(reg);
        Expr::Variable(reg, Some(name))
    }

    /// Get expression for a register, checking for constants defined in a specific block.
    /// This is useful for loop conditions where the constant may be re-assigned elsewhere.
    fn reg_to_expr_in_block(&self, reg: Reg, block: NodeIndex) -> Expr {
        // First check for constants defined in this specific block
        let blk = &self.cfg.graph[block];
        for op_idx in blk.start..=blk.end {
            match &self.func.ops[op_idx] {
                Opcode::Int { dst, ptr } if *dst == reg => {
                    return Expr::Constant(Constant::Int(*ptr));
                }
                Opcode::Float { dst, ptr } if *dst == reg => {
                    return Expr::Constant(Constant::Float(*ptr));
                }
                Opcode::Bool { dst, value } if *dst == reg => {
                    return Expr::Constant(Constant::Bool(*value));
                }
                Opcode::String { dst, ptr } if *dst == reg => {
                    return Expr::Constant(Constant::String(*ptr));
                }
                Opcode::Null { dst } if *dst == reg => {
                    return Expr::Constant(Constant::Null);
                }
                _ => {}
            }
        }
        // Fall back to normal lookup
        self.reg_to_expr(reg)
    }

    fn compute_target(&self, op_idx: usize, offset: i32) -> Option<NodeIndex> {
        let target_idx = (op_idx as i64 + offset as i64 + 1) as usize;
        self.cfg.block_for_op(target_idx)
    }
}

/// Simplify a list of statements by:
/// 1. Merging consecutive assignments (r3 = expr; r0 = r3; → r0 = expr;)
/// 2. Recursively simplifying nested blocks
fn simplify_statements(stmts: Vec<Statement>) -> Vec<Statement> {
    // First, collect all statements into a vec so we can scan ahead
    let stmts: Vec<_> = stmts.into_iter().collect();
    let mut result = Vec::new();
    let mut i = 0;

    while i < stmts.len() {
        let stmt = &stmts[i];

        // Try to merge with next statement
        if let Statement::Assign {
            declaration: decl1,
            variable: var1,
            assign: assign1,
        } = stmt
        {
            // Check if next statement is an assignment that uses our variable
            if i + 1 < stmts.len() {
                if let Statement::Assign {
                    declaration: decl2,
                    variable: var2,
                    assign: assign2,
                } = &stmts[i + 1]
                {
                    // Pattern: r3 = expr; r0 = r3; → r0 = expr;
                    // BUT: Don't merge if first statement is a declaration, as this would
                    // lose the variable declaration and leave it undeclared for later uses.
                    // ALSO: Don't merge if var1 is used later (other than in assign2)!
                    let var1_used_later = is_var_used_in_stmts(var1, &stmts[i + 2..]);
                    if is_same_expr(var1, assign2) && !is_same_expr(var1, var2) && !*decl1 && !var1_used_later {
                        // Merge: replace with var2 = assign1
                        // Field and Array accesses can NEVER be declarations
                        let is_field_or_array = matches!(var2, Expr::Field(_, _) | Expr::Array(_, _));
                        let merged = Statement::Assign {
                            declaration: if is_field_or_array { false } else { *decl2 },
                            variable: var2.clone(),
                            assign: assign1.clone(),
                        };
                        i += 2; // skip both statements
                        result.push(merged);
                        continue;
                    }
                }
            }
        }

        // Recursively simplify nested structures
        let simplified = match stmt.clone() {
            Statement::While { cond, stmts } => Statement::While {
                cond,
                stmts: simplify_statements(stmts),
            },
            Statement::IfElse { cond, if_, else_ } => Statement::IfElse {
                cond,
                if_: simplify_statements(if_),
                else_: simplify_statements(else_),
            },
            Statement::Block { stmts } => Statement::Block {
                stmts: simplify_statements(stmts),
            },
            Statement::Sequence { stmts } => Statement::Sequence {
                stmts: simplify_statements(stmts),
            },
            Statement::TryCatch {
                try_stmts,
                catch_stmts,
                catch_var,
            } => Statement::TryCatch {
                try_stmts: simplify_statements(try_stmts),
                catch_stmts: simplify_statements(catch_stmts),
                catch_var,
            },
            other => other,
        };
        result.push(simplified);
        i += 1;
    }

    result
}

/// Check if two expressions refer to the same variable
fn is_same_expr(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Variable(reg_a, _), Expr::Variable(reg_b, _)) => reg_a == reg_b,
        _ => false,
    }
}

/// Check if a variable is used anywhere in a list of statements
fn is_var_used_in_stmts(var: &Expr, stmts: &[Statement]) -> bool {
    let Expr::Variable(target_reg, _) = var else {
        return false;
    };

    for stmt in stmts {
        if is_var_used_in_stmt(target_reg, stmt) {
            return true;
        }
    }
    false
}

/// Check if a register is used in a statement (as a read, not just assignment target)
fn is_var_used_in_stmt(reg: &Reg, stmt: &Statement) -> bool {
    match stmt {
        Statement::Assign { variable: _, assign, .. } => {
            // Check if the expression uses this register
            is_var_used_in_expr(reg, assign)
        }
        Statement::ExprStatement(expr) => is_var_used_in_expr(reg, expr),
        Statement::Return(Some(expr)) => is_var_used_in_expr(reg, expr),
        Statement::Return(None) => false,
        Statement::While { cond, stmts } => {
            is_var_used_in_expr(reg, cond) || stmts.iter().any(|s| is_var_used_in_stmt(reg, s))
        }
        Statement::IfElse { cond, if_, else_ } => {
            is_var_used_in_expr(reg, cond)
                || if_.iter().any(|s| is_var_used_in_stmt(reg, s))
                || else_.iter().any(|s| is_var_used_in_stmt(reg, s))
        }
        Statement::Block { stmts } | Statement::Sequence { stmts } => {
            stmts.iter().any(|s| is_var_used_in_stmt(reg, s))
        }
        Statement::TryCatch { try_stmts, catch_stmts, .. } => {
            try_stmts.iter().any(|s| is_var_used_in_stmt(reg, s))
                || catch_stmts.iter().any(|s| is_var_used_in_stmt(reg, s))
        }
        Statement::Throw(expr) => is_var_used_in_expr(reg, expr),
        Statement::Switch { arg, default, cases, .. } => {
            is_var_used_in_expr(reg, arg)
                || default.iter().any(|s| is_var_used_in_stmt(reg, s))
                || cases.iter().any(|(_, body)| body.iter().any(|s| is_var_used_in_stmt(reg, s)))
        }
        Statement::Comment(_) | Statement::Break | Statement::Continue | Statement::VarDecl { .. } => false,
    }
}

/// Check if a register is used in an expression
fn is_var_used_in_expr(reg: &Reg, expr: &Expr) -> bool {
    match expr {
        Expr::Variable(r, _) => r == reg,
        Expr::Field(base, _) => is_var_used_in_expr(reg, base),
        Expr::Array(base, idx) => is_var_used_in_expr(reg, base) || is_var_used_in_expr(reg, idx),
        Expr::Call(call) => {
            is_var_used_in_expr(reg, &call.fun)
                || call.args.iter().any(|a| is_var_used_in_expr(reg, a))
        }
        Expr::Op(op) => is_var_used_in_operation(reg, op),
        Expr::IfElse { cond, if_, else_ } => {
            is_var_used_in_expr(reg, cond)
                || if_.iter().any(|s| is_var_used_in_stmt(reg, s))
                || else_.iter().any(|s| is_var_used_in_stmt(reg, s))
        }
        Expr::Constructor(ctor) => ctor.args.iter().any(|a| is_var_used_in_expr(reg, a)),
        Expr::Anonymous(_, fields) => fields.values().any(|v| is_var_used_in_expr(reg, v)),
        Expr::ArrayLiteral(elems) => elems.iter().any(|e| is_var_used_in_expr(reg, e)),
        Expr::EnumConstr(_, _, args) => args.iter().any(|a| is_var_used_in_expr(reg, a)),
        Expr::Closure(_, stmts) => stmts.iter().any(|s| is_var_used_in_stmt(reg, s)),
        // These don't contain variable references
        Expr::Constant(_) | Expr::Ident(_) | Expr::FunRef(_) | Expr::Unknown(_) => false,
    }
}

/// Check if a register is used in an operation
fn is_var_used_in_operation(reg: &Reg, op: &Operation) -> bool {
    use Operation::*;
    match op {
        Add(l, r) | Sub(l, r) | Mul(l, r) | Div(l, r) | Mod(l, r) |
        Shl(l, r) | Shr(l, r) | And(l, r) | Or(l, r) | Xor(l, r) |
        Eq(l, r) | NotEq(l, r) | Gt(l, r) | Gte(l, r) | Lt(l, r) | Lte(l, r) => {
            is_var_used_in_expr(reg, l) || is_var_used_in_expr(reg, r)
        }
        Neg(e) | Not(e) | Incr(e) | Decr(e) => is_var_used_in_expr(reg, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::CfgAnalysis;
    use crate::lifter::Cfg;
    use crate::ssa::SsaCfg;
    use crate::type_prop::TypePropagator;
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
