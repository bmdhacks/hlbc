//! SESE (Single-Entry-Single-Exit) Region Identification
//!
//! This module identifies SESE regions in a control flow graph using dominator
//! and post-dominator trees. A SESE region is a subgraph with exactly one entry
//! edge and one exit edge - the fundamental building block for structured code.
//!
//! The algorithm follows the approach from:
//! - Johnson et al. "The Program Structure Tree" (1994)
//! - SAILR (USENIX 2024): Uses SESE regions for compiler-aware structuring
//!
//! Key insight: An edge (A→B) defines a SESE region if and only if:
//! - A dominates B (all paths to B go through A)
//! - B post-dominates A (all paths from A lead to B)

use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet};

use crate::analyzer::CfgAnalysis;
use crate::lifter::Cfg;

/// A Single-Entry-Single-Exit region in the CFG.
///
/// SESE regions form a hierarchy: larger regions contain smaller ones.
/// This property enables iterative "collapse" of the CFG into a region tree.
#[derive(Debug, Clone)]
pub struct SeseRegion {
    /// The entry node of this region (where control flow enters).
    pub entry: NodeIndex,

    /// The exit node of this region (where control flow leaves).
    /// This is the immediate post-dominator of the entry.
    pub exit: NodeIndex,

    /// All nodes contained within this region (including entry, excluding exit).
    /// The exit node is NOT part of the region - it's where we exit TO.
    pub nodes: HashSet<NodeIndex>,

    /// The depth in the region hierarchy (0 = outermost/function level).
    pub depth: usize,
}

impl SeseRegion {
    /// Check if this region contains a given node.
    pub fn contains(&self, node: NodeIndex) -> bool {
        self.nodes.contains(&node)
    }

    /// Check if this region properly contains another region.
    /// A region A properly contains B if all of B's nodes are in A,
    /// and A has at least one node not in B.
    pub fn properly_contains(&self, other: &SeseRegion) -> bool {
        other.nodes.is_subset(&self.nodes) && self.nodes.len() > other.nodes.len()
    }

    /// Number of nodes in this region.
    pub fn size(&self) -> usize {
        self.nodes.len()
    }
}

/// Hierarchical collection of SESE regions for a CFG.
///
/// Provides efficient queries for finding regions and their relationships.
pub struct SeseTree {
    /// All identified SESE regions, sorted by size (smallest first).
    /// Smaller regions are nested inside larger ones.
    regions: Vec<SeseRegion>,

    /// Map from entry node to the innermost region starting at that node.
    entry_to_region: HashMap<NodeIndex, usize>,

    /// Map from node to the innermost region containing it.
    node_to_innermost: HashMap<NodeIndex, usize>,

    /// Parent relationship: region index -> parent region index.
    /// None means the region is at the top level (function scope).
    parent: HashMap<usize, usize>,

    /// Children relationship: region index -> child region indices.
    children: HashMap<usize, Vec<usize>>,
}

impl SeseTree {
    /// Build the SESE region tree from a CFG and its analysis.
    pub fn build(cfg: &Cfg, analysis: &CfgAnalysis) -> Self {
        let regions = find_sese_regions(cfg, analysis);
        let mut tree = SeseTree {
            regions,
            entry_to_region: HashMap::new(),
            node_to_innermost: HashMap::new(),
            parent: HashMap::new(),
            children: HashMap::new(),
        };
        tree.build_hierarchy();
        tree
    }

    /// Build the containment hierarchy after regions are identified.
    fn build_hierarchy(&mut self) {
        // Sort regions by size (smallest first) for proper nesting detection
        self.regions.sort_by_key(|r| r.nodes.len());

        // Assign depths and build entry map
        for (i, region) in self.regions.iter().enumerate() {
            self.entry_to_region.insert(region.entry, i);
        }

        // Build parent-child relationships
        // For each region, find the smallest region that properly contains it
        for i in 0..self.regions.len() {
            for j in (i + 1)..self.regions.len() {
                if self.regions[j].properly_contains(&self.regions[i]) {
                    self.parent.insert(i, j);
                    self.children.entry(j).or_default().push(i);
                    break; // Found immediate parent
                }
            }
        }

        // Build node-to-innermost-region map
        // Process smallest regions first so they override larger ones
        for (i, region) in self.regions.iter().enumerate() {
            for &node in &region.nodes {
                self.node_to_innermost.insert(node, i);
            }
        }

        // Update depths based on parent chain
        for i in 0..self.regions.len() {
            let depth = self.compute_depth(i);
            self.regions[i].depth = depth;
        }
    }

    /// Compute the depth of a region (distance to root).
    fn compute_depth(&self, region_idx: usize) -> usize {
        let mut depth = 0;
        let mut current = region_idx;
        while let Some(&parent) = self.parent.get(&current) {
            depth += 1;
            current = parent;
        }
        depth
    }

