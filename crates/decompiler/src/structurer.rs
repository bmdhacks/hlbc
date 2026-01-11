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
use hlbc::types::{Function, Reg};
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
        }
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
                // Switch - emit as comment for now
                stmts.push(Statement::Comment("switch statement".into()));
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
                        stmts.push(Statement::VarDecl { name });
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
                let call = Call::new_fun(*fun, vec![self.reg_to_expr(*arg0)]);
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::Call2 { dst, fun, arg0, arg1 } => {
                let call = Call::new_fun(*fun, vec![self.reg_to_expr(*arg0), self.reg_to_expr(*arg1)]);
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::Call3 { dst, fun, arg0, arg1, arg2 } => {
                let call = Call::new_fun(*fun, vec![
                    self.reg_to_expr(*arg0),
                    self.reg_to_expr(*arg1),
                    self.reg_to_expr(*arg2),
                ]);
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::Call4 { dst, fun, arg0, arg1, arg2, arg3 } => {
                let call = Call::new_fun(*fun, vec![
                    self.reg_to_expr(*arg0),
                    self.reg_to_expr(*arg1),
                    self.reg_to_expr(*arg2),
                    self.reg_to_expr(*arg3),
                ]);
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::CallN { dst, fun, args } => {
                let arg_exprs: Vec<_> = args.iter().map(|r| self.reg_to_expr(*r)).collect();
                let call = Call::new_fun(*fun, arg_exprs);
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::CallMethod { dst, field, args } => {
                if args.is_empty() {
                    return Some(Statement::Comment("callmethod with no args".into()));
                }
                let obj = self.reg_to_expr(args[0]);
                let field_name = self.get_field_name(args[0], *field);
                let method = Expr::Field(Box::new(obj), field_name);
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
                    let ctor = ConstructorCall::new(type_ref, vec![]);
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

    fn get_type_ref(&self, reg: Reg) -> RefType {
        let reg_idx = reg.0 as usize;
        if reg_idx < self.func.regs.len() {
            self.func.regs[reg_idx]
        } else {
            RefType(0) // Fallback to void type
        }
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
        // assigns is Vec<(RefString name, usize op_idx)>
        // IMPORTANT: op_idx points to the opcode AFTER the definition.
        // The actual definition is at op_idx - 1.
        // This is a HashLink convention where assigns mark scope boundaries.
        if let Some(assigns) = &self.func.assigns {
            for (str_ref, op_idx) in assigns {
                // The definition is at the previous opcode
                let def_idx = op_idx.saturating_sub(1);
                if def_idx < self.func.ops.len() {
                    if let Some(dst_reg) = get_opcode_dst(&self.func.ops[def_idx]) {
                        if dst_reg == reg {
                            if let Some(name) = self.code.strings.get(str_ref.0) {
                                // Validate that this looks like a real identifier
                                // (not a string constant value like "Hello.hx")
                                if self.is_valid_identifier(name) {
                                    return Some(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
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
                        return name.clone();
                    }
                }
            }
        }
        format!("field_{}", field.0).into()
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
    let mut result = Vec::new();
    let mut iter = stmts.into_iter().peekable();

    while let Some(stmt) = iter.next() {
        // Try to merge with next statement
        if let Statement::Assign {
            declaration: decl1,
            variable: var1,
            assign: assign1,
        } = &stmt
        {
            // Check if next statement is an assignment that uses our variable
            if let Some(Statement::Assign {
                declaration: decl2,
                variable: var2,
                assign: assign2,
            }) = iter.peek()
            {
                // Pattern: r3 = expr; r0 = r3; → r0 = expr;
                if is_same_expr(var1, assign2) && !is_same_expr(var1, var2) {
                    // Merge: replace with var2 = assign1
                    // Field and Array accesses can NEVER be declarations
                    let is_field_or_array = matches!(var2, Expr::Field(_, _) | Expr::Array(_, _));
                    let merged = Statement::Assign {
                        declaration: if is_field_or_array { false } else { *decl1 || *decl2 },
                        variable: var2.clone(),
                        assign: assign1.clone(),
                    };
                    iter.next(); // consume the second statement
                    result.push(merged);
                    continue;
                }
            }
        }

        // Recursively simplify nested structures
        let simplified = match stmt {
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
