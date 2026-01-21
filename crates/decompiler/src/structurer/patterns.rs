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
use std::collections::{HashMap, HashSet};

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, Reg};
use hlbc::{Bytecode, Resolve};

use crate::analyzer::{CfgAnalysis, NaturalLoop};
use crate::lifter::{Cfg, EdgeKind};
use crate::ssa::{SsaCfg, SsaInstr, SsaVar};
use crate::structurer::region::LoopKind;
use crate::structurer::region_graph::RegionGraph;
use crate::type_prop::TypeInfo;

/// Context for pattern matching with access to SSA and type information.
/// This allows pattern matchers to look at the SSA definitions to detect
/// higher-level patterns like for-in iterator loops.
pub struct PatternContext<'a> {
    pub code: &'a Bytecode,
    pub func: &'a Function,
    pub ssa: &'a SsaCfg,
    pub type_info: &'a TypeInfo,
}

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

    /// The CFG block index containing the Switch opcode.
    /// Used during lowering to properly extract the selector expression.
    pub selector_cfg_block: NodeIndex,

    /// Case target nodes (one per case).
    pub case_nodes: Vec<NodeIndex>,

    /// Default case node.
    pub default_node: Option<NodeIndex>,

    /// The merge point after the switch.
    pub merge: NodeIndex,

    /// All nodes in the switch body.
    pub body_nodes: HashSet<NodeIndex>,

    /// The register being switched on (if ctx was available).
    pub selector_reg: Option<Reg>,

    /// Map from case node (region index) to case values that jump there.
    /// Multiple values may map to the same node (fallthrough/combined cases).
    pub case_values: HashMap<NodeIndex, Vec<i32>>,
}

/// Find loop patterns in the graph that can be collapsed.
///
/// Uses the NaturalLoop information from the analyzer. We look for loops
/// where all body nodes are still present in the RegionGraph (not yet collapsed).
///
/// Returns loops in innermost-first order for proper nesting.
///
/// If `ctx` is provided, enables detection of higher-level patterns like for-in loops.
pub fn find_loop_patterns(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    ctx: Option<&PatternContext<'_>>,
) -> Vec<LoopPattern> {
    let mut patterns = Vec::new();

    // Sort loops by size (smallest/innermost first)
    let mut loops: Vec<_> = analysis.loops.iter().collect();
    loops.sort_by_key(|l| l.body.len());

    for natural_loop in loops {
        if let Some(pattern) = match_loop_pattern(region_graph, cfg, analysis, natural_loop, ctx) {
            patterns.push(pattern);
        }
    }

    patterns
}