    /// Get all regions, sorted by size (smallest first).
    pub fn regions(&self) -> &[SeseRegion] {
        &self.regions
    }

    /// Get the region starting at a given entry node (if any).
    pub fn region_at(&self, entry: NodeIndex) -> Option<&SeseRegion> {
        self.entry_to_region.get(&entry).map(|&i| &self.regions[i])
    }

    /// Get the innermost region containing a given node.
    pub fn innermost_region(&self, node: NodeIndex) -> Option<&SeseRegion> {
        self.node_to_innermost.get(&node).map(|&i| &self.regions[i])
    }

    /// Get the parent region of a given region (if any).
    pub fn parent_region(&self, region: &SeseRegion) -> Option<&SeseRegion> {
        // Find this region's index
        let idx = self.regions.iter().position(|r| r.entry == region.entry)?;
        self.parent.get(&idx).map(|&i| &self.regions[i])
    }

    /// Get child regions of a given region.
    pub fn child_regions(&self, region: &SeseRegion) -> Vec<&SeseRegion> {
        let idx = match self.regions.iter().position(|r| r.entry == region.entry) {
            Some(i) => i,
            None => return Vec::new(),
        };
        self.children
            .get(&idx)
            .map(|children| children.iter().map(|&i| &self.regions[i]).collect())
            .unwrap_or_default()
    }

    /// Get all top-level regions (those with no parent).
    pub fn top_level_regions(&self) -> Vec<&SeseRegion> {
        self.regions
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.parent.contains_key(i))
            .map(|(_, r)| r)
            .collect()
    }

    /// Check if the CFG has any SESE regions.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// Number of identified SESE regions.
    pub fn len(&self) -> usize {
        self.regions.len()
    }
}

/// Find all SESE regions in a CFG.
///
/// Algorithm:
/// 1. For each edge (A→B) in the CFG
/// 2. Check if A dominates B and B post-dominates A
/// 3. If so, collect all nodes in the region (reachable from A, post-dominated by B)
/// 4. Filter out trivial single-edge regions
fn find_sese_regions(cfg: &Cfg, analysis: &CfgAnalysis) -> Vec<SeseRegion> {
    let mut regions = Vec::new();
    let mut seen_pairs: HashSet<(NodeIndex, NodeIndex)> = HashSet::new();

    // Iterate over all edges in the CFG
    for node in cfg.graph.node_indices() {
        for edge in cfg.graph.edges(node) {
            let a = edge.source();
            let b = edge.target();

            // Skip if we've already processed this (entry, exit) pair
            if seen_pairs.contains(&(a, b)) {
                continue;
            }

            // Check SESE condition: A dominates B AND B post-dominates A
            // But we actually want: entry dominates all nodes, exit post-dominates all nodes
            // The canonical SESE check is based on finding regions where ipdom(entry) = exit

            // For each node, check if (node, ipdom(node)) forms a valid SESE region
            if let Some(exit) = analysis.ipdom(a) {
                if seen_pairs.contains(&(a, exit)) {
                    continue;
                }
                seen_pairs.insert((a, exit));

                // Collect all nodes in the potential region:
                // - Reachable from entry (a)
                // - Post-dominated by exit
                // - Not the exit node itself
                let nodes = collect_region_nodes(cfg, analysis, a, exit);

                // Skip trivial regions (empty or single node that equals entry)
                if nodes.is_empty() {
                    continue;
                }

                // Skip if region only contains the entry and nothing else interesting
                if nodes.len() == 1 && nodes.contains(&a) {
                    // Check if this is just a simple fall-through
                    let succs = cfg.successors(a);
                    if succs.len() == 1 && succs[0] == exit {
                        continue; // Trivial single-block region
                    }
                }

                regions.push(SeseRegion {
                    entry: a,
                    exit,
                    nodes,
                    depth: 0, // Will be set during hierarchy building
                });
            }
        }
    }

    // Also check the entry node specifically
    if let Some(exit) = analysis.ipdom(cfg.entry) {
        if !seen_pairs.contains(&(cfg.entry, exit)) {
            let nodes = collect_region_nodes(cfg, analysis, cfg.entry, exit);
            if !nodes.is_empty() {
                regions.push(SeseRegion {
                    entry: cfg.entry,
                    exit,
                    nodes,
                    depth: 0,
                });
            }
        }
    }

    regions
}

