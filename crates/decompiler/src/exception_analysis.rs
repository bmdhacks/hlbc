//! Exception region analysis for try/catch structuring.
//!
//! This module analyzes Trap/EndTrap opcode pairs to identify exception regions.
//! Trap/EndTrap form balanced pairs like parentheses, allowing us to match them
//! with a stack-based algorithm and structure try/catch blocks by opcode ranges.

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, Reg};

/// A try/catch region identified by Trap/EndTrap pair.
#[derive(Debug, Clone)]
pub struct TryRegion {
    /// Index of the Trap opcode (start of try setup)
    pub trap_op: usize,
    /// Index of the EndTrap opcode (end of catch body)
    pub end_trap_op: usize,
    /// Index of the first opcode in the catch handler
    pub handler_op: usize,
    /// Register that holds the caught exception
    pub exc_reg: Reg,
    /// Nested try regions within this one
    pub nested: Vec<TryRegion>,
}

impl TryRegion {
    /// Returns the range of opcodes in the try body.
    /// Try body is from trap+1 to end_trap (exclusive), NOT including EndTrap.
    /// The handler_op is where catch jumps to, which is AFTER end_trap.
    pub fn try_body_range(&self) -> std::ops::Range<usize> {
        self.trap_op + 1..self.end_trap_op
    }

    /// Returns the range of opcodes in the catch body.
    /// Catch body starts at handler_op. The end must be computed from context
    /// (next sibling region, outer region boundary, or function end).
    /// This method returns handler_op..end as a helper when you know the end.
    pub fn catch_body_range(&self, catch_end: usize) -> std::ops::Range<usize> {
        self.handler_op..catch_end
    }

    /// Check if an opcode index is within the try body
    pub fn in_try_body(&self, op_idx: usize) -> bool {
        op_idx > self.trap_op && op_idx < self.end_trap_op
    }

    /// Check if an opcode index is at or after the handler start
    pub fn in_handler(&self, op_idx: usize) -> bool {
        op_idx >= self.handler_op
    }
}

/// Analysis result containing all exception regions in a function.
#[derive(Debug, Default)]
pub struct ExceptionAnalysis {
    /// Top-level try regions (not nested inside another)
    pub regions: Vec<TryRegion>,
}

impl ExceptionAnalysis {
    /// Analyze a function for exception regions.
    pub fn analyze(func: &Function) -> Self {
        let pairs = find_trap_pairs(&func.ops);
        if pairs.is_empty() {
            return Self::default();
        }

        // Build regions from pairs
        let mut regions: Vec<TryRegion> = pairs
            .into_iter()
            .map(|(trap_idx, end_trap_idx, handler_op, exc_reg)| TryRegion {
                trap_op: trap_idx,
                end_trap_op: end_trap_idx,
                handler_op,
                exc_reg,
                nested: vec![],
            })
            .collect();

        // Sort by trap_op for consistent ordering
        regions.sort_by_key(|r| r.trap_op);

        // Build nesting structure
        let top_level = build_nesting(&mut regions);

        Self {
            regions: top_level,
        }
    }

    /// Get the region that starts at the given opcode index, if any.
    pub fn region_starting_at(&self, op_idx: usize) -> Option<&TryRegion> {
        self.find_region_starting_at(&self.regions, op_idx)
    }

    fn find_region_starting_at<'a>(
        &self,
        regions: &'a [TryRegion],
        op_idx: usize,
    ) -> Option<&'a TryRegion> {
        for region in regions {
            if region.trap_op == op_idx {
                return Some(region);
            }
            // Check nested regions
            if let Some(nested) = self.find_region_starting_at(&region.nested, op_idx) {
                return Some(nested);
            }
        }
        None
    }

    /// Check if there are any exception regions.
    pub fn has_exceptions(&self) -> bool {
        !self.regions.is_empty()
    }

    /// Get all top-level regions.
    pub fn top_level_regions(&self) -> &[TryRegion] {
        &self.regions
    }
}

/// Find all Trap/EndTrap pairs using a stack-based algorithm.
/// Returns Vec of (trap_idx, end_trap_idx, handler_op, exc_reg).
fn find_trap_pairs(ops: &[Opcode]) -> Vec<(usize, usize, usize, Reg)> {
    let mut stack: Vec<(usize, usize, Reg)> = vec![]; // (trap_idx, handler_op, exc_reg)
    let mut pairs: Vec<(usize, usize, usize, Reg)> = vec![];

    for (i, op) in ops.iter().enumerate() {
        match op {
            Opcode::Trap { exc, offset } => {
                // Handler is at: current_op + 1 + offset
                // But offset is relative to the instruction AFTER Trap
                let handler_op = (i as i32 + 1 + *offset) as usize;
                stack.push((i, handler_op, *exc));
            }
            Opcode::EndTrap { exc: _ } => {
                if let Some((trap_idx, handler_op, exc_reg)) = stack.pop() {
                    pairs.push((trap_idx, i, handler_op, exc_reg));
                }
            }
            _ => {}
        }
    }

    pairs
}

/// Build nesting structure from flat list of regions.
/// Returns only top-level regions; nested ones are moved into parent's `nested` field.
fn build_nesting(regions: &mut Vec<TryRegion>) -> Vec<TryRegion> {
    if regions.is_empty() {
        return vec![];
    }

    // Sort by start position, then by size (larger first for same start)
    // This ensures we process outer regions before inner regions
    regions.sort_by(|a, b| {
        a.trap_op
            .cmp(&b.trap_op)
            .then_with(|| b.end_trap_op.cmp(&a.end_trap_op))
    });

    let mut result: Vec<TryRegion> = vec![];

    // Process regions in sorted order (front to back)
    // This way, outer regions are added to result before we try to nest inner ones
    for region in regions.drain(..) {
        // Find if this region should be nested in any existing result region
        let mut inserted = false;
        for existing in result.iter_mut() {
            if insert_nested(existing, region.clone()) {
                inserted = true;
                break;
            }
        }
        if !inserted {
            result.push(region);
        }
    }

    // Sort result by trap_op
    result.sort_by_key(|r| r.trap_op);
    result
}

