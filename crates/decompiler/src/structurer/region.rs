//! Region-based control flow representation
//!
//! This module defines the `Region` enum which represents structured control flow
//! as a tree of Single-Entry-Single-Exit (SESE) regions. This intermediate representation
//! sits between the raw CFG and the final Statement AST, enabling iterative graph reduction
//! and high-level pattern detection.
//!
//! The design follows concepts from:
//! - SAILR (USENIX 2024): Compiler-aware decompilation with SESE regions
//! - Phoenix framework: Region-based control flow structuring
//! - DREAM (NDSS 2015): Iterative graph reduction

use petgraph::graph::NodeIndex;
use std::collections::HashSet;

use crate::ast::{Constant, Expr};

/// The kind of loop detected during structuring.
///
/// HashLink bytecode compiles all Haxe loop constructs (for, while, do-while)
/// into equivalent control flow patterns. During structuring, we identify
/// the original loop kind where possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopKind {
    /// Standard while loop: `while (cond) { body }`
    /// Condition tested at loop entry.
    While,

    /// Do-while loop: `do { body } while (cond)`
    /// Condition tested at loop exit; body executes at least once.
    DoWhile,

    /// For loop: `for (init; cond; incr) { body }`
    /// Detected when loop has clear init/cond/incr pattern.
    /// The init and increment are stored as opcode indices to be lowered later.
    For {
        /// Opcode index of the init statement (before loop).
        init_op: Option<usize>,
        /// Opcode index of the increment statement (end of loop body).
        incr_op: Option<usize>,
    },

    /// Infinite loop: `while (true) { body }`
    /// No exit condition; exits only via break/return/throw.
    Endless,
}

/// A switch case in a Region::Switch.
#[derive(Debug, Clone)]
pub struct SwitchCase {
    /// Pattern values that match this case (may be multiple for fallthrough).
    /// Empty for default case.
    pub patterns: Vec<Constant>,

    /// The body of this case as a region.
    pub body: Region,
}

/// A control flow region representing structured code.
///
/// Regions form a tree where each node represents a SESE (Single-Entry-Single-Exit)
/// subgraph of the original CFG. The structurer works by iteratively identifying
/// and collapsing these regions until the entire function is a single region.
#[derive(Debug, Clone)]
pub enum Region {
    /// A leaf region containing a single basic block.
    /// During lowering, this is converted to statements for the opcodes in the block.
    Block(NodeIndex),

    /// A linear sequence of regions executed in order.
    /// `Sequence([A, B, C])` means: execute A, then B, then C.
    Sequence(Vec<Region>),

    /// An if-then-else control flow region.
    IfThenElse {
        /// The condition expression (from the conditional branch).
        cond: Expr,

        /// The "then" branch region (executed when cond is true).
        then_region: Box<Region>,

        /// The optional "else" branch region (executed when cond is false).
        /// None for if-without-else.
        else_region: Option<Box<Region>>,

        /// The merge point where control flow reconverges.
        /// This is the immediate post-dominator of the condition node.
        merge: NodeIndex,
    },

    /// A loop region (while, do-while, for, or endless).
    Loop {
        /// The type of loop detected.
        kind: LoopKind,

        /// The loop header node (entry point, where back-edges target).
        header: NodeIndex,

        /// The loop condition expression (None for endless loops).
        condition: Option<Expr>,

        /// The loop body as a region.
        body: Box<Region>,

        /// The loop exit node (first node after the loop).
        /// This is where break statements jump to.
        exit: NodeIndex,
    },

    /// A switch/match region.
    Switch {
        /// The expression being switched on.
        selector: Expr,

        /// The cases with their patterns and body regions.
        cases: Vec<SwitchCase>,

        /// The default case body (may be empty region for no default).
        default: Box<Region>,

        /// The merge point after the switch.
        merge: NodeIndex,
    },

    /// A goto to handle irreducible control flow.
    /// This is the fallback when proper structuring isn't possible.
    /// During lowering, this emits a labeled statement and goto.
    Goto {
        /// The target node (will become a label).
        target: NodeIndex,
    },

