//! Dataflow analysis for register liveness
//!
//! Computes live_in and live_out sets for each basic block using
//! the standard backward dataflow algorithm.
//!
//! ## Algorithm
//!
//! ```text
//! repeat until no changes:
//!     for each block B in reverse postorder:
//!         live_out[B] = union(live_in[S] for S in successors(B))
//!         live_in[B] = use[B] ∪ (live_out[B] - def[B])
//! ```

use std::collections::{HashMap, HashSet};

use hlbc::types::Reg;

use super::cfg::CFG;

/// Liveness information for all blocks.
#[derive(Debug)]
pub struct LivenessInfo {
    /// Registers live at the start of each block
    pub live_in: HashMap<usize, HashSet<Reg>>,
    /// Registers live at the end of each block
    pub live_out: HashMap<usize, HashSet<Reg>>,
}

/// Compute liveness information for a CFG.
pub fn compute_liveness(cfg: &CFG) -> LivenessInfo {
    let mut live_in: HashMap<usize, HashSet<Reg>> = HashMap::new();
    let mut live_out: HashMap<usize, HashSet<Reg>> = HashMap::new();

    // Initialize all to empty sets
    for &block_start in &cfg.block_order {
        live_in.insert(block_start, HashSet::new());
        live_out.insert(block_start, HashSet::new());
    }

    // Fixed-point iteration
    // We iterate backwards through blocks since liveness flows backward
    let mut changed = true;
    let mut iterations = 0;
    const MAX_ITERATIONS: usize = 1000;

    while changed && iterations < MAX_ITERATIONS {
        changed = false;
        iterations += 1;

        // Process blocks in reverse order (approximate reverse postorder)
        for &block_start in cfg.block_order.iter().rev() {
            let block = &cfg.blocks[&block_start];

            // live_out = union of live_in of all successors
            let mut new_live_out = HashSet::new();
            for &succ_start in &block.successors {
                if let Some(succ_live_in) = live_in.get(&succ_start) {
                    for r in succ_live_in {
                        new_live_out.insert(*r);
                    }
                }
            }

            // live_in = use ∪ (live_out - def)
            let mut new_live_in = block.use_.clone();
            for r in &new_live_out {
                if !block.def.contains(r) {
                    new_live_in.insert(*r);
                }
            }

            // Check if anything changed
            if new_live_in != live_in[&block_start] || new_live_out != live_out[&block_start] {
                changed = true;
                live_in.insert(block_start, new_live_in);
                live_out.insert(block_start, new_live_out);
            }
        }
    }

    if iterations == MAX_ITERATIONS {
        eprintln!(
            "Warning: liveness analysis did not converge after {} iterations",
            MAX_ITERATIONS
        );
    }

    LivenessInfo { live_in, live_out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liveness::cfg::CFG;
    use hlbc::opcodes::Opcode;
    use hlbc::types::RefInt;

    #[test]
    fn test_simple_liveness() {
        // r0 = 1
        // r1 = 2
        // r2 = r0 + r1
        // ret r2
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
        let liveness = compute_liveness(&cfg);

        // At the end, only r2 needs to be live (for ret)
        // At the add, r0 and r1 need to be live
        // At the start, nothing external is needed
        let block = &cfg.blocks[&0];
        let live_in = &liveness.live_in[&block.start];
        let live_out = &liveness.live_out[&block.start];

        // live_out should be empty (ret terminates)
        assert!(live_out.is_empty());
        // live_in should be empty (all registers defined before use)
        assert!(live_in.is_empty());
    }

    #[test]
    fn test_cross_block_liveness() {
        // Block 0: r0 = true; if (!r0) goto block 2
        // Block 1: r1 = 1; goto block 3
        // Block 2: r1 = 2
        // Block 3: ret r1
        let ops = vec![
            Opcode::Bool {
                dst: Reg(0),
                value: true,
            },
            Opcode::JFalse {
                cond: Reg(0),
                offset: 2,
            }, // -> op 4
            // Block 1: ops 2-3
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(0),
            },
            Opcode::JAlways { offset: 1 }, // -> op 5
            // Block 2: op 4
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            // Block 3: op 5
            Opcode::Ret { ret: Reg(1) },
        ];

        let cfg = CFG::build(&ops);
        let liveness = compute_liveness(&cfg);

        // At block 5 (ret), r1 must be live_in
        assert!(liveness.live_in[&5].contains(&Reg(1)));

        // Blocks 2 and 4 define r1, so r1 should NOT be in their live_in
        // (they define it before use)
    }
}
