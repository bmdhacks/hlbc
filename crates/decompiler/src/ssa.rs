//! Pass 3: SSA Builder - Convert CFG to Static Single Assignment form
//!
//! This module implements the Cytron et al. algorithm for SSA construction:
//! 1. Compute dominance frontiers
//! 2. Insert φ-functions at dominance frontiers
//! 3. Rename variables via dominator tree walk
//!
//! Reference: "Efficiently Computing Static Single Assignment Form and the
//! Control Dependence Graph" (Cytron et al., 1991)

use petgraph::graph::NodeIndex;
use std::collections::{HashMap, HashSet, VecDeque};

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, Reg};

use crate::analyzer::CfgAnalysis;
use crate::lifter::Cfg;

/// An SSA variable: original register + version number
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SsaVar {
    pub reg: Reg,
    pub version: u32,
}

/// Use-def information for an SSA variable (ILSpy-style analysis)
#[derive(Debug, Clone, Default)]
pub struct UseDefInfo {
    /// Number of definitions (always 1 for proper SSA)
    pub def_count: usize,
    /// Number of times this variable is used in regular operations
    pub use_count: usize,
    /// Number of times this variable is used in φ-functions (cross-block use)
    pub phi_use_count: usize,
    /// True if the defining operation is a constant (Int, Float, Bool, String, Null)
    pub is_constant: bool,
    /// True if the defining operation is pure (no side effects)
    /// Pure ops: constants, moves, arithmetic, field reads
    /// Impure ops: function calls, field writes, array writes
    pub is_pure: bool,
}

impl UseDefInfo {
    /// Can this variable be inlined? Implements ILSpy-style safety guards:
    ///
    /// Guard 1 (Side-Effect Barrier): Pure expressions can be freely inlined.
    ///         Impure expressions need adjacency check (not implemented here).
    /// Guard 2 (Debug Name Barrier): User-named variables are preserved.
    ///         (Checked separately in structurer since it has debug info access)
    /// Guard 3 (Phi Node Barrier): Can't inline across block boundaries.
    ///         Variables used by φ-functions are not inlinable.
    ///
    /// NOTE: Constants are NOT always inlinable - if they have phi_use_count > 0,
    /// they shouldn't be inlined because that would lose the variable's identity
    /// at the loop header.
    pub fn can_inline(&self) -> bool {
        // Guard 3: Can't inline if used by phi function (cross-block boundary)
        if self.phi_use_count > 0 {
            return false;
        }

        // Constants with only in-block uses can be inlined
        if self.is_constant {
            return true;
        }

        // Basic requirement: exactly one def, one use
        if self.def_count != 1 || self.use_count != 1 {
            return false;
        }

        // Guard 1: Only inline pure expressions freely
        // (Impure expressions would need adjacency checking)
        self.is_pure
    }

    /// Is this variable dead? (defined but never used)
    pub fn is_dead(&self) -> bool {
        self.def_count > 0 && self.use_count == 0 && self.phi_use_count == 0
    }

    /// Total uses (regular + phi)
    pub fn total_uses(&self) -> usize {
        self.use_count + self.phi_use_count
    }
}

impl SsaVar {
    pub fn new(reg: Reg, version: u32) -> Self {
        SsaVar { reg, version }
    }

    /// Format as variable name (e.g., "r3_2" for reg 3, version 2)
    pub fn name(&self) -> String {
        format!("r{}_{}", self.reg.0, self.version)
    }
}

/// An SSA instruction
#[derive(Debug, Clone)]
pub enum SsaInstr {
    /// φ-function: selects value based on which predecessor we came from
    Phi {
        dst: SsaVar,
        /// (predecessor block, variable from that predecessor)
        sources: Vec<(NodeIndex, SsaVar)>,
    },
    /// Regular operation with SSA variables
    Op {
        /// Original opcode index
        op_idx: usize,
        /// Destination variable (if any)
        dst: Option<SsaVar>,
        /// Source variables used by this operation
        uses: Vec<SsaVar>,
    },
}

/// An SSA basic block
#[derive(Debug, Clone)]
pub struct SsaBlock {
    /// φ-functions at block entry
    pub phis: Vec<SsaInstr>,
    /// SSA-converted operations
    pub ops: Vec<SsaInstr>,
}

impl SsaBlock {
    fn new() -> Self {
        SsaBlock {
            phis: Vec::new(),
            ops: Vec::new(),
        }
    }
}

