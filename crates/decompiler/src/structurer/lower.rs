//! Region-to-AST Lowering
//!
//! This module converts the Region tree (produced by the iterative reducer) into
//! the Statement AST that can be formatted as Haxe code.
//!
//! The lowering process:
//! 1. Traverses the Region tree depth-first
//! 2. For each Region variant, produces the corresponding Statement(s)
//! 3. Uses the Structurer context for:
//!    - Converting basic blocks to statements (opcode_to_statements)
//!    - Extracting conditions from conditional jumps
//!    - Variable naming and expression building
//!
//! This module implements Phase 7 of the SAILR-inspired refactoring plan.

use petgraph::graph::NodeIndex;

use hlbc::opcodes::Opcode;

use crate::ast::{Constant, Expr, Operation, Statement};
use crate::structurer::region::{LoopKind, Region};
use crate::structurer::Structurer;

/// Lower a Region tree to Statement AST.
///
/// This is the main entry point for the new structurer architecture.
/// It takes a Region (from `reduce_to_region`) and produces the final
/// Statement list that can be formatted as Haxe code.
pub fn lower_region(region: &Region, ctx: &mut LoweringContext<'_>) -> Vec<Statement> {
    match region {
        Region::Block(node) => lower_block(*node, ctx),
        Region::Sequence(regions) => lower_sequence(regions, ctx),
        Region::IfThenElse {
            cond,
            then_region,
            else_region,
            merge,
        } => lower_if_then_else(cond, then_region, else_region.as_deref(), *merge, ctx),
        Region::Loop {
            kind,
            header,
            condition,
            body,
            exit,
        } => lower_loop(kind, *header, condition.as_ref(), body, *exit, ctx),
        Region::Switch {
            selector,
            cases,
            default,
            merge,
        } => lower_switch(selector, cases, default, *merge, ctx),
        Region::Goto { target } => lower_goto(*target, ctx),
        Region::Empty => Vec::new(),
    }
}

/// Lowering context wraps the Structurer and provides access to its methods.
///
/// This indirection allows us to:
/// - Track which blocks have been lowered (to avoid re-lowering in nested regions)
/// - Manage scope depth for variable declarations
/// - Access the existing opcode_to_statements and expression building logic
pub struct LoweringContext<'a> {
    pub structurer: &'a mut Structurer<'a>,
}

impl<'a> LoweringContext<'a> {
    pub fn new(structurer: &'a mut Structurer<'a>) -> Self {
        LoweringContext { structurer }
    }

    /// Lower a basic block's opcodes to statements.
    ///
    /// This delegates to the existing opcode_to_statements logic,
    /// but skips control flow instructions (they're implicit in the Region structure).
    fn lower_block_opcodes(&mut self, node: NodeIndex) -> Vec<Statement> {
        let block = &self.structurer.cfg.graph[node];
        let mut stmts = Vec::new();

        for op_idx in block.start..=block.end {
            self.structurer.current_op = op_idx;

            // Set up SSA context for this opcode
            if let Some((dst, uses)) = self.structurer.ssa.get_instr_for_op(op_idx) {
                self.structurer.current_ssa_dst = dst;
                self.structurer.current_ssa_uses = uses.to_vec();
            } else {
                self.structurer.current_ssa_dst = None;
                self.structurer.current_ssa_uses.clear();
            }

            // Skip control flow opcodes - they're implicit in the Region structure
            if self.structurer.is_control_flow_op(op_idx) {
                continue;
            }

            // Check for terminal instructions (Ret, Throw)
            if let Some(term_stmt) = self.structurer.check_terminal(op_idx) {
                stmts.push(term_stmt);
                continue;
            }

            // Invalidate conflicting inlines before processing
            let invalidated = self.structurer.invalidate_conflicting_inlines(
                &self.structurer.func.ops[op_idx].clone(),
            );
            stmts.extend(invalidated);

            // Generate statements for this opcode
            let new_stmts = self.structurer.opcode_to_statements(op_idx);
            stmts.extend(new_stmts);
        }

        stmts
    }

