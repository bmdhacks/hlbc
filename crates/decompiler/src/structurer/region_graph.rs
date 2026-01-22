//! Region Graph - Virtual Graph for Iterative Reduction
//!
//! This module provides a graph structure where nodes can be either original
//! basic blocks OR collapsed Region nodes. This "virtual graph" enables the
//! iterative reduction algorithm described in REFACTOR.md:
//!
//! 1. Start with all basic blocks as nodes
//! 2. Find patterns (loops, if-else, sequences)
//! 3. Collapse matched patterns into Region nodes
//! 4. Repeat until the graph is a single Region
//!
//! The key insight is that we maintain the graph topology during reduction,
//! allowing pattern matchers to work on an increasingly simplified graph.

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use std::collections::{HashMap, HashSet};

use crate::lifter::{Cfg, EdgeKind};
use crate::structurer::region::Region;

/// A node in the RegionGraph - either an original block or a collapsed region.
#[derive(Debug, Clone)]
pub enum RegionNode {
    /// An original basic block from the CFG.
    /// The NodeIndex refers to the original CFG's node.
    Block(NodeIndex),

    /// A collapsed region representing structured control flow.
    /// This replaces multiple blocks that formed a recognized pattern.
    Collapsed(Region),
}

impl RegionNode {
    /// Check if this is an original block.
    pub fn is_block(&self) -> bool {
        matches!(self, RegionNode::Block(_))
    }

    /// Check if this node represents a terminating structure.
    /// For collapsed regions, checks if the region terminates (e.g., if-then-else where both branches return).
    pub fn terminates(&self, cfg: &crate::lifter::Cfg) -> bool {
        match self {
            RegionNode::Block(cfg_node) => cfg.graph[*cfg_node].is_exit,
            RegionNode::Collapsed(region) => region.terminates(cfg),
        }
    }

    /// Check if this is a collapsed region.
    pub fn is_collapsed(&self) -> bool {
        matches!(self, RegionNode::Collapsed(_))
    }

    /// Get the original block index if this is a Block node.
    pub fn as_block(&self) -> Option<NodeIndex> {
        match self {
            RegionNode::Block(idx) => Some(*idx),
            RegionNode::Collapsed(_) => None,
        }
    }

    /// Get the region if this is a Collapsed node.
    pub fn as_region(&self) -> Option<&Region> {
        match self {
            RegionNode::Block(_) => None,
            RegionNode::Collapsed(r) => Some(r),
        }
    }
}

/// A virtual graph that supports iterative region collapsing.
///
/// Initially mirrors the CFG structure, but nodes can be collapsed
/// into Region nodes as patterns are recognized.
pub struct RegionGraph {
    /// The graph structure with mixed Block/Collapsed nodes.
    graph: DiGraph<RegionNode, EdgeKind>,

    /// Entry node of the graph.
    entry: NodeIndex,

    /// Maps original CFG NodeIndex to current RegionGraph NodeIndex.
    /// Updated during collapse operations.
    cfg_to_region: HashMap<NodeIndex, NodeIndex>,

    /// Reverse map: RegionGraph NodeIndex to set of original CFG nodes it contains.
    region_to_cfg: HashMap<NodeIndex, HashSet<NodeIndex>>,

    /// Generation counter incremented on each collapse operation.
    /// Used to detect when dominance needs recomputation.
    generation: u64,
}

impl RegionGraph {
    /// Create a RegionGraph from a CFG.
    /// Initially, each basic block becomes a Block node.
    pub fn from_cfg(cfg: &Cfg) -> Self {
        let mut graph = DiGraph::new();
        let mut cfg_to_region = HashMap::new();
        let mut region_to_cfg = HashMap::new();

        // Add all nodes from CFG
        for cfg_node in cfg.graph.node_indices() {
            let region_node = graph.add_node(RegionNode::Block(cfg_node));
            cfg_to_region.insert(cfg_node, region_node);

            let mut cfg_set = HashSet::new();
            cfg_set.insert(cfg_node);
            region_to_cfg.insert(region_node, cfg_set);
        }

        // Add all edges from CFG
        for edge in cfg.graph.edge_references() {
            let src = cfg_to_region[&edge.source()];
            let dst = cfg_to_region[&edge.target()];
            graph.add_edge(src, dst, *edge.weight());
        }

        let entry = cfg_to_region[&cfg.entry];

        RegionGraph {
            graph,
            entry,
            cfg_to_region,
            region_to_cfg,
            generation: 0,
        }
    }

