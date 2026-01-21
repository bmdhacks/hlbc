//! Iterative Reducer - Core Region-Collapsing Algorithm
//!
//! This module implements the main reduction loop that iteratively identifies
//! and collapses control flow patterns until the graph becomes a single Region.
//!
//! The algorithm follows the approach from REFACTOR.md:
//! 1. Try to find and collapse innermost loops (smallest body first)
//! 2. Try to find and collapse if-then-else patterns
//! 3. Try to collapse linear sequences
//! 4. If stuck, virtualize an edge (insert goto) to make progress
//! 5. Repeat until the graph is a single node
//!
//! This iterative approach avoids stack overflows from deep recursion and
//! handles complex control flow patterns more robustly than recursive descent.

use petgraph::graph::NodeIndex;
use std::collections::HashSet;

use crate::analyzer::CfgAnalysis;
use crate::ast::Expr;
use crate::lifter::Cfg;
use crate::structurer::patterns::{
    find_if_patterns, find_loop_patterns, find_switch_patterns, IfPattern, LoopPattern,
    PatternContext, SwitchPattern,
};
use crate::structurer::region::{Region, SwitchCase};
use hlbc::types::RefInt;
use crate::structurer::region_graph::{RegionGraph, RegionNode};

/// Maximum iterations before we give up and emit gotos.
/// This prevents infinite loops in pathological cases.
const MAX_ITERATIONS: usize = 1000;

/// Reduce a CFG to a single Region using iterative graph reduction.
///
/// This is the main entry point for the new structurer architecture.
/// It creates a RegionGraph, iteratively collapses patterns, and returns
/// the final Region tree.
///
/// If `ctx` is provided, enables detection of higher-level patterns like for-in loops.
pub fn reduce_to_region(
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    ctx: Option<&PatternContext<'_>>,
) -> Region {
    let mut graph = RegionGraph::from_cfg(cfg);
    let mut iterations = 0;

    while !graph.is_fully_reduced() && iterations < MAX_ITERATIONS {
        iterations += 1;
        let made_progress = reduce_one_step(&mut graph, cfg, analysis, ctx);

        if !made_progress {
            // No patterns found - try to make the graph reducible
            if graph.node_count() > 1 {
                virtualize_edge(&mut graph);
            } else {
                break;
            }
        }
    }

    if iterations >= MAX_ITERATIONS {
        // Safety fallback - wrap remaining nodes in a sequence with gotos
        // Note: Would log a warning here if logging was available
        return create_fallback_region(&graph);
    }

    graph.into_region().unwrap_or(Region::Empty)
}

/// Perform one reduction step on the graph.
///
/// Tries patterns in priority order:
/// 1. Innermost loops (smallest body first)
/// 2. If-then-else patterns
/// 3. Switch patterns
/// 4. Linear sequences
///
/// Returns true if any reduction was made.
fn reduce_one_step(
    graph: &mut RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    ctx: Option<&PatternContext<'_>>,
) -> bool {
    // Priority 1: Collapse innermost loops first
    // This ensures nested loops are reduced from inside out
    let loop_patterns = find_loop_patterns(graph, cfg, analysis, ctx);
    if let Some(lp) = loop_patterns.into_iter().next() {
        collapse_loop(graph, cfg, &lp);
        return true;
    }

    // Priority 2: Collapse if-then-else patterns
    let if_patterns = find_if_patterns(graph, cfg, analysis);
    if let Some(ip) = if_patterns.into_iter().next() {
        collapse_if(graph, cfg, &ip);
        return true;
    }

    // Priority 3: Collapse switch patterns
    let switch_patterns = find_switch_patterns(graph, cfg, analysis);
    if let Some(sp) = switch_patterns.into_iter().next() {
        collapse_switch(graph, cfg, &sp);
        return true;
    }

    // Priority 4: Collapse linear sequences
    if collapse_sequences(graph) {
        return true;
    }

    false
}

/// Collapse a loop pattern into a Region::Loop node.
fn collapse_loop(graph: &mut RegionGraph, _cfg: &Cfg, pattern: &LoopPattern) {
    // Build the loop body region from the body nodes
    let body_region = build_region_from_nodes(graph, &pattern.body_nodes, pattern.header);

    // Create the loop region
    // For now, use a placeholder condition - this will be filled in during lowering
    let condition = pattern
        .condition_node
        .map(|_| Expr::Constant(crate::ast::Constant::Bool(true)));

    let loop_region = Region::Loop {
        kind: pattern.kind.clone(),
        header: pattern.header,
        condition,
        body: Box::new(body_region),
        exit: pattern.exit,
    };

    // Collapse all body nodes into the loop region
    let mut nodes_to_collapse = pattern.body_nodes.clone();
    // Don't include the exit node - it's where we exit TO, not part of the loop
    nodes_to_collapse.remove(&graph.get_region_node(pattern.exit).unwrap_or(NodeIndex::new(0)));

    if !nodes_to_collapse.is_empty() {
        graph.collapse(&nodes_to_collapse, loop_region);
    }
}

