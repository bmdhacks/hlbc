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
use crate::exception_analysis::ExceptionAnalysis;
use crate::lifter::{Cfg, EdgeKind};
use crate::ssa::{SsaCfg, SsaInstr, SsaVar};
use crate::structurer::region::LoopKind;
use crate::structurer::region_dominance::RegionDominators;
use crate::structurer::region_graph::{RegionGraph, RegionNode};
use crate::type_prop::TypeInfo;

/// Pattern matcher that uses RegionGraph as the single source of truth.
///
/// This struct provides methods for detecting control flow patterns while
/// respecting collapsed regions as atomic, opaque units.
///
/// Key design principle: After a collapse, the RegionGraph is the source of
/// truth for graph structure. Pattern matchers traverse the RegionGraph,
/// not the original CFG, and use region-level dominance.
pub struct PatternMatcher<'a> {
    /// The current state of the region graph
    region_graph: &'a RegionGraph,
    /// The original CFG (for opcode lookups only, NOT for traversal)
    cfg: &'a Cfg,
    /// Original CFG analysis (for loop info only)
    #[allow(dead_code)]
    analysis: &'a CfgAnalysis,
    /// Optional SSA/type context for advanced pattern detection
    #[allow(dead_code)]
    ctx: Option<&'a PatternContext<'a>>,
    /// Cached dominance computed on RegionGraph
    region_dominators: Option<RegionDominators>,
}

impl<'a> PatternMatcher<'a> {
    /// Create a new pattern matcher.
    pub fn new(
        region_graph: &'a RegionGraph,
        cfg: &'a Cfg,
        analysis: &'a CfgAnalysis,
        ctx: Option<&'a PatternContext<'a>>,
    ) -> Self {
        Self {
            region_graph,
            cfg,
            analysis,
            ctx,
            region_dominators: None,
        }
    }

    /// Ensure region dominance is computed and not stale.
    fn ensure_dominance(&mut self) {
        let needs_compute = self.region_dominators.as_ref()
            .map_or(true, |d| d.is_stale(self.region_graph));
        if needs_compute {
            self.region_dominators = Some(RegionDominators::compute(self.region_graph));
        }
    }

    /// Get the region dominators, computing if needed.
    pub fn dominators(&mut self) -> &RegionDominators {
        self.ensure_dominance();
        self.region_dominators.as_ref().unwrap()
    }

    /// Collect nodes in a branch using RegionGraph traversal.
    ///
    /// This is the core fix for the pattern matching bug. Instead of traversing
    /// the original CFG, we traverse the RegionGraph directly:
    /// - Collapsed regions are atomic (we don't traverse INTO them)
    /// - We use region-level dominance, not CFG dominance
    /// - The membership firewall prevents traversing to removed nodes
    ///
    /// # Arguments
    /// * `start` - The RegionGraph node where the branch starts
    /// * `merge` - The merge point (stop traversal here)
    /// * `condition` - The condition node (don't traverse back to it)
    pub fn collect_branch_region(
        &mut self,
        start: NodeIndex,
        merge: NodeIndex,
        condition: NodeIndex,
    ) -> HashSet<NodeIndex> {
        self.ensure_dominance();
        let dominators = self.region_dominators.as_ref().unwrap();

        // Collect loop headers and back-edge sources as barriers.
        // This prevents the branch collector from crossing back-edges and
        // absorbing loop body nodes from the next iteration.
        // - Loop headers: don't enter the header from a branch
        // - Back-edge sources: nodes whose only purpose is jumping to the header
        let loop_header_cfgs: HashSet<NodeIndex> = self.analysis.loops.iter()
            .map(|l| l.header)
            .collect();
        let back_edge_cfgs: HashSet<NodeIndex> = self.analysis.loops.iter()
            .flat_map(|l| l.back_edge_sources.iter().copied())
            .collect();

        let mut nodes = HashSet::new();
        let mut worklist = vec![start];
        let mut visited = HashSet::new();

        while let Some(node) = worklist.pop() {
            // MEMBERSHIP FIREWALL: check node exists in current graph
            if !self.region_graph.contains(node) {
                continue;
            }

            if visited.contains(&node) || node == merge || node == condition {
                continue;
            }

            // Don't traverse into loop headers or back-edge sources (unless
            // it's the start node). These are loop structural nodes — including
            // them in a branch would absorb loop structure before loop detection.
            if node != start {
                if let Some(cfg_node) = self.region_graph.get_node(node).and_then(|n| n.as_block()) {
                    if loop_header_cfgs.contains(&cfg_node) || back_edge_cfgs.contains(&cfg_node) {
                        continue;
                    }
                }
            }

            visited.insert(node);

            // Check dominance using region-level dominators
            // The condition must dominate this node for it to be part of the branch
            if !dominators.dominates(condition, node) {
                continue;
            }

            // For non-terminating blocks, check post-dominance by merge.
            // Terminating nodes (exits) don't need this check.
            let is_terminating = self.region_graph.get_node(node)
                .map_or(false, |n| n.terminates(self.cfg));
            let is_dummy_merge = merge == condition;
            let jumps_to_merge = self.region_graph.successors(node).contains(&merge);

            if !is_terminating && !is_dummy_merge && !jumps_to_merge
                && !dominators.post_dominates(merge, node)
            {
                continue;
            }

            nodes.insert(node);

            // KEY CHANGE: Traverse REGION GRAPH successors (not CFG!)
            // This respects collapsed regions as atomic units
            for succ in self.region_graph.successors(node) {
                if !visited.contains(&succ) {
                    worklist.push(succ);
                }
            }
        }

        nodes
    }

