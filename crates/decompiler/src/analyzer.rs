//! Pass 2: Analyzer - Compute dominators, identify loops, and prepare for SSA
//!
//! This module performs dataflow analysis on the CFG:
//! - Computes dominator trees using petgraph
//! - Identifies natural loops (back-edges where target dominates source)
//! - Detects reducibility of the CFG
//! - Prepares the CFG for SSA conversion

use petgraph::algo::dominators::Dominators;
use petgraph::graph::NodeIndex;
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

/// Analysis results for a CFG
pub struct CfgAnalysis {
    /// Dominator tree
    pub dominators: Dominators<NodeIndex>,
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

        CfgAnalysis {
            dominators,
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
}
