//! Pass 4: Type Propagator - Infer types through the SSA graph
//!
//! This module performs forward type inference on the SSA-CFG:
//! - Infers types from function call signatures
//! - Propagates types through operators and assignments
//! - Unifies types at φ-functions
//! - Tracks type conflicts for debugging
//!
//! The type information enables better variable naming and cast elimination
//! in the final output.

use petgraph::graph::NodeIndex;
use std::collections::{HashMap, HashSet, VecDeque};

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, RefType, Reg, Type};
use hlbc::Bytecode;

use crate::lifter::Cfg;
use crate::ssa::{SsaCfg, SsaInstr, SsaVar};

/// Inferred type for an SSA variable
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferredType {
    /// Known concrete type from bytecode
    Known(RefType),
    /// Type inferred from usage (e.g., "must be Int because used in addition")
    Inferred(RefType),
    /// Multiple possible types (from φ-function with different branch types)
    Union(Vec<RefType>),
    /// Unknown type (not yet inferred)
    Unknown,
    /// Type conflict detected
    Conflict(String),
}

impl InferredType {
    /// Check if this is a known or inferred type (not unknown/conflict)
    pub fn is_resolved(&self) -> bool {
        matches!(self, InferredType::Known(_) | InferredType::Inferred(_))
    }

    /// Get the RefType if known/inferred
    pub fn as_ref_type(&self) -> Option<RefType> {
        match self {
            InferredType::Known(t) | InferredType::Inferred(t) => Some(*t),
            _ => None,
        }
    }

    /// Try to unify two types
    pub fn unify(&self, other: &InferredType) -> InferredType {
        match (self, other) {
            // Unknown unifies with anything
            (InferredType::Unknown, t) | (t, InferredType::Unknown) => t.clone(),

            // Same types unify
            (InferredType::Known(a), InferredType::Known(b)) if a == b => InferredType::Known(*a),
            (InferredType::Inferred(a), InferredType::Inferred(b)) if a == b => {
                InferredType::Inferred(*a)
            }
            (InferredType::Known(a), InferredType::Inferred(b))
            | (InferredType::Inferred(a), InferredType::Known(b))
                if a == b =>
            {
                InferredType::Known(*a)
            }

            // Different types create a union
            (InferredType::Known(a), InferredType::Known(b))
            | (InferredType::Inferred(a), InferredType::Inferred(b))
            | (InferredType::Known(a), InferredType::Inferred(b))
            | (InferredType::Inferred(a), InferredType::Known(b)) => {
                InferredType::Union(vec![*a, *b])
            }

            // Union with a type adds to the union
            (InferredType::Union(types), InferredType::Known(t))
            | (InferredType::Union(types), InferredType::Inferred(t))
            | (InferredType::Known(t), InferredType::Union(types))
            | (InferredType::Inferred(t), InferredType::Union(types)) => {
                let mut new_types = types.clone();
                if !new_types.contains(t) {
                    new_types.push(*t);
                }
                InferredType::Union(new_types)
            }

            // Union with union merges
            (InferredType::Union(a), InferredType::Union(b)) => {
                let mut merged: Vec<RefType> = a.clone();
                for t in b {
                    if !merged.contains(t) {
                        merged.push(*t);
                    }
                }
                InferredType::Union(merged)
            }

            // Conflict propagates
            (InferredType::Conflict(msg), _) | (_, InferredType::Conflict(msg)) => {
                InferredType::Conflict(msg.clone())
            }
        }
    }
}

/// Type information for all SSA variables in a function
pub struct TypeInfo {
    /// Inferred type for each SSA variable
    pub var_types: HashMap<SsaVar, InferredType>,
    /// Type conflicts encountered during inference
    pub conflicts: Vec<(SsaVar, String)>,
}

impl TypeInfo {
    /// Create empty type info
    pub fn new() -> Self {
        TypeInfo {
            var_types: HashMap::new(),
            conflicts: Vec::new(),
        }
    }

    /// Get the type of an SSA variable
    pub fn get_type(&self, var: &SsaVar) -> &InferredType {
        self.var_types.get(var).unwrap_or(&InferredType::Unknown)
    }

    /// Set the type of an SSA variable
    pub fn set_type(&mut self, var: SsaVar, ty: InferredType) {
        if let InferredType::Conflict(msg) = &ty {
            self.conflicts.push((var, msg.clone()));
        }
        self.var_types.insert(var, ty);
    }

    /// Number of resolved types
    pub fn resolved_count(&self) -> usize {
        self.var_types.values().filter(|t| t.is_resolved()).count()
    }

    /// Number of unknown types
    pub fn unknown_count(&self) -> usize {
        self.var_types
            .values()
            .filter(|t| matches!(t, InferredType::Unknown))
            .count()
    }
}