    /// Match an if pattern at a given node using RegionGraph traversal.
    ///
    /// This is the region-aware version of `match_if_pattern`.
    pub fn match_if_pattern_region(&mut self, node: NodeIndex) -> Option<IfPattern> {
        let region_node = self.region_graph.get_node(node)?;
        let is_collapsed = region_node.is_collapsed();

        // Get then/else targets and negation flag
        let (then_target, else_target, negated, then_cfg_target_opt) = if !is_collapsed {
            // Block node: use CFG edges for branch identification
            let cfg_node = region_node.as_block()?;
            let cfg_succs = self.cfg.successors_with_edges(cfg_node);
            if cfg_succs.len() != 2 {
                return None;
            }
            let (then_cfg_target, else_cfg_target, negated) = identify_branches(&cfg_succs)?;
            let then_target = self.region_graph.cfg_owner(then_cfg_target)?;
            let else_target = self.region_graph.cfg_owner(else_cfg_target)?;
            (then_target, else_target, negated, Some(then_cfg_target))
        } else {
            // Collapsed node: use region graph edges for branch identification
            let region_succs = self.region_graph.successors_with_edges(node);
            if region_succs.len() != 2 {
                return None;
            }
            // Try standard branch identification first. If edge kinds are missing or
            // duplicated (e.g., both ConditionalTrue after multiple collapses), fall back
            // to using successors directly with first=then, second=else.
            match identify_branches(&region_succs) {
                Some((then_target, else_target, negated)) => {
                    (then_target, else_target, negated, None)
                }
                None => {
                    // Fallback: treat the two successors as then/else
                    let (target_a, _) = region_succs[0];
                    let (target_b, _) = region_succs[1];
                    (target_a, target_b, false, None)
                }
            }
        };

        // Check termination using RegionGraph nodes
        let then_terminates = self.region_graph.get_node(then_target)
            .map_or(false, |n| n.terminates(self.cfg));
        let else_terminates = self.region_graph.get_node(else_target)
            .map_or(false, |n| n.terminates(self.cfg));

        // Find merge point using REGION-LEVEL post-dominance
        self.ensure_dominance();
        let dominators = self.region_dominators.as_ref().unwrap();
        let real_ipdom = dominators.ipdom(node);

        let (merge, is_one_branch_early_return) = match real_ipdom {
            Some(m) => (m, false),
            None => {
                // No post-dominator - check for early-return pattern
                if then_terminates && !else_terminates {
                    (else_target, true)
                } else if else_terminates && !then_terminates {
                    (then_target, true)
                } else if then_terminates && else_terminates {
                    // Both branches terminate - use then_target as dummy merge
                    (then_target, false)
                } else {
                    // Neither terminates and no post-dominator
                    (then_target, false)
                }
            }
        };

        // Don't match if merge is one of the direct successors (trivial case)
        if then_target == merge && else_target == merge {
            return None;
        }

        // Collect nodes in each branch using REGION GRAPH TRAVERSAL
        let then_nodes = self.collect_branch_region(then_target, merge, node);
        let else_nodes = self.collect_branch_region(else_target, merge, node);

        // Handle collapsed targets
        let mut then_region_nodes = then_nodes.clone();
        let mut else_region_nodes = else_nodes.clone();

        // Handle short-circuit && patterns: if both branches can reach the same nodes
        // (e.g., `if (a && b) {} else { body }` where failing either condition goes to body),
        // the branches may overlap. In this case, assign overlapping nodes to only one branch.
        // Rule: nodes reachable from BOTH branches belong to whichever branch reaches them
        // more directly. Since then_target is the direct jump target, give priority to then_nodes.
        for node in then_region_nodes.iter() {
            else_region_nodes.remove(node);
        }
        // Also exclude the then_target from else_nodes (it's the then branch's entry point)
        else_region_nodes.remove(&then_target);

        // If a branch is empty but the target is a collapsed region, include it
        if then_region_nodes.is_empty() {
            if then_target != merge && self.region_graph.get_node(then_target)
                .map_or(false, |n| n.is_collapsed())
            {
                then_region_nodes.insert(then_target);
            }
        }
        if else_region_nodes.is_empty() {
            if else_target != merge && self.region_graph.get_node(else_target)
                .map_or(false, |n| n.is_collapsed())
            {
                else_region_nodes.insert(else_target);
            }
        }

        // Filter out merge node from branches
        then_region_nodes.remove(&merge);
        else_region_nodes.remove(&merge);

        // Reject patterns with cross-branch edges: a node in one branch has a
        // successor in the other branch. This indicates an OR/AND chain pattern
        // that should be handled by the OR chain detector instead.
        //
        // Exception: allow cross-branch edges when the target is a terminating
        // region (throw/return). This handles inlined AND chains like
        // `if (x != b.x) { handler } if (y != b.y) { handler }` where both
        // condition failures jump to the same terminating handler.
        for &else_node in &else_region_nodes {
            for succ in self.region_graph.successors(else_node) {
                if then_region_nodes.contains(&succ) {
                    let succ_terminates = self.region_graph.get_node(succ)
                        .map_or(false, |n| n.terminates(self.cfg));
                    if !succ_terminates {
                        return None;
                    }
                }
            }
        }
        for &then_node in &then_region_nodes {
            for succ in self.region_graph.successors(then_node) {
                if else_region_nodes.contains(&succ) {
                    let succ_terminates = self.region_graph.get_node(succ)
                        .map_or(false, |n| n.terminates(self.cfg));
                    if !succ_terminates {
                        return None;
                    }
                }
            }
        }

        let _both_branches_terminate = then_terminates && else_terminates && real_ipdom.is_none();

        // For early-return patterns
        let then_exit_target = if then_region_nodes.is_empty()
            && then_terminates
            && is_one_branch_early_return
        {
            then_cfg_target_opt
        } else {
            None
        };

        Some(IfPattern {
            condition_node: node,
            then_nodes: then_region_nodes,
            else_nodes: else_region_nodes,
            merge,
            negated,
            then_exit_target,
        })
    }
}

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

    /// For early-return patterns: the exit node that the then branch jumps to.
    /// This is set when then_nodes is empty but then_target is an exit block
    /// (not dominated by condition due to being a shared exit).
    /// The collapse logic can use this to generate: if (cond) { goto exit; }
    pub then_exit_target: Option<NodeIndex>,
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