    /// Extract the condition expression from a conditional block.
    ///
    /// Looks at the block's terminating jump instruction and builds
    /// the appropriate condition expression.
    fn extract_condition(&mut self, node: NodeIndex) -> Expr {
        let block = &self.structurer.cfg.graph[node];
        let last_op = &self.structurer.func.ops[block.end];

        match last_op {
            Opcode::JTrue { cond, .. } => self.structurer.reg_to_expr_in_block(*cond, node),
            Opcode::JFalse { cond, .. } => {
                let expr = self.structurer.reg_to_expr_in_block(*cond, node);
                Expr::Op(Operation::Not(Box::new(expr)))
            }
            Opcode::JNull { reg, .. } => {
                let expr = self.structurer.reg_to_expr_in_block(*reg, node);
                Expr::Op(Operation::Eq(
                    Box::new(expr),
                    Box::new(Expr::Constant(Constant::Null)),
                ))
            }
            Opcode::JNotNull { reg, .. } => {
                let expr = self.structurer.reg_to_expr_in_block(*reg, node);
                Expr::Op(Operation::NotEq(
                    Box::new(expr),
                    Box::new(Expr::Constant(Constant::Null)),
                ))
            }
            Opcode::JEq { a, b, .. } => {
                let a_expr = self.structurer.reg_to_expr_in_block(*a, node);
                let b_expr = self.structurer.reg_to_expr_in_block(*b, node);
                Expr::Op(Operation::Eq(Box::new(a_expr), Box::new(b_expr)))
            }
            Opcode::JNotEq { a, b, .. } => {
                let a_expr = self.structurer.reg_to_expr_in_block(*a, node);
                let b_expr = self.structurer.reg_to_expr_in_block(*b, node);
                Expr::Op(Operation::NotEq(Box::new(a_expr), Box::new(b_expr)))
            }
            Opcode::JSLt { a, b, .. } => {
                let a_expr = self.structurer.reg_to_expr_in_block(*a, node);
                let b_expr = self.structurer.reg_to_expr_in_block(*b, node);
                Expr::Op(Operation::Lt(Box::new(a_expr), Box::new(b_expr)))
            }
            Opcode::JSGte { a, b, .. } => {
                let a_expr = self.structurer.reg_to_expr_in_block(*a, node);
                let b_expr = self.structurer.reg_to_expr_in_block(*b, node);
                Expr::Op(Operation::Gte(Box::new(a_expr), Box::new(b_expr)))
            }
            Opcode::JSLte { a, b, .. } => {
                let a_expr = self.structurer.reg_to_expr_in_block(*a, node);
                let b_expr = self.structurer.reg_to_expr_in_block(*b, node);
                Expr::Op(Operation::Lte(Box::new(a_expr), Box::new(b_expr)))
            }
            Opcode::JSGt { a, b, .. } => {
                let a_expr = self.structurer.reg_to_expr_in_block(*a, node);
                let b_expr = self.structurer.reg_to_expr_in_block(*b, node);
                Expr::Op(Operation::Gt(Box::new(a_expr), Box::new(b_expr)))
            }
            Opcode::JULt { a, b, .. } => {
                // Unsigned comparison - treat as signed for now
                let a_expr = self.structurer.reg_to_expr_in_block(*a, node);
                let b_expr = self.structurer.reg_to_expr_in_block(*b, node);
                Expr::Op(Operation::Lt(Box::new(a_expr), Box::new(b_expr)))
            }
            Opcode::JUGte { a, b, .. } => {
                let a_expr = self.structurer.reg_to_expr_in_block(*a, node);
                let b_expr = self.structurer.reg_to_expr_in_block(*b, node);
                Expr::Op(Operation::Gte(Box::new(a_expr), Box::new(b_expr)))
            }
            Opcode::JNotLt { a, b, .. } => {
                let a_expr = self.structurer.reg_to_expr_in_block(*a, node);
                let b_expr = self.structurer.reg_to_expr_in_block(*b, node);
                Expr::Op(Operation::Gte(Box::new(a_expr), Box::new(b_expr)))
            }
            Opcode::JNotGte { a, b, .. } => {
                let a_expr = self.structurer.reg_to_expr_in_block(*a, node);
                let b_expr = self.structurer.reg_to_expr_in_block(*b, node);
                Expr::Op(Operation::Lt(Box::new(a_expr), Box::new(b_expr)))
            }
            _ => {
                // No condition found - return true (for unconditional or switch)
                Expr::Constant(Constant::Bool(true))
            }
        }
    }
}

/// Lower a basic block to statements.
fn lower_block(node: NodeIndex, ctx: &mut LoweringContext<'_>) -> Vec<Statement> {
    ctx.lower_block_opcodes(node)
}

