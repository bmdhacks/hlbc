//! Control Flow Graph construction for liveness analysis
//!
//! Builds a CFG from HashLink opcodes, identifying basic blocks and edges.
//!
//! NOTE: This module will be reworked in Phase 2 to use petgraph.

use std::collections::{HashMap, HashSet};

use hlbc::opcodes::Opcode;
use hlbc::types::Reg;

use super::def_use::{get_defs, get_uses};

/// A basic block in the CFG.
#[derive(Debug, Clone)]
pub struct BasicBlock {
    /// Index of first opcode in this block
    pub start: usize,
    /// Index of last opcode in this block (inclusive)
    pub end: usize,
    /// Registers defined in this block (before being used)
    pub def: HashSet<Reg>,
    /// Registers used in this block (before being defined)
    pub use_: HashSet<Reg>,
    /// Indices of successor blocks
    pub successors: Vec<usize>,
}

/// Control Flow Graph for a function.
#[derive(Debug)]
pub struct CFG {
    /// Basic blocks indexed by their start opcode index
    pub blocks: HashMap<usize, BasicBlock>,
    /// Ordered list of block start indices (for iteration)
    pub block_order: Vec<usize>,
    /// The opcodes array (reference)
    pub ops_len: usize,
}

impl CFG {
    /// Build a CFG from an opcode array.
    pub fn build(ops: &[Opcode]) -> CFG {
        if ops.is_empty() {
            return CFG {
                blocks: HashMap::new(),
                block_order: Vec::new(),
                ops_len: 0,
            };
        }

        // Step 1: Find all block boundaries
        let block_starts = find_block_starts(ops);

        // Step 2: Build blocks with def/use info
        let mut blocks = HashMap::new();
        let mut block_order: Vec<usize> = block_starts.iter().copied().collect();
        block_order.sort();

        for (idx, &start) in block_order.iter().enumerate() {
            // Find end of this block
            let end = if idx + 1 < block_order.len() {
                block_order[idx + 1] - 1
            } else {
                ops.len() - 1
            };

            // Compute def/use for this block
            let (def, use_) = compute_block_def_use(ops, start, end);

            // Find successors
            let successors = find_successors(ops, start, end, &block_starts);

            blocks.insert(
                start,
                BasicBlock {
                    start,
                    end,
                    def,
                    use_,
                    successors,
                },
            );
        }

        CFG {
            blocks,
            block_order,
            ops_len: ops.len(),
        }
    }

    /// Get the block containing a specific opcode index.
    pub fn block_for_op(&self, op_index: usize) -> Option<usize> {
        // Find the largest block start <= op_index
        let mut result = None;
        for &start in &self.block_order {
            if start <= op_index {
                result = Some(start);
            } else {
                break;
            }
        }
        result
    }
}

/// Find all opcode indices that start a new basic block.
fn find_block_starts(ops: &[Opcode]) -> HashSet<usize> {
    let mut starts = HashSet::new();
    starts.insert(0); // Function entry always starts a block

    for (i, op) in ops.iter().enumerate() {
        match op {
            // Conditional jumps: both target and fall-through start blocks
            &Opcode::JTrue { offset, .. }
            | &Opcode::JFalse { offset, .. }
            | &Opcode::JNull { offset, .. }
            | &Opcode::JNotNull { offset, .. }
            | &Opcode::JSLt { offset, .. }
            | &Opcode::JSGte { offset, .. }
            | &Opcode::JSGt { offset, .. }
            | &Opcode::JSLte { offset, .. }
            | &Opcode::JULt { offset, .. }
            | &Opcode::JUGte { offset, .. }
            | &Opcode::JNotLt { offset, .. }
            | &Opcode::JNotGte { offset, .. }
            | &Opcode::JEq { offset, .. }
            | &Opcode::JNotEq { offset, .. } => {
                // Jump target
                let target = compute_jump_target(i, offset);
                if target < ops.len() {
                    starts.insert(target);
                }
                // Fall-through
                if i + 1 < ops.len() {
                    starts.insert(i + 1);
                }
            }

            // Unconditional jump
            &Opcode::JAlways { offset } => {
                let target = compute_jump_target(i, offset);
                if target < ops.len() {
                    starts.insert(target);
                }
                // Fall-through is not taken, but may be targeted by other jumps
                if i + 1 < ops.len() {
                    starts.insert(i + 1);
                }
            }

            // Switch - multiple targets
            Opcode::Switch { offsets, end, .. } => {
                for &off in offsets {
                    let target = (i as i32 + 1 + off) as usize;
                    if target < ops.len() {
                        starts.insert(target);
                    }
                }
                let end_target = compute_jump_target(i, *end);
                if end_target < ops.len() {
                    starts.insert(end_target);
                }
                if i + 1 < ops.len() {
                    starts.insert(i + 1);
                }
            }

            // Trap - sets up exception handler
            &Opcode::Trap { offset, .. } => {
                let target = compute_jump_target(i, offset);
                if target < ops.len() {
                    starts.insert(target);
                }
                if i + 1 < ops.len() {
                    starts.insert(i + 1);
                }
            }

            // Label marks a potential jump target
            Opcode::Label => {
                starts.insert(i);
            }

            // Ret/Throw terminate blocks
            Opcode::Ret { .. } | Opcode::Throw { .. } | Opcode::Rethrow { .. } => {
                if i + 1 < ops.len() {
                    starts.insert(i + 1);
                }
            }

            _ => {}
        }
    }

    starts
}

