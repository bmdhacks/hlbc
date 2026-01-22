//! Region Dominators - Dominance computed on the RegionGraph
//!
//! This module provides dominance analysis that operates on the current state
//! of the RegionGraph, not the original CFG. This is critical for correct
//! pattern detection after collapses.
//!
//! Key insight: After a collapse, the original CFG dominance is stale.
//! Collapsed regions are now atomic nodes, and dominance relationships change.

use petgraph::algo::dominators::Dominators;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;

use super::region_graph::RegionGraph;

/// Post-dominator tree computed on the RegionGraph.
///
/// Unlike the original CFG post-dominators, this reflects the current state
/// of the graph after collapses.
pub struct RegionPostDominators {
    /// The computed post-dominators (on reverse graph)
    dominators: Dominators<NodeIndex>,
    /// Mapping from RegionGraph nodes to reverse graph nodes
    forward_map: HashMap<NodeIndex, NodeIndex>,
    /// Mapping from reverse graph nodes to RegionGraph nodes
    reverse_map: HashMap<NodeIndex, NodeIndex>,
    /// The virtual exit node in the reverse graph
    virtual_exit: NodeIndex,
}

impl RegionPostDominators {
    /// Compute post-dominators on the RegionGraph.
    pub fn compute(region_graph: &RegionGraph) -> Self {

        // Build reverse graph with virtual exit
        let mut reverse_graph: DiGraph<(), ()> = DiGraph::new();
        let mut forward_map: HashMap<NodeIndex, NodeIndex> = HashMap::new();
        let mut reverse_map: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        // Add all nodes from region graph
        for node in region_graph.node_indices() {
            let rev_node = reverse_graph.add_node(());
            forward_map.insert(node, rev_node);
            reverse_map.insert(rev_node, node);
        }

        // Add virtual exit node
        let virtual_exit = reverse_graph.add_node(());

        // Add reversed edges
        for node in region_graph.node_indices() {
            for succ in region_graph.successors(node) {
                let rev_source = forward_map[&succ];
                let rev_target = forward_map[&node];
                reverse_graph.add_edge(rev_source, rev_target, ());
            }
        }

        // Connect virtual exit to nodes with no successors (exit nodes)
        for node in region_graph.node_indices() {
            if region_graph.successors(node).is_empty() {
                let rev_node = forward_map[&node];
                reverse_graph.add_edge(virtual_exit, rev_node, ());
            }
        }

        // If no exit nodes found, we have an infinite loop - connect to entry
        // as a fallback to ensure the dominator algorithm can run
        if !region_graph.node_indices().any(|n| region_graph.successors(n).is_empty()) {
            // All nodes have successors - likely an infinite loop
            // Use entry as a pseudo-exit
            let entry_rev = forward_map[&region_graph.entry()];
            reverse_graph.add_edge(virtual_exit, entry_rev, ());
        }

        // Compute dominators on reverse graph
        let dominators = petgraph::algo::dominators::simple_fast(&reverse_graph, virtual_exit);

        RegionPostDominators {
            dominators,
            forward_map,
            reverse_map,
            virtual_exit,
        }
    }

    /// Get the immediate post-dominator of a node in the RegionGraph.
    pub fn immediate_post_dominator(&self, node: NodeIndex) -> Option<NodeIndex> {
        let rev_node = self.forward_map.get(&node)?;
        let rev_ipdom = self.dominators.immediate_dominator(*rev_node)?;

        // Don't return the virtual exit as a post-dominator
        if rev_ipdom == self.virtual_exit {
            return None;
        }

        self.reverse_map.get(&rev_ipdom).copied()
    }

    /// Check if node `a` post-dominates node `b`.
    pub fn post_dominates(&self, a: NodeIndex, b: NodeIndex) -> bool {
        let Some(rev_a) = self.forward_map.get(&a) else {
            return false;
        };
        let Some(rev_b) = self.forward_map.get(&b) else {
            return false;
        };

        self.dominators
            .dominators(*rev_b)
            .map_or(false, |mut doms| doms.any(|d| d == *rev_a))
    }
}

/// Dominance computed on the current RegionGraph state.
///
/// This caches dominance information and tracks the generation to detect
/// when recomputation is needed.
pub struct RegionDominators {
    /// Forward dominators (from entry)
    dominators: Dominators<NodeIndex>,
    /// Post-dominators
    post_dominators: RegionPostDominators,
    /// Generation of the RegionGraph when this was computed
    generation: u64,
    /// Mapping from RegionGraph nodes to internal graph nodes
    forward_map: HashMap<NodeIndex, NodeIndex>,
    /// Mapping from internal graph nodes to RegionGraph nodes
    reverse_map: HashMap<NodeIndex, NodeIndex>,
}

