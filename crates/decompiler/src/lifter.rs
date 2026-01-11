//! Pass 1: Lifter - Build petgraph CFG from HashLink bytecode
//!
//! This module constructs a Control Flow Graph (CFG) from HashLink bytecode opcodes.
//! Each node is a basic block containing sequential opcodes, and edges represent
//! control flow transitions (fall-through, jumps, conditional branches).
//!
//! The CFG is built using petgraph's DiGraph, which enables efficient dominator
//! computation and graph traversal in subsequent passes.

use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::{HashMap, HashSet};

use hlbc::opcodes::Opcode;
use hlbc::types::Function;

/// Edge type in the CFG
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// Fall-through to next block
    FallThrough,
    /// Unconditional jump (JAlways)
    Jump,
    /// Conditional branch taken (JTrue, JNotNull, etc.)
    ConditionalTrue,
    /// Conditional branch not taken (fall-through on false)
    ConditionalFalse,
    /// Exception handler edge
    ExceptionHandler,
    /// Return edge (to exit node)
    Return,
}

/// A basic block in the CFG
#[derive(Debug, Clone)]
pub struct BasicBlock {
    /// Index of first opcode in this block
    pub start: usize,
    /// Index of last opcode in this block (inclusive)
    pub end: usize,
    /// Whether this is the entry block
    pub is_entry: bool,
    /// Whether this is an exit block (contains Ret/Throw)
    pub is_exit: bool,
}

impl BasicBlock {
    /// Number of opcodes in this block
    pub fn len(&self) -> usize {
        self.end - self.start + 1
    }

    /// Check if block is empty
    pub fn is_empty(&self) -> bool {
        false // A block always has at least one opcode
    }

    /// Iterate over opcode indices in this block
    pub fn op_indices(&self) -> impl Iterator<Item = usize> {
        self.start..=self.end
    }
}

/// The Control Flow Graph
pub struct Cfg {
    /// The graph structure
    pub graph: DiGraph<BasicBlock, EdgeKind>,
    /// Entry node
    pub entry: NodeIndex,
    /// Map from opcode index to containing block
    pub op_to_block: HashMap<usize, NodeIndex>,
    /// Original opcodes reference (indices only)
    pub num_ops: usize,
}

impl Cfg {
    /// Build a CFG from a function's opcodes
    pub fn build(f: &Function) -> Self {
        Self::from_ops(&f.ops)
    }

    /// Build a CFG from an opcode slice
    pub fn from_ops(ops: &[Opcode]) -> Self {
        if ops.is_empty() {
            let mut graph = DiGraph::new();
            let entry = graph.add_node(BasicBlock {
                start: 0,
                end: 0,
                is_entry: true,
                is_exit: true,
            });
            return Cfg {
                graph,
                entry,
                op_to_block: HashMap::new(),
                num_ops: 0,
            };
        }

        // Step 1: Find all block start points (leaders)
        let leaders = find_leaders(ops);

        // Step 2: Create basic blocks
        let (graph, _block_map, entry) = create_blocks(ops, &leaders);

        // Step 3: Build op_to_block mapping
        let mut op_to_block = HashMap::new();
        for (node_idx, block) in graph.node_indices().zip(graph.node_weights()) {
            for op_idx in block.start..=block.end {
                op_to_block.insert(op_idx, node_idx);
            }
        }

        Cfg {
            graph,
            entry,
            op_to_block,
            num_ops: ops.len(),
        }
    }

    /// Get the block containing a specific opcode
    pub fn block_for_op(&self, op_idx: usize) -> Option<NodeIndex> {
        self.op_to_block.get(&op_idx).copied()
    }

    /// Get all successor blocks of a node
    pub fn successors(&self, node: NodeIndex) -> Vec<NodeIndex> {
        self.graph.neighbors(node).collect()
    }

    /// Get all predecessor blocks of a node
    pub fn predecessors(&self, node: NodeIndex) -> Vec<NodeIndex> {
        self.graph
            .neighbors_directed(node, petgraph::Direction::Incoming)
            .collect()
    }