    /// An empty region (placeholder, typically collapsed away).
    Empty,
}

impl Region {
    /// Returns the entry node of this region.
    /// For structured regions, this is where control flow enters.
    pub fn entry_node(&self) -> Option<NodeIndex> {
        match self {
            Region::Block(node) => Some(*node),
            Region::Sequence(regions) => regions.first().and_then(|r| r.entry_node()),
            Region::IfThenElse { then_region, .. } => then_region.entry_node(),
            Region::Loop { header, .. } => Some(*header),
            Region::Switch { cases, default, .. } => {
                // Entry is the first case or default
                cases
                    .first()
                    .and_then(|c| c.body.entry_node())
                    .or_else(|| default.entry_node())
            }
            Region::Goto { target } => Some(*target),
            Region::Empty => None,
        }
    }

    /// Returns the exit node of this region.
    /// For structured regions, this is where control flow exits.
    pub fn exit_node(&self) -> Option<NodeIndex> {
        match self {
            Region::Block(node) => Some(*node),
            Region::Sequence(regions) => regions.last().and_then(|r| r.exit_node()),
            Region::IfThenElse { merge, .. } => Some(*merge),
            Region::Loop { exit, .. } => Some(*exit),
            Region::Switch { merge, .. } => Some(*merge),
            Region::Goto { target } => Some(*target),
            Region::Empty => None,
        }
    }

    /// Collects all basic block nodes contained within this region.
    pub fn contained_nodes(&self) -> HashSet<NodeIndex> {
        let mut nodes = HashSet::new();
        self.collect_nodes(&mut nodes);
        nodes
    }

    /// Helper to recursively collect nodes.
    fn collect_nodes(&self, nodes: &mut HashSet<NodeIndex>) {
        match self {
            Region::Block(node) => {
                nodes.insert(*node);
            }
            Region::Sequence(regions) => {
                for r in regions {
                    r.collect_nodes(nodes);
                }
            }
            Region::IfThenElse {
                then_region,
                else_region,
                ..
            } => {
                then_region.collect_nodes(nodes);
                if let Some(else_r) = else_region {
                    else_r.collect_nodes(nodes);
                }
            }
            Region::Loop { body, .. } => {
                body.collect_nodes(nodes);
            }
            Region::Switch { cases, default, .. } => {
                for case in cases {
                    case.body.collect_nodes(nodes);
                }
                default.collect_nodes(nodes);
            }
            Region::Goto { .. } | Region::Empty => {}
        }
    }

    /// Returns true if this region is empty (no statements would be generated).
    pub fn is_empty(&self) -> bool {
        match self {
            Region::Empty => true,
            Region::Sequence(regions) => regions.is_empty() || regions.iter().all(|r| r.is_empty()),
            _ => false,
        }
    }

    /// Returns the number of basic blocks in this region.
    pub fn block_count(&self) -> usize {
        match self {
            Region::Block(_) => 1,
            Region::Sequence(regions) => regions.iter().map(|r| r.block_count()).sum(),
            Region::IfThenElse {
                then_region,
                else_region,
                ..
            } => {
                then_region.block_count()
                    + else_region.as_ref().map_or(0, |r| r.block_count())
            }
            Region::Loop { body, .. } => body.block_count(),
            Region::Switch { cases, default, .. } => {
                cases.iter().map(|c| c.body.block_count()).sum::<usize>() + default.block_count()
            }
            Region::Goto { .. } | Region::Empty => 0,
        }
    }

    /// Creates a sequence from multiple regions, flattening nested sequences.
    pub fn sequence(regions: Vec<Region>) -> Region {
        let mut flat: Vec<Region> = Vec::new();
        for r in regions {
            match r {
                Region::Sequence(inner) => flat.extend(inner),
                Region::Empty => {} // Skip empty regions
                other => flat.push(other),
            }
        }
        match flat.len() {
            0 => Region::Empty,
            1 => flat.pop().unwrap(),
            _ => Region::Sequence(flat),
        }
    }

