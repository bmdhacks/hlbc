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
use crate::ast::{Call, Constant, Expr, Operation, Statement};
use crate::lifter::Cfg;
use crate::ssa::{SsaCfg, SsaInstr, SsaVar};
use crate::type_prop::TypeInfo;

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
        }
    }

    /// Structure the entire function into statements
    pub fn structure(&mut self) -> Vec<Statement> {
        self.structure_from(self.cfg.entry, None)
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
                let cond_var = self.reg_to_expr(*reg);
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
                let a_expr = self.reg_to_expr(*a);
                let b_expr = self.reg_to_expr(*b);

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
        succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
    ) -> Vec<Statement> {
        let block_data = &self.cfg.graph[block];
        let last_op = &self.func.ops[block_data.end];

        let (condition, then_target, else_target) = self.extract_condition(block_data.end, last_op);

        // Find merge point
        let merge = self.find_merge_point(then_target, else_target);

        // Structure branches
        let then_stmts = if let Some(t) = then_target {
            if Some(t) != merge && !self.processed.contains(&t) {
                self.structure_from(t, merge)
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        let else_stmts = if let Some(e) = else_target {
            if Some(e) != merge && !self.processed.contains(&e) {
                self.structure_from(e, merge)
            } else {
                vec![]
            }
        } else {
            vec![]
        };

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

    /// Structure a single basic block into statements
    fn structure_block(&mut self, node: NodeIndex) -> Vec<Statement> {
        let block = &self.cfg.graph[node];
        let mut stmts = Vec::new();

        // Process φ-functions
        if let Some(ssa_block) = self.ssa.blocks.get(&node) {
            for phi in &ssa_block.phis {
                if let SsaInstr::Phi { dst, .. } = phi {
                    let name = self.get_var_name(*dst);
                    stmts.push(Statement::VarDecl { name });
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
            Opcode::Ret { ret } => Some(Statement::Return(Some(self.reg_to_expr(*ret)))),
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
                Some(Statement::Assign { declaration: false, variable: var, assign: expr })
            }

            Opcode::Int { dst, ptr } => {
                let var = self.reg_to_expr(*dst);
                let val = Expr::Constant(Constant::Int(*ptr));
                Some(Statement::Assign { declaration: false, variable: var, assign: val })
            }

            Opcode::Float { dst, ptr } => {
                let var = self.reg_to_expr(*dst);
                let val = Expr::Constant(Constant::Float(*ptr));
                Some(Statement::Assign { declaration: false, variable: var, assign: val })
            }

            Opcode::Bool { dst, value } => {
                let var = self.reg_to_expr(*dst);
                let val = Expr::Constant(Constant::Bool(*value));
                Some(Statement::Assign { declaration: false, variable: var, assign: val })
            }

            Opcode::String { dst, ptr } => {
                let var = self.reg_to_expr(*dst);
                let val = Expr::Constant(Constant::String(*ptr));
                Some(Statement::Assign { declaration: false, variable: var, assign: val })
            }

            Opcode::Null { dst } => {
                let var = self.reg_to_expr(*dst);
                let val = Expr::Constant(Constant::Null);
                Some(Statement::Assign { declaration: false, variable: var, assign: val })
            }

            Opcode::Add { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Add(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(Statement::Assign { declaration: false, variable: var, assign: expr })
            }

            Opcode::Sub { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Sub(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(Statement::Assign { declaration: false, variable: var, assign: expr })
            }

            Opcode::Mul { dst, a, b } => {
                let var = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Mul(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(Statement::Assign { declaration: false, variable: var, assign: expr })
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
                Some(Statement::Assign { declaration: false, variable: var, assign: expr })
            }

            Opcode::Call0 { dst, fun } => {
                let var = self.reg_to_expr(*dst);
                let call = Call::new_fun(*fun, vec![]);
                Some(Statement::Assign { declaration: false, variable: var, assign: Expr::Call(Box::new(call)) })
            }

            Opcode::Call1 { dst, fun, arg0 } => {
                let var = self.reg_to_expr(*dst);
                let call = Call::new_fun(*fun, vec![self.reg_to_expr(*arg0)]);
                Some(Statement::Assign { declaration: false, variable: var, assign: Expr::Call(Box::new(call)) })
            }

            Opcode::Call2 { dst, fun, arg0, arg1 } => {
                let var = self.reg_to_expr(*dst);
                let call = Call::new_fun(*fun, vec![self.reg_to_expr(*arg0), self.reg_to_expr(*arg1)]);
                Some(Statement::Assign { declaration: false, variable: var, assign: Expr::Call(Box::new(call)) })
            }

            _ => Some(Statement::Comment(format!("// unhandled: {:?}", op))),
        }
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
        if let Some(assigns) = &self.func.assigns {
            for (str_ref, reg_idx) in assigns {
                if *reg_idx as u32 == reg.0 {
                    return self.code.strings.get(str_ref.0).map(|s| s.to_string());
                }
            }
        }
        None
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
        let name = self.reg_name(reg);
        Expr::Variable(reg, Some(name))
    }

    fn compute_target(&self, op_idx: usize, offset: i32) -> Option<NodeIndex> {
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