    /// Number of basic blocks
    pub fn num_blocks(&self) -> usize {
        self.graph.node_count()
    }
}

/// Find all leader instructions (block start points)
fn find_leaders(ops: &[Opcode]) -> HashSet<usize> {
    let mut leaders = HashSet::new();

    // First instruction is always a leader
    leaders.insert(0);

    for (i, op) in ops.iter().enumerate() {
        match op {
            // Jump targets are leaders
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
                let target = compute_target(i, *offset, ops.len());
                if let Some(t) = target {
                    leaders.insert(t);
                }
                // Instruction after conditional jump is also a leader
                if !matches!(op, Opcode::JAlways { .. }) && i + 1 < ops.len() {
                    leaders.insert(i + 1);
                }
                // Instruction after unconditional jump is a leader (if reachable)
                if matches!(op, Opcode::JAlways { .. }) && i + 1 < ops.len() {
                    leaders.insert(i + 1);
                }
            }

            // Switch targets
            Opcode::Switch { offsets, .. } => {
                for &offset in offsets.iter() {
                    if let Some(t) = compute_target(i, offset, ops.len()) {
                        leaders.insert(t);
                    }
                }
                // Fall-through after switch
                if i + 1 < ops.len() {
                    leaders.insert(i + 1);
                }
            }

            // Try/catch - the handler is a leader
            Opcode::Trap { offset, .. } => {
                if let Some(t) = compute_target(i, *offset, ops.len()) {
                    leaders.insert(t);
                }
            }

            // Return/throw terminate the block, next instruction is a leader
            Opcode::Ret { .. } | Opcode::Throw { .. } | Opcode::Rethrow { .. } => {
                if i + 1 < ops.len() {
                    leaders.insert(i + 1);
                }
            }

            // Label is a leader (can be jumped to)
            Opcode::Label => {
                leaders.insert(i);
            }

            _ => {}
        }
    }

    leaders
}

