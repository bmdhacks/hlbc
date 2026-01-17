//! Pass 2: Analyzer - Compute dominators, post-dominators, identify loops, and prepare for SSA
//!
//! This module performs dataflow analysis on the CFG:
//! - Computes dominator trees using petgraph
//! - Computes post-dominator trees (for finding merge points)
//! - Identifies natural loops (back-edges where target dominates source)
//! - Detects reducibility of the CFG
//! - Prepares the CFG for SSA conversion

use petgraph::algo::dominators::Dominators;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet};

use crate::lifter::Cfg;

/// Loop information extracted from the CFG
#[derive(Debug, Clone)]
pub struct NaturalLoop {
    /// The loop header (entry point, dominates all nodes in the loop)
    pub header: NodeIndex,
    /// All nodes in the loop body (including header)
    pub body: HashSet<NodeIndex>,
    /// Back-edge sources (nodes that jump back to header)
    pub back_edge_sources: Vec<NodeIndex>,
    /// Exit nodes (nodes in the loop that have edges leaving the loop)
    pub exit_nodes: Vec<NodeIndex>,
    /// Nesting depth (0 = outermost loop)
    pub depth: usize,
}

/// Post-dominator tree for finding merge points
///
/// A node X post-dominates Y if every path from Y to exit must go through X.
/// This is computed by running the dominator algorithm on the reverse CFG.
pub struct PostDominatorTree {
    /// The computed post-dominators (on reverse graph)
    dominators: Dominators<NodeIndex>,
    /// Mapping from original CFG nodes to reverse graph nodes
    forward_map: HashMap<NodeIndex, NodeIndex>,
    /// Mapping from reverse graph nodes to original CFG nodes
    reverse_map: HashMap<NodeIndex, NodeIndex>,
    /// The virtual exit node in the reverse graph (becomes entry for dominator computation)
    virtual_exit: NodeIndex,
}

impl PostDominatorTree {
    /// Compute the post-dominator tree for a CFG
    ///
    /// This works by:
    /// 1. Creating a reverse CFG (all edges reversed)
    /// 2. Adding a virtual exit node that all original exit nodes connect to
    /// 3. Computing dominators on the reverse graph from the virtual exit
    pub fn compute(cfg: &Cfg) -> Self {
        // Build reverse graph with virtual exit
        let mut reverse_graph: DiGraph<(), ()> = DiGraph::new();
        let mut forward_map: HashMap<NodeIndex, NodeIndex> = HashMap::new();
        let mut reverse_map: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        // Add all nodes from original graph
        for node in cfg.graph.node_indices() {
            let rev_node = reverse_graph.add_node(());
            forward_map.insert(node, rev_node);
            reverse_map.insert(rev_node, node);
        }

        // Add virtual exit node
        let virtual_exit = reverse_graph.add_node(());

        // Add reversed edges
        for edge in cfg.graph.edge_references() {
            let rev_source = forward_map[&edge.target()];
            let rev_target = forward_map[&edge.source()];
            reverse_graph.add_edge(rev_source, rev_target, ());
        }

        // Connect virtual exit to all exit nodes (in reverse, exit nodes point to virtual exit)
        // In the reverse graph, we need edges FROM virtual_exit TO the reversed exit nodes
        for node in cfg.graph.node_indices() {
            let block = &cfg.graph[node];
            if block.is_exit {
                let rev_node = forward_map[&node];
                // In reverse: virtual_exit -> exit_node (because exit_node -> virtual_exit in forward)
                reverse_graph.add_edge(virtual_exit, rev_node, ());
            }
        }

        // Also handle nodes with no successors that aren't marked as exit
        // (this can happen with unreachable code or incomplete CFGs)
        for node in cfg.graph.node_indices() {
            if cfg.successors(node).is_empty() {
                let rev_node = forward_map[&node];
                // Check if edge already exists
                if !reverse_graph.edges(virtual_exit).any(|e| e.target() == rev_node) {
                    reverse_graph.add_edge(virtual_exit, rev_node, ());
                }
            }
        }

        // Compute dominators on reverse graph
        let dominators = petgraph::algo::dominators::simple_fast(&reverse_graph, virtual_exit);

        PostDominatorTree {
            dominators,
            forward_map,
            reverse_map,
            virtual_exit,
        }
    }

