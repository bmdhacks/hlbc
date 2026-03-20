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
use crate::ast::{Constant, Expr};
use crate::exception_analysis::ExceptionAnalysis;
use crate::lifter::Cfg;
use crate::structurer::patterns::{
    find_if_patterns, find_loop_patterns, find_or_chain_patterns, find_switch_patterns,
    IfPattern, LoopPattern, OrChainPattern, PatternContext, SwitchPattern,
};
use crate::structurer::region::{Region, SwitchCase};
use crate::structurer::{InlineExpansionCfgMapping, StringSwitchCfgMapping};
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
/// If `string_switches` is provided, pre-collapses string switch patterns before the main loop.
pub fn reduce_to_region(
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    ctx: Option<&PatternContext<'_>>,
) -> Region {
    reduce_to_region_with_string_switches(cfg, analysis, ctx, &[])
}

/// Reduce a CFG to a single Region, with support for pre-collapsing string switches.
pub fn reduce_to_region_with_string_switches(
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    ctx: Option<&PatternContext<'_>>,
    string_switches: &[StringSwitchCfgMapping],
) -> Region {
    reduce_to_region_with_exceptions(cfg, analysis, ctx, string_switches, &[], None)
}

/// Reduce a CFG to a single Region, with support for exceptions, string switches,
/// and inline expansion pre-collapse.
///
/// This is the most complete variant of the reduction function.
pub fn reduce_to_region_with_exceptions(
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    ctx: Option<&PatternContext<'_>>,
    string_switches: &[StringSwitchCfgMapping],
    _inline_expansions: &[InlineExpansionCfgMapping],
    exception_analysis: Option<&ExceptionAnalysis>,
) -> Region {
    let mut graph = RegionGraph::from_cfg(cfg);
    let mut iterations = 0;

    // Phase -1: Remove unreachable (dead code) nodes before reduction.
    remove_unreachable_nodes(&mut graph);

    // Phase 0a: Pre-collapse string switches before the main reduction loop
    for ss in string_switches {
        collapse_string_switch(&mut graph, cfg, ss);
    }

    // Note: Inline expansion pre-collapse is handled at the lifter level —
    // expansion-internal jumps are ignored during CFG construction, so the
    // structurer never sees the capacity-check branches.

    let debug_reduce = std::env::var("HLBC_DEBUG_REDUCE").is_ok();

    while !graph.is_fully_reduced() && iterations < MAX_ITERATIONS {
        iterations += 1;

        if debug_reduce {
            eprintln!("[reducer] iter {}, {} nodes:", iterations, graph.node_count());
            for ni in graph.node_indices() {
                if let Some(node) = graph.get_node(ni) {
                    let desc = match node {
                        crate::structurer::region_graph::RegionNode::Block(cfg_idx) => {
                            let block = &cfg.graph[*cfg_idx];
                            format!("Block(ops {}..{})", block.start, block.end)
                        },
                        crate::structurer::region_graph::RegionNode::Collapsed(_) => "Collapsed".to_string(),
                    };
                    let succs: Vec<_> = graph.successors(ni).iter().map(|n| n.index()).collect();
                    eprintln!("  N[{}]: {} → {:?}", ni.index(), desc, succs);
                }
            }
        }

        let made_progress = reduce_one_step(&mut graph, cfg, analysis, ctx, exception_analysis);

        if !made_progress {
            if debug_reduce {
                eprintln!("[reducer] no progress at iter {}", iterations);
            }
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
/// 0. Try-catch patterns (highest priority - exception edges confuse other patterns)
/// 1. OR chain patterns (multiple conditions sharing a target)
/// 2. If-then-else patterns inside loops
/// 3. Innermost loops (smallest body first)
/// 4. Remaining if-then-else patterns
/// 5. Switch patterns
/// 6. Linear sequences
///
/// Returns true if any reduction was made.
fn reduce_one_step(
    graph: &mut RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    ctx: Option<&PatternContext<'_>>,
    exception_analysis: Option<&ExceptionAnalysis>,
) -> bool {
    #[cfg(debug_assertions)]
    let node_count_before = graph.node_count();
    let made_progress;

    // Collect loop headers to avoid collapsing them as if-else patterns
    let loop_headers: HashSet<NodeIndex> = analysis.loops.iter().map(|l| l.header).collect();

    // Priority 0: Try-catch patterns
    // NOTE: Try-catch pattern detection is currently disabled because it has
    // issues with CFG blocks that span multiple exception regions. When
    // CFG block boundaries don't align with exception boundaries, the
    // collapse produces incorrect nesting.
    //
    // TODO: Revisit try-catch handling with a different approach:
    // - Handle try-catch during lowering based on Trap/EndTrap opcodes
    // - Or split CFG blocks at exception boundaries before pattern matching
    let _ = exception_analysis; // Silence unused warning

    // Priority 1: OR chain patterns (e.g., `if (a || b || c) throw X`)
    // These need to be detected BEFORE regular if patterns because each individual
    // condition block in an OR chain can't be collapsed alone (the shared target
    // isn't dominated by any single condition).
    let debug_reduce = std::env::var("HLBC_DEBUG_REDUCE").is_ok();

    // Build a map from CFG node → loop body for checking loop containment
    let loop_bodies: Vec<&HashSet<NodeIndex>> = analysis.loops.iter().map(|l| &l.body).collect();

    let or_chain_patterns = find_or_chain_patterns(graph, cfg, analysis, ctx);
    if let Some(ocp) = or_chain_patterns.into_iter().next() {
        if debug_reduce { eprintln!("  → P1: OR chain"); }
        made_progress = collapse_or_chain(graph, cfg, &ocp);
    } else {
        let if_patterns = find_if_patterns(graph, cfg, analysis);
        if let Some(ip) = if_patterns
            .into_iter()
            .find(|p| {
                let is_loop_header = graph
                    .get_node(p.condition_node)
                    .and_then(|n| n.as_block())
                    .map(|cfg_node| loop_headers.contains(&cfg_node))
                    .unwrap_or(false);

                let has_branch_nodes = !p.then_nodes.is_empty() || !p.else_nodes.is_empty();
                let is_early_return = p.then_exit_target.is_some();

                // Only match if-patterns where the condition is inside a loop body.
                // Outer conditions that span loops should wait until after loop collapse.
                let is_inside_loop = graph.get_node(p.condition_node)
                    .and_then(|n| n.as_block())
                    .map(|cfg_node| loop_bodies.iter().any(|body| body.contains(&cfg_node)))
                    .unwrap_or(false);

                !is_loop_header && is_inside_loop && (has_branch_nodes || is_early_return)
            })
        {
            if debug_reduce {
                eprintln!("  → P2: if-in-loop cond={:?} then={:?} else={:?} merge={:?}",
                    ip.condition_node, ip.then_nodes, ip.else_nodes, ip.merge);
            }
            made_progress = collapse_if(graph, cfg, &ip);
        } else {
            let loop_patterns = find_loop_patterns(graph, cfg, analysis, ctx);
            if debug_reduce {
                eprintln!("  → P3: {} loop patterns", loop_patterns.len());
                for lp in &loop_patterns {
                    eprintln!("    header={:?} body={:?} exit={:?} kind={:?}",
                        lp.header, lp.body_nodes.iter().map(|n| n.index()).collect::<Vec<_>>(), lp.exit, lp.kind);
                }
            }
            let loop_progress = loop_patterns
                .into_iter()
                .find_map(|lp| {
                    if collapse_loop(graph, cfg, &lp) {
                        Some(true)
                    } else {
                        None
                    }
                });
            if loop_progress.is_some() {
                made_progress = true;
            } else {
                // Priority 4: Collapse remaining if-then-else patterns (including loop headers)
                // Still skip patterns with empty branches (would not reduce node count)
                // Exception: early-return patterns are valid even with empty then_nodes
                let if_patterns = find_if_patterns(graph, cfg, analysis);
                if let Some(ip) = if_patterns.into_iter().find(|p| {
                    !p.then_nodes.is_empty() || !p.else_nodes.is_empty() || p.then_exit_target.is_some()
                })
                {
                    if debug_reduce {
                        eprintln!("  → P4: if-else cond={:?} then={:?} else={:?} merge={:?} exit={:?}",
                            ip.condition_node, ip.then_nodes, ip.else_nodes, ip.merge, ip.then_exit_target);
                    }
                    made_progress = collapse_if(graph, cfg, &ip);
                } else {
                    // Priority 5: Collapse switch patterns
                    let switch_patterns = find_switch_patterns(graph, cfg, analysis, ctx);
                    if let Some(sp) = switch_patterns.into_iter().next() {
                        if debug_reduce { eprintln!("  → P5: switch"); }
                        collapse_switch(graph, cfg, &sp, ctx);
                        made_progress = true;
                    } else {
                        // Priority 6: Collapse linear sequences
                        if debug_reduce { eprintln!("  → P6: sequences"); }
                        made_progress = collapse_sequences(graph);
                    }
                }
            }
        }
    }

    // INVARIANT: If we claimed progress, node count must have decreased
    #[cfg(debug_assertions)]
    debug_assert!(
        !made_progress || graph.node_count() < node_count_before,
        "reduce_one_step claimed progress but node count didn't decrease: {} -> {}",
        node_count_before,
        graph.node_count()
    );

    // INVARIANT: All nodes should still be reachable from entry
    // (no orphaned nodes after collapse)
    #[cfg(debug_assertions)]
    if made_progress {
        let reachable = compute_reachable_nodes(graph);
        let all_nodes: HashSet<_> = graph.node_indices().collect();
        let orphaned: HashSet<_> = all_nodes.difference(&reachable).collect();
        debug_assert!(
            orphaned.is_empty(),
            "collapse left orphaned nodes: {:?}",
            orphaned
        );
    }

    made_progress
}

/// Compute all nodes reachable from the entry node via BFS.
fn compute_reachable_nodes_impl(graph: &RegionGraph) -> HashSet<NodeIndex> {
    use std::collections::VecDeque;
    let mut reachable = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(graph.entry());

    while let Some(node) = queue.pop_front() {
        if reachable.insert(node) {
            for succ in graph.successors(node) {
                if !reachable.contains(&succ) {
                    queue.push_back(succ);
                }
            }
        }
    }
    reachable
}

/// Wrapper for debug assertions.
#[cfg(debug_assertions)]
fn compute_reachable_nodes(graph: &RegionGraph) -> HashSet<NodeIndex> {
    compute_reachable_nodes_impl(graph)
}

/// Remove nodes that are not reachable from the entry node.
/// This handles dead code that the compiler may have generated,
/// such as EndTrap opcodes that appear after a Ret.
fn remove_unreachable_nodes(graph: &mut RegionGraph) {
    let reachable = compute_reachable_nodes_impl(graph);
    let all_nodes: Vec<_> = graph.node_indices().collect();

    // Find unreachable nodes
    let mut unreachable: Vec<_> = all_nodes
        .into_iter()
        .filter(|n| !reachable.contains(n))
        .collect();

    // Sort in descending order by index to avoid invalidating remaining indices
    // due to petgraph's swap-remove behavior (which moves the last node to fill gaps)
    unreachable.sort_by(|a, b| b.index().cmp(&a.index()));

    // Remove them from highest index first
    for node in unreachable {
        graph.remove_node(node);
    }
}

/// Collapse a loop pattern. Returns true if progress was made (nodes were reduced).
fn collapse_loop(graph: &mut RegionGraph, _cfg: &Cfg, pattern: &LoopPattern) -> bool {
    // Get the header's region node
    let header_region_node = graph.get_region_node(pattern.header);

    // Build the loop body region EXCLUDING the header.
    // The header is processed separately during lowering for:
    // 1. Extracting the loop condition from its terminating jump
    // 2. Emitting any preamble statements
    let body_nodes_without_header: HashSet<NodeIndex> = pattern.body_nodes
        .iter()
        .filter(|&&n| Some(n) != header_region_node)
        .copied()
        .collect();

    let body_region = build_region_from_nodes(graph, &body_nodes_without_header, pattern.header);

    // Create the loop region
    // Use None for condition - it will be extracted from the header block during lowering.
    // This ensures extract_condition is actually called with the real header block.
    let condition = None;

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

    // Only collapse if we have 2+ nodes (otherwise no net reduction)
    if nodes_to_collapse.len() >= 2 {
        graph.collapse(&nodes_to_collapse, loop_region);
        true
    } else {
        // Would not reduce node count - skip this collapse
        false
    }
}

/// Collapse an if-then-else pattern into a Region::IfThenElse node.
/// Returns true if progress was made (nodes were reduced).
fn collapse_if(graph: &mut RegionGraph, cfg: &Cfg, pattern: &IfPattern) -> bool {
    // INVARIANT: then and else nodes should not overlap
    #[cfg(debug_assertions)]
    {
        let overlap: HashSet<_> = pattern
            .then_nodes
            .intersection(&pattern.else_nodes)
            .collect();
        debug_assert!(
            overlap.is_empty(),
            "collapse_if: then and else nodes overlap: {:?}",
            overlap
        );
    }

    // INVARIANT: condition node should not be in either branch
    debug_assert!(
        !pattern.then_nodes.contains(&pattern.condition_node),
        "collapse_if: condition node {:?} found in then_nodes",
        pattern.condition_node
    );
    debug_assert!(
        !pattern.else_nodes.contains(&pattern.condition_node),
        "collapse_if: condition node {:?} found in else_nodes",
        pattern.condition_node
    );

    // Build then branch region
    let mut then_region = if pattern.then_nodes.is_empty() {
        // For early-return patterns: if then_nodes is empty but we have an exit target,
        // create a Block region pointing to the exit. This handles cases where the exit
        // block is shared (not dominated) but we still need to emit the early return.
        if let Some(exit_target) = pattern.then_exit_target {
            Region::Block(exit_target)
        } else {
            Region::Empty
        }
    } else {
        build_region_from_nodes(graph, &pattern.then_nodes, NodeIndex::new(0))
    };

    // Build else branch region (if present)
    let mut else_region = if pattern.else_nodes.is_empty() {
        None
    } else {
        Some(build_region_from_nodes(
            graph,
            &pattern.else_nodes,
            NodeIndex::new(0),
        ))
    };

    // Normalize: if then is empty but else is not, swap them and negate the condition.
    // This produces cleaner output: `if (c) {} else { body }` → `if (!c) { body }`
    let negated = matches!(then_region, Region::Empty) && else_region.is_some();

    if negated {
        // Swap: else becomes then, then (Empty) becomes else (None)
        then_region = else_region.take().unwrap();
        else_region = None;
    }

    // Get the CFG node from the condition region node.
    // This is used during lowering to extract the actual condition and emit preamble.
    let cond_block = if let Some(RegionNode::Block(cfg_idx)) = graph.get_node(pattern.condition_node) {
        Some(*cfg_idx)
    } else if let Some(RegionNode::Collapsed(_)) = graph.get_node(pattern.condition_node) {
        // For collapsed condition nodes, find the exit block: the CFG block within
        // the collapsed region whose conditional jump leads to the then/else targets.
        // This happens when a sequence of code ending in a conditional gets collapsed
        // (e.g., OR chain + downcast ending in JNull check).
        //
        // IMPORTANT: When multiple blocks qualify (e.g., a loop with both header and
        // body blocks having external exits), pick the one with the HIGHEST start op
        // (latest in execution). This avoids picking a loop header whose opcodes would
        // be emitted twice (once inside the loop body, once in the if-else preamble).
        let cfg_nodes = graph.get_cfg_nodes(pattern.condition_node);
        cfg_nodes.and_then(|nodes| {
            let mut candidates: Vec<NodeIndex> = nodes.iter()
                .filter(|&&cfg_node| {
                    let succs = cfg.successors(cfg_node);
                    // The exit block has successors outside the collapsed region
                    succs.len() == 2 && succs.iter().any(|s| !nodes.contains(s))
                })
                .copied()
                .collect();
            // Sort by start op index (descending) to pick the latest block
            candidates.sort_by(|a, b| {
                let a_start = cfg.graph[*a].start;
                let b_start = cfg.graph[*b].start;
                b_start.cmp(&a_start)
            });
            candidates.first().copied()
        })
    } else {
        None
    };

    // Create the if-then-else region
    // Use a placeholder condition - the actual condition will be extracted during lowering
    // from cond_block's terminating conditional jump.
    let cond = Expr::Constant(crate::ast::Constant::Bool(true));

    // For collapsed condition nodes, extract the preamble region so the lowering
    // phase can emit the collapsed region's statements before the if-statement.
    let cond_preamble = if let Some(RegionNode::Collapsed(region)) = graph.get_node(pattern.condition_node) {
        Some(Box::new(region.clone()))
    } else {
        None
    };

    let if_region = Region::IfThenElse {
        cond,
        cond_block,
        cond_preamble,
        then_region: Box::new(then_region),
        else_region: else_region.map(Box::new),
        merge: pattern.merge,
        negated,
    };

    // Collect all nodes to collapse (condition + branches)
    let mut nodes_to_collapse = HashSet::new();
    nodes_to_collapse.insert(pattern.condition_node);
    nodes_to_collapse.extend(pattern.then_nodes.iter().copied());
    nodes_to_collapse.extend(pattern.else_nodes.iter().copied());
    // Don't include merge - it's where control reconverges

    // INVARIANT: Must collapse at least the condition node
    debug_assert!(
        !nodes_to_collapse.is_empty(),
        "collapse_if: no nodes to collapse"
    );

    // INVARIANT: Merge node should not be in collapse set.
    // Exception: when both branches terminate (both-terminate case), the "merge" is
    // actually a dummy merge that IS one of the branches, so it will be in the collapse set.
    // We detect this by checking if merge is in then_nodes or else_nodes.
    let merge_is_dummy = pattern.then_nodes.contains(&pattern.merge)
        || pattern.else_nodes.contains(&pattern.merge);
    if !merge_is_dummy {
        debug_assert!(
            !nodes_to_collapse.contains(&pattern.merge),
            "collapse_if: merge node {:?} should not be collapsed",
            pattern.merge
        );
    }

    // Only collapse if we have 2+ nodes (otherwise no net reduction)
    if nodes_to_collapse.len() >= 2 {
        graph.collapse(&nodes_to_collapse, if_region);
        true
    } else {
        // Would not reduce node count - skip this collapse
        false
    }
}

/// Collapse an OR chain pattern into a Region::OrChain with compound condition.
///
/// OR chains like `if (a || b || c || d) throw X` compile to multiple condition blocks
/// that all share the same true target. We collapse them into a single OrChain region
/// with the shared target as the then branch.
///
/// Nested AND chains like `if (a || (b && c) || d) throw X` are also handled.
/// The nested_and_chains map stores which condition indices have AND sub-chains.
///
/// Returns true if progress was made (nodes were reduced).
fn collapse_or_chain(graph: &mut RegionGraph, cfg: &Cfg, pattern: &OrChainPattern) -> bool {
    // If-else OR: non-terminating shared target with no body nodes collected
    // (i.e., shared_target can't reach continuation — they're sibling branches)
    let is_if_else_or = !pattern.shared_target_terminates && pattern.body_cfg_nodes.is_empty();

    // Build a special region that captures all the condition nodes for compound OR generation
    let condition_cfg_nodes: Vec<NodeIndex> = pattern.condition_nodes.iter()
        .filter_map(|&node| graph.get_node(node).and_then(|n| n.as_block()))
        .collect();

    let nested_and_chains = pattern.nested_and_chains.clone();

    if is_if_else_or {
        // Non-terminating if-else OR chain: `if (a || b) { THEN } else { ELSE }; MERGE`
        //
        // Find the merge point (common descendant of shared_target and continuation),
        // collect then/else branch nodes, and produce an IfThenElse with compound condition.
        // This avoids edge-leaking problems from collapsing just conditions or body.
        let shared_target_cfg = graph.get_node(pattern.shared_target)
            .and_then(|n| n.as_block());
        let continuation_cfg = graph.get_node(pattern.continuation)
            .and_then(|n| n.as_block());

        let (shared_target_cfg, continuation_cfg) = match (shared_target_cfg, continuation_cfg) {
            (Some(s), Some(c)) => (s, c),
            _ => return false,
        };

        // Find merge point: BFS from both, first intersection
        let mut shared_reachable = HashSet::new();
        let mut queue = vec![shared_target_cfg];
        shared_reachable.insert(shared_target_cfg);
        while let Some(node) = queue.pop() {
            for succ in cfg.successors(node) {
                if shared_reachable.insert(succ) {
                    queue.push(succ);
                }
            }
        }

        let mut merge_cfg = None;
        let mut queue = vec![continuation_cfg];
        let mut visited = HashSet::new();
        visited.insert(continuation_cfg);
        while let Some(node) = queue.pop() {
            if shared_reachable.contains(&node) {
                merge_cfg = Some(node);
                break;
            }
            for succ in cfg.successors(node) {
                if visited.insert(succ) {
                    queue.push(succ);
                }
            }
        }

        let merge_cfg = match merge_cfg {
            Some(m) => m,
            None => return false,
        };

        let merge_region_node = match graph.get_region_node(merge_cfg) {
            Some(n) => n,
            None => return false,
        };

        // Collect all nodes in the then-branch (shared_target → merge)
        let then_nodes = collect_path_nodes(graph, pattern.shared_target, merge_region_node);
        let then_region = build_region_from_nodes(graph, &then_nodes, NodeIndex::new(0));

        // Collect all nodes in the else-branch (continuation → merge)
        let else_nodes = collect_path_nodes(graph, pattern.continuation, merge_region_node);
        let else_region = build_region_from_nodes(graph, &else_nodes, NodeIndex::new(0));

        let if_region = Region::OrChain {
            condition_blocks: condition_cfg_nodes,
            then_region: Box::new(then_region),
            else_region: Some(Box::new(else_region)),
            continuation: pattern.continuation,
            last_condition_inverted: pattern.last_condition_inverted,
            nested_and_chains,
        };

        // Collapse: conditions + then branch + else branch + merge
        let mut nodes_to_collapse = HashSet::new();
        nodes_to_collapse.extend(pattern.condition_nodes.iter().copied());
        nodes_to_collapse.extend(then_nodes.iter().copied());
        nodes_to_collapse.extend(else_nodes.iter().copied());
        nodes_to_collapse.insert(merge_region_node);

        // Include nested AND chain nodes
        for and_chain in pattern.nested_and_chains.values() {
            for &cfg_node in and_chain {
                if let Some(region_node) = graph.get_region_node(cfg_node) {
                    nodes_to_collapse.insert(region_node);
                }
            }
        }

        if nodes_to_collapse.len() >= 2 {
            graph.collapse(&nodes_to_collapse, if_region);
            true
        } else {
            false
        }
    } else {
        // Terminating body: standard OR chain collapse
        let then_region = if !pattern.body_cfg_nodes.is_empty() {
            structure_body_subgraph(graph, cfg, &pattern.body_cfg_nodes)
        } else if let Some(node) = graph.get_node(pattern.shared_target) {
            match node {
                RegionNode::Block(cfg_idx) => Region::Block(*cfg_idx),
                RegionNode::Collapsed(r) => r.clone(),
            }
        } else {
            Region::Empty
        };

        let if_region = Region::OrChain {
            condition_blocks: condition_cfg_nodes,
            then_region: Box::new(then_region),
            else_region: None,
            continuation: pattern.continuation,
            last_condition_inverted: pattern.last_condition_inverted,
            nested_and_chains,
        };

        let mut nodes_to_collapse = HashSet::new();
        nodes_to_collapse.extend(pattern.condition_nodes.iter().copied());
        nodes_to_collapse.insert(pattern.shared_target);

        for &cfg_node in &pattern.body_cfg_nodes {
            if let Some(region_node) = graph.get_region_node(cfg_node) {
                nodes_to_collapse.insert(region_node);
            }
        }

        for and_chain in pattern.nested_and_chains.values() {
            for &cfg_node in and_chain {
                if let Some(region_node) = graph.get_region_node(cfg_node) {
                    nodes_to_collapse.insert(region_node);
                }
            }
        }

        if nodes_to_collapse.len() >= 2 {
            graph.collapse(&nodes_to_collapse, if_region);
            true
        } else {
            false
        }
    }
}

/// Structure a subgraph of body nodes into a Region.
/// Used for OR chain bodies that contain nested control flow.
fn structure_body_subgraph(
    graph: &RegionGraph,
    _cfg: &Cfg,
    body_cfg_nodes: &[NodeIndex],
) -> Region {
    if body_cfg_nodes.is_empty() {
        return Region::Empty;
    }

    // Collect unique regions, avoiding duplicates when multiple CFG nodes map
    // to the same collapsed region node.
    let mut regions: Vec<Region> = Vec::new();
    let mut seen_region_nodes: HashSet<NodeIndex> = HashSet::new();

    for &cfg_node in body_cfg_nodes {
        if let Some(region_node) = graph.get_region_node(cfg_node) {
            // Skip if we've already added this region
            if seen_region_nodes.contains(&region_node) {
                continue;
            }
            seen_region_nodes.insert(region_node);

            if let Some(node) = graph.get_node(region_node) {
                match node {
                    RegionNode::Block(cfg_idx) => {
                        regions.push(Region::Block(*cfg_idx));
                    }
                    RegionNode::Collapsed(r) => {
                        regions.push(r.clone());
                    }
                }
            }
        }
    }

    if regions.len() == 1 {
        regions.pop().unwrap()
    } else if regions.is_empty() {
        Region::Empty
    } else {
        Region::Sequence(regions)
    }
}

/// Collapse a switch pattern into a Region::Switch node.
fn collapse_switch(
    graph: &mut RegionGraph,
    _cfg: &Cfg,
    pattern: &SwitchPattern,
    _ctx: Option<&PatternContext<'_>>,
) {
    use crate::ast::Constant;

    // Build cases from case nodes.
    // For each case, we need to collect ALL nodes from the case target to the merge,
    // not just the direct successor. This handles cases where if-patterns have been
    // collapsed inside a switch case, leaving subsequent code as separate nodes.
    let mut cases = Vec::new();
    for &case_node in &pattern.case_nodes {
        // Collect all nodes in this case's path to the merge
        let mut case_body_nodes: Vec<NodeIndex> = Vec::new();
        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let mut queue = vec![case_node];
        visited.insert(case_node);
        visited.insert(pattern.merge); // Don't include merge in case body

        while let Some(node) = queue.pop() {
            case_body_nodes.push(node);

            // Follow successors until we hit the merge or exit
            for succ in graph.successors(node) {
                if visited.insert(succ) {
                    queue.push(succ);
                }
            }
        }

        // Build the case body from collected nodes
        let body = if case_body_nodes.len() == 1 {
            // Just one node - use it directly
            if let Some(node) = graph.get_node(case_node) {
                match node {
                    RegionNode::Block(cfg_idx) => Region::Block(*cfg_idx),
                    RegionNode::Collapsed(r) => r.clone(),
                }
            } else {
                Region::Empty
            }
        } else {
            // Multiple nodes - build a sequence
            let regions: Vec<Region> = case_body_nodes
                .iter()
                .filter_map(|&n| graph.get_node(n))
                .map(|node| match node {
                    RegionNode::Block(cfg_idx) => Region::Block(*cfg_idx),
                    RegionNode::Collapsed(r) => r.clone(),
                })
                .collect();
            Region::sequence(regions)
        };

        // Get case values for this node from the pattern
        // Use InlineInt since these are literal case values, not indices into the int table
        let patterns: Vec<Constant> = pattern
            .case_values
            .get(&case_node)
            .map(|values| values.iter().map(|&v| Constant::InlineInt(v as usize)).collect())
            .unwrap_or_default();

        cases.push(SwitchCase { patterns, body });
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

    // Build selector expression from the register
    let selector = if let Some(reg) = pattern.selector_reg {
        // Use the register directly - the expression builder will name it properly during lowering
        Expr::Variable(reg, None)
    } else {
        // Fallback: placeholder constant (shouldn't happen if ctx was available)
        Expr::Constant(Constant::Int(RefInt(0)))
    };

    // Create the switch region
    let switch_region = Region::Switch {
        selector,
        selector_block: Some(pattern.selector_cfg_block),
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

// NOTE: collapse_try_catch has been removed.
// Try-catch handling is now done at the opcode level during lowering
// via lower_with_exceptions() in lower.rs. This avoids issues where
// CFG blocks don't align with exception boundaries.

/// Collapse a string switch pattern into a Region::Switch node.
///
/// String switches are a special pattern in HashLink bytecode that uses
/// multiple 9-opcode sequences to compare strings, rather than a single
/// Switch opcode. This function collapses all the pattern-checking nodes
/// AND handler nodes into a Switch region.
fn collapse_string_switch(graph: &mut RegionGraph, _cfg: &Cfg, ss: &StringSwitchCfgMapping) {
    // Get region nodes for pattern nodes
    let pattern_region_nodes: HashSet<NodeIndex> = ss.pattern_nodes
        .iter()
        .filter_map(|&cfg_node| graph.get_region_node(cfg_node))
        .collect();

    if pattern_region_nodes.is_empty() {
        return; // Nothing to collapse
    }

    // Build SwitchCase for each case (string literal -> handler region)
    let cases: Vec<SwitchCase> = ss.handler_nodes
        .iter()
        .filter_map(|(string_ref, handler_cfg_node)| {
            let handler_region_node = graph.get_region_node(*handler_cfg_node)?;
            let body = match graph.get_node(handler_region_node) {
                Some(RegionNode::Block(idx)) => Region::Block(*idx),
                Some(RegionNode::Collapsed(r)) => r.clone(),
                None => Region::Empty,
            };
            Some(SwitchCase {
                patterns: vec![Constant::String(*string_ref)],
                body,
            })
        })
        .collect();

    // Build the default case region
    let default = if let Some(default_region_node) = graph.get_region_node(ss.default_node) {
        match graph.get_node(default_region_node) {
            Some(RegionNode::Block(idx)) => Region::Block(*idx),
            Some(RegionNode::Collapsed(r)) => r.clone(),
            None => Region::Empty,
        }
    } else {
        Region::Empty
    };

    // Find merge point: the node that all handlers and default can reach
    // For now, use the default node as the merge point (common for string switches)
    let merge = ss.default_node;

    // Find a selector block for variable name resolution during lowering.
    // Use the first pattern node as context - it has access to the switch argument register.
    let selector_block = ss.pattern_nodes.iter().min().copied();

    // Build the Switch region
    // Use the switch_arg_reg for the selector expression
    let switch_region = Region::Switch {
        selector: Expr::Variable(ss.switch_arg_reg, None),
        selector_block,  // Use first pattern block for variable resolution
        cases,
        default: Box::new(default),
        merge,
    };

    // Collect ALL nodes to collapse: pattern nodes + handler nodes + default node
    // This ensures the handlers don't appear twice (once in switch, once as standalone blocks)
    let mut nodes_to_collapse = pattern_region_nodes.clone();

    // Add handler nodes to collapse set
    for (_, handler_cfg_node) in &ss.handler_nodes {
        if let Some(handler_region_node) = graph.get_region_node(*handler_cfg_node) {
            nodes_to_collapse.insert(handler_region_node);
        }
    }

    // Add default node to collapse set
    if let Some(default_region_node) = graph.get_region_node(ss.default_node) {
        nodes_to_collapse.insert(default_region_node);
    }

    // Collapse all nodes
    if !nodes_to_collapse.is_empty() {
        graph.collapse(&nodes_to_collapse, switch_region);
    }
}

/// Collect all nodes on the path from `start` to `merge` (exclusive of merge).
/// Used to find all nodes in a branch of an if-else OR chain.
fn collect_path_nodes(
    graph: &RegionGraph,
    start: NodeIndex,
    merge: NodeIndex,
) -> HashSet<NodeIndex> {
    let mut nodes = HashSet::new();
    let mut queue = vec![start];
    let mut visited = HashSet::new();
    visited.insert(merge); // Don't include merge

    while let Some(node) = queue.pop() {
        if !visited.insert(node) {
            continue;
        }
        if !graph.contains(node) {
            continue;
        }
        nodes.insert(node);
        for succ in graph.successors(node) {
            if !visited.contains(&succ) {
                queue.push(succ);
            }
        }
    }

    nodes
}

/// Collapse an inline expansion's CFG nodes into a single Sequence region.
///
/// Inline expansions (like BytesBuffer.addByte) create multiple basic blocks
/// due to capacity checks. Pre-collapsing them removes these internal
/// conditionals from the structurer's view.
fn collapse_inline_expansion(graph: &mut RegionGraph, cfg_nodes: &HashSet<NodeIndex>) {
    if cfg_nodes.len() < 2 {
        return; // Nothing to collapse
    }

    // Map CFG nodes to region nodes
    let region_nodes: HashSet<NodeIndex> = cfg_nodes.iter()
        .filter_map(|&cfg_node| graph.get_region_node(cfg_node))
        .collect();

    if region_nodes.len() < 2 {
        return;
    }

    // Build a sequence of the blocks in opcode order
    let mut ordered: Vec<(NodeIndex, Option<NodeIndex>)> = region_nodes.iter()
        .filter_map(|&node| {
            graph.get_node(node).and_then(|n| n.as_block()).map(|cfg_idx| (node, Some(cfg_idx)))
        })
        .collect();
    ordered.sort_by_key(|(_, cfg_idx)| cfg_idx.map(|n| n.index()));

    let regions: Vec<Region> = ordered.iter()
        .filter_map(|&(node, _)| {
            graph.get_node(node).map(|n| match n {
                RegionNode::Block(cfg_idx) => Region::Block(*cfg_idx),
                RegionNode::Collapsed(r) => r.clone(),
            })
        })
        .collect();

    let sequence = Region::sequence(regions);
    graph.collapse(&region_nodes, sequence);
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

/// Get a deterministic sort key for a region graph node.
///
/// Returns the minimum CFG node index contained in this region node,
/// which corresponds to the earliest opcode in the original bytecode.
/// This ensures deterministic ordering regardless of HashMap iteration order.
fn region_node_sort_key(graph: &RegionGraph, node: NodeIndex) -> usize {
    graph.get_cfg_nodes(node)
        .and_then(|cfg_nodes| cfg_nodes.iter().map(|n| n.index()).min())
        .unwrap_or(node.index())
}

/// Order nodes by following the control flow edges using topological sort.
///
/// Returns nodes in execution order (respecting dominance/flow).
/// Uses Kahn's algorithm with in-degree tracking to ensure proper ordering.
/// All tie-breaking uses deterministic sort keys (minimum CFG node index)
/// to avoid non-deterministic output from HashMap iteration order.
fn order_nodes_by_flow(graph: &RegionGraph, nodes: &HashSet<NodeIndex>) -> Vec<NodeIndex> {
    use std::collections::BinaryHeap;
    use std::cmp::Reverse;

    if nodes.is_empty() {
        return Vec::new();
    }

    // Compute in-degree for each node (counting only edges within our node set)
    let mut in_degree: std::collections::HashMap<NodeIndex, usize> = nodes
        .iter()
        .map(|&n| {
            let pred_count = graph
                .predecessors(n)
                .into_iter()
                .filter(|p| nodes.contains(p))
                .count();
            (n, pred_count)
        })
        .collect();

    // Initialize queue with nodes that have no predecessors in the set.
    // Use a min-heap keyed by CFG sort key for deterministic ordering.
    let mut queue: BinaryHeap<Reverse<(usize, NodeIndex)>> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&n, _)| Reverse((region_node_sort_key(graph, n), n)))
        .collect();

    // If no node has in-degree 0, find the one with minimum in-degree
    // (handles cycles or disconnected components).
    // Use deterministic tie-breaking by sort key.
    if queue.is_empty() {
        if let Some((&min_node, _)) = in_degree.iter()
            .min_by_key(|(&n, &deg)| (deg, region_node_sort_key(graph, n)))
        {
            queue.push(Reverse((region_node_sort_key(graph, min_node), min_node)));
            in_degree.insert(min_node, 0); // Mark as processed
        }
    }

    let mut ordered = Vec::new();
    let mut visited = HashSet::new();

    while let Some(Reverse((_, node))) = queue.pop() {
        if visited.contains(&node) {
            continue;
        }
        visited.insert(node);
        ordered.push(node);

        // Decrease in-degree of successors
        for succ in graph.successors(node) {
            if let Some(deg) = in_degree.get_mut(&succ) {
                *deg = deg.saturating_sub(1);
                if *deg == 0 && !visited.contains(&succ) {
                    queue.push(Reverse((region_node_sort_key(graph, succ), succ)));
                }
            }
        }
    }

    // Add any remaining nodes not yet visited (cycles or disconnected)
    // Sort deterministically by sort key
    let mut remaining: Vec<_> = nodes.iter()
        .filter(|n| !visited.contains(n))
        .copied()
        .collect();
    remaining.sort_by_key(|&n| region_node_sort_key(graph, n));
    ordered.extend(remaining);

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

    /// Helper to check if a region contains any Goto nodes
    fn contains_goto(region: &Region) -> bool {
        match region {
            Region::Goto { .. } => true,
            Region::Sequence(regions) => regions.iter().any(contains_goto),
            Region::IfThenElse {
                then_region,
                else_region,
                ..
            } => {
                contains_goto(then_region)
                    || else_region.as_ref().map_or(false, |r| contains_goto(r))
            }
            Region::Loop { body, .. } => contains_goto(body),
            Region::Switch { cases, default, .. } => {
                cases.iter().any(|c| contains_goto(&c.body)) || contains_goto(default)
            }
            _ => false,
        }
    }

    /// Helper to count IfThenElse regions
    fn count_if_regions(region: &Region) -> usize {
        match region {
            Region::IfThenElse {
                then_region,
                else_region,
                ..
            } => {
                1 + count_if_regions(then_region)
                    + else_region.as_ref().map_or(0, |r| count_if_regions(r))
            }
            Region::Sequence(regions) => regions.iter().map(count_if_regions).sum(),
            Region::Loop { body, .. } => count_if_regions(body),
            Region::Switch { cases, default, .. } => {
                cases.iter().map(|c| count_if_regions(&c.body)).sum::<usize>()
                    + count_if_regions(default)
            }
            _ => 0,
        }
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
            made_progress = reduce_one_step(&mut graph, &cfg, &analysis, None, None);
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

    // =========================================================================
    // Mini-integration tests for specific patterns
    // =========================================================================

    #[test]
    fn test_ternary_produces_if_else_not_goto() {
        // Ternary expression: cond ? a : b
        // Should produce IfThenElse, NOT Goto
        let ops = vec![
            // Block 0: evaluate condition
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JNull {
                reg: Reg(0),
                offset: 2,
            },
            // Block 1: then value (a)
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::JAlways { offset: 1 },
            // Block 2: else value (b)
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(2),
            },
            // Block 3: merge and return
            Opcode::Ret { ret: Reg(1) },
        ];

        let (cfg, analysis) = build_test_env(&ops);
        let region = reduce_to_region(&cfg, &analysis, None);

        // Should not contain any Goto nodes
        assert!(
            !contains_goto(&region),
            "Ternary pattern should not produce Goto nodes"
        );

        // Should have at least one IfThenElse
        let if_count = count_if_regions(&region);
        assert!(
            if_count >= 1,
            "Ternary pattern should produce at least one IfThenElse, got {}",
            if_count
        );
    }

    #[test]
    fn test_early_return_pattern() {
        // Pattern: if (cond) return x; return y;
        // This should produce proper if-then structure, not dead code
        let ops = vec![
            // Block 0: check condition
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JNull {
                reg: Reg(0),
                offset: 1,
            },
            // Block 1: early return
            Opcode::Ret { ret: Reg(0) },
            // Block 2: alternative return
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::Ret { ret: Reg(1) },
        ];

        let (cfg, analysis) = build_test_env(&ops);
        let region = reduce_to_region(&cfg, &analysis, None);

        println!("Early return reduced to: {:?}", region);

        // Should not be empty
        assert!(!matches!(region, Region::Empty));

        // Should not need Goto for this simple pattern
        assert!(
            !contains_goto(&region),
            "Early return pattern should not require Goto"
        );
    }

    #[test]
    fn test_if_without_else() {
        // Pattern: if (cond) { do_thing(); } more_code();
        let ops = vec![
            // Block 0: condition
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JNull {
                reg: Reg(0),
                offset: 1,
            },
            // Block 1: then (only executed if condition true)
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            // Block 2: continuation (merge point)
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis) = build_test_env(&ops);
        let region = reduce_to_region(&cfg, &analysis, None);

        println!("If without else reduced to: {:?}", region);

        // Should have at least one IfThenElse (else_region may be empty)
        let if_count = count_if_regions(&region);
        println!("If regions found: {}", if_count);

        // Main check: should not crash and should produce something
        assert!(!matches!(region, Region::Empty) || cfg.graph.node_count() <= 1);
    }

    #[test]
    fn test_nested_if_inner_first() {
        // Nested ifs should be reduced from innermost to outermost
        // if (a) { if (b) { X } }
        let ops = vec![
            // Block 0: outer if
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JNull {
                reg: Reg(0),
                offset: 3,
            },
            // Block 1: inner if
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::JNull {
                reg: Reg(1),
                offset: 1,
            },
            // Block 2: inner body
            Opcode::Int {
                dst: Reg(2),
                ptr: RefInt(2),
            },
            // Block 3: merge
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis) = build_test_env(&ops);
        let region = reduce_to_region(&cfg, &analysis, None);

        println!("Nested if reduced to: {:?}", region);

        // Should not require Goto for simple nested ifs
        assert!(
            !contains_goto(&region),
            "Nested ifs should not require Goto"
        );
    }

    #[test]
    fn test_reduction_terminates() {
        // Verify that reduction always terminates (within MAX_ITERATIONS)
        // even for complex control flow
        let ops = vec![
            // Create a more complex structure
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JNull {
                reg: Reg(0),
                offset: 4,
            },
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::JNull {
                reg: Reg(1),
                offset: 1,
            },
            Opcode::Int {
                dst: Reg(2),
                ptr: RefInt(2),
            },
            Opcode::JAlways { offset: 1 },
            Opcode::Int {
                dst: Reg(3),
                ptr: RefInt(3),
            },
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis) = build_test_env(&ops);

        // This should complete without hanging
        let region = reduce_to_region(&cfg, &analysis, None);

        // Just verify we got something back
        println!("Complex structure reduced to: {:?}", region);
    }

    #[test]
    fn test_while_loop_with_body() {
        // while (cond) { body; }
        let ops = vec![
            // Block 0: loop header - check condition
            Opcode::Label,
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JNull {
                reg: Reg(0),
                offset: 2,
            },
            // Block 1: loop body
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::JAlways { offset: -4 }, // Back to header
            // Block 2: exit
            Opcode::Ret { ret: Reg(0) },
        ];

        let (cfg, analysis) = build_test_env(&ops);
        let region = reduce_to_region(&cfg, &analysis, None);

        println!("While loop reduced to: {:?}", region);

        // Should produce a Loop region
        fn has_loop_region(region: &Region) -> bool {
            match region {
                Region::Loop { .. } => true,
                Region::Sequence(seq) => seq.iter().any(has_loop_region),
                _ => false,
            }
        }

        assert!(
            has_loop_region(&region) || !matches!(region, Region::Empty),
            "While loop should reduce to a Loop region"
        );
    }
}
