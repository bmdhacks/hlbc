//! SSA variable mapping, expression building, and inlining logic.
//!
//! This module handles the "leaf" logic of the structurer - turning registers
//! into SSA-versioned variable names or expressions. It doesn't care about
//! loops or if-statements; it only knows how to turn a register into a name.
//!
//! Key responsibilities:
//! - `reg_to_expr*` functions: Convert registers to expressions
//! - `ssa_var_name_*` functions: Generate SSA-versioned variable names
//! - `get_debug_name*` functions: Look up debug names from bytecode
//! - Inlining logic: ILSpy-style expression inlining with memory dependency tracking

use hlbc::opcodes::Opcode;
use hlbc::types::{Reg, RefField, Type};
use hlbc::Str;

use crate::ast::{Expr, Statement};
use crate::ssa::{SsaVar, get_dst_reg as get_opcode_dst};

use super::{MemoryDep, Structurer};

impl<'a> Structurer<'a> {
    /// Get register name (destination context).
    pub(super) fn reg_name(&self, reg: Reg) -> Str {
        // If this register was marked to use raw name (to avoid type conflicts),
        // always use the raw rN format
        if self.use_raw_name_regs.contains(&reg) {
            return format!("r{}", reg.0).into();
        }
        self.get_debug_name(reg, false)
            .unwrap_or_else(|| format!("r{}", reg.0))
            .into()
    }

    /// Get register name for source context (reading from register).
    /// Only uses debug names assigned BEFORE current_op.
    pub(super) fn reg_name_for_source(&self, reg: Reg) -> Str {
        // If this register was marked to use raw name (to avoid type conflicts),
        // always use the raw rN format
        if self.use_raw_name_regs.contains(&reg) {
            return format!("r{}", reg.0).into();
        }
        self.get_debug_name_at(reg, self.current_op, true)
            .unwrap_or_else(|| format!("r{}", reg.0))
            .into()
    }

    /// Get register name for source context at a specific opcode position.
    pub(super) fn reg_name_at(&self, reg: Reg, at_op: usize) -> Str {
        self.get_debug_name_at(reg, at_op, true)
            .unwrap_or_else(|| format!("r{}", reg.0))
            .into()
    }

    /// Get SSA-versioned variable name for a destination register.
    /// If a debug name exists, use it (preserves user variable names).
    /// For same-register phi destinations/sources, use non-SSA name (allows value flow-through).
    /// Otherwise, use SSA-versioned name like "r3_1" to enable inlining.
    pub(super) fn ssa_var_name_dst(&self, var: SsaVar) -> Str {
        // Check for raw name override first (set by CallMethod for .next() results)
        // Use non-versioned name for consistency
        if self.use_raw_name_regs.contains(&var.reg) {
            return format!("r{}", var.reg.0).into();
        }
        // If debug name exists, check for type conflicts before using it
        if let Some(debug_name) = self.get_debug_name(var.reg, false) {
            // Get the type of this register
            if let Some(&reg_type) = self.func.regs.get(var.reg.0 as usize) {
                let mut debug_name_types = self.debug_name_types.borrow_mut();
                if let Some(&existing_type) = debug_name_types.get(&debug_name) {
                    // Check if types differ - if so, use SSA-versioned name to avoid conflict
                    if existing_type != reg_type {
                        // Type conflict: same name, different types
                        // Use SSA-versioned name instead
                        return format!("r{}_{}", var.reg.0, var.version).into();
                    }
                } else {
                    // First use of this debug name - record its type
                    debug_name_types.insert(debug_name.clone(), reg_type);
                }
            }
            return debug_name.into();
        }
        // For same-register phi destinations or sources, use non-SSA name
        // This allows the value to flow through if/else branches without explicit phi assignments
        // BUT exclude dead phis - their sources should use SSA-versioned names since the phi is unused
        if self.ssa.is_same_register_phi(var) || self.ssa.is_same_register_phi_source_live(var, &self.dead_phis) {
            return format!("r{}", var.reg.0).into();
        }
        // No debug name → use SSA-versioned name
        format!("r{}_{}", var.reg.0, var.version).into()
    }

    /// Get SSA-versioned variable name for a source register.
    /// Uses source context (only debug names assigned BEFORE current_op).
    pub(super) fn ssa_var_name_src(&self, var: SsaVar) -> Str {
        self.ssa_var_name_src_at(var, self.current_op)
    }

