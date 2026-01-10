//! Live range extraction from liveness information
//!
//! Converts block-level liveness into per-opcode live ranges, where each
//! live range represents a distinct value that should get a unique variable.

use std::collections::{HashMap, HashSet};

use hlbc::opcodes::Opcode;
use hlbc::types::Reg;

use super::cfg::CFG;
use super::dataflow::LivenessInfo;
use super::def_use::{get_defs, get_uses};

/// A live range for a register value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LiveRange {
    /// The register
    pub reg: Reg,
    /// Opcode index where this value was defined
    pub def_point: usize,
    /// Opcode index of last use (may equal def_point if only assigned but not used)
    pub last_use: usize,
    /// Number of times this value is used
    pub use_count: usize,
}

/// Map from (register, opcode_index) to live range id.
///
/// Used to look up which live range a register access belongs to.
#[derive(Debug)]
pub struct LiveRangeMap {
    /// All live ranges
    pub ranges: Vec<LiveRange>,
    /// For each register, maps opcode index to live range index
    /// Key: (reg, op_index), Value: index into ranges
    pub lookup: HashMap<(Reg, usize), usize>,
}

impl LiveRangeMap {
    /// Get the live range ID for a register at a specific opcode index.
    ///
    /// Returns None if no live range covers this point.
    pub fn get_range_id(&self, reg: Reg, op_index: usize) -> Option<usize> {
        self.lookup.get(&(reg, op_index)).copied()
    }

    /// Get the live range for a register at a specific opcode index.
    pub fn get_range(&self, reg: Reg, op_index: usize) -> Option<&LiveRange> {
        self.get_range_id(reg, op_index)
            .map(|id| &self.ranges[id])
    }

    /// Get all live ranges for a specific register.
    pub fn ranges_for_reg(&self, reg: Reg) -> Vec<&LiveRange> {
        self.ranges.iter().filter(|r| r.reg == reg).collect()
    }
}

/// Extract live ranges from CFG and liveness info.
pub fn extract_live_ranges(cfg: &CFG, liveness: &LivenessInfo, _num_regs: usize) -> LiveRangeMap {
    if cfg.ops_len == 0 {
        return LiveRangeMap {
            ranges: Vec::new(),
            lookup: HashMap::new(),
        };
    }

    // We need to track, for each register:
    // - Current active live range (if any)
    // - All completed live ranges
    //
    // Strategy: Walk through opcodes, tracking definitions and uses.
    // When we see a def, start a new live range.
    // When we see a use, extend the current live range.

    // First, collect all ops from CFG blocks in order
    // Note: We don't have direct access to ops, so we track by index
    let mut ranges: Vec<LiveRange> = Vec::new();
    let mut lookup: HashMap<(Reg, usize), usize> = HashMap::new();

    // Track active ranges: reg -> (range_index, def_point)
    let mut active: HashMap<Reg, usize> = HashMap::new();

    // We need to process ops in order. Since CFG has blocks, we need to
    // handle control flow merge points carefully.
    //
    // Simplified approach for now: process each block separately, using
    // liveness info to handle cross-block flow.

    // For each block in order
    for &block_start in &cfg.block_order {
        let block = &cfg.blocks[&block_start];
        let live_in = &liveness.live_in[&block_start];

        // At block entry, registers in live_in may have active ranges from predecessors
        // For these, we need to ensure they're tracked
        for reg in live_in {
            if !active.contains_key(reg) {
                // This register is live coming into the block but we haven't seen its def
                // This means it was defined in a predecessor block
                // Create a synthetic "incoming" range - we'll fix this up later
                // For now, mark it as defined at block_start (approximate)
                let range_id = ranges.len();
                ranges.push(LiveRange {
                    reg: *reg,
                    def_point: block_start,
                    last_use: block_start,
                    use_count: 0,
                });
                active.insert(*reg, range_id);
            }
        }

        // Process each opcode in the block
        // We don't have direct access to ops here, but we have def/use info
        // Let's track by index using the block's def/use sets

        // For now, use a simplified per-block approach
        // This won't be 100% accurate for complex control flow but handles
        // the common case of linear code within blocks

        // Registers defined in this block: start new ranges
        for reg in &block.def {
            // If there's an existing active range, it ends here
            active.remove(reg);

            // Start new range at block start (approximate - ideally at exact def point)
            let range_id = ranges.len();
            ranges.push(LiveRange {
                reg: *reg,
                def_point: block_start,
                last_use: block.end,
                use_count: 1, // At least one use (the def)
            });
            active.insert(*reg, range_id);
            // Add lookup for all ops in this block for this register
            for i in block_start..=block.end {
                lookup.insert((*reg, i), range_id);
            }
        }

        // Registers used but not defined in this block: they come from earlier blocks
        for reg in &block.use_ {
            if let Some(&range_id) = active.get(reg) {
                // Extend the range
                let range = &mut ranges[range_id];
                if block.end > range.last_use {
                    range.last_use = block.end;
                }
                range.use_count += 1;
                // Add lookup entries
                for i in block_start..=block.end {
                    lookup.entry((*reg, i)).or_insert(range_id);
                }
            }
        }

        // Registers in live_out but not defined here: maintain their ranges
        let live_out = &liveness.live_out[&block_start];
        for reg in live_out {
            if !block.def.contains(reg) {
                if let Some(&range_id) = active.get(reg) {
                    // Ensure range covers this block
                    let range = &mut ranges[range_id];
                    if block.end > range.last_use {
                        range.last_use = block.end;
                    }
                }
            }
        }
    }

    LiveRangeMap { ranges, lookup }
}