/// Collect all nodes belonging to a SESE region.
///
/// A node N is in the region (entry, exit) if:
/// - N is reachable from entry without going through exit
/// - N is post-dominated by exit (all paths from N go through exit)
fn collect_region_nodes(
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    entry: NodeIndex,
    exit: NodeIndex,
) -> HashSet<NodeIndex> {
    let mut nodes = HashSet::new();
    let mut worklist = vec![entry];
    let mut visited = HashSet::new();

    while let Some(node) = worklist.pop() {
        if visited.contains(&node) {
            continue;
        }
        visited.insert(node);

        // Don't include the exit node in the region
        if node == exit {
            continue;
        }

        // Check if this node is post-dominated by exit
        if !analysis.post_dominates(exit, node) {
            continue;
        }

        // This node is part of the region
        nodes.insert(node);

        // Add successors to worklist
        for succ in cfg.successors(node) {
            if !visited.contains(&succ) {
                worklist.push(succ);
            }
        }
    }

    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifter::Cfg;
    use hlbc::opcodes::Opcode;
    use hlbc::types::{RefInt, Reg};

    /// Helper to build a CFG and analysis from opcodes
    fn build_test_cfg(ops: &[Opcode]) -> (Cfg, CfgAnalysis) {
        let cfg = Cfg::from_ops(ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        (cfg, analysis)
    }

    #[test]
    fn test_simple_if_else_sese() {
        // if (cond) { A } else { B }; C
        // Block 0: cond, JNull -> 3
        // Block 1: A (then)
        // Block 2: JAlways -> 4
        // Block 3: B (else)
        // Block 4: C (merge)
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 }, // Jump to block 3 (else)
            // Block 1: then
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JAlways { offset: 1 }, // Jump to merge
            // Block 2: else
            Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
            // Block 3: merge + return
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis) = build_test_cfg(&ops);
        let tree = SeseTree::build(&cfg, &analysis);

        // Should find at least one SESE region (the if-else construct)
        assert!(!tree.is_empty(), "Should find SESE regions in if-else");

        // The entry node should have a region
        let entry_region = tree.region_at(cfg.entry);
        assert!(entry_region.is_some(), "Entry should start a SESE region");
    }

    #[test]
    fn test_simple_loop_sese() {
        // while (cond) { body }
        // Block 0: header (cond check)
        // Block 1: body
        // Block 2: back-edge to header
        // Block 3: exit
        let ops = vec![
            // Block 0: header
            Opcode::Label,
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 3 }, // Exit loop
            // Block 1: body
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JAlways { offset: -4 }, // Back to header
            // Block 2: exit
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis) = build_test_cfg(&ops);
        let tree = SeseTree::build(&cfg, &analysis);

        // Loops are trickier for SESE - the back edge creates complexity
        // But the overall loop structure should still be identifiable
        println!("Found {} SESE regions in loop", tree.len());
        for region in tree.regions() {
            println!(
                "  Region: entry={:?}, exit={:?}, nodes={:?}",
                region.entry, region.exit, region.nodes
            );
        }
    }

    #[test]
    fn test_nested_if_sese() {
        // if (a) { if (b) { X } }
        let ops = vec![
            // Block 0: outer if
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 4 }, // Skip to merge
            // Block 1: inner if
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JNull { reg: Reg(1), offset: 1 }, // Skip inner body
            // Block 2: inner body
            Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
            // Block 3: merge
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis) = build_test_cfg(&ops);
        let tree = SeseTree::build(&cfg, &analysis);

        // Should find nested regions
        println!("Found {} SESE regions in nested if", tree.len());
        for region in tree.regions() {
            println!(
                "  Region: entry={:?}, exit={:?}, size={}, depth={}",
                region.entry,
                region.exit,
                region.size(),
                region.depth
            );
        }

        // Check hierarchy
        let top_level = tree.top_level_regions();
        println!("Top-level regions: {}", top_level.len());
    }

    #[test]
    fn test_linear_code_minimal_regions() {
        // Linear code should have minimal SESE regions
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Add { dst: Reg(2), a: Reg(0), b: Reg(1) },
            Opcode::Ret { ret: Reg(2) },
        ];

        let (cfg, analysis) = build_test_cfg(&ops);
        let tree = SeseTree::build(&cfg, &analysis);

        // Linear code has one block, so at most one trivial region
        println!("Linear code regions: {}", tree.len());
    }

    #[test]
    fn test_region_containment() {
        // Test that properly_contains works correctly
        let mut inner_nodes = HashSet::new();
        inner_nodes.insert(NodeIndex::new(1));
        inner_nodes.insert(NodeIndex::new(2));

        let mut outer_nodes = HashSet::new();
        outer_nodes.insert(NodeIndex::new(0));
        outer_nodes.insert(NodeIndex::new(1));
        outer_nodes.insert(NodeIndex::new(2));
        outer_nodes.insert(NodeIndex::new(3));

        let inner = SeseRegion {
            entry: NodeIndex::new(1),
            exit: NodeIndex::new(3),
            nodes: inner_nodes,
            depth: 1,
        };

        let outer = SeseRegion {
            entry: NodeIndex::new(0),
            exit: NodeIndex::new(4),
            nodes: outer_nodes,
            depth: 0,
        };

        assert!(outer.properly_contains(&inner));
        assert!(!inner.properly_contains(&outer));
        assert!(!inner.properly_contains(&inner)); // Not proper
    }
}