    /// Get SSA-versioned variable name for a source register at a specific opcode position.
    /// Uses source context (only debug names assigned BEFORE at_op).
    pub(super) fn ssa_var_name_src_at(&self, var: SsaVar, at_op: usize) -> Str {
        // Check for raw name override first (set by CallMethod for .next() results)
        // Use non-versioned name to match the destination
        if self.use_raw_name_regs.contains(&var.reg) {
            return format!("r{}", var.reg.0).into();
        }
        // If debug name exists, check for type conflicts before using it
        if let Some(debug_name) = self.get_debug_name_at(var.reg, at_op, true) {
            // Get the type of this register
            if let Some(&reg_type) = self.func.regs.get(var.reg.0 as usize) {
                let debug_name_types = self.debug_name_types.borrow();
                if let Some(&existing_type) = debug_name_types.get(&debug_name) {
                    // Check if types differ - if so, use SSA-versioned name to avoid conflict
                    if existing_type != reg_type {
                        // Type conflict: same name, different types
                        // Use SSA-versioned name instead
                        return format!("r{}_{}", var.reg.0, var.version).into();
                    }
                }
                // No conflict or first use - use debug name
            }
            return debug_name.into();
        }
        // For same-register phi results or sources, use non-SSA name
        // This ensures consistency with phi destinations (allows value flow-through)
        // BUT exclude dead phis - their sources should use SSA-versioned names since the phi is unused
        if self.ssa.is_same_register_phi(var) || self.ssa.is_same_register_phi_source_live(var, &self.dead_phis) {
            return format!("r{}", var.reg.0).into();
        }
        // No debug name → use SSA-versioned name
        format!("r{}_{}", var.reg.0, var.version).into()
    }

    /// Create expression for destination register using SSA-versioned name.
    pub(super) fn reg_to_expr_ssa_dst(&self, reg: Reg, ssa_var: SsaVar) -> Expr {
        let name = self.ssa_var_name_dst(ssa_var);
        Expr::Variable(reg, Some(name))
    }

    /// Create expression for source register using SSA-versioned name.
    pub(super) fn reg_to_expr_ssa_src(&self, reg: Reg, ssa_var: SsaVar) -> Expr {
        let name = self.ssa_var_name_src(ssa_var);
        Expr::Variable(reg, Some(name))
    }

    /// Find the SSA variable for a source register in the current instruction's uses.
    pub(super) fn find_ssa_use(&self, reg: Reg) -> Option<SsaVar> {
        self.current_ssa_uses.iter().find(|v| v.reg == reg).copied()
    }

    /// Check if an SSA variable can be inlined and return its stored expression.
    /// Returns None if the variable shouldn't be inlined (multi-use, impure, has debug name, etc.)
    /// IMPORTANT: For non-constants, removes the expression from inline_exprs after returning it,
    /// since once inlined, it should not be invalidated or emitted as a separate statement.
    /// For constants, clones instead of removing since constants can be used multiple times.
    pub(super) fn try_get_inline_expr(&self, var: SsaVar) -> Option<Expr> {
        // First check if it's a constant - constants can be used multiple times
        {
            let inline_exprs = self.inline_exprs.borrow();
            if let Some(inline) = inline_exprs.get(&var) {
                if matches!(inline.expr, Expr::Constant(_)) {
                    return Some(inline.expr.clone());
                }
            }
        }

        // For non-constants, remove after retrieval (single use)
        self.inline_exprs.borrow_mut().remove(&var).map(|inline| inline.expr)
    }

    /// Check if a variable should be inlined based on use-def info and debug names.
    /// This implements the ILSpy-style inlining safety guards.
    pub(super) fn can_inline_var(&self, var: SsaVar) -> bool {
        // Check use-def info first
        let use_info = match self.use_info.get(&var) {
            Some(info) => info,
            None => return false,
        };

        if !use_info.can_inline() {
            return false;
        }

        // Guard 2 (Debug Name Barrier): Don't inline user-named variables
        // This preserves meaningful variable names in the output
        // Check if this register has a debug name at the current opcode
        if self.get_debug_name(var.reg, false).is_some() {
            return false;
        }

        true
    }