/// Try to insert `child` as a nested region of `parent`.
/// Returns true if inserted.
fn insert_nested(parent: &mut TryRegion, child: TryRegion) -> bool {
    // Child must be strictly within parent's range
    if child.trap_op > parent.trap_op && child.end_trap_op < parent.end_trap_op {
        // First try to insert into existing nested regions
        for nested in parent.nested.iter_mut() {
            if insert_nested(nested, child.clone()) {
                return true;
            }
        }
        // Otherwise add as direct child
        parent.nested.push(child);
        parent.nested.sort_by_key(|r| r.trap_op);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use hlbc::types::{RefFun, RefString, RefType, Reg};

    fn make_trap(exc_reg: u32, offset: i32) -> Opcode {
        Opcode::Trap {
            exc: Reg(exc_reg),
            offset,
        }
    }

    fn make_end_trap(exc_reg: u32) -> Opcode {
        Opcode::EndTrap { exc: Reg(exc_reg) }
    }

    fn make_nop() -> Opcode {
        Opcode::Nop
    }

    /// Create a minimal Function with just the ops we need for testing
    fn make_func(ops: Vec<Opcode>) -> Function {
        Function {
            t: RefType(0),
            findex: RefFun(0),
            regs: vec![],
            ops,
            debug_info: None,
            assigns: None,
            name: RefString(0),
            parent: None,
        }
    }

    #[test]
    fn test_single_try_catch() {
        // Simple: Trap at 0, handler at 3, EndTrap at 5
        // Ops: [Trap(offset=2), nop, nop, handler_start, nop, EndTrap]
        let ops = vec![
            make_trap(0, 2),  // 0: Trap, handler at 0+1+2=3
            make_nop(),       // 1: try body
            make_nop(),       // 2: try body
            make_nop(),       // 3: catch handler start
            make_nop(),       // 4: catch body
            make_end_trap(0), // 5: EndTrap
        ];

        let func = make_func(ops);

        let analysis = ExceptionAnalysis::analyze(&func);
        assert_eq!(analysis.regions.len(), 1);

        let region = &analysis.regions[0];
        assert_eq!(region.trap_op, 0);
        assert_eq!(region.end_trap_op, 5);
        assert_eq!(region.handler_op, 3);
        assert_eq!(region.exc_reg, Reg(0));
        assert!(region.nested.is_empty());
    }

    #[test]
    fn test_nested_try_catch() {
        // Outer: Trap at 0, handler at 6, EndTrap at 9
        // Inner: Trap at 2, handler at 4, EndTrap at 5
        let ops = vec![
            make_trap(0, 5),  // 0: Outer Trap, handler at 0+1+5=6
            make_nop(),       // 1: outer try body
            make_trap(1, 1),  // 2: Inner Trap, handler at 2+1+1=4
            make_nop(),       // 3: inner try body
            make_nop(),       // 4: inner catch handler
            make_end_trap(1), // 5: Inner EndTrap
            make_nop(),       // 6: outer catch handler
            make_nop(),       // 7: outer catch body
            make_nop(),       // 8: outer catch body
            make_end_trap(0), // 9: Outer EndTrap
        ];

        let func = make_func(ops);

        let analysis = ExceptionAnalysis::analyze(&func);
        assert_eq!(analysis.regions.len(), 1);

        let outer = &analysis.regions[0];
        assert_eq!(outer.trap_op, 0);
        assert_eq!(outer.end_trap_op, 9);
        assert_eq!(outer.nested.len(), 1);

        let inner = &outer.nested[0];
        assert_eq!(inner.trap_op, 2);
        assert_eq!(inner.end_trap_op, 5);
        assert_eq!(inner.handler_op, 4);
    }

    #[test]
    fn test_sequential_try_catch() {
        // Two sequential try/catch blocks
        let ops = vec![
            make_trap(0, 2),  // 0: First Trap, handler at 3
            make_nop(),       // 1: first try body
            make_nop(),       // 2: first try body
            make_nop(),       // 3: first catch handler
            make_end_trap(0), // 4: First EndTrap
            make_trap(1, 1),  // 5: Second Trap, handler at 7
            make_nop(),       // 6: second try body
            make_nop(),       // 7: second catch handler
            make_end_trap(1), // 8: Second EndTrap
        ];

        let func = make_func(ops);

        let analysis = ExceptionAnalysis::analyze(&func);
        assert_eq!(analysis.regions.len(), 2);

        let first = &analysis.regions[0];
        assert_eq!(first.trap_op, 0);
        assert_eq!(first.end_trap_op, 4);

        let second = &analysis.regions[1];
        assert_eq!(second.trap_op, 5);
        assert_eq!(second.end_trap_op, 8);
    }

    #[test]
    fn test_region_starting_at() {
        let ops = vec![
            make_trap(0, 2),
            make_nop(),
            make_nop(),
            make_nop(),
            make_end_trap(0),
        ];

        let func = make_func(ops);

        let analysis = ExceptionAnalysis::analyze(&func);

        assert!(analysis.region_starting_at(0).is_some());
        assert!(analysis.region_starting_at(1).is_none());
        assert!(analysis.region_starting_at(5).is_none());
    }
}
