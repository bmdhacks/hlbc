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
use hlbc::types::{Reg, Type};

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
            cond_block,
            then_region,
            else_region,
            merge,
            negated,
        } => lower_if_then_else(cond, *cond_block, then_region, else_region.as_deref(), *merge, *negated, ctx),
        Region::Loop {
            kind,
            header,
            condition,
            body,
            exit,
        } => lower_loop(kind, *header, condition.as_ref(), body, *exit, ctx),
        Region::Switch {
            selector,
            selector_block,
            cases,
            default,
            merge,
        } => lower_switch(selector, *selector_block, cases, default, *merge, ctx),
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

        if std::env::var("HLBC_DEBUG_LOWER").is_ok() {
            eprintln!("DEBUG lower_block_opcodes: node={:?}, ops {}..={}", node, block.start, block.end);
            for op_idx in block.start..=block.end {
                eprintln!("  op {}: {:?}", op_idx, self.structurer.func.ops[op_idx]);
            }
        }

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
    /// Extract the condition expression from a conditional block.
    ///
    /// Looks at the block's terminating jump instruction and builds
    /// the appropriate condition expression.
    fn extract_condition(&mut self, node: NodeIndex) -> Expr {
        let block = &self.structurer.cfg.graph[node];
        let last_op = &self.structurer.func.ops[block.end];

        let result = match last_op {
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
                // No conditional jump found - this block ends with an unconditional
                // jump or other terminator. Return true as a fallback.
                // NOTE: This is expected for JAlways, Switch, Ret, Throw, etc.
                // It's only a problem if the caller expected a real condition.
                Expr::Constant(Constant::Bool(true))
            }
        };

        // DEBUG: Log when we return a placeholder condition from a block
        // that looks like it should have had a real condition.
        // This helps catch cases where we're extracting from the wrong block.
        #[cfg(debug_assertions)]
        if matches!(result, Expr::Constant(Constant::Bool(true))) {
            // Check if this block actually ends with a conditional jump
            let is_conditional = matches!(
                last_op,
                Opcode::JTrue { .. }
                    | Opcode::JFalse { .. }
                    | Opcode::JNull { .. }
                    | Opcode::JNotNull { .. }
                    | Opcode::JEq { .. }
                    | Opcode::JNotEq { .. }
                    | Opcode::JSLt { .. }
                    | Opcode::JSGte { .. }
                    | Opcode::JSLte { .. }
                    | Opcode::JSGt { .. }
                    | Opcode::JULt { .. }
                    | Opcode::JUGte { .. }
                    | Opcode::JNotLt { .. }
                    | Opcode::JNotGte { .. }
            );
            debug_assert!(
                !is_conditional,
                "extract_condition returned placeholder for block {:?} which has conditional jump {:?}",
                node,
                last_op
            );
        }

        result
    }
}

/// Lower a basic block to statements.
///
/// This handles normal opcodes via `lower_block_opcodes`, but also detects
/// break and continue statements by checking if the block ends with a JAlways
/// that jumps to the loop exit (break) or loop header (continue).
fn lower_block(node: NodeIndex, ctx: &mut LoweringContext<'_>) -> Vec<Statement> {
    let mut stmts = ctx.lower_block_opcodes(node);

    // Check if this block ends with a JAlways that represents break/continue
    if let Some(header) = ctx.structurer.current_loop_header {
        let block = &ctx.structurer.cfg.graph[node];
        let last_op = &ctx.structurer.func.ops[block.end];

        if let Opcode::JAlways { offset } = last_op {
            // Compute target address
            let target_addr = (block.end as i64 + *offset as i64 + 1) as usize;

            // Find the target CFG node
            if let Some(&target_node) = ctx.structurer.cfg.op_to_block.get(&target_addr) {
                // Check if target is the loop header → continue
                if target_node == header {
                    stmts.push(Statement::Continue);
                }
                // Check if target is outside the loop → break
                else if let Some(loop_info) = ctx
                    .structurer
                    .analysis
                    .loops
                    .iter()
                    .find(|l| l.header == header)
                {
                    if !loop_info.body.contains(&target_node) {
                        stmts.push(Statement::Break);
                    }
                }
            }
        }
    }

    stmts
}