    /// Creates an if-then-else region.
    pub fn if_then_else(
        cond: Expr,
        then_region: Region,
        else_region: Option<Region>,
        merge: NodeIndex,
    ) -> Region {
        Region::IfThenElse {
            cond,
            then_region: Box::new(then_region),
            else_region: else_region.map(Box::new),
            merge,
        }
    }

    /// Creates a while loop region.
    pub fn while_loop(header: NodeIndex, condition: Expr, body: Region, exit: NodeIndex) -> Region {
        Region::Loop {
            kind: LoopKind::While,
            header,
            condition: Some(condition),
            body: Box::new(body),
            exit,
        }
    }

    /// Creates an endless loop region (while(true)).
    pub fn endless_loop(header: NodeIndex, body: Region, exit: NodeIndex) -> Region {
        Region::Loop {
            kind: LoopKind::Endless,
            header,
            condition: None,
            body: Box::new(body),
            exit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_region() {
        let node = NodeIndex::new(0);
        let region = Region::Block(node);

        assert_eq!(region.entry_node(), Some(node));
        assert_eq!(region.exit_node(), Some(node));
        assert_eq!(region.block_count(), 1);
        assert!(!region.is_empty());

        let nodes = region.contained_nodes();
        assert!(nodes.contains(&node));
        assert_eq!(nodes.len(), 1);
    }

    #[test]
    fn test_empty_region() {
        let region = Region::Empty;

        assert!(region.entry_node().is_none());
        assert!(region.exit_node().is_none());
        assert_eq!(region.block_count(), 0);
        assert!(region.is_empty());
    }

    #[test]
    fn test_sequence_flattening() {
        let n0 = NodeIndex::new(0);
        let n1 = NodeIndex::new(1);
        let n2 = NodeIndex::new(2);

        // Create nested sequences
        let inner = Region::Sequence(vec![Region::Block(n1), Region::Block(n2)]);
        let outer = Region::sequence(vec![Region::Block(n0), inner]);

        // Should flatten to a single sequence
        if let Region::Sequence(regions) = outer {
            assert_eq!(regions.len(), 3);
        } else {
            panic!("Expected flattened sequence");
        }
    }

    #[test]
    fn test_sequence_single_element() {
        let node = NodeIndex::new(0);
        let seq = Region::sequence(vec![Region::Block(node)]);

        // Single-element sequence should unwrap to just the block
        assert!(matches!(seq, Region::Block(_)));
    }

    #[test]
    fn test_sequence_empty_elements() {
        let node = NodeIndex::new(0);
        let seq = Region::sequence(vec![Region::Empty, Region::Block(node), Region::Empty]);

        // Should skip empties and return just the block
        assert!(matches!(seq, Region::Block(_)));
    }

    #[test]
    fn test_if_then_else_nodes() {
        let n0 = NodeIndex::new(0);
        let n1 = NodeIndex::new(1);
        let n2 = NodeIndex::new(2);

        let region = Region::if_then_else(
            Expr::Constant(Constant::Bool(true)),
            Region::Block(n0),
            Some(Region::Block(n1)),
            n2,
        );

        assert_eq!(region.exit_node(), Some(n2));
        assert_eq!(region.block_count(), 2);

        let nodes = region.contained_nodes();
        assert!(nodes.contains(&n0));
        assert!(nodes.contains(&n1));
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn test_loop_region() {
        let header = NodeIndex::new(0);
        let body_node = NodeIndex::new(1);
        let exit = NodeIndex::new(2);

        let region = Region::while_loop(
            header,
            Expr::Constant(Constant::Bool(true)),
            Region::Block(body_node),
            exit,
        );

        assert_eq!(region.entry_node(), Some(header));
        assert_eq!(region.exit_node(), Some(exit));
        assert_eq!(region.block_count(), 1);

        if let Region::Loop { kind, .. } = region {
            assert_eq!(kind, LoopKind::While);
        }
    }
}
