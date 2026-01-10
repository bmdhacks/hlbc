//! Liveness analysis for HashLink bytecode
//!
//! This module provides register liveness analysis to support accurate variable naming
//! during decompilation. It computes live ranges for each register, where each live range
//! represents a distinct value that should get its own variable name.
//!
//! ## Key Concepts
//!
//! - **Live range**: A (register, def_point, last_use) tuple representing one value
//! - **Def point**: The opcode index where a register is written
//! - **Use**: An opcode index where a register is read
//! - **Liveness**: A register is "live" at a point if its current value will be read later

mod cfg;
mod dataflow;
mod def_use;
pub mod ranges;

pub use cfg::{BasicBlock, CFG};
pub use dataflow::compute_liveness;
pub use def_use::{get_defs, get_uses};
pub use ranges::{LiveRange, LiveRangeMap};

use hlbc::types::Function;

/// Compute live ranges for all registers in a function.
///
/// This is the main entry point for liveness analysis. It:
/// 1. Builds a CFG from the function's opcodes
/// 2. Computes liveness for each basic block using dataflow analysis
/// 3. Extracts live ranges from the liveness information
///
/// Returns a `LiveRangeMap` that can be queried to find which live range
/// a register access belongs to.
pub fn analyze_function(f: &Function) -> LiveRangeMap {
    // Build CFG
    let cfg = CFG::build(&f.ops);

    // Compute liveness (live_in, live_out for each block)
    let liveness = compute_liveness(&cfg);

    // Extract live ranges
    ranges::extract_live_ranges(&cfg, &liveness, f.regs.len())
}