/// Lower a sequence of regions to statements.
fn lower_sequence(regions: &[Region], ctx: &mut LoweringContext<'_>) -> Vec<Statement> {
    let mut stmts = Vec::new();
    for region in regions {
        stmts.extend(lower_region(region, ctx));
    }

    // Check if we need to emit a fallthrough for the last element's merge.
    // This handles the case where multiple if-then-else patterns share an exit block:
    // - The exit block might be emitted INSIDE one pattern's then_region
    // - But when other patterns' conditions fail, they should fall through to that exit
    // - If the last pattern's merge is an exit block, emit it as a fallthrough
    if let Some(last) = regions.last() {
        if let Some(merge_block) = find_innermost_exit_merge(last, ctx) {
            // Check if this merge block is already emitted as a standalone Block in the sequence
            let already_standalone = regions.iter().any(|r| {
                matches!(r, Region::Block(b) if *b == merge_block)
            });

            if !already_standalone {
                // Check if the last statement already returns (avoid double return)
                let last_stmt_returns = stmts.last().map_or(false, |s| {
                    matches!(s, Statement::Return(_))
                });

                if !last_stmt_returns {
                    // Emit the merge block as a fallthrough
                    let merge_stmts = lower_block(merge_block, ctx);
                    stmts.extend(merge_stmts);
                }
            }
        }
    }

    stmts
}

/// Find the innermost merge point of an IfThenElse that's an exit block.
/// Returns None if no such merge exists.
fn find_innermost_exit_merge(region: &Region, ctx: &LoweringContext<'_>) -> Option<NodeIndex> {
    match region {
        Region::IfThenElse {
            then_region,
            else_region,
            merge,
            ..
        } => {
            // Check if the then_region has a deeper exit merge
            if let Some(inner) = find_innermost_exit_merge(then_region, ctx) {
                return Some(inner);
            }
            // Check if the else_region has a deeper exit merge
            if let Some(else_r) = else_region {
                if let Some(inner) = find_innermost_exit_merge(else_r, ctx) {
                    return Some(inner);
                }
            }
            // Check if this merge is an exit block
            let is_exit = ctx.structurer.cfg.graph[*merge].is_exit;
            if is_exit && else_region.is_none() {
                // Only return merge if there's no else (meaning control can fall through to merge)
                // and the then_region terminates (so we don't unreachably emit the merge)
                let then_terminates = then_region.terminates(&ctx.structurer.cfg);
                if then_terminates {
                    return Some(*merge);
                }
            }
            None
        }
        Region::Sequence(regions) => {
            // Check the last element of the sequence
            regions.last().and_then(|r| find_innermost_exit_merge(r, ctx))
        }
        _ => None,
    }
}