/// SSA form of the entire function
pub struct SsaCfg {
    /// SSA blocks indexed by NodeIndex
    pub blocks: HashMap<NodeIndex, SsaBlock>,
    /// Dominance frontiers for each node
    pub dom_frontiers: HashMap<NodeIndex, HashSet<NodeIndex>>,
    /// Current version counter for each register
    version_counters: HashMap<Reg, u32>,
}

impl SsaCfg {
    /// Build SSA form from a CFG and its analysis
    pub fn build(f: &Function, cfg: &Cfg, analysis: &CfgAnalysis) -> Self {
        let mut ssa = SsaCfg {
            blocks: HashMap::new(),
            dom_frontiers: HashMap::new(),
            version_counters: HashMap::new(),
        };

        // Step 1: Compute dominance frontiers
        ssa.compute_dominance_frontiers(cfg, analysis);

        // Step 2: Find all variable definitions per block
        let defs_per_block = ssa.find_definitions(f, cfg);

        // Step 3: Insert φ-functions at dominance frontiers
        ssa.insert_phi_functions(cfg, analysis, &defs_per_block);

        // Step 4: Rename variables via dominator tree walk
        ssa.rename_variables(f, cfg, analysis);

        ssa
    }

    /// Compute dominance frontiers for all nodes
    ///
    /// DF(n) = set of nodes where n's dominance ends
    /// A node y is in DF(n) if:
    /// - n dominates a predecessor of y, but
    /// - n does not strictly dominate y
    fn compute_dominance_frontiers(&mut self, cfg: &Cfg, analysis: &CfgAnalysis) {
        for node in cfg.graph.node_indices() {
            self.dom_frontiers.insert(node, HashSet::new());
        }

        // For each node y with multiple predecessors (join points)
        for y in cfg.graph.node_indices() {
            let preds: Vec<_> = cfg.predecessors(y);
            if preds.len() >= 2 {
                // For each predecessor p of y
                for p in preds {
                    // Walk up the dominator tree from p until we reach idom(y)
                    let mut runner = p;
                    let idom_y = analysis.idom(y);

                    while Some(runner) != idom_y && Some(runner) != None {
                        // Add y to DF(runner)
                        self.dom_frontiers.get_mut(&runner).unwrap().insert(y);

                        // Move up to immediate dominator
                        if let Some(idom) = analysis.idom(runner) {
                            runner = idom;
                        } else {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Find which registers are defined in each block
    fn find_definitions(&self, f: &Function, cfg: &Cfg) -> HashMap<NodeIndex, HashSet<Reg>> {
        let mut defs: HashMap<NodeIndex, HashSet<Reg>> = HashMap::new();

        for node in cfg.graph.node_indices() {
            let block = &cfg.graph[node];
            let mut block_defs = HashSet::new();

            for op_idx in block.start..=block.end {
                if let Some(dst) = get_dst_reg(&f.ops[op_idx]) {
                    block_defs.insert(dst);
                }
            }

            defs.insert(node, block_defs);
        }

        defs
    }

    /// Insert φ-functions at dominance frontiers
    ///
    /// For each variable v defined in block B:
    ///   Insert φ for v at each node in DF(B)
    ///   Iterate until fixed point
    fn insert_phi_functions(
        &mut self,
        cfg: &Cfg,
        _analysis: &CfgAnalysis,
        defs_per_block: &HashMap<NodeIndex, HashSet<Reg>>,
    ) {
        // Initialize empty blocks
        for node in cfg.graph.node_indices() {
            self.blocks.insert(node, SsaBlock::new());
        }

        // Track which blocks already have φ for each variable
        let mut has_phi: HashMap<Reg, HashSet<NodeIndex>> = HashMap::new();
        // Track which blocks define each variable (including via φ)
        let mut def_sites: HashMap<Reg, HashSet<NodeIndex>> = HashMap::new();

        // Initialize def_sites from original definitions
        for (node, regs) in defs_per_block {
            for &reg in regs {
                def_sites.entry(reg).or_default().insert(*node);
            }
        }

        // Worklist algorithm: for each variable
        for (&reg, sites) in &def_sites.clone() {
            let mut worklist: VecDeque<NodeIndex> = sites.iter().copied().collect();
            has_phi.entry(reg).or_default();

            while let Some(node) = worklist.pop_front() {
                // For each node in DF(node)
                if let Some(frontier) = self.dom_frontiers.get(&node) {
                    for &df_node in frontier {
                        // If we haven't already inserted a φ for this variable here
                        if !has_phi.get(&reg).unwrap().contains(&df_node) {
                            // Insert φ-function (sources will be filled during renaming)
                            let preds: Vec<_> = cfg.predecessors(df_node);
                            let phi = SsaInstr::Phi {
                                dst: SsaVar::new(reg, 0), // Version assigned during renaming
                                sources: preds.iter().map(|&p| (p, SsaVar::new(reg, 0))).collect(),
                            };
                            self.blocks.get_mut(&df_node).unwrap().phis.push(phi);

                            has_phi.get_mut(&reg).unwrap().insert(df_node);

                            // The φ-function is itself a definition, add to worklist
                            if !def_sites.get(&reg).unwrap().contains(&df_node) {
                                def_sites.get_mut(&reg).unwrap().insert(df_node);
                                worklist.push_back(df_node);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Rename variables via dominator tree walk
    ///
    /// Algorithm:
    /// - Maintain a stack of versions for each register
    /// - Walk the dominator tree
    /// - On definition: push new version
    /// - On use: use top of stack
    /// - On leaving subtree: pop versions defined in this block
    fn rename_variables(&mut self, f: &Function, cfg: &Cfg, analysis: &CfgAnalysis) {
        // Stack of versions for each register: top is current version
        let mut stacks: HashMap<Reg, Vec<u32>> = HashMap::new();

        // Initialize all registers with version 0 (undefined/parameter)
        for reg_idx in 0..f.regs.len() {
            let reg = Reg(reg_idx as u32);
            stacks.insert(reg, vec![0]);
            self.version_counters.insert(reg, 1); // Next version to assign
        }

        // Build dominator tree children map
        let dom_children = build_dominator_children(cfg, analysis);

        // Recursive rename starting from entry
        self.rename_block(cfg.entry, f, cfg, analysis, &dom_children, &mut stacks);
    }

    /// Rename a single block and its dominated children
    fn rename_block(
        &mut self,
        node: NodeIndex,
        f: &Function,
        cfg: &Cfg,
        analysis: &CfgAnalysis,
        dom_children: &HashMap<NodeIndex, Vec<NodeIndex>>,
        stacks: &mut HashMap<Reg, Vec<u32>>,
    ) {
        let block_data = &cfg.graph[node];
        let mut new_defs: Vec<Reg> = Vec::new(); // Track what we defined to pop later

        // Process φ-functions: they define new versions
        let phis = std::mem::take(&mut self.blocks.get_mut(&node).unwrap().phis);
        let mut renamed_phis = Vec::new();

        for phi in phis {
            if let SsaInstr::Phi { dst, sources } = phi {
                // Assign new version for destination
                let new_version = self.next_version(dst.reg);
                stacks.get_mut(&dst.reg).unwrap().push(new_version);
                new_defs.push(dst.reg);

                renamed_phis.push(SsaInstr::Phi {
                    dst: SsaVar::new(dst.reg, new_version),
                    sources, // Source versions filled when processing predecessors
                });
            }
        }
        self.blocks.get_mut(&node).unwrap().phis = renamed_phis;

        // Process regular operations
        let mut ops = Vec::new();
        for op_idx in block_data.start..=block_data.end {
            let opcode = &f.ops[op_idx];

            // Get uses (with current versions)
            let use_regs = get_use_regs(opcode);
            let uses: Vec<SsaVar> = use_regs
                .iter()
                .map(|&reg| {
                    let version = *stacks.get(&reg).and_then(|s| s.last()).unwrap_or(&0);
                    SsaVar::new(reg, version)
                })
                .collect();

            // Get definition (assign new version)
            let dst = if let Some(dst_reg) = get_dst_reg(opcode) {
                let new_version = self.next_version(dst_reg);
                stacks.get_mut(&dst_reg).unwrap().push(new_version);
                new_defs.push(dst_reg);
                Some(SsaVar::new(dst_reg, new_version))
            } else {
                None
            };

            ops.push(SsaInstr::Op { op_idx, dst, uses });
        }
        self.blocks.get_mut(&node).unwrap().ops = ops;

        // Fill in φ-function sources in successor blocks
        for succ in cfg.successors(node) {
            let succ_phis = &mut self.blocks.get_mut(&succ).unwrap().phis;
            for phi in succ_phis.iter_mut() {
                if let SsaInstr::Phi { dst, sources } = phi {
                    // Find this predecessor's slot and fill in current version
                    for (pred, var) in sources.iter_mut() {
                        if *pred == node {
                            let version =
                                *stacks.get(&dst.reg).and_then(|s| s.last()).unwrap_or(&0);
                            *var = SsaVar::new(dst.reg, version);
                        }
                    }
                }
            }
        }

        // Recurse to dominated children
        if let Some(children) = dom_children.get(&node) {
            for &child in children {
                self.rename_block(child, f, cfg, analysis, dom_children, stacks);
            }
        }

        // Pop definitions made in this block
        for reg in new_defs {
            stacks.get_mut(&reg).unwrap().pop();
        }
    }

    /// Get next version number for a register
    fn next_version(&mut self, reg: Reg) -> u32 {
        let counter = self.version_counters.entry(reg).or_insert(1);
        let version = *counter;
        *counter += 1;
        version
    }

    /// Get the number of φ-functions in the SSA form
    pub fn phi_count(&self) -> usize {
        self.blocks.values().map(|b| b.phis.len()).sum()
    }

    /// Compute use counts for all SSA variables.
    /// Returns a map from SsaVar to UseDefInfo including purity information.
    pub fn compute_use_counts(&self, f: &Function) -> HashMap<SsaVar, UseDefInfo> {
        let mut info: HashMap<SsaVar, UseDefInfo> = HashMap::new();

        // First, record all definitions and their purity
        for block in self.blocks.values() {
            // φ-functions define variables - they are NOT pure (can't inline directly)
            // and NOT constants
            for phi in &block.phis {
                if let SsaInstr::Phi { dst, .. } = phi {
                    let entry = info.entry(*dst).or_default();
                    entry.def_count = 1;
                    entry.is_constant = false;
                    entry.is_pure = false; // φ-functions are not inlinable
                }
            }
            // Regular operations - check opcode for purity
            for op in &block.ops {
                if let SsaInstr::Op { op_idx, dst: Some(dst), .. } = op {
                    let opcode = &f.ops[*op_idx];
                    let (is_const, is_pure) = classify_opcode_purity(opcode);
                    let entry = info.entry(*dst).or_default();
                    entry.def_count = 1;
                    entry.is_constant = is_const;
                    entry.is_pure = is_pure;
                }
            }
        }

        // Then, count all uses
        for block in self.blocks.values() {
            // Uses in φ-function sources - these are cross-block uses!
            // Must track separately to prevent inlining across block boundaries.
            for phi in &block.phis {
                if let SsaInstr::Phi { sources, .. } = phi {
                    for (_, src_var) in sources {
                        // Skip version 0 (undefined/parameter)
                        if src_var.version > 0 {
                            info.entry(*src_var).or_default().phi_use_count += 1;
                        }
                    }
                }
            }
            // Uses in regular operations - these are in-block uses
            for op in &block.ops {
                if let SsaInstr::Op { uses, .. } = op {
                    for use_var in uses {
                        // Skip version 0 (undefined/parameter)
                        if use_var.version > 0 {
                            info.entry(*use_var).or_default().use_count += 1;
                        }
                    }
                }
            }
        }

        info
    }

    /// Get the SsaInstr for a given opcode index.
    /// Returns the destination and uses for that operation.
    pub fn get_instr_for_op(&self, op_idx: usize) -> Option<(Option<SsaVar>, &Vec<SsaVar>)> {
        for block in self.blocks.values() {
            for op in &block.ops {
                if let SsaInstr::Op { op_idx: idx, dst, uses } = op {
                    if *idx == op_idx {
                        return Some((*dst, uses));
                    }
                }
            }
        }
        None
    }

    /// Find the defining opcode index for an SSA variable.
    /// Returns the op_idx where this variable was defined, or None if it's a φ or undefined.
    pub fn find_def(&self, var: SsaVar) -> Option<usize> {
        for block in self.blocks.values() {
            for op in &block.ops {
                if let SsaInstr::Op { op_idx, dst: Some(dst), .. } = op {
                    if *dst == var {
                        return Some(*op_idx);
                    }
                }
            }
        }
        None
    }

    /// Check if an SSA variable is defined by a same-register phi function.
    /// Same-register phis merge different SSA versions of the SAME register.
    /// For these, we should use non-SSA naming to allow value flow-through.
    pub fn is_same_register_phi(&self, var: SsaVar) -> bool {
        for block in self.blocks.values() {
            for phi in &block.phis {
                if let SsaInstr::Phi { dst, sources } = phi {
                    if *dst == var {
                        // Check if all sources are the same register as destination
                        return sources.iter().all(|(_, src)| src.reg == var.reg);
                    }
                }
            }
        }
        false
    }

    /// Check if an SSA variable is a SOURCE of a same-register phi function.
    /// For phi sources that feed into same-register phis, we should also use
    /// non-SSA naming to ensure the value flows through properly.
    pub fn is_same_register_phi_source(&self, var: SsaVar) -> bool {
        for block in self.blocks.values() {
            for phi in &block.phis {
                if let SsaInstr::Phi { dst, sources } = phi {
                    // Check if all sources are the same register as destination (same-register phi)
                    let is_same_reg_phi = sources.iter().all(|(_, src)| src.reg == dst.reg);
                    if is_same_reg_phi {
                        // Check if var is one of the sources
                        if sources.iter().any(|(_, src)| *src == var) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Debug: print SSA form
    #[allow(dead_code)]
    pub fn dump(&self, f: &Function, cfg: &Cfg) {
        for node in cfg.graph.node_indices() {
            let block = &cfg.graph[node];
            eprintln!("Block {:?} (ops {}..={})", node, block.start, block.end);

            if let Some(ssa_block) = self.blocks.get(&node) {
                for phi in &ssa_block.phis {
                    if let SsaInstr::Phi { dst, sources } = phi {
                        let sources_str: Vec<_> = sources
                            .iter()
                            .map(|(pred, var)| format!("[{:?}]: {}", pred, var.name()))
                            .collect();
                        eprintln!("  φ: {} = φ({})", dst.name(), sources_str.join(", "));
                    }
                }

                for instr in &ssa_block.ops {
                    if let SsaInstr::Op { op_idx, dst, uses } = instr {
                        let dst_str = dst.map_or("_".to_string(), |v| v.name());
                        let uses_str: Vec<_> = uses.iter().map(|v| v.name()).collect();
                        eprintln!(
                            "  {}: {} = {:?} (uses: {})",
                            op_idx,
                            dst_str,
                            &f.ops[*op_idx],
                            uses_str.join(", ")
                        );
                    }
                }
            }
            eprintln!();
        }
    }
}

/// Build a map from each node to its dominated children
fn build_dominator_children(
    cfg: &Cfg,
    analysis: &CfgAnalysis,
) -> HashMap<NodeIndex, Vec<NodeIndex>> {
    let mut children: HashMap<NodeIndex, Vec<NodeIndex>> = HashMap::new();

    for node in cfg.graph.node_indices() {
        children.insert(node, Vec::new());
    }

    for node in cfg.graph.node_indices() {
        if let Some(idom) = analysis.idom(node) {
            children.get_mut(&idom).unwrap().push(node);
        }
    }

    children
}

/// Extract the destination register from an opcode (if any)
pub fn get_dst_reg(op: &Opcode) -> Option<Reg> {
    use Opcode::*;
    match op {
        // Arithmetic
        Add { dst, .. }
        | Sub { dst, .. }
        | Mul { dst, .. }
        | SDiv { dst, .. }
        | UDiv { dst, .. }
        | SMod { dst, .. }
        | UMod { dst, .. }
        | Shl { dst, .. }
        | SShr { dst, .. }
        | UShr { dst, .. }
        | And { dst, .. }
        | Or { dst, .. }
        | Xor { dst, .. }
        | Neg { dst, .. }
        | Not { dst, .. }
        | Incr { dst }
        | Decr { dst } => Some(*dst),

        // Loads/constants
        Int { dst, .. }
        | Float { dst, .. }
        | Bool { dst, .. }
        | Bytes { dst, .. }
        | String { dst, .. }
        | Null { dst } => Some(*dst),

        // Memory
        Mov { dst, .. }
        | GetGlobal { dst, .. }
        | Field { dst, .. }
        | GetThis { dst, .. }
        | DynGet { dst, .. }
        | GetI8 { dst, .. }
        | GetI16 { dst, .. }
        | GetMem { dst, .. }
        | GetArray { dst, .. }
        | GetType { dst, .. }
        | GetTID { dst, .. }
        | Ref { dst, .. }
        | Unref { dst, .. }
        | ArraySize { dst, .. }
        | Type { dst, .. }
        | EnumIndex { dst, .. }
        | EnumField { dst, .. }
        | EnumAlloc { dst, .. } => Some(*dst),

        // Calls
        Call0 { dst, .. }
        | Call1 { dst, .. }
        | Call2 { dst, .. }
        | Call3 { dst, .. }
        | Call4 { dst, .. }
        | CallN { dst, .. }
        | CallMethod { dst, .. }
        | CallThis { dst, .. }
        | CallClosure { dst, .. } => Some(*dst),

        // Object creation
        New { dst }
        | MakeEnum { dst, .. }
        | InstanceClosure { dst, .. }
        | StaticClosure { dst, .. }
        | VirtualClosure { dst, .. } => Some(*dst),

        // Conversions
        ToDyn { dst, .. }
        | ToSFloat { dst, .. }
        | ToUFloat { dst, .. }
        | ToInt { dst, .. }
        | SafeCast { dst, .. }
        | UnsafeCast { dst, .. }
        | ToVirtual { dst, .. } => Some(*dst),

        // Misc with dst
        RefData { dst, .. }
        | RefOffset { dst, .. } => Some(*dst),

        // No destination
        SetGlobal { .. }
        | SetField { .. }
        | SetThis { .. }
        | DynSet { .. }
        | SetI8 { .. }
        | SetI16 { .. }
        | SetMem { .. }
        | SetArray { .. }
        | SetEnumField { .. }
        | Setref { .. }
        | Ret { .. }
        | Throw { .. }
        | Rethrow { .. }
        | Switch { .. }
        | JTrue { .. }
        | JFalse { .. }
        | JNull { .. }
        | JNotNull { .. }
        | JSLt { .. }
        | JSGte { .. }
        | JSGt { .. }
        | JSLte { .. }
        | JULt { .. }
        | JUGte { .. }
        | JNotLt { .. }
        | JNotGte { .. }
        | JEq { .. }
        | JNotEq { .. }
        | JAlways { .. }
        | Label
        | Nop
        | EndTrap { .. }
        | Trap { .. }
        | NullCheck { .. }
        | Assert
        | Prefetch { .. }
        | Asm { .. } => None,
    }
}

/// Extract source registers used by an opcode
fn get_use_regs(op: &Opcode) -> Vec<Reg> {
    use Opcode::*;
    match op {
        // Binary arithmetic
        Add { a, b, .. }
        | Sub { a, b, .. }
        | Mul { a, b, .. }
        | SDiv { a, b, .. }
        | UDiv { a, b, .. }
        | SMod { a, b, .. }
        | UMod { a, b, .. }
        | Shl { a, b, .. }
        | SShr { a, b, .. }
        | UShr { a, b, .. }
        | And { a, b, .. }
        | Or { a, b, .. }
        | Xor { a, b, .. } => vec![*a, *b],

        // Unary
        Neg { src, .. } | Not { src, .. } | Mov { src, .. } => vec![*src],

        // Increment/decrement read and modify
        Incr { dst } | Decr { dst } => vec![*dst],

        // Casts / conversions
        ToDyn { src, .. }
        | ToSFloat { src, .. }
        | ToUFloat { src, .. }
        | ToInt { src, .. }
        | SafeCast { src, .. }
        | UnsafeCast { src, .. }
        | ToVirtual { src, .. }
        | GetType { src, .. }
        | GetTID { src, .. } => vec![*src],

        // Memory stores
        SetGlobal { src, .. } => vec![*src],
        SetField { obj, src, .. } => vec![*obj, *src],
        SetThis { src, .. } => vec![*src],
        DynSet { obj, src, .. } => vec![*obj, *src], // field is RefString, not Reg
        SetI8 { bytes, index, src } | SetI16 { bytes, index, src } | SetMem { bytes, index, src } => {
            vec![*bytes, *index, *src]
        }
        SetArray { array, index, src } => vec![*array, *index, *src],
        SetEnumField { value, src, .. } => vec![*value, *src],
        Setref { dst, value } => vec![*dst, *value],

        // Memory loads
        Field { obj, .. } => vec![*obj],
        DynGet { obj, .. } => vec![*obj], // field is RefString, not Reg
        GetI8 { bytes, index, .. } | GetI16 { bytes, index, .. } | GetMem { bytes, index, .. } => {
            vec![*bytes, *index]
        }
        GetArray { array, index, .. } => vec![*array, *index],
        Ref { src, .. } => vec![*src],
        Unref { src, .. } => vec![*src],
        ArraySize { array, .. } => vec![*array],
        EnumIndex { value, .. } => vec![*value],
        EnumField { value, .. } => vec![*value],

        // Calls
        Call0 { .. } => vec![],
        Call1 { arg0, .. } => vec![*arg0],
        Call2 { arg0, arg1, .. } => vec![*arg0, *arg1],
        Call3 { arg0, arg1, arg2, .. } => vec![*arg0, *arg1, *arg2],
        Call4 {
            arg0,
            arg1,
            arg2,
            arg3,
            ..
        } => vec![*arg0, *arg1, *arg2, *arg3],
        CallN { args, .. } => args.iter().copied().collect(),
        CallMethod { args, .. } => args.iter().copied().collect(), // first arg is obj
        CallThis { args, .. } => args.iter().copied().collect(),
        CallClosure { fun, args, .. } => {
            let mut v = vec![*fun];
            v.extend(args.iter().copied());
            v
        }

        // Object creation
        MakeEnum { args, .. } => args.iter().copied().collect(),
        InstanceClosure { obj, .. } => vec![*obj],
        VirtualClosure { obj, field, .. } => vec![*obj, *field],
        EnumAlloc { .. } | StaticClosure { .. } | New { .. } => vec![],

        // Control flow
        JTrue { cond, .. } | JFalse { cond, .. } => vec![*cond],
        JNull { reg, .. } | JNotNull { reg, .. } => vec![*reg],

        JSLt { a, b, .. }
        | JSGte { a, b, .. }
        | JSGt { a, b, .. }
        | JSLte { a, b, .. }
        | JULt { a, b, .. }
        | JUGte { a, b, .. }
        | JNotLt { a, b, .. }
        | JNotGte { a, b, .. }
        | JEq { a, b, .. }
        | JNotEq { a, b, .. } => vec![*a, *b],

        Switch { reg, .. } => vec![*reg],
        Ret { ret } => vec![*ret],
        Throw { exc } | Rethrow { exc } | EndTrap { exc } => vec![*exc],
        NullCheck { reg } => vec![*reg],
        Prefetch { value, .. } => vec![*value],

        // Misc
        RefData { src, .. } => vec![*src],
        RefOffset { reg, offset, .. } => vec![*reg, *offset],

        // No sources
        GetGlobal { .. }
        | GetThis { .. }
        | Int { .. }
        | Float { .. }
        | Bool { .. }
        | Bytes { .. }
        | String { .. }
        | Null { .. }
        | Type { .. }
        | Label
        | JAlways { .. }
        | Nop
        | Trap { .. }
        | Assert
        | Asm { .. } => vec![],
    }
}

/// Classify an opcode's purity for inlining decisions.
/// Returns (is_constant, is_pure).
///
/// - is_constant: true for literal constants (Int, Float, Bool, String, Null)
///   Constants can ALWAYS be inlined regardless of use count.
///
/// - is_pure: true for operations with no side effects
///   Pure operations can be inlined when they have single use.
///   Examples: constants, moves, arithmetic, field reads
///   Impure: function calls, object creation, field writes
fn classify_opcode_purity(op: &Opcode) -> (bool, bool) {
    use Opcode::*;
    match op {
        // Constants - always inlinable
        Int { .. } | Float { .. } | Bool { .. } | String { .. } | Bytes { .. } | Null { .. } => {
            (true, true)
        }

        // Pure operations - inlinable with single use
        // Moves and casts
        Mov { .. } | ToDyn { .. } | ToSFloat { .. } | ToUFloat { .. } | ToInt { .. }
        | SafeCast { .. } | UnsafeCast { .. } | ToVirtual { .. } => (false, true),

        // Arithmetic (no side effects)
        Add { .. } | Sub { .. } | Mul { .. } | SDiv { .. } | UDiv { .. } | SMod { .. }
        | UMod { .. } | Shl { .. } | SShr { .. } | UShr { .. } | And { .. } | Or { .. }
        | Xor { .. } | Neg { .. } | Not { .. } => (false, true),

        // Reads from memory (pure - don't modify state)
        Field { .. } | GetThis { .. } | GetGlobal { .. } | DynGet { .. } | GetI8 { .. }
        | GetI16 { .. } | GetMem { .. } | GetArray { .. } | ArraySize { .. } | EnumIndex { .. }
        | EnumField { .. } | Type { .. } | GetType { .. } | GetTID { .. } | Ref { .. }
        | Unref { .. } | RefData { .. } | RefOffset { .. } => (false, true),

        // Incr/Decr - modifies register, but can be inlined with care
        // Actually these are impure because they modify the destination in place
        Incr { .. } | Decr { .. } => (false, false),

        // IMPURE operations - have side effects, can't inline freely
        // Function calls may have arbitrary side effects
        Call0 { .. } | Call1 { .. } | Call2 { .. } | Call3 { .. } | Call4 { .. } | CallN { .. }
        | CallMethod { .. } | CallThis { .. } | CallClosure { .. } => (false, false),

        // Object creation - allocates memory, may have initializers
        New { .. } | EnumAlloc { .. } | MakeEnum { .. } | InstanceClosure { .. }
        | StaticClosure { .. } | VirtualClosure { .. } => (false, false),

        // Memory writes - obvious side effects
        SetGlobal { .. } | SetField { .. } | SetThis { .. } | DynSet { .. } | SetI8 { .. }
        | SetI16 { .. } | SetMem { .. } | SetArray { .. } | SetEnumField { .. }
        | Setref { .. } => (false, false),

        // Control flow - not expressions, don't have destinations anyway
        JTrue { .. } | JFalse { .. } | JNull { .. } | JNotNull { .. } | JSLt { .. }
        | JSGte { .. } | JSGt { .. } | JSLte { .. } | JULt { .. } | JUGte { .. }
        | JNotLt { .. } | JNotGte { .. } | JEq { .. } | JNotEq { .. } | JAlways { .. }
        | Switch { .. } | Label | Nop | Ret { .. } | Throw { .. } | Rethrow { .. }
        | Trap { .. } | EndTrap { .. } | NullCheck { .. } | Assert | Prefetch { .. }
        | Asm { .. } => (false, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::CfgAnalysis;
    use crate::lifter::Cfg;
    use hlbc::types::RefInt;

    #[test]
    fn test_simple_ssa() {
        // Simple linear sequence
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

        // Create a mock function
        let f = create_mock_function(&ops, 3);

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        let ssa = SsaCfg::build(&f, &cfg, &analysis);

        // No φ-functions needed for linear code
        assert_eq!(ssa.phi_count(), 0);

        // Should have one block with 4 operations
        assert_eq!(ssa.blocks.len(), 1);
    }

    #[test]
    fn test_diamond_cfg_needs_phi() {
        // Diamond CFG: if (cond) { x = 1 } else { x = 2 }; use x
        let ops = vec![
            // Block 0: op 0-1
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            }, // cond
            Opcode::JNull {
                reg: Reg(0),
                offset: 2,
            }, // jump to op 4 if null
            // Block 1: op 2-3 (then branch)
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            }, // x = 1
            Opcode::JAlways { offset: 1 },     // jump to op 5
            // Block 2: op 4 (else branch)
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(2),
            }, // x = 2
            // Block 3: op 5 (merge)
            Opcode::Ret { ret: Reg(1) }, // return x
        ];

        let f = create_mock_function(&ops, 2);

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        let ssa = SsaCfg::build(&f, &cfg, &analysis);

        // Should need a φ-function for Reg(1) at the merge point
        assert!(ssa.phi_count() >= 1, "Diamond CFG should have φ-function");
    }

    #[test]
    fn test_loop_phi() {
        // Simple loop: while (cond) { i++ }
        let ops = vec![
            // Block 0: op 0 - init
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            }, // i = 0
            // Block 1: op 1-2 - loop header
            Opcode::Label,
            Opcode::JNull {
                reg: Reg(0),
                offset: 3,
            }, // if i == null, exit
            // Block 2: op 3-4 - loop body
            Opcode::Incr { dst: Reg(0) }, // i++
            Opcode::JAlways { offset: -3 }, // back to label
            // Block 3: op 5 - exit
            Opcode::Ret { ret: Reg(0) },
        ];

        let f = create_mock_function(&ops, 1);

        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        let ssa = SsaCfg::build(&f, &cfg, &analysis);

        // Loop header should have a φ-function for the loop variable
        assert!(ssa.phi_count() >= 1, "Loop should have φ-function at header");
    }

    /// Create a minimal mock function for testing
    fn create_mock_function(ops: &[Opcode], num_regs: usize) -> Function {
        use hlbc::types::{RefFun, RefString, RefType};

        Function {
            name: RefString(0),
            t: RefType(0),
            findex: RefFun(0),
            regs: vec![RefType(0); num_regs],
            ops: ops.to_vec(),
            debug_info: None,
            assigns: None,
            parent: None,
        }
    }
}