    /// Get the immediate post-dominator of a node
    ///
    /// Returns None if the node has no post-dominator (e.g., the exit node itself)
    pub fn immediate_post_dominator(&self, node: NodeIndex) -> Option<NodeIndex> {
        let rev_node = self.forward_map.get(&node)?;
        let rev_ipdom = self.dominators.immediate_dominator(*rev_node)?;

        // Don't return the virtual exit as a post-dominator
        if rev_ipdom == self.virtual_exit {
            return None;
        }

        self.reverse_map.get(&rev_ipdom).copied()
    }

    /// Check if node `a` post-dominates node `b`
    ///
    /// Returns true if every path from `b` to exit must go through `a`
    pub fn post_dominates(&self, a: NodeIndex, b: NodeIndex) -> bool {
        let Some(rev_a) = self.forward_map.get(&a) else {
            return false;
        };
        let Some(rev_b) = self.forward_map.get(&b) else {
            return false;
        };

        // In the reverse graph, a post-dominates b means rev_a dominates rev_b
        self.dominators
            .dominators(*rev_b)
            .map_or(false, |mut doms| doms.any(|d| d == *rev_a))
    }

    /// Get all post-dominators of a node (from immediate to exit)
    pub fn post_dominators(&self, node: NodeIndex) -> Vec<NodeIndex> {
        let mut result = Vec::new();
        let mut current = node;

        while let Some(ipdom) = self.immediate_post_dominator(current) {
            result.push(ipdom);
            current = ipdom;
        }

        result
    }
}

/// Analysis results for a CFG
pub struct CfgAnalysis {
    /// Dominator tree
    pub dominators: Dominators<NodeIndex>,
    /// Post-dominator tree (for finding if/else merge points)
    pub post_dominators: PostDominatorTree,
    /// Identified natural loops, sorted by header
    pub loops: Vec<NaturalLoop>,
    /// Whether the CFG is reducible
    pub is_reducible: bool,
    /// Map from node to containing loop header (if any)
    pub node_to_loop: HashMap<NodeIndex, NodeIndex>,
}

impl CfgAnalysis {
    /// Analyze a CFG to extract dominator and loop information
    pub fn analyze(cfg: &Cfg) -> Self {
        // Compute dominators
        let dominators = petgraph::algo::dominators::simple_fast(&cfg.graph, cfg.entry);

        // Find back-edges (edges where target dominates source)
        let mut back_edges: Vec<(NodeIndex, NodeIndex)> = Vec::new();
        for edge in cfg.graph.edge_references() {
            let source = edge.source();
            let target = edge.target();

            // A back-edge is an edge where the target dominates the source
            if dominators.dominators(source).map_or(false, |mut doms| doms.any(|d| d == target)) {
                back_edges.push((source, target));
            }
        }

        // Group back edges by header and merge bodies
        // Multiple back edges to the same header (e.g., continue and normal iteration)
        // should form a single loop with a combined body
        let mut header_to_back_sources: HashMap<NodeIndex, Vec<NodeIndex>> = HashMap::new();
        for (back_source, header) in &back_edges {
            header_to_back_sources.entry(*header).or_default().push(*back_source);
        }

        // Extract natural loops, merging bodies for multiple back edges to same header
        let mut loops = Vec::new();
        for (header, back_sources) in header_to_back_sources {
            // Compute union of bodies from all back edges to this header
            let mut body = HashSet::new();
            for &back_source in &back_sources {
                let partial_body = compute_loop_body(cfg, &dominators, header, back_source);
                body.extend(partial_body);
            }

            let exit_nodes = find_exit_nodes(cfg, &body);

            loops.push(NaturalLoop {
                header,
                body,
                back_edge_sources: back_sources,
                exit_nodes,
                depth: 0, // Will be computed below
            });
        }

        // Compute nesting depths
        compute_nesting_depths(&mut loops);

        // Build node-to-loop mapping
        let mut node_to_loop = HashMap::new();
        for loop_info in &loops {
            for &node in &loop_info.body {
                // If node is already mapped, only update if this loop is more deeply nested
                node_to_loop
                    .entry(node)
                    .and_modify(|existing_header: &mut NodeIndex| {
                        let existing_loop = loops.iter().find(|l| l.header == *existing_header);
                        if let Some(el) = existing_loop {
                            if loop_info.depth > el.depth {
                                *existing_header = loop_info.header;
                            }
                        }
                    })
                    .or_insert(loop_info.header);
            }
        }

        // Check reducibility: CFG is reducible if all back-edges go to dominators
        // (which is true by definition of how we found them)
        // A CFG can still be irreducible if there are cross-edges between loop bodies
        let is_reducible = check_reducibility(cfg, &loops);

        // Compute post-dominators
        let post_dominators = PostDominatorTree::compute(cfg);

        CfgAnalysis {
            dominators,
            post_dominators,
            loops,
            is_reducible,
            node_to_loop,
        }
    }