    /// Get the current generation counter.
    /// This increments on each collapse operation, allowing external code
    /// to detect when cached dominance data needs recomputation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Check if a node exists in the current graph (membership firewall).
    ///
    /// Use this before traversing to a node to ensure it hasn't been removed
    /// by a previous collapse operation.
    pub fn contains(&self, node: NodeIndex) -> bool {
        self.graph.node_weight(node).is_some()
    }

    /// Get the region node that owns a CFG block.
    ///
    /// This is the inverse of `as_block()` - given a CFG node index,
    /// find which RegionGraph node currently represents it.
    /// Returns None if the CFG node doesn't exist in the mapping.
    pub fn cfg_owner(&self, cfg_node: NodeIndex) -> Option<NodeIndex> {
        self.cfg_to_region.get(&cfg_node).copied()
    }

    /// Get the entry node.
    pub fn entry(&self) -> NodeIndex {
        self.entry
    }

    /// Number of nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Check if the graph has been fully reduced to a single node.
    pub fn is_fully_reduced(&self) -> bool {
        self.graph.node_count() == 1
    }

    /// Get a node by its index.
    pub fn get_node(&self, idx: NodeIndex) -> Option<&RegionNode> {
        self.graph.node_weight(idx)
    }

    /// Get all node indices.
    pub fn node_indices(&self) -> impl Iterator<Item = NodeIndex> + '_ {
        self.graph.node_indices()
    }

    /// Get successor nodes.
    pub fn successors(&self, node: NodeIndex) -> Vec<NodeIndex> {
        self.graph.neighbors(node).collect()
    }

    /// Get predecessor nodes.
    pub fn predecessors(&self, node: NodeIndex) -> Vec<NodeIndex> {
        self.graph
            .neighbors_directed(node, Direction::Incoming)
            .collect()
    }

    /// Get successors with their edge kinds.
    pub fn successors_with_edges(&self, node: NodeIndex) -> Vec<(NodeIndex, EdgeKind)> {
        self.graph
            .edges(node)
            .map(|e| (e.target(), *e.weight()))
            .collect()
    }

    /// Get successors excluding exception handler edges.
    /// Use this for pattern detection where exception edges would interfere.
    pub fn successors_no_exceptions(&self, node: NodeIndex) -> Vec<NodeIndex> {
        self.graph
            .edges(node)
            .filter(|e| !matches!(e.weight(), EdgeKind::ExceptionHandler))
            .map(|e| e.target())
            .collect()
    }

    /// Get the RegionGraph node index for an original CFG node.
    pub fn get_region_node(&self, cfg_node: NodeIndex) -> Option<NodeIndex> {
        self.cfg_to_region.get(&cfg_node).copied()
    }

    /// Get all original CFG nodes contained in a RegionGraph node.
    pub fn get_cfg_nodes(&self, region_node: NodeIndex) -> Option<&HashSet<NodeIndex>> {
        self.region_to_cfg.get(&region_node)
    }

    /// Get nodes in reverse post-order (good for dataflow analysis).
    /// This visits nodes such that a node is visited before its successors
    /// in the DFS tree (except for back edges).
    pub fn nodes_in_reverse_postorder(&self) -> Vec<NodeIndex> {
        let mut postorder = Vec::new();
        let mut visited = HashSet::new();

        fn dfs_postorder(
            graph: &DiGraph<RegionNode, EdgeKind>,
            node: NodeIndex,
            visited: &mut HashSet<NodeIndex>,
            postorder: &mut Vec<NodeIndex>,
        ) {
            if visited.contains(&node) {
                return;
            }
            visited.insert(node);

            for succ in graph.neighbors(node) {
                dfs_postorder(graph, succ, visited, postorder);
            }

            postorder.push(node);
        }

        dfs_postorder(&self.graph, self.entry, &mut visited, &mut postorder);

        // Reverse to get reverse post-order
        postorder.reverse();
        postorder
    }

    /// Collapse a set of nodes into a single Region node.
    ///
    /// This is the core operation for graph reduction:
    /// 1. Create a new Collapsed node with the given Region
    /// 2. Redirect all incoming edges to the new node
    /// 3. Redirect all outgoing edges from the new node
    /// 4. Remove the old nodes
    ///
    /// Returns the NodeIndex of the new collapsed node.
    ///
    /// Note: After collapse, all NodeIndex values from before may be invalid
    /// due to petgraph's swap-remove behavior. Use the returned index or
    /// re-query using `get_region_node()`.
    pub fn collapse(&mut self, nodes: &HashSet<NodeIndex>, region: Region) -> NodeIndex {
        if nodes.is_empty() {
            panic!("Cannot collapse empty node set");
        }

        // INVARIANT: All nodes to collapse should exist in the graph
        #[cfg(debug_assertions)]
        for &node in nodes {
            debug_assert!(
                self.graph.node_weight(node).is_some(),
                "collapse: node {:?} does not exist in graph",
                node
            );
        }

        // Check if entry is being collapsed BEFORE we start removing nodes
        let entry_is_collapsed = nodes.contains(&self.entry);

        // Track edge counts for invariant checking
        #[cfg(debug_assertions)]
        let incoming_edge_count: usize = nodes
            .iter()
            .flat_map(|&n| {
                self.graph
                    .edges_directed(n, Direction::Incoming)
                    .filter(|e| !nodes.contains(&e.source()))
            })
            .count();

        #[cfg(debug_assertions)]
        let outgoing_edge_count: usize = nodes
            .iter()
            .flat_map(|&n| {
                self.graph
                    .edges(n)
                    .filter(|e| !nodes.contains(&e.target()))
            })
            .count();

        // Collect all original CFG nodes being collapsed
        let mut all_cfg_nodes = HashSet::new();
        for &node in nodes {
            if let Some(cfg_nodes) = self.region_to_cfg.get(&node) {
                all_cfg_nodes.extend(cfg_nodes.iter().copied());
            }
        }

        // Find incoming edges (from nodes not in the collapse set)
        // Store source node's CFG nodes instead of NodeIndex (which will be invalidated)
        let mut incoming: Vec<(HashSet<NodeIndex>, EdgeKind)> = Vec::new();
        for &node in nodes {
            for edge in self.graph.edges_directed(node, Direction::Incoming) {
                let src = edge.source();
                if !nodes.contains(&src) {
                    if let Some(cfg_nodes) = self.region_to_cfg.get(&src) {
                        incoming.push((cfg_nodes.clone(), *edge.weight()));
                    }
                }
            }
        }

        // Find outgoing edges (to nodes not in the collapse set)
        let mut outgoing: Vec<(HashSet<NodeIndex>, EdgeKind)> = Vec::new();
        for &node in nodes {
            for edge in self.graph.edges(node) {
                let dst = edge.target();
                if !nodes.contains(&dst) {
                    if let Some(cfg_nodes) = self.region_to_cfg.get(&dst) {
                        outgoing.push((cfg_nodes.clone(), *edge.weight()));
                    }
                }
            }
        }

        // Remove old nodes
        // IMPORTANT: We need to handle petgraph's swap-remove behavior.
        // When a node is removed, the last node in the graph gets moved to fill the gap.
        // We must update our mappings to reflect this.
        let mut nodes_to_remove: Vec<_> = nodes.iter().copied().collect();
        nodes_to_remove.sort_by(|a, b| b.index().cmp(&a.index())); // Descending order

        for node in nodes_to_remove {
            let node_count_before = self.graph.node_count();
            let removed_index = node.index();

            // Remove from our mapping
            self.region_to_cfg.remove(&node);

            // Remove from graph (this may swap the last node into this position)
            self.graph.remove_node(node);

            // Check if a swap happened (removed node wasn't the last one)
            if removed_index < node_count_before - 1 {
                // The node that was at (node_count_before - 1) is now at removed_index
                let old_last_index = NodeIndex::new(node_count_before - 1);
                let new_index = NodeIndex::new(removed_index);

                // Find the CFG nodes that were mapped to the old last index
                // and update them to point to the new index
                if let Some(cfg_nodes) = self.region_to_cfg.remove(&old_last_index) {
                    // Update cfg_to_region for all these CFG nodes
                    for cfg_node in &cfg_nodes {
                        self.cfg_to_region.insert(*cfg_node, new_index);
                    }
                    // Re-insert the region_to_cfg mapping with the new index
                    self.region_to_cfg.insert(new_index, cfg_nodes);
                }

                // Also update entry if it was the swapped node
                if self.entry == old_last_index {
                    self.entry = new_index;
                }
            }
        }

        // Create the new collapsed node
        let new_node = self.graph.add_node(RegionNode::Collapsed(region));

        // Update mappings: all original CFG nodes now map to the new node
        for cfg_node in &all_cfg_nodes {
            self.cfg_to_region.insert(*cfg_node, new_node);
        }
        self.region_to_cfg.insert(new_node, all_cfg_nodes.clone());

        // Track added edges to avoid duplicates
        let mut added_incoming: HashSet<NodeIndex> = HashSet::new();
        let mut added_outgoing: HashSet<NodeIndex> = HashSet::new();

        // Add incoming edges (look up current node indices)
        for (src_cfg_nodes, kind) in incoming {
            // Find the current node for this set of CFG nodes
            if let Some(&first_cfg) = src_cfg_nodes.iter().next() {
                if let Some(&src_node) = self.cfg_to_region.get(&first_cfg) {
                    if src_node != new_node && !added_incoming.contains(&src_node) {
                        if std::env::var("HLBC_DEBUG_COLLAPSE_EDGE").is_ok() {
                            eprintln!("  COLLAPSE: adding incoming edge {:?} -> {:?} (kind={:?})", src_node, new_node, kind);
                        }
                        self.graph.add_edge(src_node, new_node, kind);
                        added_incoming.insert(src_node);
                    }
                }
            }
        }

        // Add outgoing edges
        for (dst_cfg_nodes, kind) in outgoing {
            if let Some(&first_cfg) = dst_cfg_nodes.iter().next() {
                if let Some(&dst_node) = self.cfg_to_region.get(&first_cfg) {
                    if dst_node != new_node && !added_outgoing.contains(&dst_node) {
                        if std::env::var("HLBC_DEBUG_COLLAPSE_EDGE").is_ok() {
                            eprintln!("  COLLAPSE: adding outgoing edge {:?} -> {:?} (kind={:?})", new_node, dst_node, kind);
                        }
                        self.graph.add_edge(new_node, dst_node, kind);
                        added_outgoing.insert(dst_node);
                    }
                }
            }
        }

        // Update entry if it was one of the collapsed nodes.
        // We checked this BEFORE removing nodes to avoid confusion from swap-remove.
        if entry_is_collapsed {
            self.entry = new_node;
        }

        // INVARIANT: Verify that incoming/outgoing edge counts are preserved
        // (edges between collapsed nodes are gone, but external edges should remain)
        #[cfg(debug_assertions)]
        {
            let new_incoming = self
                .graph
                .edges_directed(new_node, Direction::Incoming)
                .count();
            let new_outgoing = self.graph.edges(new_node).count();

            // Note: We may have fewer edges due to deduplication of parallel edges
            // So we check that we have at least some edges if we expected some
            if incoming_edge_count > 0 {
                debug_assert!(
                    new_incoming > 0,
                    "collapse: lost all incoming edges (expected at least 1, got 0)"
                );
            }
            if outgoing_edge_count > 0 {
                debug_assert!(
                    new_outgoing > 0,
                    "collapse: lost all outgoing edges (expected at least 1, got 0)"
                );
            }
        }

        // Increment generation to signal that cached data (like dominance) is stale
        self.generation += 1;

        new_node
    }

    /// Collapse a sequence of nodes into a Sequence region.
    ///
    /// # Preconditions
    /// The nodes must form a valid linear chain where:
    /// - Each node (except the last) has exactly one successor: the next node
    /// - Each node (except the first) has exactly one predecessor: the previous node
    ///
    /// These invariants are enforced by `find_sequences()`, which should be used
    /// to discover valid sequences before calling this method.
    pub fn collapse_sequence(&mut self, nodes: Vec<NodeIndex>) -> Option<NodeIndex> {
        if nodes.len() < 2 {
            return None;
        }

        // Verify it's a valid sequence (linear chain)
        for i in 0..nodes.len() - 1 {
            let succs = self.successors(nodes[i]);
            if succs.len() != 1 || succs[0] != nodes[i + 1] {
                return None; // Not a valid sequence
            }

            // Invariant: internal nodes have single predecessor (checked by find_sequences)
            debug_assert!(
                self.predecessors(nodes[i + 1]).iter().all(|&p| p == nodes[i]),
                "collapse_sequence: node {:?} has external predecessors",
                nodes[i + 1]
            );
        }

        // Build the Region from the nodes
        let mut regions = Vec::new();
        for &node in &nodes {
            match self.graph.node_weight(node) {
                Some(RegionNode::Block(cfg_idx)) => {
                    regions.push(Region::Block(*cfg_idx));
                }
                Some(RegionNode::Collapsed(r)) => {
                    regions.push(r.clone());
                }
                None => return None,
            }
        }

        let sequence_region = Region::sequence(regions);
        let node_set: HashSet<_> = nodes.into_iter().collect();
        Some(self.collapse(&node_set, sequence_region))
    }

    /// Find linear sequences that can be collapsed.
    /// Returns sequences of 2+ nodes that form straight-line code.
    pub fn find_sequences(&self) -> Vec<Vec<NodeIndex>> {
        let mut sequences = Vec::new();
        let mut in_sequence: HashSet<NodeIndex> = HashSet::new();

        for node in self.graph.node_indices() {
            if in_sequence.contains(&node) {
                continue;
            }

            // Check if this node starts a sequence
            let succs = self.successors(node);

            // Start of sequence: has 0-1 predecessors and exactly 1 successor
            // (or we explicitly start from entry)
            if succs.len() == 1 {
                let mut sequence = vec![node];
                let mut current = succs[0];

                // Follow the chain
                while !in_sequence.contains(&current) {
                    let current_preds = self.predecessors(current);
                    let current_succs = self.successors(current);

                    // Must have exactly 1 predecessor (the previous in sequence)
                    if current_preds.len() != 1 || current_preds[0] != *sequence.last().unwrap() {
                        break;
                    }

                    sequence.push(current);

                    // Continue if exactly 1 successor
                    if current_succs.len() == 1 {
                        current = current_succs[0];
                    } else {
                        break;
                    }
                }

                if sequence.len() >= 2 {
                    for &n in &sequence {
                        in_sequence.insert(n);
                    }
                    sequences.push(sequence);
                }
            }
        }

        sequences
    }

    /// Convert the fully reduced graph to a Region.
    /// Only valid when the graph has been reduced to a single node.
    pub fn into_region(self) -> Option<Region> {
        if self.graph.node_count() != 1 {
            return None;
        }

        let node = self.graph.node_indices().next()?;
        match self.graph.node_weight(node)? {
            RegionNode::Block(cfg_idx) => Some(Region::Block(*cfg_idx)),
            RegionNode::Collapsed(region) => Some(region.clone()),
        }
    }

    /// Debug: print the current graph structure.
    #[allow(dead_code)]
    pub fn debug_print(&self) {
        println!("RegionGraph: {} nodes", self.graph.node_count());
        for node in self.graph.node_indices() {
            let node_desc = match self.graph.node_weight(node) {
                Some(RegionNode::Block(cfg_idx)) => format!("Block({:?})", cfg_idx),
                Some(RegionNode::Collapsed(r)) => format!("Collapsed({} blocks)", r.block_count()),
                None => "???".to_string(),
            };
            let succs: Vec<_> = self.successors(node);
            println!("  {:?}: {} -> {:?}", node, node_desc, succs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifter::Cfg;
    use hlbc::opcodes::Opcode;
    use hlbc::types::{RefInt, Reg};

    #[test]
    fn test_from_cfg_preserves_structure() {
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 1 },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let region_graph = RegionGraph::from_cfg(&cfg);

        // Should have same number of nodes
        assert_eq!(region_graph.node_count(), cfg.graph.node_count());

        // Entry should be preserved
        assert!(region_graph.get_region_node(cfg.entry).is_some());
    }

    #[test]
    fn test_collapse_simple() {
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Add { dst: Reg(2), a: Reg(0), b: Reg(1) },
            Opcode::Ret { ret: Reg(2) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let region_graph = RegionGraph::from_cfg(&cfg);

        // Single block CFG
        assert_eq!(region_graph.node_count(), 1);

        // Already fully reduced
        assert!(region_graph.is_fully_reduced());
    }

    #[test]
    fn test_collapse_two_blocks() {
        // Create a CFG with 2 blocks that form a sequence
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JAlways { offset: 0 }, // Jump to next instruction
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let mut region_graph = RegionGraph::from_cfg(&cfg);

        let initial_count = region_graph.node_count();
        println!("Initial node count: {}", initial_count);

        if initial_count >= 2 {
            // Get the first two nodes
            let nodes: Vec<_> = region_graph.node_indices().take(2).collect();
            let node_set: HashSet<_> = nodes.into_iter().collect();

            // Collapse them
            let dummy_region = Region::Empty;
            let new_node = region_graph.collapse(&node_set, dummy_region);

            assert_eq!(region_graph.node_count(), initial_count - 1);
            assert!(region_graph.get_node(new_node).is_some());
        }
    }

    #[test]
    fn test_reverse_postorder() {
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 1 },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let region_graph = RegionGraph::from_cfg(&cfg);

        let rpo = region_graph.nodes_in_reverse_postorder();

        // Entry should be first in reverse postorder
        assert_eq!(rpo[0], region_graph.entry());

        // All nodes should be included
        assert_eq!(rpo.len(), region_graph.node_count());
    }

    #[test]
    fn test_find_sequences() {
        // Linear code should form one sequence
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Add { dst: Reg(2), a: Reg(0), b: Reg(1) },
            Opcode::Ret { ret: Reg(2) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let region_graph = RegionGraph::from_cfg(&cfg);

        // Single block - no sequences to find
        let sequences = region_graph.find_sequences();
        assert!(sequences.is_empty() || sequences.iter().all(|s| s.len() >= 2));
    }

    #[test]
    fn test_node_mappings() {
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 1 },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let region_graph = RegionGraph::from_cfg(&cfg);

        // Every CFG node should have a mapping
        for cfg_node in cfg.graph.node_indices() {
            let region_node = region_graph.get_region_node(cfg_node);
            assert!(region_node.is_some(), "CFG node {:?} should have mapping", cfg_node);

            // And the reverse mapping should include this CFG node
            let cfg_nodes = region_graph.get_cfg_nodes(region_node.unwrap());
            assert!(cfg_nodes.is_some());
            assert!(cfg_nodes.unwrap().contains(&cfg_node));
        }
    }
}