impl Default for TypeInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// Type propagator that infers types through the SSA graph
pub struct TypePropagator<'a> {
    code: &'a Bytecode,
    func: &'a Function,
    cfg: &'a Cfg,
    ssa: &'a SsaCfg,
}

impl<'a> TypePropagator<'a> {
    /// Create a new type propagator
    pub fn new(code: &'a Bytecode, func: &'a Function, cfg: &'a Cfg, ssa: &'a SsaCfg) -> Self {
        TypePropagator {
            code,
            func,
            cfg,
            ssa,
        }
    }

    /// Run type propagation and return type info
    pub fn propagate(&self) -> TypeInfo {
        let mut info = TypeInfo::new();

        // Phase 1: Initialize types from register declarations
        self.init_register_types(&mut info);

        // Phase 2: Forward propagation through operations
        self.forward_propagate(&mut info);

        // Phase 3: Backward propagation for uses that constrain types
        self.backward_propagate(&mut info);

        // Phase 4: Resolve φ-functions
        self.resolve_phi_types(&mut info);

        info
    }

    /// Initialize types from function's register type declarations
    fn init_register_types(&self, info: &mut TypeInfo) {
        // Function parameters and locals have declared types in func.regs
        // We use version 0 as the "initial" version for parameters
        for (reg_idx, &ref_type) in self.func.regs.iter().enumerate() {
            let var = SsaVar::new(Reg(reg_idx as u32), 0);
            info.set_type(var, InferredType::Known(ref_type));
        }
    }

    /// Forward propagation: infer types from definitions
    fn forward_propagate(&self, info: &mut TypeInfo) {
        // Process blocks in topological order (roughly)
        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let mut worklist: VecDeque<NodeIndex> = VecDeque::new();
        worklist.push_back(self.cfg.entry);

        while let Some(node) = worklist.pop_front() {
            if visited.contains(&node) {
                continue;
            }
            visited.insert(node);

            if let Some(ssa_block) = self.ssa.blocks.get(&node) {
                // Process φ-functions first (placeholder types)
                for phi in &ssa_block.phis {
                    if let SsaInstr::Phi { dst, .. } = phi {
                        // φ types resolved later after all sources are typed
                        if !info.var_types.contains_key(dst) {
                            info.set_type(*dst, InferredType::Unknown);
                        }
                    }
                }

                // Process operations
                for op in &ssa_block.ops {
                    if let SsaInstr::Op { op_idx, dst, uses } = op {
                        if let Some(dst_var) = dst {
                            let inferred = self.infer_op_type(*op_idx, uses, info);
                            info.set_type(*dst_var, inferred);
                        }
                    }
                }
            }

            // Add successors to worklist
            for succ in self.cfg.successors(node) {
                if !visited.contains(&succ) {
                    worklist.push_back(succ);
                }
            }
        }
    }