    /// Store an expression for potential inlining, or emit as assignment.
    /// If the current SSA destination variable can be inlined:
    ///   - Stores the expression with its memory dependency and definition context, returns None
    /// Otherwise:
    ///   - Returns Some(assignment statement)
    pub(super) fn try_inline_or_assign(&mut self, dst: Reg, expr: Expr) -> Option<Statement> {
        // Check if we have an SSA destination that can be inlined
        if let Some(ssa_var) = self.current_ssa_dst {
            if ssa_var.reg == dst && self.can_inline_var(ssa_var) {
                // Compute memory dependency for this expression based on source opcode
                let mem_dep = self.compute_memory_dep();
                // Get the definition block for escape analysis
                let def_block = self.cfg.block_for_op(self.current_op)
                    .expect("opcode should have a block");
                // Store for inlining with full context - don't emit statement
                let inline = super::InlineExpr {
                    expr,
                    mem_dep,
                    def_block,
                    def_scope: self.scope_depth,
                };
                self.inline_exprs.borrow_mut().insert(ssa_var, inline);
                return None;
            }
        }
        // Not inlinable - emit assignment statement
        let var = self.reg_to_expr_dst(dst);
        Some(self.make_assign(var, expr))
    }

    /// Compute memory dependency for the current opcode's result.
    /// This determines what memory the expression reads from, which is used
    /// to invalidate the inline when conflicting writes occur.
    pub(super) fn compute_memory_dep(&self) -> MemoryDep {
        let op = &self.func.ops[self.current_op];
        match op {
            // Field access depends on the specific field of the object
            Opcode::Field { obj, field, .. } => MemoryDep::Field { obj: *obj, field: field.0 },
            // Global access depends on the specific global
            Opcode::GetGlobal { global, .. } => MemoryDep::Global { global: *global },
            // Constants, moves, arithmetic - no memory dependency
            Opcode::Mov { .. }
            | Opcode::Int { .. }
            | Opcode::Float { .. }
            | Opcode::Bool { .. }
            | Opcode::String { .. }
            | Opcode::Null { .. }
            | Opcode::Bytes { .. }
            | Opcode::Add { .. }
            | Opcode::Sub { .. }
            | Opcode::Mul { .. }
            | Opcode::SDiv { .. }
            | Opcode::UDiv { .. }
            | Opcode::SMod { .. }
            | Opcode::UMod { .. }
            | Opcode::Shl { .. }
            | Opcode::SShr { .. }
            | Opcode::UShr { .. }
            | Opcode::And { .. }
            | Opcode::Or { .. }
            | Opcode::Xor { .. }
            | Opcode::Neg { .. }
            | Opcode::Not { .. }
            | Opcode::Incr { .. }
            | Opcode::Decr { .. }
            | Opcode::ToInt { .. }
            | Opcode::ToSFloat { .. }
            | Opcode::ToUFloat { .. }
            | Opcode::SafeCast { .. }
            | Opcode::UnsafeCast { .. }
            | Opcode::ToDyn { .. }
            | Opcode::GetType { .. }
            | Opcode::Type { .. }
            | Opcode::Ref { .. }
            | Opcode::EnumIndex { .. }
            | Opcode::GetTID { .. } => MemoryDep::None,
            // Calls can read any memory - conservative
            Opcode::Call0 { .. }
            | Opcode::Call1 { .. }
            | Opcode::Call2 { .. }
            | Opcode::Call3 { .. }
            | Opcode::Call4 { .. }
            | Opcode::CallN { .. }
            | Opcode::CallMethod { .. }
            | Opcode::CallThis { .. }
            | Opcode::CallClosure { .. }
            | Opcode::GetArray { .. }
            | Opcode::GetMem { .. }
            | Opcode::GetI8 { .. }
            | Opcode::GetI16 { .. }
            | Opcode::ArraySize { .. }
            | Opcode::Unref { .. }
            | Opcode::EnumAlloc { .. }
            | Opcode::New { .. }
            | Opcode::MakeEnum { .. }
            | Opcode::DynGet { .. }
            | Opcode::ToVirtual { .. } => MemoryDep::AnyMemory,
            // NullCheck doesn't read/write memory, just throws on null
            Opcode::NullCheck { .. } => MemoryDep::None,
            // Other opcodes - conservative default
            _ => MemoryDep::AnyMemory,
        }
    }

    /// Invalidate pending inline expressions that conflict with the given opcode.
    pub(super) fn invalidate_conflicting_inlines(&mut self, op: &Opcode) -> Vec<Statement> {
        self.invalidate_conflicting_inlines_excluding(op, None)
    }

