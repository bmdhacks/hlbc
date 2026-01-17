//! Pattern Matchers for Control Flow Structures
//!
//! This module implements pattern detection for common control flow structures:
//! - Loops (while, do-while, for, endless)
//! - Conditionals (if-then, if-then-else)
//! - Switch statements
//!
//! Each matcher identifies a pattern in the RegionGraph and returns the nodes
//! involved plus metadata needed to create a Region. The reducer then uses
//! these patterns to collapse the graph.

use petgraph::graph::NodeIndex;
use std::collections::HashSet;

use crate::analyzer::{CfgAnalysis, NaturalLoop};
use crate::lifter::{Cfg, EdgeKind};
use crate::structurer::region::LoopKind;
use crate::structurer::region_graph::RegionGraph;

/// A detected loop pattern ready for collapse.
#[derive(Debug, Clone)]
pub struct LoopPattern {
    /// The loop header node (entry point).
    pub header: NodeIndex,

    /// All nodes in the loop body (including header).
    pub body_nodes: HashSet<NodeIndex>,

    /// The loop exit node (first node after the loop).
    pub exit: NodeIndex,

    /// The detected loop kind.
    pub kind: LoopKind,

    /// The node containing the loop condition (usually header for while loops).
    pub condition_node: Option<NodeIndex>,

    /// Back-edge source nodes (jump back to header).
    pub back_edge_sources: Vec<NodeIndex>,
}

/// A detected if-then-else pattern ready for collapse.
#[derive(Debug, Clone)]
pub struct IfPattern {
    /// The condition node (contains the conditional branch).
    pub condition_node: NodeIndex,

    /// Nodes in the "then" branch.
    pub then_nodes: HashSet<NodeIndex>,

    /// Nodes in the "else" branch (empty for if-without-else).
    pub else_nodes: HashSet<NodeIndex>,

    /// The merge point (immediate post-dominator of condition).
    pub merge: NodeIndex,

    /// Whether this is a negated condition (jump-on-true vs jump-on-false).
    pub negated: bool,
}

/// A detected switch pattern ready for collapse.
#[derive(Debug, Clone)]
pub struct SwitchPattern {
    /// The switch selector node (contains the Switch opcode).
    pub selector_node: NodeIndex,

    /// Case target nodes (one per case).
    pub case_nodes: Vec<NodeIndex>,

    /// Default case node.
    pub default_node: Option<NodeIndex>,

    /// The merge point after the switch.
    pub merge: NodeIndex,

    /// All nodes in the switch body.
    pub body_nodes: HashSet<NodeIndex>,
}

/// Find loop patterns in the graph that can be collapsed.
///
/// Uses the NaturalLoop information from the analyzer. We look for loops
/// where all body nodes are still present in the RegionGraph (not yet collapsed).
///
/// Returns loops in innermost-first order for proper nesting.
pub fn find_loop_patterns(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
) -> Vec<LoopPattern> {
    let mut patterns = Vec::new();

    // Sort loops by size (smallest/innermost first)
    let mut loops: Vec<_> = analysis.loops.iter().collect();
    loops.sort_by_key(|l| l.body.len());

    for natural_loop in loops {
        if let Some(pattern) = match_loop_pattern(region_graph, cfg, analysis, natural_loop) {
            patterns.push(pattern);
        }
    }

    patterns
}

/// Try to match a single natural loop as a collapsible pattern.
fn match_loop_pattern(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    _analysis: &CfgAnalysis,
    natural_loop: &NaturalLoop,
) -> Option<LoopPattern> {
    // Check that all loop body nodes are still in the region graph
    let mut body_region_nodes = HashSet::new();
    for &cfg_node in &natural_loop.body {
        let region_node = region_graph.get_region_node(cfg_node)?;
        body_region_nodes.insert(region_node);
    }

    // Find the exit node (first node outside the loop that's reachable from inside)
    let exit = find_loop_exit(cfg, natural_loop)?;

    // Determine loop kind
    let kind = detect_loop_kind(cfg, natural_loop);

    // The condition is typically at the header for while loops
    let condition_node = if kind == LoopKind::While || kind == LoopKind::Endless {
        Some(natural_loop.header)
    } else {
        // For do-while, condition is at a back-edge source
        natural_loop.back_edge_sources.first().copied()
    };

    Some(LoopPattern {
        header: natural_loop.header,
        body_nodes: body_region_nodes,
        exit,
        kind,
        condition_node,
        back_edge_sources: natural_loop.back_edge_sources.clone(),
    })
}