/// Collapse an if-then-else pattern into a Region::IfThenElse node.
fn collapse_if(graph: &mut RegionGraph, _cfg: &Cfg, pattern: &IfPattern) {
    // Build then branch region
    let then_region = if pattern.then_nodes.is_empty() {
        Region::Empty
    } else {
        build_region_from_nodes(graph, &pattern.then_nodes, NodeIndex::new(0))
    };

    // Build else branch region (if present)
    let else_region = if pattern.else_nodes.is_empty() {
        None
    } else {
        Some(build_region_from_nodes(
            graph,
            &pattern.else_nodes,
            NodeIndex::new(0),
        ))
    };

    // Create the if-then-else region
    // Use a placeholder condition - this will be extracted during lowering
    let cond = Expr::Constant(crate::ast::Constant::Bool(true));

    let if_region = Region::IfThenElse {
        cond,
        then_region: Box::new(then_region),
        else_region: else_region.map(Box::new),
        merge: pattern.merge,
    };

    // Collect all nodes to collapse (condition + branches)
    let mut nodes_to_collapse = HashSet::new();
    nodes_to_collapse.insert(pattern.condition_node);
    nodes_to_collapse.extend(pattern.then_nodes.iter().copied());
    nodes_to_collapse.extend(pattern.else_nodes.iter().copied());
    // Don't include merge - it's where control reconverges

    if !nodes_to_collapse.is_empty() {
        graph.collapse(&nodes_to_collapse, if_region);
    }
}

/// Collapse a switch pattern into a Region::Switch node.
fn collapse_switch(graph: &mut RegionGraph, _cfg: &Cfg, pattern: &SwitchPattern) {
    // Build cases from case nodes
    let mut cases = Vec::new();
    for &case_node in &pattern.case_nodes {
        // Each case becomes a SwitchCase with empty patterns for now
        // The actual patterns will be determined during lowering
        let body = if let Some(node) = graph.get_node(case_node) {
            match node {
                RegionNode::Block(cfg_idx) => Region::Block(*cfg_idx),
                RegionNode::Collapsed(r) => r.clone(),
            }
        } else {
            Region::Empty
        };

        cases.push(SwitchCase {
            patterns: Vec::new(), // Will be filled during lowering
            body,
        });
    }

    // Build default case
    let default = if let Some(default_node) = pattern.default_node {
        if let Some(node) = graph.get_node(default_node) {
            match node {
                RegionNode::Block(cfg_idx) => Region::Block(*cfg_idx),
                RegionNode::Collapsed(r) => r.clone(),
            }
        } else {
            Region::Empty
        }
    } else {
        Region::Empty
    };

    // Create the switch region
    let switch_region = Region::Switch {
        selector: Expr::Constant(crate::ast::Constant::Int(RefInt(0))), // Placeholder
        cases,
        default: Box::new(default),
        merge: pattern.merge,
    };

    // Collect all nodes to collapse
    let mut nodes_to_collapse = HashSet::new();
    nodes_to_collapse.insert(pattern.selector_node);
    nodes_to_collapse.extend(pattern.body_nodes.iter().copied());

    if !nodes_to_collapse.is_empty() {
        graph.collapse(&nodes_to_collapse, switch_region);
    }
}

/// Collapse any linear sequences in the graph.
///
/// A sequence is a chain of nodes where each has exactly one successor
/// to the next node in the chain.
///
/// Returns true if any sequence was collapsed.
fn collapse_sequences(graph: &mut RegionGraph) -> bool {
    let sequences = graph.find_sequences();

    // Collapse the first sequence found
    if let Some(sequence) = sequences.into_iter().next() {
        if sequence.len() >= 2 {
            graph.collapse_sequence(sequence);
            return true;
        }
    }

    false
}