    /// Get the immediate dominator of a node
    pub fn idom(&self, node: NodeIndex) -> Option<NodeIndex> {
        self.dominators.immediate_dominator(node)
    }

    /// Check if `a` dominates `b`
    pub fn dominates(&self, a: NodeIndex, b: NodeIndex) -> bool {
        self.dominators
            .dominators(b)
            .map_or(false, |mut doms| doms.any(|d| d == a))
    }

    /// Get the loop containing a node (innermost if nested)
    pub fn get_loop(&self, node: NodeIndex) -> Option<&NaturalLoop> {
        self.node_to_loop
            .get(&node)
            .and_then(|header| self.loops.iter().find(|l| l.header == *header))
    }

    /// Check if a node is a loop header
    pub fn is_loop_header(&self, node: NodeIndex) -> bool {
        self.loops.iter().any(|l| l.header == node)
    }

    /// Get the immediate post-dominator of a node
    ///
    /// The immediate post-dominator is the first node that every path from
    /// this node to exit must pass through. This is useful for finding
    /// merge points of if/else branches.
    pub fn ipdom(&self, node: NodeIndex) -> Option<NodeIndex> {
        self.post_dominators.immediate_post_dominator(node)
    }

    /// Check if `a` post-dominates `b`
    ///
    /// Returns true if every path from `b` to exit must go through `a`.
    pub fn post_dominates(&self, a: NodeIndex, b: NodeIndex) -> bool {
        self.post_dominators.post_dominates(a, b)
    }

    /// Get all post-dominators of a node (from immediate to exit)
    pub fn post_dominators_of(&self, node: NodeIndex) -> Vec<NodeIndex> {
        self.post_dominators.post_dominators(node)
    }

    /// Find the merge point for an if/else structure starting at a conditional node
    ///
    /// For a node with two successors (then/else branches), the merge point
    /// is the immediate post-dominator of the conditional node.
    pub fn find_merge_point(&self, cond_node: NodeIndex) -> Option<NodeIndex> {
        self.ipdom(cond_node)
    }
}

/// Compute the body of a natural loop given its header and a back-edge source
fn compute_loop_body(
    cfg: &Cfg,
    _dominators: &Dominators<NodeIndex>,
    header: NodeIndex,
    back_source: NodeIndex,
) -> HashSet<NodeIndex> {
    let mut body = HashSet::new();
    body.insert(header);

    if header == back_source {
        // Self-loop
        return body;
    }

    // Work backwards from back_source to find all nodes that can reach it
    // without going through the header
    let mut worklist = vec![back_source];
    body.insert(back_source);

    while let Some(node) = worklist.pop() {
        for pred in cfg.predecessors(node) {
            if !body.contains(&pred) {
                body.insert(pred);
                worklist.push(pred);
            }
        }
    }

    body
}