    /// Invalidate with optional exclusion (for deferred call invalidation).
    pub(super) fn invalidate_conflicting_inlines_excluding(&mut self, op: &Opcode, exclude: Option<SsaVar>) -> Vec<Statement> {
        let conflicts_with: Box<dyn Fn(&MemoryDep) -> bool> = match op {
            // SetField invalidates any pending inline that reads the same field
            Opcode::SetField { obj, field, .. } => {
                let obj = *obj;
                let field_idx = field.0;
                Box::new(move |dep: &MemoryDep| {
                    matches!(dep, MemoryDep::Field { obj: dep_obj, field: dep_field }
                             if *dep_obj == obj && *dep_field == field_idx)
                })
            }
            // SetGlobal invalidates any pending inline that reads the same global
            Opcode::SetGlobal { global, .. } => {
                let global = *global;
                Box::new(move |dep: &MemoryDep| {
                    matches!(dep, MemoryDep::Global { global: dep_global }
                             if *dep_global == global)
                })
            }
            // Calls can modify any memory - invalidate Field, Global, and AnyMemory deps
            Opcode::Call0 { .. }
            | Opcode::Call1 { .. }
            | Opcode::Call2 { .. }
            | Opcode::Call3 { .. }
            | Opcode::Call4 { .. }
            | Opcode::CallN { .. }
            | Opcode::CallMethod { .. }
            | Opcode::CallThis { .. }
            | Opcode::CallClosure { .. } => {
                Box::new(|dep: &MemoryDep| !matches!(dep, MemoryDep::None))
            }
            // SetArray/SetMem/SetI* could modify anything accessed via pointers
            Opcode::SetArray { .. } | Opcode::SetMem { .. } | Opcode::Setref { .. }
            | Opcode::SetI8 { .. } | Opcode::SetI16 { .. } => {
                Box::new(|dep: &MemoryDep| !matches!(dep, MemoryDep::None))
            }
            // Other opcodes don't modify memory - no invalidation needed
            _ => return Vec::new(),
        };

        // Find all conflicting entries and emit them as statements
        let mut stmts = Vec::new();
        let mut to_remove = Vec::new();

        // Borrow for reading to find conflicts
        {
            let inline_exprs = self.inline_exprs.borrow();
            for (ssa_var, inline) in inline_exprs.iter() {
                if exclude == Some(*ssa_var) {
                    continue;
                }
                if conflicts_with(&inline.mem_dep) {
                    // Create assignment statement for the invalidated expression
                    let var_name: Str = ssa_var.name().into();
                    let var_expr = Expr::Variable(ssa_var.reg, Some(var_name.clone()));
                    // Ensure variable is declared
                    if !self.declared_vars.contains(&var_name) {
                        self.declared_vars.insert(var_name.clone());
                        self.hoisted_vars.insert(var_name.clone());
                        self.actually_used_vars.insert(var_name.clone());
                        // Track type for hoisted var
                        if let Some(tr) = self.func.regs.get(ssa_var.reg.0 as usize).copied() {
                            self.hoisted_var_types.insert(var_name, tr);
                        }
                    }
                    stmts.push(Statement::Assign {
                        declaration: false,
                        variable: var_expr,
                        assign: inline.expr.clone(),
                    });
                    to_remove.push(*ssa_var);
                }
            }
        }

        // Remove invalidated entries (separate borrow)
        {
            let mut inline_exprs = self.inline_exprs.borrow_mut();
            for var in to_remove {
                inline_exprs.remove(&var);
            }
        }

        stmts
    }