/// Virtualize an edge to make the graph reducible.
///
/// This is the fallback when no pattern can be matched. We insert a Goto
/// region to break the problematic edge, making the graph more reducible.
///
/// Strategy: Find an edge that creates irreducibility and virtualize it.
fn virtualize_edge(graph: &mut RegionGraph) {
    // Find a node with multiple predecessors that isn't reducible
    // This is likely the target of a "back jump" or cross-edge

    // Collect node indices first to avoid borrow checker issues
    let nodes: Vec<_> = graph.node_indices().collect();

    for node in nodes {
        let preds = graph.predecessors(node);
        if preds.len() > 1 {
            // This node has multiple entries - virtualize one of them
            // Pick the first predecessor that isn't the "main" entry
            if let Some(&pred) = preds.get(1) {
                // Get the target CFG node if this is a Block
                let target = if let Some(RegionNode::Block(cfg_idx)) = graph.get_node(node) {
                    *cfg_idx
                } else {
                    // For collapsed regions, use a placeholder
                    NodeIndex::new(0)
                };

                // Create a Goto region for the predecessor
                let goto_region = Region::Goto { target };

                // Collapse just the predecessor into a goto
                let mut collapse_set = HashSet::new();
                collapse_set.insert(pred);
                graph.collapse(&collapse_set, goto_region);
                return;
            }
        }
    }

    // If we couldn't find a good edge to virtualize, just collapse any pair
    // This ensures we always make progress
    let nodes: Vec<_> = graph.node_indices().take(2).collect();
    if nodes.len() == 2 {
        let region1 = graph
            .get_node(nodes[0])
            .map(|n| match n {
                RegionNode::Block(idx) => Region::Block(*idx),
                RegionNode::Collapsed(r) => r.clone(),
            })
            .unwrap_or(Region::Empty);

        let region2 = graph
            .get_node(nodes[1])
            .map(|n| match n {
                RegionNode::Block(idx) => Region::Block(*idx),
                RegionNode::Collapsed(r) => r.clone(),
            })
            .unwrap_or(Region::Empty);

        let sequence = Region::sequence(vec![region1, region2]);
        let collapse_set: HashSet<_> = nodes.into_iter().collect();
        graph.collapse(&collapse_set, sequence);
    }
}

/// Build a Region from a set of nodes in the graph.
///
/// This creates a Sequence of the nodes if there are multiple,
/// or returns the single node's region if there's only one.
fn build_region_from_nodes(
    graph: &RegionGraph,
    nodes: &HashSet<NodeIndex>,
    _entry_hint: NodeIndex,
) -> Region {
    if nodes.is_empty() {
        return Region::Empty;
    }

    if nodes.len() == 1 {
        let node = *nodes.iter().next().unwrap();
        return match graph.get_node(node) {
            Some(RegionNode::Block(cfg_idx)) => Region::Block(*cfg_idx),
            Some(RegionNode::Collapsed(r)) => r.clone(),
            None => Region::Empty,
        };
    }

    // Multiple nodes - create a sequence
    // Try to order them by following edges
    let ordered = order_nodes_by_flow(graph, nodes);

    let regions: Vec<Region> = ordered
        .into_iter()
        .filter_map(|node| {
            graph.get_node(node).map(|n| match n {
                RegionNode::Block(cfg_idx) => Region::Block(*cfg_idx),
                RegionNode::Collapsed(r) => r.clone(),
            })
        })
        .collect();

    Region::sequence(regions)
}

/// Order nodes by following the control flow edges.
///
/// Returns nodes in execution order where possible.
fn order_nodes_by_flow(graph: &RegionGraph, nodes: &HashSet<NodeIndex>) -> Vec<NodeIndex> {
    if nodes.is_empty() {
        return Vec::new();
    }

    // Find entry node (node with no predecessors in the set)
    let mut entry = None;
    for &node in nodes {
        let preds_in_set = graph
            .predecessors(node)
            .into_iter()
            .filter(|p| nodes.contains(p))
            .count();
        if preds_in_set == 0 {
            entry = Some(node);
            break;
        }
    }

    // If no clear entry, just pick the first node
    let start = entry.unwrap_or_else(|| *nodes.iter().next().unwrap());

    // DFS to collect nodes in order
    let mut ordered = Vec::new();
    let mut visited = HashSet::new();
    let mut stack = vec![start];

    while let Some(node) = stack.pop() {
        if !nodes.contains(&node) || visited.contains(&node) {
            continue;
        }
        visited.insert(node);
        ordered.push(node);

        // Add successors that are in our node set
        for succ in graph.successors(node) {
            if nodes.contains(&succ) && !visited.contains(&succ) {
                stack.push(succ);
            }
        }
    }

    // Add any remaining nodes (for disconnected subgraphs)
    for &node in nodes {
        if !visited.contains(&node) {
            ordered.push(node);
        }
    }

    ordered
}