/// Lower an if-then-else region to statements.
fn lower_if_then_else(
    _cond: &Expr,
    cond_block: Option<NodeIndex>,
    then_region: &Region,
    else_region: Option<&Region>,
    merge: NodeIndex,
    negated: bool,
    ctx: &mut LoweringContext<'_>,
) -> Vec<Statement> {
    // INVARIANT: cond_block should be present for well-formed if-then-else regions
    debug_assert!(
        cond_block.is_some(),
        "lower_if_then_else: cond_block is None, cannot extract condition"
    );

    if std::env::var("HLBC_DEBUG_LOWER").is_ok() {
        eprintln!("DEBUG lower_if_then_else: cond_block={:?}, merge={:?}, negated={}",
            cond_block, merge, negated);
    }

    let mut stmts = Vec::new();

    // Lower the condition block's preamble (non-control-flow opcodes) first.
    // This ensures any setup code runs before the if-statement.
    if let Some(block) = cond_block {
        let preamble = ctx.lower_block_opcodes(block);
        if std::env::var("HLBC_DEBUG_LOWER").is_ok() && !preamble.is_empty() {
            eprintln!("  preamble has {} statements", preamble.len());
        }
        stmts.extend(preamble);
    }

    // Extract the actual condition from the block's terminating conditional jump.
    // This replaces the placeholder condition from the Region.
    let mut actual_cond = if let Some(block) = cond_block {
        ctx.extract_condition(block)
    } else {
        // No condition block - should not happen in well-formed regions,
        // but fall back to true if it does.
        Expr::Constant(Constant::Bool(true))
    };

    // If the branches were swapped during structuring (empty-then normalization),
    // negate the condition to maintain correct semantics.
    if negated {
        actual_cond = Expr::Op(Operation::Not(Box::new(actual_cond)));
    }

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

    // Increment scope depth BEFORE processing header and body.
    // The header is inside the loop (runs each iteration), so variables declared
    // there should be hoisted to function scope, not declared inline.
    ctx.structurer.scope_depth += 1;
    let old_loop_header = ctx.structurer.current_loop_header;
    ctx.structurer.current_loop_header = Some(header);

    // Lower header block (for setup code before the condition)
    let header_stmts = ctx.lower_block_opcodes(header);

    // Get loop condition
    let loop_cond = condition.cloned().unwrap_or_else(|| {
        // Try to extract condition from header block's terminating jump
        ctx.extract_condition(header)
    });

    // Lower body (already at increased scope depth)
    let body_stmts = lower_region(body, ctx);

    ctx.structurer.current_loop_header = old_loop_header;
    ctx.structurer.scope_depth -= 1;

    // Note on loop condition semantics:
    // extract_condition() returns the condition for the TRUE branch (jump taken).
    // For while loops, the TRUE branch typically goes to EXIT (when exit condition is true).
    // So loop_cond is the EXIT condition, and the CONTINUE condition is !loop_cond.
    //
    // For `while (continue_cond) { body }`: use !exit_cond (negate to get continue condition)
    // For `while (true) { if (exit_cond) break; body }`: use exit_cond directly
    let continue_cond = Expr::Op(Operation::Not(Box::new(loop_cond.clone())));
    let exit_cond = loop_cond;

    match kind {
        LoopKind::While => {
            if header_stmts.is_empty() {
                // Simple while(continue_condition) { body }
                stmts.push(Statement::While {
                    cond: continue_cond,
                    stmts: body_stmts,
                });
            } else {
                // Header has setup code - use while(true) { setup; if (exit_cond) break; body }
                let mut loop_body = header_stmts;
                loop_body.push(Statement::IfElse {
                    cond: exit_cond,
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
            // do { body } while (continue_condition)
            // Emit as while(true) { body; if (exit_cond) break; }
            let mut loop_body = body_stmts;
            loop_body.push(Statement::IfElse {
                cond: exit_cond,
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
                    cond: continue_cond,
                    stmts: body_stmts,
                });
            } else {
                let mut loop_body = header_stmts;
                loop_body.push(Statement::IfElse {
                    cond: exit_cond,
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
        LoopKind::ForIn { iterator_reg, value_reg, next_op, iterator_init_op } => {
            // For-in iterator loop: `for (value in collection) { body }`
            //
            // If we have iterator_init_op, we can recover the collection expression
            // and emit a proper for-in loop. Otherwise, fall back to while loop.

            if let Some(init_op) = iterator_init_op {
                // Try to extract the collection expression from the iterator init
                if let Some(collection_expr) = extract_collection_expr(*init_op, ctx) {
                    // Get the variable name from value_reg
                    let var_name = ctx.structurer.reg_name(*value_reg);

                    // Filter out the .next() assignment from the body
                    let filtered_body: Vec<Statement> = body_stmts
                        .into_iter()
                        .filter(|stmt| !is_next_assignment(stmt, *value_reg))
                        .collect();

                    stmts.push(Statement::ForIn {
                        var_name,
                        iterable: collection_expr,
                        stmts: filtered_body,
                    });

                    // Suppress unused warnings
                    let _ = (iterator_reg, next_op);

                    return stmts;
                }
            }

            // Fallback: emit as while loop (keeping .next() call)
            // NOTE: We do NOT filter out the .next() assignment because we're
            // emitting as a while loop, not a proper for-in. The .next() call
            // is essential to advance the iterator.
            let _ = (iterator_reg, value_reg, next_op, iterator_init_op);

            // For hasNext(), the condition IS the continue condition (true = continue)
            // so we need to negate for break check
            if header_stmts.is_empty() {
                stmts.push(Statement::While {
                    cond: continue_cond,
                    stmts: body_stmts,
                });
            } else {
                let mut loop_body = header_stmts;
                loop_body.push(Statement::IfElse {
                    cond: exit_cond,
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
    selector_block: Option<NodeIndex>,
    cases: &[crate::structurer::region::SwitchCase],
    default: &Region,
    _merge: NodeIndex,
    ctx: &mut LoweringContext<'_>,
) -> Vec<Statement> {
    use hlbc::opcodes::Opcode;

    let mut result = Vec::new();
    ctx.structurer.scope_depth += 1;

    // Extract proper selector expression from the selector block
    // Also emit any statements from the selector block that come before the Switch
    let switch_arg = if let Some(block_idx) = selector_block {
        // Get the block and find the Switch opcode
        let block = &ctx.structurer.cfg.graph[block_idx];
        let switch_op_idx = block.end;
        let switch_op = &ctx.structurer.func.ops[switch_op_idx];

        // First, emit statements from the selector block (except the Switch itself)
        // These are the setup statements (like var x = 2) before the switch
        for op_idx in block.start..block.end {
            ctx.structurer.current_op = op_idx;
            if let Some((dst, uses)) = ctx.structurer.ssa.get_instr_for_op(op_idx) {
                ctx.structurer.current_ssa_dst = dst;
                ctx.structurer.current_ssa_uses = uses.to_vec();
            } else {
                ctx.structurer.current_ssa_dst = None;
                ctx.structurer.current_ssa_uses.clear();
            }
            // Skip control flow instructions
            let op = &ctx.structurer.func.ops[op_idx];
            if !matches!(op, Opcode::Switch { .. } | Opcode::JTrue { .. } | Opcode::JFalse { .. } | Opcode::JAlways { .. }) {
                let stmts = ctx.structurer.opcode_to_statements(op_idx);
                result.extend(stmts);
            }
        }

        if let Opcode::Switch { reg, .. } = switch_op {
            // Set up SSA context for proper register naming
            ctx.structurer.current_op = switch_op_idx;
            if let Some((ssa_dst, ssa_uses)) = ctx.structurer.ssa.get_instr_for_op(switch_op_idx) {
                ctx.structurer.current_ssa_dst = ssa_dst;
                ctx.structurer.current_ssa_uses = ssa_uses.clone();
            } else {
                ctx.structurer.current_ssa_dst = None;
                ctx.structurer.current_ssa_uses.clear();
            }
            // Build proper expression using reg_to_expr
            ctx.structurer.reg_to_expr(*reg)
        } else {
            // Fallback to the stored selector if Switch opcode not found
            selector.clone()
        }
    } else {
        // No block info, use stored selector
        selector.clone()
    };

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
    let mut default_stmts = lower_region(default, ctx);

    // If default is empty and all cases terminate (return/throw), add a synthetic
    // default return to satisfy Haxe's type checker for exhaustive switches.
    if default_stmts.is_empty() && !lowered_cases.is_empty() {
        let all_cases_terminate = lowered_cases.iter().all(|(_, stmts)| {
            stmts.last().map_or(false, |s| {
                matches!(s, Statement::Return(_) | Statement::Throw(_))
            })
        });
        if all_cases_terminate {
            // Get function return type and create appropriate default
            let ret_type = ctx.structurer.func.ty(ctx.structurer.code).ret;
            let default_expr = default_for_type(ret_type, ctx.structurer.code);
            default_stmts.push(Statement::Return(Some(default_expr)));
        }
    }

    ctx.structurer.scope_depth -= 1;

    // Add the switch statement to the result
    result.push(Statement::Switch {
        arg: switch_arg,
        default: default_stmts,
        cases: lowered_cases,
        enum_type: None,
    });

    result
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

/// Check if a statement is an assignment to the value register from .next() call.
/// Used to filter out the iterator next() assignment from for-in loop bodies
/// when emitting proper `for (x in collection)` statements.
fn is_next_assignment(stmt: &Statement, value_reg: Reg) -> bool {
    match stmt {
        Statement::Assign { variable, .. } => {
            // Check if the variable is a Reg matching value_reg
            if let Expr::Variable(reg, _) = variable {
                *reg == value_reg
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Extract the collection/iterator expression from an iterator initialization opcode.
///
/// Given an opcode like `it = map.keys()` or `it = collection.iterator()`,
/// extracts the full iterator expression (e.g., `map.keys()` or `collection.iterator()`).
/// This preserves the method call so the for-in loop shows the proper iterable.
fn extract_collection_expr(init_op_idx: usize, ctx: &mut LoweringContext<'_>) -> Option<Expr> {
    use crate::ast::Call;

    let op = &ctx.structurer.func.ops[init_op_idx].clone();

    // Set up SSA context for proper register resolution
    ctx.structurer.current_op = init_op_idx;
    if let Some((dst, uses)) = ctx.structurer.ssa.get_instr_for_op(init_op_idx) {
        ctx.structurer.current_ssa_dst = dst;
        ctx.structurer.current_ssa_uses = uses.to_vec();
    } else {
        ctx.structurer.current_ssa_dst = None;
        ctx.structurer.current_ssa_uses.clear();
    }

    match &op {
        // Call1: it = fn(collection) - build the full call expression
        // e.g., map.keys() where fun is the keys function
        Opcode::Call1 { fun, arg0, .. } => {
            let obj_expr = ctx.structurer.reg_to_expr(*arg0);
            let method_name = fun.name(ctx.structurer.code);
            Some(Expr::Call(Box::new(Call::new(
                Expr::Field(Box::new(obj_expr), method_name),
                vec![],
            ))))
        }
        // CallMethod: it = collection.keys() - build the full call expression
        Opcode::CallMethod { field, args, .. } if !args.is_empty() => {
            let obj_expr = ctx.structurer.reg_to_expr(args[0]);
            let method_name = ctx.structurer.get_field_name(args[0], *field);
            Some(Expr::Call(Box::new(Call::new(
                Expr::Field(Box::new(obj_expr), method_name),
                vec![],
            ))))
        }
        _ => None,
    }
}

/// Return a default expression for a given type.
/// Used for synthetic default cases in exhaustive switches.
fn default_for_type(type_ref: hlbc::types::RefType, code: &hlbc::Bytecode) -> Expr {
    match code.types.get(type_ref.0) {
        Some(Type::I32) | Some(Type::I64) | Some(Type::UI8) | Some(Type::UI16) => {
            Expr::Constant(Constant::InlineInt(0))
        }
        Some(Type::F32) | Some(Type::F64) => Expr::Constant(Constant::Float(hlbc::types::RefFloat(0))),
        Some(Type::Bool) => Expr::Constant(Constant::Bool(false)),
        _ => Expr::Constant(Constant::Null),
    }
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
        let cond_node = petgraph::graph::NodeIndex::new(3);

        let region = Region::if_then_else(
            Expr::Constant(Constant::Bool(true)),
            Some(cond_node),
            Region::Block(n0),
            Some(Region::Block(n1)),
            n2,
        );

        match region {
            Region::IfThenElse {
                cond_block,
                then_region,
                else_region,
                merge,
                ..
            } => {
                assert!(matches!(*then_region, Region::Block(_)));
                assert!(else_region.is_some());
                assert_eq!(merge, n2);
                assert_eq!(cond_block, Some(cond_node));
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