    /// Flush pending inline expressions that would escape into a conditional.
    /// Called before entering IfThenElse/Loop/Switch to prevent scope violations.
    ///
    /// The problem: when expressions with `MemoryDep::AnyMemory` (like call results)
    /// are stored for inlining before a conditional, they can be incorrectly invalidated
    /// and flushed **inside** a branch instead of before it. This causes "variable used
    /// without being initialized" errors when the actual use is after the merge point.
    ///
    /// Solution: before entering branches, flush expressions that:
    /// - Are defined at the current scope depth (not deeper)
    /// - Have AnyMemory dependency (could be invalidated by calls inside branches)
    pub(super) fn flush_escaping_inlines(&mut self) -> Vec<Statement> {
        let mut stmts = Vec::new();
        let mut to_remove = Vec::new();

        {
            let inline_exprs = self.inline_exprs.borrow();
            for (ssa_var, inline) in inline_exprs.iter() {
                // Escape condition: defined at current scope with AnyMemory dep
                // These could be invalidated by calls inside branches, causing
                // the assignment to appear inside a branch instead of before it
                if inline.def_scope == self.scope_depth
                   && matches!(inline.mem_dep, MemoryDep::AnyMemory) {
                    // Create assignment statement
                    let var_name: Str = ssa_var.name().into();
                    let var_expr = Expr::Variable(ssa_var.reg, Some(var_name.clone()));

                    // Ensure variable is declared
                    if !self.declared_vars.contains(&var_name) {
                        self.declared_vars.insert(var_name.clone());
                        self.hoisted_vars.insert(var_name.clone());
                        self.actually_used_vars.insert(var_name.clone());
                        if let Some(tr) = self.func.regs.get(ssa_var.reg.0 as usize).copied() {
                            self.hoisted_var_types.insert(var_name, tr);
                        }
                    }

                    stmts.push(Statement::Assign {
                        declaration: false,
                        variable: var_expr,
                        assign: inline.expr.clone(),
                    });
                    to_remove.push(*ssa_var);
                }
            }
        }

        // Remove flushed entries
        let mut inline_exprs = self.inline_exprs.borrow_mut();
        for var in to_remove {
            inline_exprs.remove(&var);
        }

        stmts
    }

    /// Get debug name for a register, optionally for source context.
    /// When `for_source` is true, only returns names assigned BEFORE current_op.
    /// This prevents using a name before it's been assigned (e.g., `var dx = dx - r3`).
    pub(super) fn get_debug_name(&self, reg: Reg, for_source: bool) -> Option<String> {
        self.get_debug_name_at(reg, self.current_op, for_source)
    }