/// Create a fallback region when reduction fails.
///
/// This wraps all remaining nodes in a sequence with gotos,
/// ensuring we always produce valid output even for irreducible graphs.
fn create_fallback_region(graph: &RegionGraph) -> Region {
    let mut regions = Vec::new();

    for node in graph.node_indices() {
        match graph.get_node(node) {
            Some(RegionNode::Block(cfg_idx)) => {
                regions.push(Region::Block(*cfg_idx));
            }
            Some(RegionNode::Collapsed(r)) => {
                regions.push(r.clone());
            }
            None => {}
        }
    }

    Region::sequence(regions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hlbc::opcodes::Opcode;
    use hlbc::types::{RefInt, Reg};

    fn build_test_env(ops: &[Opcode]) -> (Cfg, CfgAnalysis) {
        let cfg = Cfg::from_ops(ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        (cfg, analysis)
    }

    #[test]
    fn test_reduce_linear_code() {
        // Simple linear code should reduce to a single block
        let ops = vec![
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::Add {
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Opcode::Ret { ret: Reg(2) },
        ];

        let (cfg, analysis) = build_test_env(&ops);
        let region = reduce_to_region(&cfg, &analysis, None);

        // Should be a single block (linear code has 1 basic block)
        assert!(matches!(region, Region::Block(_)));
    }

    #[test]
    fn test_reduce_simple_if() {
        // if (cond) { then } else { else }
        let ops = vec![
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JNull {
                reg: Reg(0),
                offset: 2,
            },
            // then branch
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::JAlways { offset: 1 },
            // else branch
            Opcode::Int {
                dst: Reg(2),
                ptr: RefInt(2),
            },
            // merge
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis) = build_test_env(&ops);
        let region = reduce_to_region(&cfg, &analysis, None);

        // Should reduce to something (not crash)
        println!("Reduced if-else to: {:?}", region);
        assert!(!matches!(region, Region::Empty));
    }

    #[test]
    fn test_reduce_simple_loop() {
        // while (cond) { body }
        let ops = vec![
            Opcode::Label,
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JNull {
                reg: Reg(0),
                offset: 2,
            },
            // body
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::JAlways { offset: -4 },
            // exit
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis) = build_test_env(&ops);
        let region = reduce_to_region(&cfg, &analysis, None);

        // Should reduce to something
        println!("Reduced loop to: {:?}", region);
        assert!(!matches!(region, Region::Empty));
    }

    #[test]
    fn test_reduce_nested_if() {
        // if (a) { if (b) { X } }
        let ops = vec![
            // outer if
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JNull {
                reg: Reg(0),
                offset: 4,
            },
            // inner if
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::JNull {
                reg: Reg(1),
                offset: 1,
            },
            // inner body
            Opcode::Int {
                dst: Reg(2),
                ptr: RefInt(2),
            },
            // merge
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis) = build_test_env(&ops);
        let region = reduce_to_region(&cfg, &analysis, None);

        println!("Reduced nested if to: {:?}", region);
        assert!(!matches!(region, Region::Empty));
    }

    #[test]
    fn test_reduce_makes_progress() {
        // Verify that reduce_one_step always makes progress or returns false
        let ops = vec![
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JNull {
                reg: Reg(0),
                offset: 1,
            },
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis) = build_test_env(&ops);
        let mut graph = RegionGraph::from_cfg(&cfg);

        let initial_count = graph.node_count();
        let mut made_progress = true;
        let mut iterations = 0;

        while made_progress && !graph.is_fully_reduced() && iterations < 100 {
            made_progress = reduce_one_step(&mut graph, &cfg, &analysis, None);
            iterations += 1;
        }

        // Either we reduced fully or we stopped making progress
        assert!(graph.is_fully_reduced() || !made_progress || iterations < 100);
        println!(
            "Reduced from {} to {} nodes in {} iterations",
            initial_count,
            graph.node_count(),
            iterations
        );
    }

    #[test]
    fn test_collapse_sequences() {
        // Test sequence collapsing
        let ops = vec![
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JAlways { offset: 0 }, // Creates a second block
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let mut graph = RegionGraph::from_cfg(&cfg);

        let initial = graph.node_count();
        let collapsed = collapse_sequences(&mut graph);

        println!(
            "Sequence collapse: {} -> {}, collapsed={}",
            initial,
            graph.node_count(),
            collapsed
        );
    }
}