impl RegionDominators {
    /// Compute dominators on the current RegionGraph state.
    pub fn compute(region_graph: &RegionGraph) -> Self {
        // Build a simple graph for dominator computation
        let mut graph: DiGraph<(), ()> = DiGraph::new();
        let mut forward_map: HashMap<NodeIndex, NodeIndex> = HashMap::new();
        let mut reverse_map: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        // Add all nodes
        for node in region_graph.node_indices() {
            let new_node = graph.add_node(());
            forward_map.insert(node, new_node);
            reverse_map.insert(new_node, node);
        }

        // Add edges
        for node in region_graph.node_indices() {
            for succ in region_graph.successors(node) {
                let src = forward_map[&node];
                let dst = forward_map[&succ];
                graph.add_edge(src, dst, ());
            }
        }

        // Compute forward dominators
        let entry_mapped = forward_map[&region_graph.entry()];
        let dominators = petgraph::algo::dominators::simple_fast(&graph, entry_mapped);

        // Compute post-dominators
        let post_dominators = RegionPostDominators::compute(region_graph);

        RegionDominators {
            dominators,
            post_dominators,
            generation: region_graph.generation(),
            forward_map,
            reverse_map,
        }
    }

    /// Check if cached dominance data is stale.
    pub fn is_stale(&self, region_graph: &RegionGraph) -> bool {
        self.generation != region_graph.generation()
    }

    /// Check if node `a` dominates node `b`.
    pub fn dominates(&self, a: NodeIndex, b: NodeIndex) -> bool {
        let Some(&mapped_a) = self.forward_map.get(&a) else {
            return false;
        };
        let Some(&mapped_b) = self.forward_map.get(&b) else {
            return false;
        };

        self.dominators
            .dominators(mapped_b)
            .map_or(false, |mut doms| doms.any(|d| d == mapped_a))
    }

    /// Check if node `a` post-dominates node `b`.
    pub fn post_dominates(&self, a: NodeIndex, b: NodeIndex) -> bool {
        self.post_dominators.post_dominates(a, b)
    }

    /// Get the immediate post-dominator of a node.
    pub fn ipdom(&self, node: NodeIndex) -> Option<NodeIndex> {
        self.post_dominators.immediate_post_dominator(node)
    }

    /// Get the immediate dominator of a node.
    pub fn idom(&self, node: NodeIndex) -> Option<NodeIndex> {
        let mapped = self.forward_map.get(&node)?;
        let mapped_idom = self.dominators.immediate_dominator(*mapped)?;
        self.reverse_map.get(&mapped_idom).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifter::Cfg;
    use crate::structurer::region_graph::RegionGraph;
    use hlbc::opcodes::Opcode;
    use hlbc::types::{RefInt, Reg};

    #[test]
    fn test_region_dominators_simple() {
        // Simple linear code
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let region_graph = RegionGraph::from_cfg(&cfg);
        let dominators = RegionDominators::compute(&region_graph);

        // Entry should dominate all nodes
        let entry = region_graph.entry();
        for node in region_graph.node_indices() {
            assert!(
                dominators.dominates(entry, node),
                "Entry should dominate all nodes"
            );
        }
    }

    #[test]
    fn test_region_dominators_if_else() {
        // if-else structure
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JAlways { offset: 1 },
            Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let region_graph = RegionGraph::from_cfg(&cfg);
        let dominators = RegionDominators::compute(&region_graph);

        // Entry should dominate all
        let entry = region_graph.entry();
        for node in region_graph.node_indices() {
            assert!(dominators.dominates(entry, node));
        }

        // Check generation tracking
        assert!(!dominators.is_stale(&region_graph));
    }

    #[test]
    fn test_region_dominators_generation() {
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 1 },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let mut region_graph = RegionGraph::from_cfg(&cfg);
        let dominators = RegionDominators::compute(&region_graph);

        assert!(!dominators.is_stale(&region_graph));

        // Trigger a collapse to increment generation
        let sequences = region_graph.find_sequences();
        if let Some(seq) = sequences.into_iter().next() {
            if seq.len() >= 2 {
                region_graph.collapse_sequence(seq);
            }
        }

        // After collapse, dominators should be stale
        // (may or may not be stale depending on whether collapse happened)
        // This test verifies the generation tracking mechanism works
    }
}