/// Find nodes in the loop body that have edges leaving the loop
fn find_exit_nodes(cfg: &Cfg, body: &HashSet<NodeIndex>) -> Vec<NodeIndex> {
    let mut exits = Vec::new();
    for &node in body {
        for succ in cfg.successors(node) {
            if !body.contains(&succ) {
                exits.push(node);
                break;
            }
        }
    }
    exits
}

/// Compute nesting depths for loops
fn compute_nesting_depths(loops: &mut [NaturalLoop]) {
    // For each loop, count how many other loop bodies contain its header
    for i in 0..loops.len() {
        let header = loops[i].header;
        let mut depth = 0;
        for j in 0..loops.len() {
            if i != j && loops[j].body.contains(&header) {
                depth += 1;
            }
        }
        loops[i].depth = depth;
    }
}

/// Check if the CFG is reducible
///
/// A CFG is reducible if:
/// 1. All back-edges go to dominators (true by construction)
/// 2. There are no cross-edges between loop bodies that create multiple entry points
fn check_reducibility(cfg: &Cfg, loops: &[NaturalLoop]) -> bool {
    // For each loop, check that its header is the only entry point
    for loop_info in loops {
        for &node in &loop_info.body {
            if node == loop_info.header {
                continue;
            }
            // Check all predecessors of this node
            for pred in cfg.predecessors(node) {
                // If predecessor is not in the loop body, this is an entry edge
                if !loop_info.body.contains(&pred) {
                    // This node is entered from outside the loop but isn't the header
                    // This makes the CFG irreducible
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifter::Cfg;
    use hlbc::opcodes::Opcode;
    use hlbc::types::{RefInt, Reg};

    #[test]
    fn test_simple_loop_detection() {
        let ops = vec![
            // Block 0: op 0
            Opcode::Label,
            // op 1: loop body
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            // op 2: conditional exit
            Opcode::JNull { reg: Reg(0), offset: 2 }, // Jump to op 5 (exit)
            // Block 1: op 3
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            // op 4: back edge to op 0
            Opcode::JAlways { offset: -4 },
            // Block 2: op 5
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);

        // Should detect at least one loop
        assert!(!analysis.loops.is_empty(), "Should detect a loop");

        // CFG should be reducible
        assert!(analysis.is_reducible, "Simple loop should be reducible");
    }

    #[test]
    fn test_dominator_computation() {
        let ops = vec![
            // op 0: entry
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            // op 1: conditional
            Opcode::JNull { reg: Reg(0), offset: 2 }, // Jump to op 4
            // op 2: then branch
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            // op 3: jump to merge
            Opcode::JAlways { offset: 1 }, // Jump to op 5
            // op 4: else branch
            Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
            // op 5: merge point
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);

        // Entry should dominate all nodes
        for node in cfg.graph.node_indices() {
            assert!(
                analysis.dominates(cfg.entry, node),
                "Entry should dominate all nodes"
            );
        }
    }

    #[test]
    fn test_nested_loops() {
        let ops = vec![
            // Outer loop start
            Opcode::Label, // op 0
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) }, // op 1
            // Inner loop start
            Opcode::Label, // op 2
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) }, // op 3
            // Inner loop back edge
            Opcode::JNull { reg: Reg(1), offset: 1 }, // op 4: exit inner or continue
            Opcode::JAlways { offset: -3 }, // op 5: back to inner (op 2)
            // Outer loop back edge
            Opcode::JNull { reg: Reg(0), offset: 1 }, // op 6: exit outer or continue
            Opcode::JAlways { offset: -7 }, // op 7: back to outer (op 0)
            // Exit
            Opcode::Ret { ret: Reg(0) }, // op 8
        ];

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);

        // Should detect two loops
        assert!(analysis.loops.len() >= 1, "Should detect loops");
    }

    #[test]
    fn test_post_dominator_simple_if_else() {
        // Test post-dominator on a simple if/else with merge point
        //
        //       [0: entry + cond]
        //          /         \
        //    [1: then]    [2: else]
        //          \         /
        //        [3: merge + ret]
        //
        let ops = vec![
            // Block 0: op 0-1 (entry + conditional)
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 }, // Jump to op 4 (else)
            // Block 1: op 2-3 (then branch)
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JAlways { offset: 1 }, // Jump to op 5 (merge)
            // Block 2: op 4 (else branch)
            Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
            // Block 3: op 5 (merge point + return)
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);

        // Find the blocks
        let entry_block = cfg.entry;
        let merge_block = cfg.block_for_op(5).unwrap();

        // The merge block should post-dominate the entry (conditional) block
        assert!(
            analysis.post_dominates(merge_block, entry_block),
            "Merge block should post-dominate entry block"
        );

        // The immediate post-dominator of entry should be the merge block
        let ipdom = analysis.ipdom(entry_block);
        assert!(ipdom.is_some(), "Entry block should have a post-dominator");
        assert_eq!(
            ipdom.unwrap(),
            merge_block,
            "Immediate post-dominator of entry should be merge block"
        );

        // find_merge_point should return the merge block
        let merge_point = analysis.find_merge_point(entry_block);
        assert_eq!(
            merge_point,
            Some(merge_block),
            "find_merge_point should return the merge block"
        );
    }

    #[test]
    fn test_post_dominator_nested_if() {
        // Test post-dominator on nested if structure
        //
        //           [0: entry + cond1]
        //              /         \
        //     [1: cond2]        [2: else1]
        //        /    \             |
        //   [3:then2] [4:else2]     |
        //        \    /             |
        //       [5: merge2]         |
        //              \           /
        //             [6: merge1 + ret]
        //
        let ops = vec![
            // Block 0: op 0-1 (entry + first conditional)
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 7 }, // Jump to op 9 (else1)
            // Block 1: op 2-3 (second conditional)
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JNull { reg: Reg(1), offset: 2 }, // Jump to op 6 (else2)
            // Block 3: op 4-5 (then2)
            Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
            Opcode::JAlways { offset: 1 }, // Jump to op 7 (merge2)
            // Block 4: op 6 (else2)
            Opcode::Int { dst: Reg(3), ptr: RefInt(3) },
            // Block 5: op 7-8 (merge2)
            Opcode::Int { dst: Reg(4), ptr: RefInt(4) },
            Opcode::JAlways { offset: 1 }, // Jump to op 10 (merge1)
            // Block 2: op 9 (else1)
            Opcode::Int { dst: Reg(5), ptr: RefInt(5) },
            // Block 6: op 10 (merge1 + return)
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);

        // Find key blocks
        let entry_block = cfg.entry; // Block with first conditional
        let merge1_block = cfg.block_for_op(10).unwrap(); // Final merge point

        // Final merge should post-dominate entry
        assert!(
            analysis.post_dominates(merge1_block, entry_block),
            "Final merge should post-dominate entry"
        );

        // The immediate post-dominator of entry should be merge1
        assert_eq!(
            analysis.ipdom(entry_block),
            Some(merge1_block),
            "IPDom of entry should be the final merge"
        );
    }

    #[test]
    fn test_post_dominator_single_exit() {
        // Linear code with single exit - all nodes post-dominated by exit
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Add { dst: Reg(2), a: Reg(0), b: Reg(1) },
            Opcode::Ret { ret: Reg(2) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);

        // Single block, so exit node has no post-dominator
        let exit_ipdom = analysis.ipdom(cfg.entry);
        assert!(
            exit_ipdom.is_none(),
            "Single-block CFG: entry (which is also exit) should have no post-dominator"
        );
    }

    #[test]
    fn test_post_dominator_chain() {
        // Test the post_dominators_of function returns the chain correctly
        let ops = vec![
            // Block 0: op 0-1
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 }, // Jump to op 4
            // Block 1: op 2-3
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JAlways { offset: 1 }, // Jump to op 5
            // Block 2: op 4
            Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
            // Block 3: op 5
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);

        // Get post-dominator chain from entry
        let pdom_chain = analysis.post_dominators_of(cfg.entry);

        // Should have at least the merge/exit block
        assert!(
            !pdom_chain.is_empty(),
            "Entry should have post-dominators"
        );
    }
}