/// Lower a sequence of regions to statements.
fn lower_sequence(regions: &[Region], ctx: &mut LoweringContext<'_>) -> Vec<Statement> {
    let mut stmts = Vec::new();
    for region in regions {
        stmts.extend(lower_region(region, ctx));
    }
    stmts
}

/// Lower an if-then-else region to statements.
fn lower_if_then_else(
    cond: &Expr,
    then_region: &Region,
    else_region: Option<&Region>,
    _merge: NodeIndex,
    ctx: &mut LoweringContext<'_>,
) -> Vec<Statement> {
    // Lower the condition block first (if it's a Block region with setup code)
    let mut stmts = Vec::new();

    // The condition might be a placeholder - try to extract from the region structure
    let actual_cond = if matches!(cond, Expr::Constant(Constant::Bool(true))) {
        // Placeholder condition - try to extract from then_region's entry
        if let Some(_entry) = then_region.entry_node() {
            // Find the predecessor that would have the conditional jump
            // For now, use the placeholder
            cond.clone()
        } else {
            cond.clone()
        }
    } else {
        cond.clone()
    };

    // Lower branches with increased scope depth
    ctx.structurer.scope_depth += 1;
    let then_stmts = lower_region(then_region, ctx);
    let else_stmts = else_region
        .map(|r| lower_region(r, ctx))
        .unwrap_or_default();
    ctx.structurer.scope_depth -= 1;

    stmts.push(Statement::IfElse {
        cond: actual_cond,
        if_: then_stmts,
        else_: else_stmts,
    });

    stmts
}

/// Lower a loop region to statements.
fn lower_loop(
    kind: &LoopKind,
    header: NodeIndex,
    condition: Option<&Expr>,
    body: &Region,
    _exit: NodeIndex,
    ctx: &mut LoweringContext<'_>,
) -> Vec<Statement> {
    let mut stmts = Vec::new();

    // Lower header block (for setup code before the condition)
    let header_stmts = ctx.lower_block_opcodes(header);

    // Get loop condition
    let loop_cond = condition.cloned().unwrap_or_else(|| {
        // Try to extract condition from header block's terminating jump
        ctx.extract_condition(header)
    });

    // Lower body with increased scope depth
    ctx.structurer.scope_depth += 1;
    let old_loop_header = ctx.structurer.current_loop_header;
    ctx.structurer.current_loop_header = Some(header);
    let body_stmts = lower_region(body, ctx);
    ctx.structurer.current_loop_header = old_loop_header;
    ctx.structurer.scope_depth -= 1;

    match kind {
        LoopKind::While => {
            if header_stmts.is_empty() {
                // Simple while(condition) { body }
                stmts.push(Statement::While {
                    cond: loop_cond,
                    stmts: body_stmts,
                });
            } else {
                // Header has setup code - use while(true) { setup; if (!cond) break; body }
                let break_cond = Expr::Op(Operation::Not(Box::new(loop_cond)));
                let mut loop_body = header_stmts;
                loop_body.push(Statement::IfElse {
                    cond: break_cond,
                    if_: vec![Statement::Break],
                    else_: vec![],
                });
                loop_body.extend(body_stmts);
                stmts.push(Statement::While {
                    cond: Expr::Constant(Constant::Bool(true)),
                    stmts: loop_body,
                });
            }
        }
        LoopKind::DoWhile => {
            // do { body } while (condition)
            // For now, emit as while(true) { body; if (!cond) break; }
            let break_cond = Expr::Op(Operation::Not(Box::new(loop_cond)));
            let mut loop_body = body_stmts;
            loop_body.push(Statement::IfElse {
                cond: break_cond,
                if_: vec![Statement::Break],
                else_: vec![],
            });
            stmts.push(Statement::While {
                cond: Expr::Constant(Constant::Bool(true)),
                stmts: loop_body,
            });
        }
        LoopKind::For { init_op, incr_op } => {
            // For loops are emitted as while loops for now
            // TODO: Detect and emit for-loop syntax in post-processing
            let _ = (init_op, incr_op); // Suppress warnings

            if header_stmts.is_empty() {
                stmts.push(Statement::While {
                    cond: loop_cond,
                    stmts: body_stmts,
                });
            } else {
                let break_cond = Expr::Op(Operation::Not(Box::new(loop_cond)));
                let mut loop_body = header_stmts;
                loop_body.push(Statement::IfElse {
                    cond: break_cond,
                    if_: vec![Statement::Break],
                    else_: vec![],
                });
                loop_body.extend(body_stmts);
                stmts.push(Statement::While {
                    cond: Expr::Constant(Constant::Bool(true)),
                    stmts: loop_body,
                });
            }
        }
        LoopKind::Endless => {
            // while(true) { body }
            let mut loop_body = header_stmts;
            loop_body.extend(body_stmts);
            stmts.push(Statement::While {
                cond: Expr::Constant(Constant::Bool(true)),
                stmts: loop_body,
            });
        }
    }

    stmts
}