/// Create basic blocks and edges from leaders
fn create_blocks(
    ops: &[Opcode],
    leaders: &HashSet<usize>,
) -> (DiGraph<BasicBlock, EdgeKind>, HashMap<usize, NodeIndex>, NodeIndex) {
    let mut graph = DiGraph::new();
    let mut block_map: HashMap<usize, NodeIndex> = HashMap::new();

    // Sort leaders to create blocks in order
    let mut sorted_leaders: Vec<usize> = leaders.iter().copied().collect();
    sorted_leaders.sort();

    // Create nodes for each block
    for (idx, &leader) in sorted_leaders.iter().enumerate() {
        let end = if idx + 1 < sorted_leaders.len() {
            sorted_leaders[idx + 1] - 1
        } else {
            ops.len() - 1
        };

        let last_op = &ops[end];
        let is_exit = matches!(last_op, Opcode::Ret { .. } | Opcode::Throw { .. } | Opcode::Rethrow { .. });

        let block = BasicBlock {
            start: leader,
            end,
            is_entry: leader == 0,
            is_exit,
        };

        let node = graph.add_node(block);
        block_map.insert(leader, node);
    }

    // Add edges based on control flow
    for &leader in &sorted_leaders {
        let node = block_map[&leader];
        // Copy the end index to avoid holding borrow during edge addition
        let block_end = graph[node].end;
        let last_op = &ops[block_end];

        match last_op {
            // Unconditional jump
            Opcode::JAlways { offset } => {
                if let Some(target) = compute_target(block_end, *offset, ops.len()) {
                    if let Some(&target_node) = block_map.get(&target) {
                        graph.add_edge(node, target_node, EdgeKind::Jump);
                    }
                }
            }

            // Conditional jumps
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
            | Opcode::JNotEq { offset, .. } => {
                // True branch (jump taken)
                if let Some(target) = compute_target(block_end, *offset, ops.len()) {
                    if let Some(&target_node) = block_map.get(&target) {
                        graph.add_edge(node, target_node, EdgeKind::ConditionalTrue);
                    }
                }
                // False branch (fall-through)
                let fall_through = block_end + 1;
                if fall_through < ops.len() {
                    if let Some(&target_node) = block_map.get(&fall_through) {
                        graph.add_edge(node, target_node, EdgeKind::ConditionalFalse);
                    }
                }
            }

            // Switch
            Opcode::Switch { offsets, .. } => {
                for &offset in offsets.iter() {
                    if let Some(target) = compute_target(block_end, offset, ops.len()) {
                        if let Some(&target_node) = block_map.get(&target) {
                            graph.add_edge(node, target_node, EdgeKind::ConditionalTrue);
                        }
                    }
                }
                // Default case (fall-through)
                let fall_through = block_end + 1;
                if fall_through < ops.len() {
                    if let Some(&target_node) = block_map.get(&fall_through) {
                        graph.add_edge(node, target_node, EdgeKind::FallThrough);
                    }
                }
            }

            // Trap (exception handler)
            Opcode::Trap { offset, .. } => {
                if let Some(target) = compute_target(block_end, *offset, ops.len()) {
                    if let Some(&target_node) = block_map.get(&target) {
                        graph.add_edge(node, target_node, EdgeKind::ExceptionHandler);
                    }
                }
                // Fall-through (normal execution)
                let fall_through = block_end + 1;
                if fall_through < ops.len() {
                    if let Some(&target_node) = block_map.get(&fall_through) {
                        graph.add_edge(node, target_node, EdgeKind::FallThrough);
                    }
                }
            }

            // Return/throw - no successors
            Opcode::Ret { .. } | Opcode::Throw { .. } | Opcode::Rethrow { .. } => {
                // No edges - these terminate the function
            }

            // Default: fall-through to next block
            _ => {
                let fall_through = block_end + 1;
                if fall_through < ops.len() {
                    if let Some(&target_node) = block_map.get(&fall_through) {
                        graph.add_edge(node, target_node, EdgeKind::FallThrough);
                    }
                }
            }
        }
    }

    let entry = block_map[&0];
    (graph, block_map, entry)
}

/// Compute jump target index from offset
fn compute_target(current: usize, offset: i32, num_ops: usize) -> Option<usize> {
    let target = current as i64 + offset as i64 + 1;
    if target >= 0 && (target as usize) < num_ops {
        Some(target as usize)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hlbc::types::{RefInt, Reg};

    #[test]
    fn test_simple_linear_cfg() {
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Add { dst: Reg(2), a: Reg(0), b: Reg(1) },
            Opcode::Ret { ret: Reg(2) },
        ];

        let cfg = Cfg::from_ops(&ops);

        // Should be single block
        assert_eq!(cfg.num_blocks(), 1);
        assert!(cfg.graph[cfg.entry].is_entry);
        assert!(cfg.graph[cfg.entry].is_exit);
    }

    #[test]
    fn test_conditional_cfg() {
        let ops = vec![
            // Block 0: op 0-1
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 }, // Jump to op 4
            // Block 1: op 2-3
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Ret { ret: Reg(1) },
            // Block 2: op 4
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);

        // Should have 3 blocks
        assert_eq!(cfg.num_blocks(), 3);

        // Entry should have 2 successors
        let successors = cfg.successors(cfg.entry);
        assert_eq!(successors.len(), 2);
    }

    #[test]
    fn test_loop_cfg() {
        let ops = vec![
            // Block 0: op 0
            Opcode::Label,
            // Block 0 continued: op 1
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            // op 2: conditional exit
            Opcode::JNull { reg: Reg(0), offset: 2 }, // Jump to op 5 (exit)
            // Block 1: op 3
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            // op 4: back edge
            Opcode::JAlways { offset: -4 }, // Jump back to op 1
            // Block 2: op 5
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);

        // Should have blocks for: entry, loop body, exit
        assert!(cfg.num_blocks() >= 3);
    }
}