/// Compute the def and use sets for a basic block.
fn compute_block_def_use(
    ops: &[Opcode],
    start: usize,
    end: usize,
) -> (HashSet<Reg>, HashSet<Reg>) {
    let mut def = HashSet::new();
    let mut use_ = HashSet::new();

    for i in start..=end {
        let op = &ops[i];

        // Uses that aren't yet defined count as block uses
        for r in get_uses(op) {
            if !def.contains(&r) {
                use_.insert(r);
            }
        }

        // All defs count as block defs
        for r in get_defs(op) {
            def.insert(r);
        }
    }

    (def, use_)
}

/// Find successor block indices for a block.
fn find_successors(
    ops: &[Opcode],
    _start: usize,
    end: usize,
    block_starts: &HashSet<usize>,
) -> Vec<usize> {
    let mut successors = Vec::new();
    let last_op = &ops[end];

    match last_op {
        // Conditional jumps have two successors
        &Opcode::JTrue { offset, .. }
        | &Opcode::JFalse { offset, .. }
        | &Opcode::JNull { offset, .. }
        | &Opcode::JNotNull { offset, .. }
        | &Opcode::JSLt { offset, .. }
        | &Opcode::JSGte { offset, .. }
        | &Opcode::JSGt { offset, .. }
        | &Opcode::JSLte { offset, .. }
        | &Opcode::JULt { offset, .. }
        | &Opcode::JUGte { offset, .. }
        | &Opcode::JNotLt { offset, .. }
        | &Opcode::JNotGte { offset, .. }
        | &Opcode::JEq { offset, .. }
        | &Opcode::JNotEq { offset, .. } => {
            let target = compute_jump_target(end, offset);
            if block_starts.contains(&target) {
                successors.push(target);
            }
            if end + 1 < ops.len() && block_starts.contains(&(end + 1)) {
                successors.push(end + 1);
            }
        }

        // Unconditional jump has one successor
        &Opcode::JAlways { offset } => {
            let target = compute_jump_target(end, offset);
            if block_starts.contains(&target) {
                successors.push(target);
            }
        }

        // Switch has multiple successors
        Opcode::Switch { offsets, end: e, .. } => {
            for &off in offsets {
                let target = (end as i32 + 1 + off) as usize;
                if block_starts.contains(&target) {
                    successors.push(target);
                }
            }
            let end_target = compute_jump_target(end, *e);
            if block_starts.contains(&end_target) {
                successors.push(end_target);
            }
        }

        // Trap has normal flow and exception handler
        &Opcode::Trap { offset, .. } => {
            let target = compute_jump_target(end, offset);
            if block_starts.contains(&target) {
                successors.push(target);
            }
            if end + 1 < ops.len() && block_starts.contains(&(end + 1)) {
                successors.push(end + 1);
            }
        }

        // Ret/Throw have no successors
        Opcode::Ret { .. } | Opcode::Throw { .. } | Opcode::Rethrow { .. } => {
            // No successors
        }

        // Other opcodes fall through
        _ => {
            if end + 1 < ops.len() && block_starts.contains(&(end + 1)) {
                successors.push(end + 1);
            }
        }
    }

    // Deduplicate
    successors.sort();
    successors.dedup();
    successors
}

/// Compute jump target from current index and offset.
fn compute_jump_target(current: usize, offset: i32) -> usize {
    ((current as i32) + 1 + offset) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use hlbc::types::RefInt;

    #[test]
    fn test_simple_linear() {
        // No jumps - single block
        let ops = vec![
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::Add {
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Opcode::Ret { ret: Reg(2) },
        ];

        let cfg = CFG::build(&ops);
        assert_eq!(cfg.block_order.len(), 1);
        assert_eq!(cfg.blocks[&0].start, 0);
        assert_eq!(cfg.blocks[&0].end, 3);
    }

    #[test]
    fn test_conditional_branch() {
        // if (cond) { x = 1 } else { x = 2 }
        let ops = vec![
            Opcode::Bool {
                dst: Reg(0),
                value: true,
            },
            Opcode::JFalse {
                cond: Reg(0),
                offset: 2,
            }, // jump to op 4
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(0),
            }, // then
            Opcode::JAlways { offset: 1 },        // skip else, jump to op 5
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            }, // else
            Opcode::Ret { ret: Reg(1) },
        ];

        let cfg = CFG::build(&ops);
        // Blocks at: 0, 2, 4, 5
        assert!(cfg.blocks.contains_key(&0));
        assert!(cfg.blocks.contains_key(&2));
        assert!(cfg.blocks.contains_key(&4));
        assert!(cfg.blocks.contains_key(&5));
    }

    #[test]
    fn test_def_use_simple() {
        let ops = vec![
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::Add {
                dst: Reg(1),
                a: Reg(0),
                b: Reg(2),
            },
        ];

        let cfg = CFG::build(&ops);
        let block = &cfg.blocks[&0];

        // Reg(0) is defined before use
        // Reg(1) is defined (but not used)
        // Reg(2) is used before any def in this block
        assert!(block.def.contains(&Reg(0)));
        assert!(block.def.contains(&Reg(1)));
        assert!(block.use_.contains(&Reg(2)));
        assert!(!block.use_.contains(&Reg(0))); // defined first
    }
}
