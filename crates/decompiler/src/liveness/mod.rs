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
//!
//! ## Usage
//!
//! The primary entry point is `ranges::extract_live_ranges_precise()`, which walks
//! opcodes directly for accurate per-instruction live range tracking.

// These modules will be reworked in Phase 2 (petgraph-based CFG)
#[allow(dead_code)]
mod cfg;
#[allow(dead_code)]
mod dataflow;

mod def_use;
pub mod ranges;

pub use def_use::{get_defs, get_uses};
pub use ranges::LiveRangeMap;
