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
use hlbc::types::Reg;

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

    /// For-in iterator loop: `for (value in collection) { body }`
    /// Detected from pattern: `it = coll.iterator(); while(it.hasNext()) { val = it.next(); ... }`
    ForIn {
        /// Register holding the iterator object
        iterator_reg: Reg,
        /// Register receiving values from .next()
        value_reg: Reg,
        /// Opcode index of the .next() call (to suppress in body output)
        next_op: Option<usize>,
        /// Opcode index where iterator was created (e.g., `it = coll.keys()`)
        /// Used to extract the collection expression for proper for-in syntax.
        iterator_init_op: Option<usize>,
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
        /// This is a placeholder; the actual condition is extracted during lowering
        /// from cond_block's terminating conditional jump.
        cond: Expr,

        /// The CFG block containing the conditional jump.
        /// Used during lowering to extract the actual condition and emit
        /// any preamble statements before the if-statement.
        cond_block: Option<NodeIndex>,

        /// The "then" branch region (executed when cond is true).
        then_region: Box<Region>,

        /// The optional "else" branch region (executed when cond is false).
        /// None for if-without-else.
        else_region: Option<Box<Region>>,

        /// The merge point where control flow reconverges.
        /// This is the immediate post-dominator of the condition node.
        merge: NodeIndex,

        /// Whether the condition should be negated during lowering.
        /// Set to true when we swap empty-then with non-empty-else to produce
        /// cleaner output: `if (c) {} else { body }` becomes `if (!c) { body }`.
        negated: bool,
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
        /// The expression being switched on (may be placeholder if selector_block is set).
        selector: Expr,

        /// The CFG block containing the Switch opcode (for extracting proper expression during lowering).
        selector_block: Option<NodeIndex>,

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

    /// Check if this region terminates (no control flow exits to a merge point).
    /// A region terminates if all paths through it end with a return/throw/etc.
    pub fn terminates(&self, cfg: &crate::lifter::Cfg) -> bool {
        match self {
            Region::Block(node) => cfg.graph[*node].is_exit,
            Region::Sequence(regions) => {
                // A sequence terminates if its last region terminates
                regions.last().map_or(false, |r| r.terminates(cfg))
            }
            Region::IfThenElse {
                then_region,
                else_region,
                merge,
                ..
            } => {
                // If-then-else terminates if BOTH branches terminate
                let then_terminates = then_region.terminates(cfg);
                let else_terminates = else_region
                    .as_ref()
                    .map_or(false, |r| r.terminates(cfg));
                if then_terminates && else_terminates {
                    return true;
                }
                // Also terminates if the then branch terminates and merge is an exit
                // (for early-return patterns: if (cond) return; followed by exit merge)
                if then_terminates && else_region.is_none() && cfg.graph[*merge].is_exit {
                    return true;
                }
                false
            }
            Region::Loop { .. } => {
                // Loops don't terminate by definition (they loop)
                // Break/return inside a loop is handled at a lower level
                false
            }
            Region::Switch { cases, default, .. } => {
                // Switch terminates if ALL cases (including default) terminate
                let all_cases_terminate = cases.iter().all(|c| c.body.terminates(cfg));
                let default_terminates = default.terminates(cfg);
                all_cases_terminate && default_terminates
            }
            Region::Goto { .. } => false,
            Region::Empty => false,
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
                cond_block,
                then_region,
                else_region,
                ..
            } => {
                if let Some(block) = cond_block {
                    nodes.insert(*block);
                }
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
        cond_block: Option<NodeIndex>,
        then_region: Region,
        else_region: Option<Region>,
        merge: NodeIndex,
    ) -> Region {
        Region::IfThenElse {
            cond,
            cond_block,
            then_region: Box::new(then_region),
            else_region: else_region.map(Box::new),
            merge,
            negated: false,
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
        let cond_block = NodeIndex::new(3);

        let region = Region::if_then_else(
            Expr::Constant(Constant::Bool(true)),
            Some(cond_block),
            Region::Block(n0),
            Some(Region::Block(n1)),
            n2,
        );

        assert_eq!(region.exit_node(), Some(n2));
        assert_eq!(region.block_count(), 2);

        let nodes = region.contained_nodes();
        assert!(nodes.contains(&n0));
        assert!(nodes.contains(&n1));
        assert!(nodes.contains(&cond_block));
        assert_eq!(nodes.len(), 3);
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