/// Find the primary exit node for a loop.
fn find_loop_exit(cfg: &Cfg, natural_loop: &NaturalLoop) -> Option<NodeIndex> {
    // Look for edges leaving the loop from exit nodes
    for &exit_node in &natural_loop.exit_nodes {
        for succ in cfg.successors(exit_node) {
            if !natural_loop.body.contains(&succ) {
                return Some(succ);
            }
        }
    }

    // Also check header for exit edges (common in while loops)
    for succ in cfg.successors(natural_loop.header) {
        if !natural_loop.body.contains(&succ) {
            return Some(succ);
        }
    }

    None
}

/// Detect the kind of loop based on structure.
fn detect_loop_kind(cfg: &Cfg, natural_loop: &NaturalLoop) -> LoopKind {
    let header = natural_loop.header;
    let header_succs = cfg.successors(header);

    // Check if header has an exit edge (while loop pattern)
    let header_exits = header_succs
        .iter()
        .any(|s| !natural_loop.body.contains(s));

    if header_exits {
        // Header checks condition and may exit -> while loop
        // Could also be a for loop, but we detect that later during lowering
        LoopKind::While
    } else if natural_loop.back_edge_sources.len() == 1 {
        // Single back-edge, no header exit -> check if back-edge source exits
        let back_source = natural_loop.back_edge_sources[0];
        let back_source_exits = cfg
            .successors(back_source)
            .iter()
            .any(|s| !natural_loop.body.contains(s));

        if back_source_exits {
            // Back-edge source can exit -> do-while
            LoopKind::DoWhile
        } else {
            // No exit from header or back source -> endless loop
            LoopKind::Endless
        }
    } else {
        // Multiple back edges or complex structure -> default to while
        LoopKind::While
    }
}

/// Find if-then-else patterns in the graph that can be collapsed.
///
/// An if pattern is detected when:
/// 1. A node has exactly 2 successors (conditional branch)
/// 2. The node has an immediate post-dominator (merge point)
/// 3. All nodes between condition and merge are part of the if structure
pub fn find_if_patterns(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
) -> Vec<IfPattern> {
    let mut patterns = Vec::new();

    // Process nodes in reverse post-order to find innermost patterns first
    let nodes = region_graph.nodes_in_reverse_postorder();

    for node in nodes {
        // Skip already-collapsed nodes
        if region_graph.get_node(node).map_or(true, |n| n.is_collapsed()) {
            continue;
        }

        if let Some(pattern) = match_if_pattern(region_graph, cfg, analysis, node) {
            patterns.push(pattern);
        }
    }

    patterns
}

/// Try to match a single node as the start of an if pattern.
fn match_if_pattern(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    node: NodeIndex,
) -> Option<IfPattern> {
    // Get the corresponding CFG node
    let cfg_node = region_graph.get_node(node)?.as_block()?;

    // Must have exactly 2 successors in the CFG
    let cfg_succs = cfg.successors_with_edges(cfg_node);
    if cfg_succs.len() != 2 {
        return None;
    }

    // Find merge point (immediate post-dominator)
    let merge_cfg = analysis.ipdom(cfg_node)?;

    // Don't match if merge is one of the direct successors (trivial case)
    // These are handled by sequence collapsing
    if cfg_succs.iter().all(|(s, _)| *s == merge_cfg) {
        return None;
    }

    // Identify then and else branches
    let (then_target, else_target, negated) = identify_branches(&cfg_succs)?;

    // Collect nodes in each branch
    let then_nodes = collect_branch_nodes(cfg, analysis, then_target, merge_cfg, cfg_node);
    let else_nodes = collect_branch_nodes(cfg, analysis, else_target, merge_cfg, cfg_node);

    // Convert to region graph nodes
    let then_region_nodes: HashSet<_> = then_nodes
        .iter()
        .filter_map(|&n| region_graph.get_region_node(n))
        .collect();

    let else_region_nodes: HashSet<_> = else_nodes
        .iter()
        .filter_map(|&n| region_graph.get_region_node(n))
        .collect();

    // Get merge point in region graph
    let merge = region_graph.get_region_node(merge_cfg)?;

    Some(IfPattern {
        condition_node: node,
        then_nodes: then_region_nodes,
        else_nodes: else_region_nodes,
        merge,
        negated,
    })
}

