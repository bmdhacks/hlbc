//! Jump offset adjustment for opcode insertion

use crate::opcodes::Opcode;
use crate::types::JumpOffset;

/// Adjust all jump offsets in a function after inserting opcodes at a given index.
///
/// When opcodes are inserted, all jumps that cross the insertion point need adjustment:
/// - Forward jumps from before the insertion to after: offset increases
/// - Backward jumps from after the insertion to before: offset decreases
///
/// # Arguments
/// * `ops` - The opcode vector to adjust (mutated in place)
/// * `insert_index` - The index where opcodes were inserted
/// * `inserted_count` - Number of opcodes that were inserted (positive)
pub fn adjust_jumps_after_insert(ops: &mut [Opcode], insert_index: usize, inserted_count: i32) {
    if inserted_count == 0 {
        return;
    }

    for (i, op) in ops.iter_mut().enumerate() {
        adjust_opcode_jumps(op, i, insert_index, inserted_count);
    }
}

/// Adjust jump offsets in a single opcode
fn adjust_opcode_jumps(op: &mut Opcode, current_index: usize, insert_index: usize, delta: i32) {
    match op {
        // Single-offset conditional jumps
        Opcode::JTrue { offset, .. }
        | Opcode::JFalse { offset, .. }
        | Opcode::JNull { offset, .. }
        | Opcode::JNotNull { offset, .. }
        | Opcode::JSLt { offset, .. }
        | Opcode::JSGte { offset, .. }
        | Opcode::JSGt { offset, .. }
        | Opcode::JSLte { offset, .. }
        | Opcode::JULt { offset, .. }
        | Opcode::JUGte { offset, .. }
        | Opcode::JNotLt { offset, .. }
        | Opcode::JNotGte { offset, .. }
        | Opcode::JEq { offset, .. }
        | Opcode::JNotEq { offset, .. }
        | Opcode::JAlways { offset } => {
            adjust_single_offset(offset, current_index, insert_index, delta);
        }

        // Trap has a jump offset to exception handler
        Opcode::Trap { offset, .. } => {
            adjust_single_offset(offset, current_index, insert_index, delta);
        }

        // Switch has multiple offsets plus an end offset
        Opcode::Switch { offsets, end, .. } => {
            for offset in offsets.iter_mut() {
                adjust_single_offset(offset, current_index, insert_index, delta);
            }
            adjust_single_offset(end, current_index, insert_index, delta);
        }

        // All other opcodes don't have jump offsets
        _ => {}
    }
}

/// Adjust a single jump offset based on insertion point
fn adjust_single_offset(offset: &mut JumpOffset, current_index: usize, insert_index: usize, delta: i32) {
    let target_index = current_index as i32 + *offset;

    // Forward jump: current < insert, target >= insert
    if current_index < insert_index && target_index as usize >= insert_index {
        *offset += delta;
    }
    // Backward jump: current >= insert, target < insert
    // After insertion, current instruction is now at current_index + delta,
    // but target is still at same absolute position, so relative offset changes
    else if current_index >= insert_index && (target_index as usize) < insert_index {
        *offset -= delta;
    }
}

/// Adjust jump offsets after removing opcodes at a given index.
///
/// # Arguments
/// * `ops` - The opcode vector to adjust (mutated in place)
/// * `remove_index` - The index where opcodes were removed
/// * `removed_count` - Number of opcodes that were removed (positive)
pub fn adjust_jumps_after_remove(ops: &mut [Opcode], remove_index: usize, removed_count: i32) {
    // Removing is the inverse of inserting
    adjust_jumps_after_insert(ops, remove_index, -removed_count);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Reg;

    #[test]
    fn test_forward_jump_before_insert() {
        // Jump from index 2 to index 5 (offset +3)
        // Insert at index 4
        // New target should be index 6 (offset +4)
        let mut ops = vec![
            Opcode::Nop,
            Opcode::Nop,
            Opcode::JAlways { offset: 3 }, // jumps to index 5
            Opcode::Nop,
            Opcode::Nop, // insertion point
            Opcode::Label, // original target
        ];

        adjust_jumps_after_insert(&mut ops, 4, 1);

        match &ops[2] {
            Opcode::JAlways { offset } => assert_eq!(*offset, 4),
            _ => panic!("Expected JAlways"),
        }
    }

    #[test]
    fn test_forward_jump_not_crossing() {
        // Jump from index 2 to index 3 (offset +1)
        // Insert at index 5
        // Jump doesn't cross insertion, should stay at offset +1
        let mut ops = vec![
            Opcode::Nop,
            Opcode::Nop,
            Opcode::JAlways { offset: 1 }, // jumps to index 3
            Opcode::Label,
            Opcode::Nop,
            Opcode::Nop, // insertion point
        ];

        adjust_jumps_after_insert(&mut ops, 5, 1);

        match &ops[2] {
            Opcode::JAlways { offset } => assert_eq!(*offset, 1),
            _ => panic!("Expected JAlways"),
        }
    }

    #[test]
    fn test_backward_jump_after_insert() {
        // Jump from index 5 to index 2 (offset -3)
        // Insert at index 3
        // Jump is after insert, target is before, offset should decrease (become more negative)
        let mut ops = vec![
            Opcode::Nop,
            Opcode::Nop,
            Opcode::Label, // target
            Opcode::Nop,   // insertion point
            Opcode::Nop,
            Opcode::JAlways { offset: -3 }, // jumps back to index 2
        ];

        adjust_jumps_after_insert(&mut ops, 3, 1);

        match &ops[5] {
            Opcode::JAlways { offset } => assert_eq!(*offset, -4),
            _ => panic!("Expected JAlways"),
        }
    }

    #[test]
    fn test_switch_adjustment() {
        let mut ops = vec![
            Opcode::Nop,
            Opcode::Switch {
                reg: Reg(0),
                offsets: vec![2, 3, 4], // targets at indices 3, 4, 5
                end: 5,                  // default at index 6
            },
            Opcode::Nop, // insertion point
            Opcode::Nop,
            Opcode::Nop,
            Opcode::Nop,
            Opcode::Nop,
        ];

        adjust_jumps_after_insert(&mut ops, 2, 1);

        match &ops[1] {
            Opcode::Switch { offsets, end, .. } => {
                assert_eq!(offsets, &vec![3, 4, 5]);
                assert_eq!(*end, 6);
            }
            _ => panic!("Expected Switch"),
        }
    }
}