/// Try to match a single natural loop as a collapsible pattern.
fn match_loop_pattern(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    natural_loop: &NaturalLoop,
    ctx: Option<&PatternContext<'_>>,
) -> Option<LoopPattern> {
    // Check that all loop body nodes are still in the region graph
    let mut body_region_nodes = HashSet::new();
    for &cfg_node in &natural_loop.body {
        let region_node = region_graph.get_region_node(cfg_node)?;
        body_region_nodes.insert(region_node);
    }

    // Skip if this loop was already collapsed - all CFG nodes map to a single
    // region node that's already collapsed. Without this check, we'd try to
    // wrap an already-collapsed region in another loop.
    if body_region_nodes.len() == 1 && natural_loop.body.len() > 1 {
        let single_node = *body_region_nodes.iter().next().unwrap();
        if let Some(crate::structurer::region_graph::RegionNode::Collapsed(_)) =
            region_graph.get_node(single_node)
        {
            return None; // Already collapsed
        }
    }

    // Find the exit node (first node outside the loop that's reachable from inside)
    let exit = find_loop_exit(cfg, natural_loop)?;

    // Determine loop kind
    let kind = detect_loop_kind(cfg, analysis, natural_loop, ctx);

    // The condition is typically at the header for while loops
    let condition_node = match &kind {
        LoopKind::While | LoopKind::Endless | LoopKind::ForIn { .. } => {
            Some(natural_loop.header)
        }
        _ => {
            // For do-while, condition is at a back-edge source
            natural_loop.back_edge_sources.first().copied()
        }
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
fn detect_loop_kind(
    cfg: &Cfg,
    _analysis: &CfgAnalysis,
    natural_loop: &NaturalLoop,
    ctx: Option<&PatternContext<'_>>,
) -> LoopKind {
    let header = natural_loop.header;
    let header_succs = cfg.successors(header);

    // Check if header has an exit edge (while loop pattern)
    let header_exits = header_succs
        .iter()
        .any(|s| !natural_loop.body.contains(s));

    if header_exits {
        // Header checks condition and may exit -> while loop
        // But first, try to detect for-in pattern if we have SSA context
        if let Some(ctx) = ctx {
            if let Some(for_in) = try_detect_for_in(cfg, natural_loop, ctx) {
                return for_in;
            }
        }
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

/// Try to detect for-in iterator pattern.
/// Pattern: `it = coll.iterator(); while(it.hasNext()) { val = it.next(); ... }`
fn try_detect_for_in(
    cfg: &Cfg,
    natural_loop: &NaturalLoop,
    ctx: &PatternContext<'_>,
) -> Option<LoopKind> {
    let header = natural_loop.header;
    let block = &cfg.graph[header];

    // Find condition register from terminating jump
    let cond_reg = extract_condition_reg(&ctx.func.ops[block.end])?;

    // Look for hasNext() call defining the condition
    let (iterator_reg, _has_next_op) = find_has_next_call(header, cond_reg, ctx)?;

    // Find .next() call in body to get value register
    let (value_reg, next_op) = find_next_call(cfg, natural_loop, iterator_reg, ctx)?;

    // Find the iterator initialization (e.g., `it = map.keys()`)
    let iterator_init_op = find_iterator_init(cfg, natural_loop, iterator_reg, ctx);

    Some(LoopKind::ForIn {
        iterator_reg,
        value_reg,
        next_op: Some(next_op),
        iterator_init_op,
    })
}

/// Find the iterator initialization opcode.
/// Traces the iterator register back to find where it was assigned
/// (e.g., `it = map.keys()` or `it = collection.iterator()`).
fn find_iterator_init(
    cfg: &Cfg,
    natural_loop: &NaturalLoop,
    iterator_reg: Reg,
    ctx: &PatternContext<'_>,
) -> Option<usize> {
    // Search predecessors of the loop header for the iterator assignment
    for pred in cfg.graph.neighbors_directed(natural_loop.header, petgraph::Direction::Incoming) {
        // Skip back-edges (nodes inside the loop)
        if natural_loop.body.contains(&pred) {
            continue;
        }

        if let Some(ssa_block) = ctx.ssa.blocks.get(&pred) {
            // Search for assignment to iterator_reg
            for instr in ssa_block.ops.iter().rev() {
                if let SsaInstr::Op { op_idx, dst: Some(dst), .. } = instr {
                    if dst.reg == iterator_reg {
                        let op = &ctx.func.ops[*op_idx];
                        if is_iterator_creation(op, ctx) {
                            return Some(*op_idx);
                        }
                    }
                }
            }
        }
    }

    // Also check the header block itself - iterator might be created there before
    // the hasNext() call in the same block.
    if let Some(ssa_block) = ctx.ssa.blocks.get(&natural_loop.header) {
        for instr in &ssa_block.ops {
            if let SsaInstr::Op { op_idx, dst: Some(dst), .. } = instr {
                if dst.reg == iterator_reg {
                    let op = &ctx.func.ops[*op_idx];
                    if is_iterator_creation(op, ctx) {
                        return Some(*op_idx);
                    }
                }
            }
        }
    }

    None
}

/// Check if an opcode creates an iterator (e.g., .iterator(), .keys(), .keyValueIterator()).
fn is_iterator_creation(op: &Opcode, ctx: &PatternContext<'_>) -> bool {
    match op {
        Opcode::Call1 { fun, .. } => {
            // Use RefFun.name() which properly looks up through findexes
            let name = fun.name(ctx.code);
            matches!(
                name.as_ref(),
                "iterator" | "keys" | "keyValueIterator" | "values"
            )
        }
        Opcode::CallMethod { field, args, .. } => {
            if let Some(obj_reg) = args.first() {
                let obj_type = &ctx.code[ctx.func.regs[obj_reg.0 as usize]];
                if let hlbc::types::Type::Virtual { fields } = obj_type {
                    if let Some(obj_field) = fields.get(field.0) {
                        let name = ctx.code.get(obj_field.name);
                        return matches!(
                            name.as_ref(),
                            "iterator" | "keys" | "keyValueIterator" | "values"
                        );
                    }
                }
            }
            false
        }
        _ => false,
    }
}

/// Extract the condition register from a conditional jump opcode.
fn extract_condition_reg(op: &Opcode) -> Option<Reg> {
    match op {
        Opcode::JTrue { cond, .. } | Opcode::JFalse { cond, .. } => Some(*cond),
        Opcode::JNotNull { reg, .. } | Opcode::JNull { reg, .. } => Some(*reg),
        _ => None,
    }
}

/// Find a hasNext() call that defines the condition register.
/// Returns the iterator register and the opcode index of the hasNext call.
fn find_has_next_call(
    header: NodeIndex,
    cond_reg: Reg,
    ctx: &PatternContext<'_>,
) -> Option<(Reg, usize)> {
    let ssa_block = ctx.ssa.blocks.get(&header)?;

    // Search backwards through ops for the definition of cond_reg
    for instr in ssa_block.ops.iter().rev() {
        if let SsaInstr::Op { op_idx, dst: Some(dst), uses } = instr {
            if dst.reg == cond_reg {
                let op = &ctx.func.ops[*op_idx];
                if is_has_next_call(op, ctx) && !uses.is_empty() {
                    // The first use is the iterator register (the `this` arg to hasNext)
                    return Some((uses[0].reg, *op_idx));
                }
            }
        }
    }
    None
}

/// Check if an opcode is a call to a function named "hasNext".
/// Handles both Call1 (direct call) and CallMethod (virtual/interface call).
fn is_has_next_call(op: &Opcode, ctx: &PatternContext<'_>) -> bool {
    match op {
        Opcode::Call1 { fun, .. } => {
            let name = ctx.code.functions[fun.0].name(ctx.code);
            name == "hasNext"
        }
        Opcode::CallMethod { field, args, .. } => {
            // CallMethod uses a field index into the object's virtual type.
            // We need to look up the type of the first arg (the object) to find the method name.
            if let Some(obj_reg) = args.first() {
                let obj_type = &ctx.code[ctx.func.regs[obj_reg.0 as usize]];
                if let hlbc::types::Type::Virtual { fields } = obj_type {
                    if let Some(obj_field) = fields.get(field.0) {
                        let name = ctx.code.get(obj_field.name);
                        return name.as_ref() == "hasNext";
                    }
                }
            }
            false
        }
        _ => false,
    }
}

/// Find a .next() call in the loop body that uses the iterator register.
/// Returns the value register (destination of next()) and the opcode index.
fn find_next_call(
    cfg: &Cfg,
    natural_loop: &NaturalLoop,
    iterator_reg: Reg,
    ctx: &PatternContext<'_>,
) -> Option<(Reg, usize)> {
    // Search through body nodes (excluding header) for the next() call
    for &body_node in &natural_loop.body {
        if body_node == natural_loop.header {
            continue;
        }
        if let Some(ssa_block) = ctx.ssa.blocks.get(&body_node) {
            for instr in &ssa_block.ops {
                if let SsaInstr::Op { op_idx, dst: Some(dst), uses } = instr {
                    let op = &ctx.func.ops[*op_idx];
                    if is_next_call(op, uses, iterator_reg, ctx) {
                        return Some((dst.reg, *op_idx));
                    }
                }
            }
        }
    }

    // Also check if next() is in the header block itself (after the condition)
    if let Some(ssa_block) = ctx.ssa.blocks.get(&natural_loop.header) {
        let block = &cfg.graph[natural_loop.header];
        for instr in &ssa_block.ops {
            if let SsaInstr::Op { op_idx, dst: Some(dst), uses } = instr {
                // Only consider ops in the header that are not the terminating jump
                if *op_idx < block.end {
                    let op = &ctx.func.ops[*op_idx];
                    if is_next_call(op, uses, iterator_reg, ctx) {
                        return Some((dst.reg, *op_idx));
                    }
                }
            }
        }
    }

    None
}

/// Check if an opcode is a call to "next" using the given iterator register.
/// Handles both Call1 (direct call) and CallMethod (virtual/interface call).
fn is_next_call(op: &Opcode, uses: &[SsaVar], iterator_reg: Reg, ctx: &PatternContext<'_>) -> bool {
    let is_next = match op {
        Opcode::Call1 { fun, .. } => {
            let name = ctx.code.functions[fun.0].name(ctx.code);
            name.as_ref() == "next"
        }
        Opcode::CallMethod { field, args, .. } => {
            // CallMethod uses a field index into the object's virtual type.
            if let Some(obj_reg) = args.first() {
                let obj_type = &ctx.code[ctx.func.regs[obj_reg.0 as usize]];
                if let hlbc::types::Type::Virtual { fields } = obj_type {
                    if let Some(obj_field) = fields.get(field.0) {
                        let name = ctx.code.get(obj_field.name);
                        return name.as_ref() == "next" && !uses.is_empty() && uses[0].reg == iterator_reg;
                    }
                }
            }
            false
        }
        _ => false,
    };
    is_next && !uses.is_empty() && uses[0].reg == iterator_reg
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

    // Identify then and else branches
    let (then_target, else_target, negated) = identify_branches(&cfg_succs)?;

    // Find merge point (immediate post-dominator)
    // If no post-dominator exists, try to handle early-return patterns
    let real_ipdom = analysis.ipdom(cfg_node);
    let then_terminates = cfg.graph[then_target].is_exit;
    let else_terminates = cfg.graph[else_target].is_exit;

    let (merge_cfg, is_one_branch_early_return) = match real_ipdom {
        Some(m) => (m, false),
        None => {
            // No post-dominator - check for early-return pattern.
            // Pattern: if (cond) return x; ...continuation...
            // In this case, one branch terminates and the other is the continuation.

            if then_terminates && !else_terminates {
                // Then branch returns, else branch continues.
                // The "merge" is the else target (the continuation).
                (else_target, true)
            } else if else_terminates && !then_terminates {
                // Else branch returns, then branch continues.
                (then_target, true)
            } else if then_terminates && else_terminates {
                // Both branches terminate (both return).
                // Use then_target as a dummy merge since there's no actual merge point.
                // The branches will be collected as terminating blocks.
                // Note: is_one_branch_early_return is false here because we don't
                // want the special early-return insertion logic to run.
                (then_target, false)
            } else {
                // Neither terminates and no post-dominator - can't match.
                return None;
            }
        }
    };

    // Track if both branches terminate (no real merge exists)
    let both_branches_terminate = then_terminates && else_terminates && real_ipdom.is_none();

    // Don't match if merge is one of the direct successors (trivial case)
    // These are handled by sequence collapsing
    if cfg_succs.iter().all(|(s, _)| *s == merge_cfg) {
        return None;
    }

    // Collect nodes in each branch
    let then_nodes = collect_branch_nodes(cfg, analysis, then_target, merge_cfg, cfg_node);
    let else_nodes = collect_branch_nodes(cfg, analysis, else_target, merge_cfg, cfg_node);

    // For early-return patterns where ONE branch terminates and the other
    // continues, the continuing branch becomes the "merge". If that continuing
    // branch (which is now merge_cfg) is also an exit block, we need to include
    // it. This does NOT apply when both branches terminate - in that case, we
    // use a dummy merge and shouldn't insert anything.
    let then_nodes = if is_one_branch_early_return
        && then_target == merge_cfg
        && then_terminates
    {
        let mut nodes = then_nodes;
        nodes.insert(then_target);
        nodes
    } else {
        then_nodes
    };

    let else_nodes = if is_one_branch_early_return
        && else_target == merge_cfg
        && else_terminates
    {
        let mut nodes = else_nodes;
        nodes.insert(else_target);
        nodes
    } else {
        else_nodes
    };

    // When both branches terminate, we need to explicitly include them
    // since collect_branch_nodes would skip the dummy merge (which is then_target).
    // In this case:
    // - then_nodes is empty because then_target == merge_cfg (the dummy merge)
    // - else_nodes has else_target (since it's a different node)
    // We add then_target to then_nodes so the then branch gets included.
    // The collapse logic handles the case where merge is in a branch set.
    let then_nodes = if both_branches_terminate && then_nodes.is_empty() {
        let mut nodes = then_nodes;
        nodes.insert(then_target);
        nodes
    } else {
        then_nodes
    };

    let else_nodes = if both_branches_terminate && else_nodes.is_empty() {
        let mut nodes = else_nodes;
        nodes.insert(else_target);
        nodes
    } else {
        else_nodes
    };

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

    // INVARIANT: The condition node should not appear in either branch
    debug_assert!(
        !then_region_nodes.contains(&node),
        "match_if_pattern: condition node {:?} found in then_region_nodes",
        node
    );
    debug_assert!(
        !else_region_nodes.contains(&node),
        "match_if_pattern: condition node {:?} found in else_region_nodes",
        node
    );

    // INVARIANT: then and else should not overlap
    #[cfg(debug_assertions)]
    {
        let overlap: HashSet<_> = then_region_nodes
            .intersection(&else_region_nodes)
            .collect();
        debug_assert!(
            overlap.is_empty(),
            "match_if_pattern: then and else branches overlap at {:?}",
            overlap
        );
    }

    // INVARIANT: merge node should not be in either branch
    // Exception: when both branches terminate, merge is a dummy and one branch
    // will contain the dummy merge (which is actually that branch's exit block)
    if !both_branches_terminate {
        debug_assert!(
            !then_region_nodes.contains(&merge),
            "match_if_pattern: merge node {:?} found in then_region_nodes",
            merge
        );
        debug_assert!(
            !else_region_nodes.contains(&merge),
            "match_if_pattern: merge node {:?} found in else_region_nodes",
            merge
        );
    }

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

        // Check this node is dominated by condition
        if !analysis.dominates(condition, node) {
            continue;
        }

        // For non-terminating blocks, check post-dominance by merge.
        // For terminating blocks (exit nodes like return/throw), they don't reach
        // the merge so post-dominance doesn't apply - include them anyway.
        let is_exit = cfg.graph[node].is_exit;
        if !is_exit && !analysis.post_dominates(merge, node) {
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
///
/// If `ctx` is provided, extracts case values from the Switch opcode.
pub fn find_switch_patterns(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    ctx: Option<&PatternContext<'_>>,
) -> Vec<SwitchPattern> {
    let mut patterns = Vec::new();

    for node in region_graph.node_indices() {
        if region_graph.get_node(node).map_or(true, |n| n.is_collapsed()) {
            continue;
        }

        if let Some(pattern) = match_switch_pattern(region_graph, cfg, analysis, node, ctx) {
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
    ctx: Option<&PatternContext<'_>>,
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
    let mut case_nodes_set = HashSet::new();
    let mut case_nodes = Vec::new();
    let mut body_nodes = HashSet::new();
    let mut default_node = None;

    // Check edge types to identify default vs cases
    // Note: Switch can have multiple edges to the same target (multiple case values),
    // so we deduplicate by using a HashSet
    for (target, kind) in cfg.successors_with_edges(cfg_node) {
        match kind {
            EdgeKind::FallThrough => {
                default_node = Some(target);
            }
            EdgeKind::ConditionalTrue | _ => {
                // Only add each target once
                if case_nodes_set.insert(target) {
                    case_nodes.push(target);
                }
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

    // Extract selector register and case values from the Switch opcode if ctx is available
    let (selector_reg, case_values) = if let Some(ctx) = ctx {
        extract_switch_info(cfg, cfg_node, region_graph, ctx)
    } else {
        (None, HashMap::new())
    };

    Some(SwitchPattern {
        selector_node: node,
        selector_cfg_block: cfg_node,
        case_nodes: case_region_nodes,
        default_node: default_node.and_then(|n| region_graph.get_region_node(n)),
        merge,
        body_nodes: body_region_nodes,
        selector_reg,
        case_values,
    })
}

/// Extract switch information from the Switch opcode in a basic block.
///
/// Returns (selector_reg, case_values_map) where case_values_map maps
/// region node indices to their corresponding case values.
fn extract_switch_info(
    cfg: &Cfg,
    cfg_node: NodeIndex,
    region_graph: &RegionGraph,
    ctx: &PatternContext<'_>,
) -> (Option<Reg>, HashMap<NodeIndex, Vec<i32>>) {
    let block = &cfg.graph[cfg_node];
    let ops = &ctx.func.ops;

    // Find the Switch opcode at the end of the block
    let switch_op = &ops[block.end];
    let (reg, offsets) = match switch_op {
        Opcode::Switch { reg, offsets, .. } => (*reg, offsets),
        _ => return (None, HashMap::new()),
    };

    // Build mapping from CFG node -> case values
    // offsets[i] = jump offset for case value i
    let mut cfg_to_values: HashMap<NodeIndex, Vec<i32>> = HashMap::new();
    let num_ops = ops.len();

    for (case_value, &offset) in offsets.iter().enumerate() {
        // Compute target address: block.end + offset + 1
        let target_addr = block.end as i64 + offset as i64 + 1;
        if target_addr < 0 || target_addr as usize >= num_ops {
            continue;
        }
        let target_addr = target_addr as usize;

        // Find the CFG node for this target
        if let Some(&target_cfg_node) = cfg.op_to_block.get(&target_addr) {
            cfg_to_values
                .entry(target_cfg_node)
                .or_default()
                .push(case_value as i32);
        }
    }

    // Convert CFG nodes to region graph indices
    let mut region_to_values: HashMap<NodeIndex, Vec<i32>> = HashMap::new();
    for (cfg_idx, values) in cfg_to_values {
        if let Some(region_idx) = region_graph.get_region_node(cfg_idx) {
            region_to_values.insert(region_idx, values);
        }
    }

    (Some(reg), region_to_values)
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
    ctx: Option<&PatternContext<'_>>,
) -> Option<Pattern> {
    // First try innermost loops
    let loop_patterns = find_loop_patterns(region_graph, cfg, analysis, ctx);
    if let Some(lp) = loop_patterns.into_iter().next() {
        return Some(Pattern::Loop(lp));
    }

    // Then try if patterns
    let if_patterns = find_if_patterns(region_graph, cfg, analysis);
    if let Some(ip) = if_patterns.into_iter().next() {
        return Some(Pattern::If(ip));
    }

    // Then try switch patterns
    let switch_patterns = find_switch_patterns(region_graph, cfg, analysis, ctx);
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
        let patterns = find_loop_patterns(&region_graph, &cfg, &analysis, None);

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
        let patterns = find_loop_patterns(&region_graph, &cfg, &analysis, None);

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
        let pattern = find_innermost_pattern(&region_graph, &cfg, &analysis, None);

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
        let analysis = CfgAnalysis::analyze(&cfg);
        let kind = detect_loop_kind(&cfg, &analysis, &natural_loop, None);

        println!("Detected loop kind: {:?}", kind);
    }

    // =========================================================================
    // Unit tests for collect_branch_nodes
    // =========================================================================

    #[test]
    fn test_collect_branch_nodes_simple_if_else() {
        // Structure: cond -> then, cond -> else, then -> merge, else -> merge
        // if (cond) { a } else { b }
        let ops = vec![
            // Block 0: condition
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 },
            // Block 1: then branch
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JAlways { offset: 1 },
            // Block 2: else branch
            Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
            // Block 3: merge + return
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);

        println!("CFG structure:");
        for node in cfg.graph.node_indices() {
            let block = &cfg.graph[node];
            println!(
                "  Block {:?}: ops {}..{}, is_exit={}",
                node, block.start, block.end, block.is_exit
            );
            println!("    succs: {:?}", cfg.successors(node));
        }

        // Find the condition block (should be the entry)
        let cond = cfg.entry;
        let succs = cfg.successors_with_edges(cond);
        assert_eq!(succs.len(), 2, "Condition should have 2 successors");

        // Find merge (ipdom of cond)
        let merge = analysis.ipdom(cond).expect("Should have a merge point");
        println!("Merge point: {:?}", merge);

        // Identify branches
        let (then_target, else_target, _) = identify_branches(&succs).expect("Should identify branches");
        println!("Then: {:?}, Else: {:?}", then_target, else_target);

        // Collect branch nodes
        let then_nodes = collect_branch_nodes(&cfg, &analysis, then_target, merge, cond);
        let else_nodes = collect_branch_nodes(&cfg, &analysis, else_target, merge, cond);

        println!("Then nodes: {:?}", then_nodes);
        println!("Else nodes: {:?}", else_nodes);

        // Verify branches don't overlap
        let overlap: HashSet<_> = then_nodes.intersection(&else_nodes).collect();
        assert!(overlap.is_empty(), "Then and else should not overlap: {:?}", overlap);

        // Verify merge is not in either branch
        assert!(!then_nodes.contains(&merge), "Merge should not be in then branch");
        assert!(!else_nodes.contains(&merge), "Merge should not be in else branch");

        // Verify condition is not in either branch
        assert!(!then_nodes.contains(&cond), "Condition should not be in then branch");
        assert!(!else_nodes.contains(&cond), "Condition should not be in else branch");
    }

    #[test]
    fn test_collect_branch_nodes_early_return() {
        // Structure: if (n >= 0) return n; return -n;
        // This tests the case where one branch terminates (returns)
        // and there is no merge point.
        let ops = vec![
            // Block 0: check n >= 0
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::Int { dst: Reg(1), ptr: RefInt(0) },  // 0 constant
            Opcode::JSLt { a: Reg(0), b: Reg(1), offset: 1 },  // if n < 0, skip return
            // Block 1: return n (early return)
            Opcode::Ret { ret: Reg(0) },
            // Block 2: return -n (continuation)
            Opcode::Neg { dst: Reg(2), src: Reg(0) },
            Opcode::Ret { ret: Reg(2) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);

        println!("CFG structure for early return:");
        for node in cfg.graph.node_indices() {
            let block = &cfg.graph[node];
            println!(
                "  Block {:?}: ops {}..{}, is_exit={}",
                node, block.start, block.end, block.is_exit
            );
            println!("    succs: {:?}", cfg.successors(node));
        }

        // Find the condition block
        let cond = cfg.entry;
        let succs = cfg.successors_with_edges(cond);

        // Should have 2 successors
        if succs.len() == 2 {
            // No merge point expected (early return pattern)
            let ipdom = analysis.ipdom(cond);
            println!("IPDOM of condition: {:?}", ipdom);

            // Check exit block detection
            for (target, _) in &succs {
                let is_exit = cfg.graph[*target].is_exit;
                println!("Block {:?} is_exit: {}", target, is_exit);
            }
        }
    }

    #[test]
    fn test_collect_branch_nodes_includes_exit_blocks() {
        // Test that exit blocks (blocks ending in Ret/Throw) are properly included
        // when they are part of a branch
        let ops = vec![
            // Block 0: condition
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 },
            // Block 1: then branch (with return)
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::Ret { ret: Reg(1) },
            // Block 2: else branch (fallthrough to merge)
            Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
            // Block 3: merge/return
            Opcode::Ret { ret: Reg(2) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);

        println!("CFG for exit block test:");
        for node in cfg.graph.node_indices() {
            let block = &cfg.graph[node];
            println!(
                "  Block {:?}: ops {}..{}, is_exit={}",
                node, block.start, block.end, block.is_exit
            );
        }

        let cond = cfg.entry;
        let succs = cfg.successors_with_edges(cond);

        if succs.len() == 2 {
            let (then_target, _else_target, _) = identify_branches(&succs).expect("Should identify branches");

            // Check if then block is an exit
            let then_is_exit = cfg.graph[then_target].is_exit;
            println!("Then block {:?} is_exit: {}", then_target, then_is_exit);

            // If there's a merge, collect nodes
            if let Some(merge) = analysis.ipdom(cond) {
                let then_nodes = collect_branch_nodes(&cfg, &analysis, then_target, merge, cond);
                println!("Then nodes (should include exit block): {:?}", then_nodes);

                // If then_target is an exit block, it should be included
                if then_is_exit {
                    assert!(
                        then_nodes.contains(&then_target),
                        "Exit block should be included in branch nodes"
                    );
                }
            }
        }
    }

    #[test]
    fn test_collect_branch_nodes_nested_if() {
        // Test nested if: if (a) { if (b) { X } }
        let ops = vec![
            // Block 0: outer if
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 4 },
            // Block 1: inner if
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JNull { reg: Reg(1), offset: 1 },
            // Block 2: inner body
            Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
            // Block 3: merge (both ifs converge here)
            Opcode::Ret { ret: Reg(0) },
        ];

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);

        println!("CFG for nested if:");
        for node in cfg.graph.node_indices() {
            let block = &cfg.graph[node];
            println!(
                "  Block {:?}: ops {}..{}, succs: {:?}",
                node, block.start, block.end, cfg.successors(node)
            );
        }

        // Find patterns
        let region_graph = RegionGraph::from_cfg(&cfg);
        let patterns = find_if_patterns(&region_graph, &cfg, &analysis);

        println!("Found {} if patterns in nested structure", patterns.len());
        for (i, p) in patterns.iter().enumerate() {
            println!(
                "  Pattern {}: cond={:?}, then={:?}, else={:?}, merge={:?}",
                i, p.condition_node, p.then_nodes, p.else_nodes, p.merge
            );
        }
    }

    #[test]
    fn test_if_pattern_then_else_no_overlap() {
        // Verify that for any matched if pattern, then and else branches don't overlap
        let test_cases = vec![
            // Simple if-else
            vec![
                Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
                Opcode::JNull { reg: Reg(0), offset: 2 },
                Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
                Opcode::JAlways { offset: 1 },
                Opcode::Int { dst: Reg(2), ptr: RefInt(2) },
                Opcode::Ret { ret: Reg(0) },
            ],
            // If without else
            vec![
                Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
                Opcode::JNull { reg: Reg(0), offset: 1 },
                Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
                Opcode::Ret { ret: Reg(0) },
            ],
        ];

        for (i, ops) in test_cases.iter().enumerate() {
            let (cfg, analysis, region_graph) = build_test_env(ops);
            let patterns = find_if_patterns(&region_graph, &cfg, &analysis);

            for p in &patterns {
                let overlap: HashSet<_> = p.then_nodes.intersection(&p.else_nodes).collect();
                assert!(
                    overlap.is_empty(),
                    "Test case {}: then and else overlap at {:?}",
                    i, overlap
                );
            }
        }
    }
}