/// Identify which successor is the then branch and which is else.
/// Returns (then_target, else_target, negated).
fn identify_branches(succs: &[(NodeIndex, EdgeKind)]) -> Option<(NodeIndex, NodeIndex, bool)> {
    let mut true_branch = None;
    let mut false_branch = None;

    for &(target, kind) in succs {
        match kind {
            EdgeKind::ConditionalTrue => true_branch = Some(target),
            EdgeKind::ConditionalFalse => false_branch = Some(target),
            _ => {}
        }
    }

    match (true_branch, false_branch) {
        (Some(t), Some(f)) => {
            // Convention: "then" is the true branch, negated=false
            Some((t, f, false))
        }
        _ => None,
    }
}

/// Collect all nodes in a branch between start and merge.
fn collect_branch_nodes(
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    start: NodeIndex,
    merge: NodeIndex,
    condition: NodeIndex,
) -> HashSet<NodeIndex> {
    let mut nodes = HashSet::new();
    let mut worklist = vec![start];
    let mut visited = HashSet::new();

    while let Some(node) = worklist.pop() {
        if visited.contains(&node) || node == merge || node == condition {
            continue;
        }
        visited.insert(node);

        // Check this node is dominated by condition and post-dominated by merge
        if !analysis.dominates(condition, node) {
            continue;
        }
        if !analysis.post_dominates(merge, node) {
            continue;
        }

        nodes.insert(node);

        for succ in cfg.successors(node) {
            if !visited.contains(&succ) {
                worklist.push(succ);
            }
        }
    }

    nodes
}

/// Find switch patterns in the graph.
///
/// A switch pattern is detected when:
/// 1. A node has 3+ successors (multi-way branch)
/// 2. All branches eventually merge at the post-dominator
pub fn find_switch_patterns(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
) -> Vec<SwitchPattern> {
    let mut patterns = Vec::new();

    for node in region_graph.node_indices() {
        if region_graph.get_node(node).map_or(true, |n| n.is_collapsed()) {
            continue;
        }

        if let Some(pattern) = match_switch_pattern(region_graph, cfg, analysis, node) {
            patterns.push(pattern);
        }
    }

    patterns
}

/// Try to match a node as the start of a switch pattern.
fn match_switch_pattern(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    node: NodeIndex,
) -> Option<SwitchPattern> {
    let cfg_node = region_graph.get_node(node)?.as_block()?;

    // Must have 3+ successors for a switch
    let cfg_succs = cfg.successors(cfg_node);
    if cfg_succs.len() < 3 {
        return None;
    }

    // Find merge point
    let merge_cfg = analysis.ipdom(cfg_node)?;

    // Collect case nodes and body
    let mut case_nodes = Vec::new();
    let mut body_nodes = HashSet::new();
    let mut default_node = None;

    // Check edge types to identify default vs cases
    for (target, kind) in cfg.successors_with_edges(cfg_node) {
        match kind {
            EdgeKind::FallThrough => {
                default_node = Some(target);
            }
            EdgeKind::ConditionalTrue => {
                case_nodes.push(target);
            }
            _ => {
                case_nodes.push(target);
            }
        }

        // Collect all nodes in this case branch
        let branch_nodes = collect_branch_nodes(cfg, analysis, target, merge_cfg, cfg_node);
        body_nodes.extend(branch_nodes);
    }

    // Convert to region graph indices
    let case_region_nodes: Vec<_> = case_nodes
        .iter()
        .filter_map(|&n| region_graph.get_region_node(n))
        .collect();

    let body_region_nodes: HashSet<_> = body_nodes
        .iter()
        .filter_map(|&n| region_graph.get_region_node(n))
        .collect();

    let merge = region_graph.get_region_node(merge_cfg)?;

    Some(SwitchPattern {
        selector_node: node,
        case_nodes: case_region_nodes,
        default_node: default_node.and_then(|n| region_graph.get_region_node(n)),
        merge,
        body_nodes: body_region_nodes,
    })
}