/// Lower a switch region to statements.
fn lower_switch(
    selector: &Expr,
    cases: &[crate::structurer::region::SwitchCase],
    default: &Region,
    _merge: NodeIndex,
    ctx: &mut LoweringContext<'_>,
) -> Vec<Statement> {
    ctx.structurer.scope_depth += 1;

    // Lower each case
    let lowered_cases: Vec<(Vec<Expr>, Vec<Statement>)> = cases
        .iter()
        .map(|case| {
            let patterns: Vec<Expr> = case
                .patterns
                .iter()
                .map(|c| Expr::Constant(c.clone()))
                .collect();
            let body = lower_region(&case.body, ctx);
            (patterns, body)
        })
        .collect();

    // Lower default case
    let default_stmts = lower_region(default, ctx);

    ctx.structurer.scope_depth -= 1;

    vec![Statement::Switch {
        arg: selector.clone(),
        default: default_stmts,
        cases: lowered_cases,
        enum_type: None,
    }]
}

/// Lower a goto region to statements.
///
/// Gotos are the fallback for irreducible control flow.
/// We emit them as comments with target info for now.
fn lower_goto(target: NodeIndex, _ctx: &mut LoweringContext<'_>) -> Vec<Statement> {
    // For now, emit a comment indicating the goto
    // In future, we could emit actual labels and gotos if the language supports them
    vec![Statement::Comment(format!(
        "goto block_{} (irreducible control flow)",
        target.index()
    ))]
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Full integration tests require a Bytecode and Function context
    // which are complex to set up. These tests focus on the Region structure.

    #[test]
    fn test_lower_empty_region() {
        // Empty region should produce no statements
        // This is a structural test that doesn't need the full context
        let region = Region::Empty;
        assert!(region.is_empty());
    }

    #[test]
    fn test_lower_sequence_region() {
        // Test that sequence regions flatten correctly
        let n0 = petgraph::graph::NodeIndex::new(0);
        let n1 = petgraph::graph::NodeIndex::new(1);

        let region = Region::sequence(vec![Region::Block(n0), Region::Block(n1)]);

        // Should create a Sequence with 2 blocks
        match region {
            Region::Sequence(regions) => {
                assert_eq!(regions.len(), 2);
            }
            _ => panic!("Expected Sequence region"),
        }
    }

    #[test]
    fn test_lower_if_then_else_structure() {
        // Test if-then-else region structure
        let n0 = petgraph::graph::NodeIndex::new(0);
        let n1 = petgraph::graph::NodeIndex::new(1);
        let n2 = petgraph::graph::NodeIndex::new(2);

        let region = Region::if_then_else(
            Expr::Constant(Constant::Bool(true)),
            Region::Block(n0),
            Some(Region::Block(n1)),
            n2,
        );

        match region {
            Region::IfThenElse {
                then_region,
                else_region,
                merge,
                ..
            } => {
                assert!(matches!(*then_region, Region::Block(_)));
                assert!(else_region.is_some());
                assert_eq!(merge, n2);
            }
            _ => panic!("Expected IfThenElse region"),
        }
    }

    #[test]
    fn test_lower_loop_structure() {
        // Test loop region structure
        let header = petgraph::graph::NodeIndex::new(0);
        let body = petgraph::graph::NodeIndex::new(1);
        let exit = petgraph::graph::NodeIndex::new(2);

        let region = Region::while_loop(
            header,
            Expr::Constant(Constant::Bool(true)),
            Region::Block(body),
            exit,
        );

        match region {
            Region::Loop {
                kind,
                header: h,
                exit: e,
                ..
            } => {
                assert_eq!(kind, LoopKind::While);
                assert_eq!(h, header);
                assert_eq!(e, exit);
            }
            _ => panic!("Expected Loop region"),
        }
    }
}