    /// Infer the result type of an operation
    fn infer_op_type(&self, op_idx: usize, uses: &[SsaVar], info: &TypeInfo) -> InferredType {
        let op = &self.func.ops[op_idx];

        match op {
            // Constants have known types
            Opcode::Int { .. } => self.find_builtin_type("Int"),
            Opcode::Float { .. } => self.find_builtin_type("Float"),
            Opcode::Bool { .. } => self.find_builtin_type("Bool"),
            Opcode::String { .. } => self.find_builtin_type("String"),
            Opcode::Bytes { .. } => self.find_builtin_type("Bytes"),
            Opcode::Null { dst } => {
                // Null takes the type of the register
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Mov copies the source type
            Opcode::Mov { .. } => {
                if !uses.is_empty() {
                    info.get_type(&uses[0]).clone()
                } else {
                    InferredType::Unknown
                }
            }

            // Arithmetic operations - result type depends on operand types
            Opcode::Add { dst, .. }
            | Opcode::Sub { dst, .. }
            | Opcode::Mul { dst, .. }
            | Opcode::SDiv { dst, .. }
            | Opcode::UDiv { dst, .. }
            | Opcode::SMod { dst, .. }
            | Opcode::UMod { dst, .. } => {
                // Result type is typically the same as operands
                // Use the declared register type as fallback
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else if !uses.is_empty() {
                    info.get_type(&uses[0]).clone()
                } else {
                    InferredType::Unknown
                }
            }

            // Bitwise operations return Int
            Opcode::Shl { .. }
            | Opcode::SShr { .. }
            | Opcode::UShr { .. }
            | Opcode::And { .. }
            | Opcode::Or { .. }
            | Opcode::Xor { .. } => self.find_builtin_type("Int"),

            // Unary operations preserve type
            Opcode::Neg { dst, .. } | Opcode::Not { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Increment/decrement preserve type
            Opcode::Incr { dst } | Opcode::Decr { dst } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Type conversions have explicit result types
            Opcode::ToInt { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    self.find_builtin_type("Int")
                }
            }
            Opcode::ToSFloat { dst, .. } | Opcode::ToUFloat { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    self.find_builtin_type("Float")
                }
            }

            // Casts use destination register type
            Opcode::SafeCast { dst, .. }
            | Opcode::UnsafeCast { dst, .. }
            | Opcode::ToDyn { dst, .. }
            | Opcode::ToVirtual { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Field access - look up field type
            Opcode::Field { dst, field, .. } => {
                // Try to get field type from object type
                if let Some(obj_type) = self.get_use_type(uses, 0, info) {
                    if let Some(field_type) = self.get_field_type(obj_type, *field) {
                        return InferredType::Inferred(field_type);
                    }
                }
                // Fallback to register type
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            Opcode::GetThis { dst, field } => {
                // this is reg0, look up field type
                if let Some(this_type) = self.func.regs.first() {
                    if let Some(field_type) = self.get_field_type(*this_type, *field) {
                        return InferredType::Inferred(field_type);
                    }
                }
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Calls - look up return type
            Opcode::Call0 { dst, fun }
            | Opcode::Call1 { dst, fun, .. }
            | Opcode::Call2 { dst, fun, .. }
            | Opcode::Call3 { dst, fun, .. }
            | Opcode::Call4 { dst, fun, .. }
            | Opcode::CallN { dst, fun, .. } => {
                if let Some(ret_type) = self.get_function_return_type(*fun) {
                    InferredType::Inferred(ret_type)
                } else {
                    let reg_idx = dst.0 as usize;
                    if reg_idx < self.func.regs.len() {
                        InferredType::Known(self.func.regs[reg_idx])
                    } else {
                        InferredType::Unknown
                    }
                }
            }

            Opcode::CallMethod { dst, .. } | Opcode::CallThis { dst, .. } => {
                // Method return type - would need virtual table lookup (field available in opcode)
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            Opcode::CallClosure { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Array/memory access
            Opcode::GetArray { dst, .. }
            | Opcode::GetI8 { dst, .. }
            | Opcode::GetI16 { dst, .. }
            | Opcode::GetMem { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Object creation
            Opcode::New { dst } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Array size returns Int
            Opcode::ArraySize { .. } => self.find_builtin_type("Int"),

            // Type operations
            Opcode::Type { ty, .. } => InferredType::Known(*ty),
            Opcode::GetType { dst, .. } | Opcode::GetTID { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Closures
            Opcode::StaticClosure { dst, .. }
            | Opcode::InstanceClosure { dst, .. }
            | Opcode::VirtualClosure { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Enums
            Opcode::MakeEnum { dst, .. } | Opcode::EnumAlloc { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }
            Opcode::EnumIndex { .. } => self.find_builtin_type("Int"),
            Opcode::EnumField { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // References
            Opcode::Ref { dst, .. } | Opcode::Unref { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Globals
            Opcode::GetGlobal { dst, global } => {
                if let Some(global_type) = self.get_global_type(*global) {
                    InferredType::Known(global_type)
                } else {
                    let reg_idx = dst.0 as usize;
                    if reg_idx < self.func.regs.len() {
                        InferredType::Known(self.func.regs[reg_idx])
                    } else {
                        InferredType::Unknown
                    }
                }
            }

            // DynGet - dynamic field access
            Opcode::DynGet { dst, .. } => {
                let reg_idx = dst.0 as usize;
                if reg_idx < self.func.regs.len() {
                    InferredType::Known(self.func.regs[reg_idx])
                } else {
                    InferredType::Unknown
                }
            }

            // Default: use register type
            _ => {
                // For any opcode with a dst, try to use register type
                InferredType::Unknown
            }
        }
    }

    /// Get type of a use at given index
    fn get_use_type(&self, uses: &[SsaVar], idx: usize, info: &TypeInfo) -> Option<RefType> {
        uses.get(idx).and_then(|var| info.get_type(var).as_ref_type())
    }

    /// Find a builtin type by name
    fn find_builtin_type(&self, name: &str) -> InferredType {
        // Search for the type in the bytecode
        for (idx, ty) in self.code.types.iter().enumerate() {
            match ty {
                Type::I32 if name == "Int" => return InferredType::Known(RefType(idx)),
                Type::F64 if name == "Float" => return InferredType::Known(RefType(idx)),
                Type::Bool if name == "Bool" => return InferredType::Known(RefType(idx)),
                Type::Bytes if name == "Bytes" => return InferredType::Known(RefType(idx)),
                Type::Obj(obj) => {
                    if let Some(obj_name) = self.code.strings.get(obj.name.0) {
                        if obj_name == name {
                            return InferredType::Known(RefType(idx));
                        }
                    }
                }
                _ => {}
            }
        }
        InferredType::Unknown
    }

    /// Get field type from an object type
    fn get_field_type(&self, obj_type: RefType, field: hlbc::types::RefField) -> Option<RefType> {
        if let Some(Type::Obj(obj)) = self.code.types.get(obj_type.0) {
            if let Some(field_def) = obj.fields.get(field.0) {
                return Some(field_def.t);
            }
        }
        None
    }

    /// Get return type of a function
    fn get_function_return_type(&self, fun: hlbc::types::RefFun) -> Option<RefType> {
        // Look up function type
        if let Some(func) = self.code.functions.get(fun.0) {
            if let Some(Type::Fun(fun_type)) = self.code.types.get(func.t.0) {
                return Some(fun_type.ret);
            }
        }
        // Try natives
        if let Some(native) = self.code.natives.get(fun.0.saturating_sub(self.code.functions.len()))
        {
            if let Some(Type::Fun(fun_type)) = self.code.types.get(native.t.0) {
                return Some(fun_type.ret);
            }
        }
        None
    }

    /// Get type of a global
    fn get_global_type(&self, global: hlbc::types::RefGlobal) -> Option<RefType> {
        self.code.globals.get(global.0).copied()
    }

    /// Backward propagation: infer types from uses
    fn backward_propagate(&self, _info: &mut TypeInfo) {
        // For now, minimal backward propagation
        // Could be extended to infer types from how variables are used
    }

    /// Resolve φ-function types by unifying all incoming types
    fn resolve_phi_types(&self, info: &mut TypeInfo) {
        let mut changed = true;
        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 10;

        while changed && iterations < MAX_ITERATIONS {
            changed = false;
            iterations += 1;

            for ssa_block in self.ssa.blocks.values() {
                for phi in &ssa_block.phis {
                    if let SsaInstr::Phi { dst, sources } = phi {
                        // Unify types from all sources
                        let mut unified = InferredType::Unknown;
                        for (_, src_var) in sources {
                            let src_type = info.get_type(src_var).clone();
                            unified = unified.unify(&src_type);
                        }

                        // Update if changed
                        let current = info.get_type(dst).clone();
                        if unified != current {
                            info.set_type(*dst, unified);
                            changed = true;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::CfgAnalysis;
    use crate::lifter::Cfg;
    use crate::ssa::SsaCfg;
    use hlbc::types::{RefFun, RefInt, RefString, RefType};

    fn create_mock_bytecode() -> Bytecode {
        // Minimal bytecode with some types
        let mut code = Bytecode::default();
        code.ints = vec![0, 1, 2, 42];
        code.strings = vec!["test".into()];
        code.types = vec![
            Type::Void,
            Type::I32, // Int at index 1
            Type::F64, // Float at index 2
            Type::Bool, // Bool at index 3
        ];
        code
    }

    fn create_mock_function(ops: &[Opcode], num_regs: usize) -> Function {
        Function {
            name: RefString(0),
            t: RefType(0),
            findex: RefFun(0),
            regs: vec![RefType(1); num_regs], // All Int
            ops: ops.to_vec(),
            debug_info: None,
            assigns: None,
            parent: None,
        }
    }

    #[test]
    fn test_basic_type_inference() {
        let code = create_mock_bytecode();
        let ops = vec![
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(3),
            }, // 42
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            }, // 1
            Opcode::Add {
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Opcode::Ret { ret: Reg(2) },
        ];

        let func = create_mock_function(&ops, 3);
        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        let ssa = SsaCfg::build(&func, &cfg, &analysis);

        let propagator = TypePropagator::new(&code, &func, &cfg, &ssa);
        let type_info = propagator.propagate();

        // Should have resolved some types
        assert!(type_info.resolved_count() > 0);
        // Should have no conflicts
        assert!(type_info.conflicts.is_empty());
    }

    #[test]
    fn test_phi_type_unification() {
        let code = create_mock_bytecode();
        // Diamond CFG with same type on both branches
        let ops = vec![
            Opcode::Int {
                dst: Reg(0),
                ptr: RefInt(0),
            },
            Opcode::JNull {
                reg: Reg(0),
                offset: 2,
            },
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(1),
            },
            Opcode::JAlways { offset: 1 },
            Opcode::Int {
                dst: Reg(1),
                ptr: RefInt(2),
            },
            Opcode::Ret { ret: Reg(1) },
        ];

        let func = create_mock_function(&ops, 2);
        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        let ssa = SsaCfg::build(&func, &cfg, &analysis);

        let propagator = TypePropagator::new(&code, &func, &cfg, &ssa);
        let type_info = propagator.propagate();

        // Should resolve types without conflicts
        assert!(type_info.conflicts.is_empty());
    }
}