/// Find the innermost pattern that can be collapsed.
///
/// Priority order:
/// 1. Innermost loops (smallest body)
/// 2. If patterns where branches don't contain other patterns
/// 3. Sequences (handled separately by RegionGraph)
pub fn find_innermost_pattern(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
) -> Option<Pattern> {
    // First try innermost loops
    let loop_patterns = find_loop_patterns(region_graph, cfg, analysis);
    if let Some(lp) = loop_patterns.into_iter().next() {
        return Some(Pattern::Loop(lp));
    }

    // Then try if patterns
    let if_patterns = find_if_patterns(region_graph, cfg, analysis);
    if let Some(ip) = if_patterns.into_iter().next() {
        return Some(Pattern::If(ip));
    }

    // Then try switch patterns
    let switch_patterns = find_switch_patterns(region_graph, cfg, analysis);
    if let Some(sp) = switch_patterns.into_iter().next() {
        return Some(Pattern::Switch(sp));
    }

    None
}

/// A detected pattern ready for collapse.
#[derive(Debug, Clone)]
pub enum Pattern {
    Loop(LoopPattern),
    If(IfPattern),
    Switch(SwitchPattern),
}

impl Pattern {
    /// Get all nodes involved in this pattern.
    pub fn nodes(&self) -> HashSet<NodeIndex> {
        match self {
            Pattern::Loop(lp) => lp.body_nodes.clone(),
            Pattern::If(ip) => {
                let mut nodes = HashSet::new();
                nodes.insert(ip.condition_node);
                nodes.extend(ip.then_nodes.iter());
                nodes.extend(ip.else_nodes.iter());
                nodes
            }
            Pattern::Switch(sp) => {
                let mut nodes = HashSet::new();
                nodes.insert(sp.selector_node);
                nodes.extend(sp.body_nodes.iter());
                nodes
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hlbc::opcodes::Opcode;
    use hlbc::types::{RefInt, Reg};

    fn build_test_env(ops: &[Opcode]) -> (Cfg, CfgAnalysis, RegionGraph) {
        let cfg = Cfg::from_ops(ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        let region_graph = RegionGraph::from_cfg(&cfg);
        (cfg, analysis, region_graph)
    }

    #[test]
    fn test_detect_simple_if() {
        // if (cond) { then } else { else }
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 },
            // then branch
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JAlways { offset: 1 },
            // else branch
            Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
            // merge
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis, region_graph) = build_test_env(&ops);
        let patterns = find_if_patterns(&region_graph, &cfg, &analysis);

        println!("Found {} if patterns", patterns.len());
        for p in &patterns {
            println!(
                "  cond={:?}, then={:?}, else={:?}, merge={:?}",
                p.condition_node, p.then_nodes, p.else_nodes, p.merge
            );
        }
    }

    #[test]
    fn test_detect_simple_loop() {
        // while (cond) { body }
        let ops = vec![
            Opcode::Label,
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 },
            // body
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JAlways { offset: -4 },
            // exit
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis, region_graph) = build_test_env(&ops);
        let patterns = find_loop_patterns(&region_graph, &cfg, &analysis);

        println!("Found {} loop patterns", patterns.len());
        for p in &patterns {
            println!(
                "  header={:?}, body={:?}, exit={:?}, kind={:?}",
                p.header, p.body_nodes, p.exit, p.kind
            );
        }

        assert!(!patterns.is_empty(), "Should detect the while loop");
    }

    #[test]
    fn test_detect_endless_loop() {
        // while (true) { body }
        let ops = vec![
            Opcode::Label,
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JAlways { offset: -2 },
        ];

        let (cfg, analysis, region_graph) = build_test_env(&ops);
        let patterns = find_loop_patterns(&region_graph, &cfg, &analysis);

        println!("Found {} loop patterns for endless loop", patterns.len());
        if let Some(p) = patterns.first() {
            println!("  kind={:?}", p.kind);
            // Should detect as endless since there's no exit condition
        }
    }

    #[test]
    fn test_find_innermost_pattern() {
        // Simple if statement
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 1 },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis, region_graph) = build_test_env(&ops);
        let pattern = find_innermost_pattern(&region_graph, &cfg, &analysis);

        println!("Innermost pattern: {:?}", pattern.is_some());
    }

    #[test]
    fn test_loop_kind_detection() {
        // Test that we correctly identify loop kinds based on structure
        let natural_loop = NaturalLoop {
            header: NodeIndex::new(0),
            body: [NodeIndex::new(0), NodeIndex::new(1)].into_iter().collect(),
            back_edge_sources: vec![NodeIndex::new(1)],
            exit_nodes: vec![NodeIndex::new(0)],
            depth: 0,
        };

        // Create a simple CFG for testing
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JAlways { offset: -3 },
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let kind = detect_loop_kind(&cfg, &natural_loop);

        println!("Detected loop kind: {:?}", kind);
    }
}