/// More precise live range extraction that walks opcodes directly.
///
/// This version requires access to the actual opcodes array.
pub fn extract_live_ranges_precise(ops: &[Opcode]) -> LiveRangeMap {
    let mut ranges: Vec<LiveRange> = Vec::new();
    let mut lookup: HashMap<(Reg, usize), usize> = HashMap::new();

    // Track: for each register, which range is currently active
    let mut active: HashMap<Reg, usize> = HashMap::new();

    for (i, op) in ops.iter().enumerate() {
        let defs = get_defs(op);
        let uses = get_uses(op);

        // Process uses first (before defs, since Incr/Decr both use and def)
        for reg in &uses {
            if let Some(&range_id) = active.get(reg) {
                // Extend range
                let range = &mut ranges[range_id];
                range.last_use = i;
                range.use_count += 1;
                lookup.insert((*reg, i), range_id);
            } else {
                // Use without prior def - this is a function parameter or bug
                // Create a range starting at 0 (function entry)
                let range_id = ranges.len();
                ranges.push(LiveRange {
                    reg: *reg,
                    def_point: 0,
                    last_use: i,
                    use_count: 1,
                });
                active.insert(*reg, range_id);
                // Fill in lookups from 0 to i
                for j in 0..=i {
                    lookup.insert((*reg, j), range_id);
                }
            }
        }

        // Process defs
        for reg in &defs {
            // New def kills any previous range and starts a new one
            let range_id = ranges.len();
            ranges.push(LiveRange {
                reg: *reg,
                def_point: i,
                last_use: i, // May be extended by later uses
                use_count: 0,
            });
            active.insert(*reg, range_id);
            lookup.insert((*reg, i), range_id);
        }
    }

    LiveRangeMap { ranges, lookup }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hlbc::types::RefInt;

    #[test]
    fn test_simple_ranges() {
        let ops = vec![
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            }, // op 0: def r0
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            }, // op 1: def r1
            Opcode::Add {
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            }, // op 2: use r0, r1; def r2
            Opcode::Ret { ret: Reg(2) },       // op 3: use r2
        ];

        let map = extract_live_ranges_precise(&ops);

        // r0: def at 0, used at 2
        let r0_range = map.get_range(Reg(0), 2).unwrap();
        assert_eq!(r0_range.def_point, 0);
        assert_eq!(r0_range.last_use, 2);
        assert_eq!(r0_range.use_count, 1);

        // r1: def at 1, used at 2
        let r1_range = map.get_range(Reg(1), 2).unwrap();
        assert_eq!(r1_range.def_point, 1);
        assert_eq!(r1_range.last_use, 2);

        // r2: def at 2, used at 3
        let r2_range = map.get_range(Reg(2), 3).unwrap();
        assert_eq!(r2_range.def_point, 2);
        assert_eq!(r2_range.last_use, 3);
    }

    #[test]
    fn test_register_reuse() {
        // r0 is used for two different values
        let ops = vec![
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            }, // op 0: def r0 (first value)
            Opcode::Ret { ret: Reg(0) },       // op 1: use r0
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(1),
            }, // op 2: def r0 (second value)
            Opcode::Ret { ret: Reg(0) },       // op 3: use r0
        ];

        let map = extract_live_ranges_precise(&ops);

        // Should have 2 distinct ranges for r0
        let ranges_for_r0 = map.ranges_for_reg(Reg(0));
        assert_eq!(ranges_for_r0.len(), 2);

        // First range: def 0, last_use 1
        let first = map.get_range(Reg(0), 1).unwrap();
        assert_eq!(first.def_point, 0);
        assert_eq!(first.last_use, 1);

        // Second range: def 2, last_use 3
        let second = map.get_range(Reg(0), 3).unwrap();
        assert_eq!(second.def_point, 2);
        assert_eq!(second.last_use, 3);

        // These should be different ranges
        assert_ne!(map.get_range_id(Reg(0), 1), map.get_range_id(Reg(0), 3));
    }
}