    /// Get debug name for a register at a specific opcode position.
    pub(super) fn get_debug_name_at(&self, reg: Reg, at_op: usize, for_source: bool) -> Option<String> {
        let reg_idx = reg.0 as usize;

        // First, check if this is a function parameter
        // Parameters are the first N registers where N = number of function args
        if let Some(Type::Fun(fun_type) | Type::Method(fun_type)) = self.code.types.get(self.func.t.0) {
            let num_args = fun_type.args.len();
            if reg_idx < num_args {
                // Check if first param is implicit:
                // - `this` for instance methods/this-bound closures (is_this_bound_closure)
                // - Enum capture context for closures (first arg is enum AND we're in a closure)
                // Note: Regular enums as parameters (like Color) are NOT implicit
                let first_is_capture_context = fun_type.args.first().map(|t| {
                    matches!(self.code.types.get(t.0), Some(hlbc::types::Type::Enum { .. }))
                }).unwrap_or(false) && self.is_current_function_closure();
                let has_implicit_first = self.is_this_bound_closure || first_is_capture_context;

                // is_this_bound_closure covers:
                // - Constructors (name starts with "__constructor__")
                // - Instance methods (first arg type matches parent type)
                // - This-bound closures (InstanceClosure opcode)
                if reg_idx == 0 && self.is_this_bound_closure {
                    return Some("this".to_string());
                }

                // Skip implicit first param (enum context or `this`) when looking up arg name
                // For reg0 when implicit, there's no debug name to look up
                if !(has_implicit_first && reg_idx == 0) {
                    let arg_name_pos = if has_implicit_first { reg_idx - 1 } else { reg_idx };
                    if let Some(name) = self.func.arg_name(self.code, arg_name_pos) {
                        if self.is_valid_identifier(&name) {
                            return Some(name.to_string());
                        }
                    }
                }

                // For closure inner functions (first param is enum context),
                // generate synthetic names that match fmt.rs output.
                // fmt.rs uses arg{counter} where counter increments for each param without debug name.
                // Since closures typically have no debug names, this is arg0, arg1, etc.
                if self.is_current_function_closure() {
                    // Count how many params before this one have no debug name
                    let mut counter = 0;
                    for i in 0..=reg_idx {
                        if self.func.arg_name(self.code, i).is_none() {
                            if i == reg_idx {
                                return Some(format!("arg{}", counter));
                            }
                            counter += 1;
                        } else if i == reg_idx {
                            // This param has a debug name but we didn't return it above
                            // (probably invalid identifier), fall through
                            break;
                        }
                    }
                }

                // For regular functions with unnamed parameters, use "_" to match fmt.rs
                // This ensures the body uses the same name as the function signature
                return Some("_".to_string());
            }
        }

        // assigns is Vec<(RefString name, usize op_idx)>
        // IMPORTANT: op_idx points to the opcode AFTER the definition.
        // The actual definition is at op_idx - 1.
        // This is a HashLink convention where assigns mark scope boundaries.
        //
        // We need to find the MOST RECENT assignment to this register that happened
        // AT OR BEFORE the current opcode. This ensures we get the right variable name
        // when a register is reused for different variables.
        if let Some(assigns) = &self.func.assigns {
            let mut best_name: Option<String> = None;
            let mut best_def_idx: usize = 0;

            for (str_ref, op_idx) in assigns {
                // Skip assigns at op_idx 0 - these are parameter names, handled above
                if *op_idx == 0 {
                    continue;
                }

                // The definition is at the previous opcode
                let def_idx = op_idx.saturating_sub(1);

                // For source registers, only use names assigned BEFORE at_op
                // For destination registers, use names assigned at or before at_op
                // This prevents `var dx = dx - r3` when dx is being defined at at_op
                if for_source {
                    if def_idx >= at_op {
                        continue;
                    }
                } else {
                    if def_idx > at_op {
                        continue;
                    }
                }

                if def_idx < self.func.ops.len() {
                    if let Some(dst_reg) = get_opcode_dst(&self.func.ops[def_idx]) {
                        if dst_reg == reg {
                            if let Some(name) = self.code.strings.get(str_ref.0) {
                                // Validate that this looks like a real identifier
                                // (not a string constant value like "Hello.hx")
                                if self.is_valid_identifier(name) {
                                    // Keep the most recent (highest def_idx) name
                                    if best_name.is_none() || def_idx >= best_def_idx {
                                        best_name = Some(name.to_string());
                                        best_def_idx = def_idx;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Don't use a name that would shadow a parameter
            if let Some(ref name) = best_name {
                if self.name_conflicts_with_param(name) {
                    return None;
                }
            }
            return best_name;
        }
        None
    }

    /// Check if a string is a valid Haxe identifier (not a constant value)
    pub(super) fn is_valid_identifier(&self, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        // Valid identifiers: start with letter or underscore, contain only alphanumeric/underscore
        let first = name.chars().next().unwrap();
        if !first.is_alphabetic() && first != '_' {
            return false;
        }
        // Reject if contains dots, spaces, or looks like a file path
        if name.contains('.') || name.contains(' ') || name.contains('/') {
            return false;
        }
        // Must be all alphanumeric/underscore
        name.chars().all(|c| c.is_alphanumeric() || c == '_')
    }

    /// Check if a name conflicts with a function parameter name.
    /// Returns true if the name would shadow a parameter.
    pub(super) fn name_conflicts_with_param(&self, name: &str) -> bool {
        if let Some(Type::Fun(fun_type) | Type::Method(fun_type)) = self.code.types.get(self.func.t.0) {
            let num_args = fun_type.args.len();
            for i in 0..num_args {
                if let Some(param_name) = self.func.arg_name(self.code, i) {
                    if param_name == name {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Get debug name for a register at a specific definition point.
    /// Returns the debug name only if there's an assigns entry that defines
    /// this register at exactly the given opcode.
    pub(super) fn get_debug_name_for_def(&self, reg: Reg, def_op: usize) -> Option<String> {
        if let Some(assigns) = &self.func.assigns {
            for (str_ref, op_idx) in assigns {
                // Skip assigns at op_idx 0 - these are parameter names
                if *op_idx == 0 {
                    continue;
                }

                // The definition is at the previous opcode
                let assign_def = op_idx.saturating_sub(1);

                // Check if this assign matches the definition point
                if assign_def != def_op {
                    continue;
                }

                if def_op < self.func.ops.len() {
                    if let Some(dst_reg) = get_opcode_dst(&self.func.ops[def_op]) {
                        if dst_reg == reg {
                            if let Some(name) = self.code.strings.get(str_ref.0) {
                                if self.is_valid_identifier(name) && !self.name_conflicts_with_param(name) {
                                    return Some(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Get field name from type information.
    pub(super) fn get_field_name(&self, obj_reg: Reg, field: RefField) -> Str {
        // Try to look up field name from type
        let reg_idx = obj_reg.0 as usize;
        if reg_idx < self.func.regs.len() {
            let type_ref = self.func.regs[reg_idx];
            if let Some(ty) = self.code.types.get(type_ref.0) {
                // Get fields from either Obj or Virtual types
                let fields: Option<&[hlbc::types::ObjField]> = match ty {
                    hlbc::types::Type::Obj(obj) => Some(&obj.fields),
                    hlbc::types::Type::Virtual { fields } => Some(fields),
                    _ => None,
                };
                if let Some(fields) = fields {
                    if let Some(f) = fields.get(field.0) {
                        if let Some(name) = self.code.strings.get(f.name.0) {
                            // Empty names are interface implementation cache fields
                            if !name.is_empty() {
                                return name.clone();
                            }
                        }
                    }
                }
            }
        }
        // Fallback for unknown or unnamed fields
        format!("__field_{}", field.0).into()
    }

    /// Get method name from a pindex (vtable index).
    /// For CallMethod, 'field' is a pindex, not a direct array index into protos.
    pub(super) fn get_proto_name(&self, obj_reg: Reg, proto_idx: RefField) -> Str {
        let reg_idx = obj_reg.0 as usize;
        if reg_idx < self.func.regs.len() {
            let type_ref = self.func.regs[reg_idx];

            // For Obj types, use TypeRef::method() to resolve by pindex with inheritance
            if let Some(proto) = type_ref.method(proto_idx.0, self.code) {
                if let Some(name) = self.code.strings.get(proto.name.0) {
                    return name.clone();
                }
            }

            // For Virtual types, proto_idx is an index into fields (not pindex)
            if let Some(ty) = self.code.types.get(type_ref.0) {
                if let hlbc::types::Type::Virtual { fields } = ty {
                    if let Some(f) = fields.get(proto_idx.0) {
                        if let Some(name) = self.code.strings.get(f.name.0) {
                            if !name.is_empty() {
                                return name.clone();
                            }
                        }
                    }
                }
            }
        }
        // Fallback
        format!("method_{}", proto_idx.0).into()
    }

    /// Get expression for reading from a register (source context).
    /// Uses SSA-versioned names when available to enable inlining.
    /// Falls back to debug names assigned BEFORE current_op.
    pub(super) fn reg_to_expr(&self, reg: Reg) -> Expr {
        // Use SSA-versioned name if we have SSA context for this register
        if let Some(ssa_var) = self.find_ssa_use(reg) {
            // Check if this variable has an expression available for inlining
            if let Some(inline_expr) = self.try_get_inline_expr(ssa_var) {
                return inline_expr;
            }
            return self.reg_to_expr_ssa_src(reg, ssa_var);
        }
        // Fallback to non-SSA naming
        let name = self.reg_name_for_source(reg);
        Expr::Variable(reg, Some(name))
    }

    /// Get expression for writing to a register (destination context).
    /// Uses SSA-versioned names when available to enable inlining.
    /// Falls back to debug names assigned at or before current_op.
    pub(super) fn reg_to_expr_dst(&self, reg: Reg) -> Expr {
        // Use SSA-versioned name if we have SSA context and the register matches
        if let Some(ssa_var) = self.current_ssa_dst {
            if ssa_var.reg == reg {
                return self.reg_to_expr_ssa_dst(reg, ssa_var);
            }
        }
        // Fallback to non-SSA naming
        let name = self.reg_name(reg);
        Expr::Variable(reg, Some(name))
    }

    /// Get expression for a register at the END of a block (for loop conditions).
    /// Uses SSA to find the correct value - the one used by the conditional jump.
    pub(super) fn reg_to_expr_in_block(&self, reg: Reg, block: petgraph::graph::NodeIndex) -> Expr {
        let blk = &self.cfg.graph[block];

        // Use SSA: look up which version of this register is used at block.end
        // The conditional jump is at block.end, so we want the SSA variable used there
        if let Some((_dst, uses)) = self.ssa.get_instr_for_op(blk.end) {
            for ssa_var in uses {
                if ssa_var.reg == reg {
                    // Check if this variable has an inlined expression.
                    // If the variable was marked for inlining, its defining statement was
                    // suppressed, so we must return the stored expression, not a variable name.
                    if let Some(inline_expr) = self.try_get_inline_expr(*ssa_var) {
                        return inline_expr;
                    }

                    // No inlined expression available - use SSA-versioned name.
                    // NOTE: We intentionally do NOT do direct constant lookup here.
                    // If a constant wasn't stored for inlining, it means its assignment
                    // statement was emitted (e.g., because it flows to a phi function),
                    // so we must reference the variable, not inline the constant again.
                    let name = self.ssa_var_name_src_at(*ssa_var, blk.end);
                    return Expr::Variable(reg, Some(name));
                }
            }
        }

        // Fallback: return as variable using non-SSA naming
        let name = self.reg_name_at(reg, blk.end);
        Expr::Variable(reg, Some(name))
    }

    /// Get the constant value of a register at a specific opcode using SSA information.
    /// Looks up which SSA variable is used for this register at this op,
    /// then finds its definition and checks if it's a constant.
    pub(super) fn get_ssa_constant_value(&self, reg: Reg, at_op: usize) -> Option<i32> {
        // Get the SSA instruction for this opcode
        if let Some((_dst, uses)) = self.ssa.get_instr_for_op(at_op) {
            // Find the SSA variable for this register in the uses
            for ssa_var in uses {
                if ssa_var.reg == reg {
                    // Found the SSA variable for this register
                    // Now find its defining instruction
                    if let Some(def_op_idx) = self.ssa.find_def(*ssa_var) {
                        // Check if the defining instruction is a constant
                        if let Opcode::Int { ptr, .. } = &self.func.ops[def_op_idx] {
                            return self.code.ints.get(ptr.0).map(|&v| v);
                        }
                    }
                    break;
                }
            }
        }
        None
    }

    /// Get an expression for a register using SSA information to find the correct definition.
    /// This is used when debug names might be unreliable due to control flow (e.g., bounds check branches).
    ///
    /// For phi-involved variables (loop counters), use the debug name since it's consistent across the loop.
    /// For non-phi variables, use raw SSA-versioned names to avoid picking up debug names from
    /// different control flow paths (e.g., bounds check failure path).
    pub(super) fn get_ssa_based_expr(&self, reg: Reg, at_op: usize) -> Expr {
        // Get the SSA instruction for this opcode
        if let Some((_dst, uses)) = self.ssa.get_instr_for_op(at_op) {
            // Find the SSA variable for this register in the uses
            for ssa_var in uses {
                if ssa_var.reg == reg {
                    // IMPORTANT: Check if this variable has an inlined expression first.
                    // If the variable was marked for inlining, its defining statement was
                    // suppressed, so we must return the stored expression, not a variable name.
                    if let Some(inline_expr) = self.try_get_inline_expr(*ssa_var) {
                        return inline_expr;
                    }

                    // Found the SSA variable for this register
                    // Check if phi-involved, but exclude dead phis
                    let is_phi_involved = self.ssa.is_same_register_phi(*ssa_var) || self.ssa.is_same_register_phi_source_live(*ssa_var, &self.dead_phis);

                    let name: Str = if is_phi_involved {
                        // For phi-involved variables (loop counters), use debug name if available
                        // since it's consistent across the loop
                        if let Some(debug_name) = self.get_debug_name_at(reg, at_op, true) {
                            debug_name.into()
                        } else {
                            format!("r{}", reg.0).into()
                        }
                    } else {
                        // For non-phi variables, be careful about debug names.
                        // Debug names from different branches can be misleading
                        // (e.g., bounds check failure path assigns "last" to default value).
                        //
                        // HOWEVER: if the debug name's definition point matches the SSA
                        // variable's definition, the name is valid and should be used.
                        // This handles the case where dead phi detection removes a phi but
                        // the variable still has a valid debug name from its definition.
                        //
                        // Also safe: Function parameters (version 0) are always safe since
                        // they're defined at function entry before any branches.
                        if ssa_var.version == 0 {
                            // Check if this is a function parameter
                            if let Some(debug_name) = self.get_debug_name_at(reg, at_op, true) {
                                debug_name.into()
                            } else {
                                format!("r{}_{}", reg.0, ssa_var.version).into()
                            }
                        } else {
                            // Non-parameter, non-phi: check if we have a debug name that matches
                            // the SSA definition point
                            if let Some(def_op) = self.ssa.find_def(*ssa_var) {
                                // Check if there's a debug name defined at this op
                                // The debug name's def_idx should equal the SSA def_op
                                if let Some(debug_name) = self.get_debug_name_for_def(reg, def_op) {
                                    return Expr::Variable(reg, Some(debug_name.into()));
                                }
                            }
                            // Fall back to raw SSA-versioned name
                            format!("r{}_{}", reg.0, ssa_var.version).into()
                        }
                    };
                    return Expr::Variable(reg, Some(name));
                }
            }
        }
        // Fallback: use normal naming. This works for:
        // - Loop variables (phi-defined) where the debug name is valid at this point
        // - Cases where SSA doesn't have the information
        self.reg_to_expr(reg)
    }
}