/// A detected try-catch pattern ready for collapse.
#[derive(Debug, Clone)]
pub struct TryCatchPattern {
    /// The CFG node containing the Trap opcode (start of try).
    pub trap_node: NodeIndex,

    /// Nodes in the try body (between Trap and EndTrap, excluding handler).
    pub try_nodes: HashSet<NodeIndex>,

    /// The CFG node that is the handler entry (catch start).
    pub handler_node: NodeIndex,

    /// Nodes in the catch body.
    pub catch_nodes: HashSet<NodeIndex>,

    /// The exception register from Trap opcode.
    pub exc_reg: Reg,

    /// Merge point (node after try-catch completes), if any.
    pub merge: Option<NodeIndex>,
}

/// A detected OR chain pattern ready for collapse.
///
/// OR chains occur when multiple consecutive condition blocks all jump to the same
/// "true" target while chaining their "false" targets. For example:
/// `if (a || b || c || d) throw X;` compiles to:
/// ```text
/// Block 0: if a jump to THROW else Block 1
/// Block 1: if b jump to THROW else Block 2
/// Block 2: if c jump to THROW else Block 3
/// Block 3: if !d jump to CONTINUE else THROW  (last condition is inverted)
/// THROW: throw X
/// CONTINUE: ...
/// ```
///
/// Nested AND chains within OR chains are also supported. For example:
/// `if (a || (b && c) || d) throw X;` compiles to:
/// ```text
/// Block 0: if a jump to THROW else Block 2 (OR)
/// Block 2: if b <= 0 jump to Block 6 else Block 4 (AND start - skips to next OR on false)
/// Block 4: if c > 0 jump to THROW else Block 6 (AND end - inherits outer jcond=true)
/// Block 6: if d >= 0 jump to CONTINUE else THROW (inverted last OR)
/// ```
///
/// Note: AND chain patterns like `if (a && b && c) work;` have a similar CFG structure
/// but with the continuation also flowing to the shared target. These are NOT matched
/// as OR chains (we return None in pattern detection) and are instead handled by the
/// normal if-pattern detector, which produces correct nested if-else structures.
#[derive(Debug, Clone)]
pub struct OrChainPattern {
    /// All condition nodes in the chain (in order).
    /// For nested AND chains, this includes the first block of each AND sub-chain
    /// (the one that starts the AND).
    pub condition_nodes: Vec<NodeIndex>,

    /// The shared "true" target (e.g., the throw block or body start).
    pub shared_target: NodeIndex,

    /// All CFG nodes that are part of the body (between shared_target and continuation).
    /// This is empty for terminating bodies (throw/return) but populated for
    /// non-terminating bodies that may contain nested control flow.
    pub body_cfg_nodes: Vec<NodeIndex>,

    /// The continuation after the OR chain (the "false" path of the last condition).
    pub continuation: NodeIndex,

    /// Whether the shared target terminates (throw/return).
    pub shared_target_terminates: bool,

    /// Whether the last condition is inverted (jumps to continuation on true).
    /// When true, the last condition should be negated in the compound OR.
    pub last_condition_inverted: bool,

    /// Nested AND chains within the OR chain.
    /// Maps OR condition index to the CFG nodes in that AND sub-chain.
    /// The first node in each Vec is the AND-start block (which is also in condition_nodes),
    /// followed by the remaining AND blocks up to and including the AND-end block.
    pub nested_and_chains: HashMap<usize, Vec<NodeIndex>>,
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
///
/// For while loops, the primary exit is typically via the header's condition.
/// We check the header first to prefer this exit over break-induced exits.
fn find_loop_exit(cfg: &Cfg, natural_loop: &NaturalLoop) -> Option<NodeIndex> {
    // Check header first for exit edges (this is the primary exit for while loops)
    // The header's exit edge represents the normal loop termination condition.
    for succ in cfg.successors(natural_loop.header) {
        if !natural_loop.body.contains(&succ) {
            return Some(succ);
        }
    }

    // Also check other exit nodes (for loops with multiple exits like break)
    for &exit_node in &natural_loop.exit_nodes {
        if exit_node == natural_loop.header {
            continue; // Already checked
        }
        for succ in cfg.successors(exit_node) {
            if !natural_loop.body.contains(&succ) {
                return Some(succ);
            }
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
            let name = ctx.code.get(*fun).name(ctx.code);
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
            let name = ctx.code.get(*fun).name(ctx.code);
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
///
/// This function now uses `PatternMatcher` internally which provides:
/// - RegionGraph-based traversal (not original CFG)
/// - Region-level dominance (recomputed after collapses)
/// - Membership firewall to prevent stale node access
pub fn find_if_patterns(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
) -> Vec<IfPattern> {
    let mut matcher = PatternMatcher::new(region_graph, cfg, analysis, None);
    let mut patterns = Vec::new();

    // Process nodes in reverse post-order (visits outer nodes first)
    let nodes = region_graph.nodes_in_reverse_postorder();

    for node in nodes {
        // Use the new region-aware pattern matching (handles both blocks and collapsed nodes)
        if let Some(pattern) = matcher.match_if_pattern_region(node) {
            patterns.push(pattern);
        }
    }

    // Reverse so innermost patterns come first - this ensures nested if-else-if
    // chains are collapsed from the inside out
    patterns.reverse();
    patterns
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

/// Find OR chain patterns in the graph.
///
/// OR chains occur when multiple consecutive condition blocks all share the same
/// "true" target (e.g., a throw block) while chaining their "false" targets.
///
/// Pattern: `if (a || b || c) throw X;` compiles to:
/// ```text
/// Block 0: if a jump to THROW else Block 1
/// Block 1: if b jump to THROW else Block 2
/// Block 2: if c jump to THROW else CONTINUE
/// THROW: throw X
/// CONTINUE: ...
/// ```
///
/// Returns patterns with the most condition nodes first (longest chains).
pub fn find_or_chain_patterns(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    ctx: Option<&PatternContext<'_>>,
) -> Vec<OrChainPattern> {
    let mut patterns = Vec::new();
    let mut used_nodes: HashSet<NodeIndex> = HashSet::new();

    // Iterate through nodes looking for potential chain starts
    for start_node in region_graph.node_indices() {
        // Skip already-collapsed or already-used nodes
        if region_graph.get_node(start_node).map_or(true, |n| n.is_collapsed()) {
            continue;
        }
        if used_nodes.contains(&start_node) {
            continue;
        }

        // Try to build an OR chain starting from this node
        if let Some(pattern) = try_build_or_chain(region_graph, cfg, analysis, start_node, &used_nodes, ctx) {
            // Only accept chains with 2+ conditions (otherwise regular if pattern handles it)
            if pattern.condition_nodes.len() >= 2 {
                // Mark all condition nodes as used
                for &node in &pattern.condition_nodes {
                    used_nodes.insert(node);
                }
                patterns.push(pattern);
            }
        }
    }

    // Sort by chain length (longest first) for priority
    patterns.sort_by(|a, b| b.condition_nodes.len().cmp(&a.condition_nodes.len()));
    patterns
}

/// Try to detect a nested AND sub-chain within an OR chain.
///
/// When building an OR chain, if we encounter a block whose `true` target is NOT
/// the shared target, it might be the start of a nested AND sub-chain.
///
/// For `if (a || (b && c) || d) throw`:
/// - Block for condition `b`: true -> next_OR_block (skip on b<=0), false -> block_c
/// - Block for condition `c`: true -> shared_target (inherited jcond), false -> next_OR_block
///
/// Returns (and_chain_cfg_nodes, rejoin_cfg_node) if a nested AND is detected.
fn try_detect_nested_and(
    cfg: &Cfg,
    start_cfg_node: NodeIndex,
    shared_target_cfg: NodeIndex,
    skip_target_cfg: NodeIndex,
) -> Option<(Vec<NodeIndex>, NodeIndex)> {
    // The start block's true target should be the skip target (next OR condition)
    // and its false target should lead into the AND chain
    let start_succs = cfg.successors_with_edges(start_cfg_node);
    if start_succs.len() != 2 {
        return None;
    }

    let (true_target, false_target, _) = identify_branches(&start_succs)?;

    // Verify the structure: true -> skip_target, false -> AND continuation
    if true_target != skip_target_cfg {
        return None;
    }

    // Start collecting AND chain nodes, beginning with the start block
    let mut and_chain = vec![start_cfg_node];
    let mut current_cfg = false_target;

    // Follow the false path to collect AND chain blocks
    // The AND chain ends when we find a block whose true target is shared_target
    loop {
        let succs = cfg.successors_with_edges(current_cfg);
        if succs.len() != 2 {
            // Not a conditional - AND chain broken
            return None;
        }

        let (next_true, next_false, _) = identify_branches(&succs)?;

        if next_true == shared_target_cfg {
            // This is the AND-end block - it inherits the outer jcond=true
            // Its condition jumps to shared_target when true
            and_chain.push(current_cfg);
            // The false path should go to skip_target (rejoin the OR chain)
            if next_false == skip_target_cfg {
                return Some((and_chain, skip_target_cfg));
            }
            // Otherwise the AND chain doesn't properly rejoin the OR chain
            return None;
        }

        // Check if this block continues the AND chain
        // AND blocks jump to skip_target on true (condition false -> skip)
        if next_true == skip_target_cfg {
            // This is another AND block in the chain
            and_chain.push(current_cfg);
            current_cfg = next_false;
        } else {
            // This block doesn't fit the AND pattern
            return None;
        }

        // Safety limit to prevent infinite loops
        if and_chain.len() > 100 {
            return None;
        }
    }
}

/// Try to build an OR chain starting from a given node.
fn try_build_or_chain(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    analysis: &CfgAnalysis,
    start_node: NodeIndex,
    used_nodes: &HashSet<NodeIndex>,
    _ctx: Option<&PatternContext<'_>>,
) -> Option<OrChainPattern> {
    // Get the CFG node
    let start_cfg_node = region_graph.get_node(start_node)?.as_block()?;

    // Must have exactly 2 successors (conditional)
    let succs = cfg.successors_with_edges(start_cfg_node);
    if succs.len() != 2 {
        return None;
    }

    // Identify true/false branches
    let (true_target, false_target, _) = identify_branches(&succs)?;

    // The true_target is our shared target candidate
    let shared_target_cfg = true_target;

    // Check if shared target terminates (throw/return) - this is typical for OR chains
    let shared_target_terminates = cfg.graph.node_weight(shared_target_cfg)
        .map(|block| block.is_exit)
        .unwrap_or(false);

    // Build the chain: follow false_targets as long as they share the same true_target
    let mut condition_nodes = vec![start_node];
    let mut current_false_target = false_target;
    let mut last_condition_inverted = false;
    let mut nested_and_chains: HashMap<usize, Vec<NodeIndex>> = HashMap::new();

    loop {
        // Get the region node for the current false target
        let current_region_node = region_graph.get_region_node(current_false_target)?;

        // Skip if already used or collapsed
        if used_nodes.contains(&current_region_node) {
            break;
        }
        if region_graph.get_node(current_region_node).map_or(true, |n| n.is_collapsed()) {
            break;
        }

        // Check if this node continues the OR chain
        let next_succs = cfg.successors_with_edges(current_false_target);
        if next_succs.len() != 2 {
            // Not a conditional - this is our continuation point
            break;
        }

        let (next_true_target, next_false_target, _) = match identify_branches(&next_succs) {
            Some(b) => b,
            None => break,
        };

        // Check if this continues the chain (same shared target)
        if next_true_target == shared_target_cfg {
            // Same shared target - extend the chain
            condition_nodes.push(current_region_node);
            current_false_target = next_false_target;
            last_condition_inverted = false;
        } else {
            // Different true target - could be:
            // 1. Inverted last condition: false -> shared_target
            // 2. Nested AND chain: true -> later_OR_block, false -> AND_continuation
            // 3. End of OR chain

            if next_false_target == shared_target_cfg {
                // Case 1: The last condition is inverted
                // e.g., `if (d >= 0) jump to CONTINUE else THROW`
                condition_nodes.push(current_region_node);
                current_false_target = next_true_target; // Continuation is true target
                last_condition_inverted = true;
                break;
            }

            // Case 2: Check for nested AND chain
            // The true target should be a later OR condition block (eventually reaches shared_target or is inverted)
            // Try to detect if there's a valid AND chain starting at current_false_target
            if let Some((and_chain_cfg, rejoin_cfg)) = try_detect_nested_and(
                cfg,
                current_false_target,
                shared_target_cfg,
                next_true_target,
            ) {
                // Found a nested AND chain!
                // Store the AND chain for this condition index
                let condition_idx = condition_nodes.len();
                nested_and_chains.insert(condition_idx, and_chain_cfg.clone());

                // Add the first block of the AND chain as the condition node
                condition_nodes.push(current_region_node);

                // Continue OR chain from the rejoin point
                current_false_target = rejoin_cfg;
                last_condition_inverted = false;
                continue;
            }

            // Case 3: End of OR chain
            break;
        }
    }

    // Need at least 2 conditions for an OR chain
    if condition_nodes.len() < 2 {
        return None;
    }

    // Reject OR chains where shared_target is a loop back-edge source.
    // This prevents absorbing loop structural nodes (continue blocks) into
    // OR chain collapses, which would destroy the loop structure and prevent
    // loop detection at a later priority.
    let back_edge_sources: HashSet<NodeIndex> = analysis.loops.iter()
        .flat_map(|l| l.back_edge_sources.iter().copied())
        .collect();
    if back_edge_sources.contains(&shared_target_cfg) {
        return None;
    }

    // Get region node for shared target
    let shared_target = region_graph.get_region_node(shared_target_cfg)?;

    // Get region node for continuation
    let continuation = region_graph.get_region_node(current_false_target)?;

    // CRITICAL: Reject OR chains where the shared target has predecessors from outside
    // the chain. This prevents the OR chain from "stealing" a shared exit node (like a
    // return block) that other control flow paths also need to reach.
    //
    // Example: `if (a.len < na) a = new(na); if (nc > 0 && (c == null || c.len < nc)) c = new(nc);`
    // The return block is shared by all if-then blocks and the OR chain's allocation body.
    // If the OR chain absorbs the return block, the other if blocks lose their exit path.
    {
        // Collect all CFG nodes that are part of the OR chain (conditions + nested ANDs)
        let mut chain_cfg_nodes: HashSet<NodeIndex> = HashSet::new();
        for &cond_node in &condition_nodes {
            if let Some(cfg_node) = region_graph.get_node(cond_node).and_then(|n| n.as_block()) {
                chain_cfg_nodes.insert(cfg_node);
            }
        }
        for and_chain in nested_and_chains.values() {
            for &cfg_node in and_chain {
                chain_cfg_nodes.insert(cfg_node);
            }
        }

        // Check if shared_target has any CFG predecessors from outside the chain
        let shared_preds = cfg.predecessors(shared_target_cfg);
        let has_external_preds = shared_preds.iter().any(|pred| !chain_cfg_nodes.contains(pred));
        if has_external_preds {
            return None;
        }
    }

    // CRITICAL: Distinguish OR chains from AND chains using reachability analysis.
    //
    // For OR chains `if (a || b) { body; }`:
    //   - Conditions jump to body (shared_target) on success
    //   - Body flows to continuation after executing
    //   - shared_target CAN reach continuation
    //
    // For AND chains `if (a && b && c) return X;`:
    //   - Conditions jump to skip (shared_target) on failure
    //   - Body is on the fall-through path, not shared_target
    //   - shared_target is the NEXT chain or continuation, doesn't flow "through" body
    //   - In terminating bodies, shared_target is disjoint from continuation path
    //
    // Check 1: If shared_target terminates, it's a valid OR chain (throw/return body)
    // Check 2: If shared_target can reach continuation, it's likely an OR chain (body → continuation)
    // Check 3: If continuation flows TO shared_target, it's an AND chain (reject)

    // Check 3: Continuation flowing to shared_target indicates AND chain
    let continuation_flows_to_shared = cfg
        .successors(current_false_target)
        .contains(&shared_target_cfg);

    if continuation_flows_to_shared {
        // Skip AND chain patterns - let the if-pattern detector handle them
        return None;
    }

    // Check 2: For non-terminating shared targets, distinguish OR chains from AND chains.
    //
    // OR if-else: `if (a || b) { BODY } else { CONT }; MERGE`
    //   - shared_target (BODY) and continuation (CONT) are siblings that merge later
    //   - Neither reaches the other; both reach a common merge point
    //
    // OR linear: `if (a || b) { BODY }; CONT`
    //   - shared_target (BODY) flows to continuation (CONT) directly
    //
    // AND chain (misidentified): `if (!a || !b) SKIP; BODY; SKIP`
    //   - continuation (BODY) reaches shared_target (SKIP)
    //
    // We accept if: shared_target reaches continuation (linear OR),
    // OR they share a common merge point (if-else OR).
    // We reject if: continuation reaches shared_target (AND chain).
    // Track whether shared_target can reach continuation (linear OR vs if-else OR).
    // This is used both for validation and for deciding body collection strategy.
    let mut can_reach_continuation = false;

    if !shared_target_terminates {
        // BFS from shared_target
        let mut shared_reachable = HashSet::new();
        let mut queue = vec![shared_target_cfg];
        shared_reachable.insert(shared_target_cfg);

        // Don't traverse through condition nodes (they're part of the chain, not body)
        for &cond_node in &condition_nodes {
            if let Some(cfg_node) = region_graph.get_node(cond_node).and_then(|n| n.as_block()) {
                shared_reachable.insert(cfg_node);
            }
        }

        while let Some(node) = queue.pop() {
            for succ in cfg.successors(node) {
                if shared_reachable.insert(succ) {
                    queue.push(succ);
                }
            }
        }

        can_reach_continuation = shared_reachable.contains(&current_false_target);

        if !can_reach_continuation {
            // Check if continuation reaches shared_target (AND chain indicator)
            let mut cont_reachable = HashSet::new();
            let mut queue = vec![current_false_target];
            cont_reachable.insert(current_false_target);

            while let Some(node) = queue.pop() {
                for succ in cfg.successors(node) {
                    if cont_reachable.insert(succ) {
                        queue.push(succ);
                    }
                }
            }

            if cont_reachable.contains(&shared_target_cfg) {
                // Continuation reaches shared_target — this is an AND chain, not OR
                return None;
            }

            // Check if they share a common merge point (OR if-else pattern)
            let has_common_merge = shared_reachable.intersection(&cont_reachable).next().is_some();
            if !has_common_merge {
                return None;
            }
        }

        // Check if shared_target (body start) has internal control flow that isn't reduced yet.
        // If the body start is a conditional, let inner patterns be reduced first.
        // Use the RegionGraph (not CFG) to check — collapsed regions are atomic.
        if let Some(region_node) = region_graph.get_region_node(shared_target_cfg) {
            let region_succs = region_graph.successors(region_node);
            if region_succs.len() > 1 {
                if let Some(node) = region_graph.get_node(region_node) {
                    if !node.is_collapsed() {
                        // Body has unreduced internal control flow - let it be reduced first
                        return None;
                    }
                }
            }
        }
    }

    // Collect body CFG nodes between shared_target and continuation.
    //
    // Three cases:
    // 1. Terminating body (throw/return): body_cfg_nodes is empty, shared_target
    //    is added to collapse set directly.
    // 2. Linear OR (shared_target flows to continuation): collect body nodes via BFS.
    // 3. If-else OR (shared_target and continuation are sibling branches):
    //    body_cfg_nodes is empty — the collapse only includes conditions, and
    //    collapse_or_chain handles the if-else structure specially.
    //
    // We distinguish cases 2 and 3 by checking can_reach_continuation (computed above).
    let mut body_cfg_nodes = Vec::new();
    if !shared_target_terminates && can_reach_continuation {
        // Case 2: Linear OR — collect body nodes between shared_target and continuation
        let mut visited = HashSet::new();
        let mut queue = vec![shared_target_cfg];
        visited.insert(shared_target_cfg);

        // Mark continuation and condition nodes as visited
        visited.insert(current_false_target);
        for &cond_node in &condition_nodes {
            if let Some(cfg_node) = region_graph.get_node(cond_node).and_then(|n| n.as_block()) {
                visited.insert(cfg_node);
            }
        }

        while let Some(node) = queue.pop() {
            body_cfg_nodes.push(node);
            for succ in cfg.successors(node) {
                if !visited.contains(&succ) {
                    visited.insert(succ);
                    queue.push(succ);
                }
            }
        }
    }
    // Cases 1 and 3: body_cfg_nodes stays empty

    Some(OrChainPattern {
        condition_nodes,
        shared_target,
        body_cfg_nodes,
        continuation,
        shared_target_terminates,
        last_condition_inverted,
        nested_and_chains,
    })
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
    let region_node = region_graph.get_node(node)?;
    let cfg_node = region_node.as_block()?;

    // Must have 3+ successors for a switch (excluding exception edges)
    // Exception edges would make a try { if/else } look like a switch
    let cfg_succs = cfg.successors_no_exceptions(cfg_node);

    if cfg_succs.len() < 3 {
        return None;
    }

    // Check if the block ends with a Switch opcode
    let block = &cfg.graph[cfg_node];
    let is_switch_block = ctx
        .map(|c| matches!(c.func.ops.get(block.end), Some(hlbc::opcodes::Opcode::Switch { .. })))
        .unwrap_or(false);

    // Find merge point
    // For switches where all cases terminate (return), there's no merge point.
    // In that case, we still want to match the pattern.
    let merge_cfg = analysis.ipdom(cfg_node);


    // If no merge point, try to find a common exit target among case successors.
    // This handles switches where some cases throw (no strict ipdom) but others
    // converge to a common point (like a return block).
    let merge_cfg = match merge_cfg {
        Some(m) => m,
        None if is_switch_block => {
            // Try to find a common exit target by looking at where the case bodies go.
            // We do a simple BFS from each case target and count which exit blocks
            // are reachable, then pick the most common one.
            let cfg_succs = cfg.successors_no_exceptions(cfg_node);
            let mut exit_counts: HashMap<NodeIndex, usize> = HashMap::new();

            for &case_target in &cfg_succs {
                // Do a BFS to find reachable exit/terminal blocks
                let mut visited: HashSet<NodeIndex> = HashSet::new();
                let mut queue = vec![case_target];
                visited.insert(case_target);

                while let Some(node) = queue.pop() {
                    let block = &cfg.graph[node];
                    // Check if this is an exit or terminal block
                    if block.is_exit || cfg.successors(node).is_empty() {
                        // This case reaches this exit
                        *exit_counts.entry(node).or_insert(0) += 1;
                        // Don't continue from exit blocks
                        continue;
                    }

                    // Continue BFS to successors
                    for succ in cfg.successors(node) {
                        if visited.insert(succ) {
                            queue.push(succ);
                        }
                    }
                }
            }

            // Find the exit block reached by the most cases
            if let Some((&common_exit, &count)) = exit_counts.iter().max_by_key(|(_, c)| *c) {
                // If at least 2 cases reach this exit, use it as merge
                // (or all cases if there's only one exit)
                if count >= 2 || (exit_counts.len() == 1 && count >= 1) {
                    common_exit
                } else {
                    // Fall back to selector as dummy merge
                    cfg_node
                }
            } else {
                // No exits found, use selector as dummy merge
                cfg_node
            }
        }
        None => return None,
    };

    // Collect case nodes and body
    let mut case_nodes_set = HashSet::new();
    let mut case_nodes = Vec::new();
    let mut default_node = None;
    let mut fallthrough_target: Option<NodeIndex> = None;

    // Check edge types to identify default vs cases
    // Note: Switch can have multiple edges to the same target (multiple case values),
    // so we deduplicate by using a HashSet.
    //
    // IMPORTANT: For Switch opcodes with an offset of 0, the case target is the
    // same as the fall-through address. We need to:
    // 1. Check if FallThrough target matches any ConditionalTrue target
    // 2. If so, that ConditionalTrue case IS the default (handles unmatched values)
    let mut conditional_targets: HashSet<NodeIndex> = HashSet::new();

    // First pass: collect all ConditionalTrue targets and find FallThrough
    for (target, kind) in cfg.successors_with_edges(cfg_node) {
        match kind {
            EdgeKind::ConditionalTrue => {
                conditional_targets.insert(target);
            }
            EdgeKind::FallThrough => {
                fallthrough_target = Some(target);
            }
            _ => {}
        }
    }

    // Second pass: build case list and identify default
    for (target, kind) in cfg.successors_with_edges(cfg_node) {
        match kind {
            EdgeKind::FallThrough => {
                // Only treat as explicit default if not already a case target
                if !conditional_targets.contains(&target) {
                    default_node = Some(target);
                }
                // If FallThrough matches a ConditionalTrue, that case serves as default
                // We'll handle this during lowering by checking if any case matches
            }
            EdgeKind::ConditionalTrue | _ => {
                // Only add each target once
                if case_nodes_set.insert(target) {
                    case_nodes.push(target);
                }
            }
        }
    }

    // If no explicit default but FallThrough matches a case, use that as default
    if default_node.is_none() {
        if let Some(ft) = fallthrough_target {
            if conditional_targets.contains(&ft) {
                default_node = Some(ft);
            }
        }
    }

    let merge = region_graph.get_region_node(merge_cfg)?;

    // Collect ALL nodes between the switch and the merge.
    // This includes direct case targets and any intermediate nodes.
    // We do a BFS from the switch selector to find all reachable nodes
    // that are dominated by the switch (excluding the merge itself).
    let mut body_cfg_nodes: HashSet<NodeIndex> = HashSet::new();
    let mut queue: Vec<NodeIndex> = case_nodes.clone();
    if let Some(def) = default_node {
        queue.push(def);
    }

    for &start in &queue {
        body_cfg_nodes.insert(start);
    }

    while let Some(node) = queue.pop() {
        // Skip the merge node
        if node == merge_cfg {
            continue;
        }

        // Add successors that we haven't seen
        for succ in cfg.successors(node) {
            if succ != merge_cfg && body_cfg_nodes.insert(succ) {
                queue.push(succ);
            }
        }
    }

    // Convert to region graph indices
    let case_region_nodes: Vec<_> = case_nodes
        .iter()
        .filter_map(|&n| region_graph.get_region_node(n))
        .collect();

    // Collect body region nodes, deduplicating since multiple CFG nodes
    // might map to the same collapsed region node
    let body_region_nodes: HashSet<_> = body_cfg_nodes
        .iter()
        .filter_map(|&n| region_graph.get_region_node(n))
        .collect();

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

/// Find try-catch patterns in the graph that can be collapsed.
///
/// Uses the ExceptionAnalysis to identify Trap/EndTrap pairs.
/// A try-catch pattern is detected when:
/// 1. A block contains a Trap opcode (start of try)
/// 2. There's a corresponding EndTrap opcode (end of catch)
/// 3. The exception handler is identified from the Trap offset
///
/// Returns patterns in innermost-first order for proper nesting.
pub fn find_try_catch_patterns(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    exception_analysis: &ExceptionAnalysis,
) -> Vec<TryCatchPattern> {
    let mut patterns = Vec::new();

    // Collect all trap CFG nodes to detect shared blocks
    let all_trap_cfg_nodes: HashSet<NodeIndex> = exception_analysis
        .top_level_regions()
        .iter()
        .filter_map(|r| cfg.op_to_block.get(&r.trap_op).copied())
        .collect();

    // Process each top-level try region (nested ones will be handled recursively)
    for try_region in exception_analysis.top_level_regions() {
        if let Some(pattern) = match_try_catch_pattern(region_graph, cfg, try_region, &all_trap_cfg_nodes) {
            patterns.push(pattern);
        }
    }

    // Sort patterns to process innermost/independent ones first:
    // 1. Prefer patterns with non-empty catch_nodes (handler not mixed)
    // 2. Then by size (smallest first)
    // This ensures that when a handler is mixed (contains another Trap),
    // we process the inner try-catch first, then the outer one can include it.
    patterns.sort_by_key(|p| {
        let has_catch = if p.catch_nodes.is_empty() { 1 } else { 0 };
        (has_catch, p.try_nodes.len() + p.catch_nodes.len())
    });

    patterns
}

/// Try to match a single TryRegion as a collapsible try-catch pattern.
fn match_try_catch_pattern(
    region_graph: &RegionGraph,
    cfg: &Cfg,
    try_region: &crate::exception_analysis::TryRegion,
    all_trap_cfg_nodes: &HashSet<NodeIndex>,
) -> Option<TryCatchPattern> {
    // Find the CFG node containing the Trap opcode
    let trap_cfg_node = cfg.op_to_block.get(&try_region.trap_op)?;
    let trap_region_node = region_graph.get_region_node(*trap_cfg_node)?;

    // Find the CFG node containing the handler entry
    let handler_cfg_node = cfg.op_to_block.get(&try_region.handler_op)?;
    let handler_region_node = region_graph.get_region_node(*handler_cfg_node)?;

    // If trap and handler are already in the same region node, this try-catch
    // has already been collapsed. Skip it to avoid infinite loops.
    if trap_region_node == handler_region_node {
        return None;
    }

    // Check if handler CFG node also contains another try's Trap opcode.
    // If so, this handler node is "mixed" - it contains both catch code AND
    // another try region. We need to be careful not to include the wrong code.
    //
    // HOWEVER, if the handler region is already a Collapsed region (e.g., another
    // TryCatch that was already processed), it's no longer "mixed" - it's safe
    // to include as the catch body.
    let handler_is_collapsed = matches!(
        region_graph.get_node(handler_region_node),
        Some(RegionNode::Collapsed(_))
    );

    let handler_is_mixed = !handler_is_collapsed
        && all_trap_cfg_nodes.contains(handler_cfg_node)
        && handler_cfg_node != trap_cfg_node;

    // Collect try body nodes: ALL opcodes from trap_op through end_trap_op (inclusive)
    // This includes:
    // - The trap block itself
    // - The try body blocks
    // - Dead code blocks (like unreached EndTrap after Throw)
    // Including dead code ensures we don't leave orphaned blocks after collapse.
    let mut try_nodes = HashSet::new();
    for op_idx in try_region.trap_op..=try_region.end_trap_op {
        if let Some(&cfg_node) = cfg.op_to_block.get(&op_idx) {
            if let Some(region_node) = region_graph.get_region_node(cfg_node) {
                // Don't include the handler node in try_nodes
                if region_node != handler_region_node {
                    try_nodes.insert(region_node);
                }
            }
        }
    }

    // Collect catch body nodes
    // If the handler node is mixed (contains another Trap), don't include it
    // in catch_nodes - the other try-catch will handle that code.
    //
    // Also, if the handler region is a collapsed TryCatch from a SUBSEQUENT
    // (sequential) try-catch (not nested), don't include it. A subsequent
    // try-catch starts AFTER this try's end_trap_op.
    let catch_nodes = if handler_is_mixed {
        HashSet::new()
    } else if handler_is_collapsed {
        // Check if the collapsed region contains a subsequent try-catch
        // by checking if any of the original CFG nodes in that region
        // had a trap_op > this try's end_trap_op
        let handler_contains_subsequent_try = all_trap_cfg_nodes.iter().any(|&trap_cfg| {
            // Check if this trap CFG node maps to the same region as handler
            if let Some(trap_region) = region_graph.get_region_node(trap_cfg) {
                if trap_region == handler_region_node && trap_cfg != *trap_cfg_node {
                    // This trap is in the handler region but is different from our trap
                    // Now check if it's a subsequent try (starts after our try ends)
                    // We need to look up the trap_op for this CFG node
                    if let Some(&first_op) = cfg.op_to_block.iter()
                        .find(|(_op, &block)| block == trap_cfg)
                        .map(|(op, _)| op)
                    {
                        // If the trap in handler region starts after our end_trap_op,
                        // it's a subsequent try, not an inner try
                        return first_op > try_region.end_trap_op;
                    }
                }
            }
            false
        });

        if handler_contains_subsequent_try {
            HashSet::new()
        } else {
            let mut nodes = HashSet::new();
            nodes.insert(handler_region_node);
            nodes
        }
    } else {
        let mut nodes = HashSet::new();
        nodes.insert(handler_region_node);
        nodes
    };

    // Find the merge point: for now, use None and let subsequent reduction handle it
    // The actual merge point depends on where try and catch flows reconverge
    let merge = None;

    // Ensure we have meaningful content
    if try_nodes.is_empty() && catch_nodes.is_empty() {
        return None;
    }

    Some(TryCatchPattern {
        trap_node: trap_region_node,
        try_nodes,
        handler_node: handler_region_node,
        catch_nodes,
        exc_reg: try_region.exc_reg,
        merge,
    })
}

/// A detected pattern ready for collapse.
#[derive(Debug, Clone)]
pub enum Pattern {
    Loop(LoopPattern),
    If(IfPattern),
    Switch(SwitchPattern),
    TryCatch(TryCatchPattern),
    OrChain(OrChainPattern),
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
            Pattern::TryCatch(tcp) => {
                let mut nodes = HashSet::new();
                nodes.insert(tcp.trap_node);
                nodes.extend(tcp.try_nodes.iter());
                nodes.insert(tcp.handler_node);
                nodes.extend(tcp.catch_nodes.iter());
                nodes
            }
            Pattern::OrChain(ocp) => {
                let mut nodes = HashSet::new();
                nodes.extend(ocp.condition_nodes.iter().copied());
                nodes.insert(ocp.shared_target);
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
