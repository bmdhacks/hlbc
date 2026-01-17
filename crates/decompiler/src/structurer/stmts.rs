//! Opcode-to-Statement translation.
//!
//! This module handles the translation of individual HashLink opcodes into
//! AST statements. It contains:
//! - `opcode_to_statements` - The main match that handles all opcodes
//! - Statement builder helpers (make_assign, make_call_stmt)
//! - Global/type helpers (get_global_name, get_type_ref, etc.)
//! - Statement simplification passes

use std::collections::HashMap;

use hlbc::opcodes::Opcode;
use hlbc::types::{Reg, RefFun, RefType, Type};
use hlbc::{Resolve, Str};

use crate::ast::{Call, Constant, ConstructorCall, Expr, Operation, Statement};

use super::Structurer;

impl<'a> Structurer<'a> {
    /// Check if an opcode is a control flow instruction.
    pub(super) fn is_control_flow_op(&self, op_idx: usize) -> bool {
        matches!(
            &self.func.ops[op_idx],
            Opcode::JTrue { .. }
                | Opcode::JFalse { .. }
                | Opcode::JNull { .. }
                | Opcode::JNotNull { .. }
                | Opcode::JSLt { .. }
                | Opcode::JSGte { .. }
                | Opcode::JSLte { .. }
                | Opcode::JSGt { .. }
                | Opcode::JULt { .. }
                | Opcode::JUGte { .. }
                | Opcode::JNotLt { .. }
                | Opcode::JNotGte { .. }
                | Opcode::JEq { .. }
                | Opcode::JNotEq { .. }
                | Opcode::JAlways { .. }
                | Opcode::Switch { .. }
        )
    }

    /// Check if a block ends with a terminal instruction (Ret or Throw).
    pub(super) fn check_terminal(&self, op_idx: usize) -> Option<Statement> {
        match &self.func.ops[op_idx] {
            Opcode::Ret { ret } => {
                // Don't return void-typed values
                if self.is_void_type(*ret) {
                    Some(Statement::Return(None))
                } else {
                    Some(Statement::Return(Some(self.reg_to_expr(*ret))))
                }
            }
            Opcode::Throw { exc } => Some(Statement::Throw(self.reg_to_expr(*exc))),
            _ => None,
        }
    }

    /// Create an assignment statement, tracking declaration status.
    /// Returns a Statement::Assign with declaration=true if this is the first
    /// assignment to this variable name.
    ///
    /// IMPORTANT: Variables are only declared (with `var`) at scope depth 0 to avoid
    /// scoping issues where a variable declared inside a loop is not visible outside.
    pub(super) fn make_assign(&mut self, variable: Expr, assign: Expr) -> Statement {
        // Check for self-assignment (x = x) where different registers map to the same name
        // This can happen when SSA/variable naming gives multiple registers the same name
        let is_self_assign = match (&variable, &assign) {
            (Expr::Variable(_, Some(lhs)), Expr::Variable(_, Some(rhs))) => lhs == rhs,
            (Expr::Ident(lhs), Expr::Ident(rhs)) => lhs == rhs,
            (Expr::Variable(_, Some(lhs)), Expr::Ident(rhs)) => lhs == rhs,
            (Expr::Ident(lhs), Expr::Variable(_, Some(rhs))) => lhs == rhs,
            _ => false,
        };
        if is_self_assign {
            // Return a no-op comment statement instead of self-assignment
            return Statement::Comment("// self-assign elided".into());
        }

        // Extract variable name to check if it's been declared
        // ONLY simple variables can have declarations (var x = ...)
        // Field access, array index, etc. are NEVER declarations
        //
        // Haxe has block-level scoping. Variables declared inside a loop/if are
        // NOT visible outside. So if we're inside a scope (scope_depth > 0),
        // we don't emit `var` inline - instead we track it for hoisting to
        // function level.
        // Extract the actual variable name from the variable expression
        // Handle TypeAnnotated by unwrapping to the inner variable
        let (var_name, is_typed) = match &variable {
            Expr::Variable(_, Some(name)) | Expr::Ident(name) => (Some(name.clone()), false),
            Expr::TypeAnnotated(inner, _) => {
                match inner.as_ref() {
                    Expr::Variable(_, Some(name)) | Expr::Ident(name) => (Some(name.clone()), true),
                    _ => (None, true),
                }
            }
            _ => (None, false),
        };

        let is_declaration = match var_name {
            Some(name) => {
                if self.declared_vars.contains(&name) {
                    // Already declared - but if assigning empty object, track for :Dynamic
                    if Self::is_empty_anonymous(&assign) {
                        self.needs_dynamic_type.insert(name.clone());
                    }
                    false
                } else if self.scope_depth > 0 && !is_typed {
                    // Inside a scope and no explicit type - don't declare inline, hoist instead
                    // (But if type-annotated, we want to declare it with the type)
                    self.hoisted_vars.insert(name.clone());
                    self.declared_vars.insert(name.clone());
                    // Track the type for type hints
                    if let Some(ssa_var) = self.current_ssa_dst {
                        let type_ref = self.func.regs.get(ssa_var.reg.0 as usize).copied();
                        if let Some(tr) = type_ref {
                            self.hoisted_var_types.insert(name.clone(), tr);
                        }
                    }
                    // Track if this hoisted var needs :Dynamic
                    if Self::is_empty_anonymous(&assign) {
                        self.needs_dynamic_type.insert(name.clone());
                    }
                    false
                } else {
                    // At function level or type-annotated - declare normally
                    self.declared_vars.insert(name.clone());
                    true
                }
            }
            None => {
                // Field access (obj.field), Array access (arr[i]), etc. - never a declaration
                false
            }
        };

        // Clear array_bytes_source mapping when a register is reassigned.
        // This is important because registers can be reused (e.g., alloc_bytes reuses a reg
        // that previously held array.bytes from a different array).
        let reg_to_clear = match &variable {
            Expr::Variable(reg, _) => Some(*reg),
            Expr::TypeAnnotated(inner, _) => match inner.as_ref() {
                Expr::Variable(reg, _) => Some(*reg),
                _ => None,
            },
            _ => None,
        };
        if let Some(reg) = reg_to_clear {
            self.array_bytes_source.remove(&reg);
        }

        Statement::Assign {
            declaration: is_declaration,
            variable,
            assign,
        }
    }

    /// Check if an expression is an empty anonymous object (needs :Dynamic type)
    pub(super) fn is_empty_anonymous(expr: &Expr) -> bool {
        matches!(expr, Expr::Anonymous(_, fields) if fields.is_empty())
    }

    /// Create a call statement, handling void return types correctly.
    /// For void functions, we emit just the call as an expression statement.
    /// For non-void functions, we assign the result to a variable.
    pub(super) fn make_call_stmt(&mut self, dst: Reg, call: Call) -> Statement {
        if self.is_void_type(dst) {
            Statement::ExprStatement(Expr::Call(Box::new(call)))
        } else {
            let var = self.reg_to_expr_dst(dst);
            self.make_assign(var, Expr::Call(Box::new(call)))
        }
    }

    /// Check if an SSA variable is dead (defined but never used)
    pub(super) fn is_dead_var(&self, var: crate::ssa::SsaVar) -> bool {
        self.use_info.get(&var).map_or(false, |info| info.is_dead())
    }

    /// Check if a register is a closure context (EnumAlloc result for a closure)
    pub(super) fn is_closure_context_reg(&self, reg: Reg) -> bool {
        if let Some(analysis) = self.closure_analysis {
            analysis.is_context_reg(self.func.findex, reg)
        } else {
            false
        }
    }

    /// Get closure info if the current opcode is an InstanceClosure that we should inline
    pub(super) fn get_closure_at_current_op(&self) -> Option<&crate::closure_analysis::CaptureInfo> {
        if let Some(analysis) = self.closure_analysis {
            analysis.get_closure_at(self.func.findex, self.current_op)
        } else {
            None
        }
    }

    /// Check if the current function is a closure (inner function with capture context)
    pub(super) fn is_current_function_closure(&self) -> bool {
        if let Some(analysis) = self.closure_analysis {
            analysis.is_closure(self.func.findex)
        } else {
            false
        }
    }

    /// Get capture info for the current function if it's a closure
    pub(super) fn get_current_capture_info(&self) -> Option<&crate::closure_analysis::CaptureInfo> {
        if let Some(analysis) = self.closure_analysis {
            analysis.get_capture_info(self.func.findex)
        } else {
            None
        }
    }

    /// Get captured variable name from an EnumField access in a closure body
    pub(super) fn get_captured_var_name(&self, field: hlbc::types::RefField) -> Option<Str> {
        if let Some(capture_info) = self.get_current_capture_info() {
            for cap in &capture_info.captures {
                if cap.field_index == field {
                    if let Some(ref name) = cap.name {
                        return Some(name.clone().into());
                    }
                }
            }
        }
        None
    }

    /// Get global name from the bytecode.
    pub(super) fn get_global_name(&self, global: hlbc::types::RefGlobal) -> Str {
        if let Some(ty) = self.code.globals.get(global.0) {
            match &self.code.types[ty.0] {
                hlbc::types::Type::Obj(obj) => {
                    let raw_name = self.code.strings.get(obj.name.0)
                        .cloned()
                        .unwrap_or_else(|| format!("global_{}", global.0).into());
                    // Strip internal $ from type names like "haxe.$Log" -> "haxe.Log"
                    // or "$Counter" -> "Counter"
                    return self.clean_internal_name(&raw_name);
                }
                hlbc::types::Type::Struct(obj) => {
                    let raw_name = self.code.strings.get(obj.name.0)
                        .cloned()
                        .unwrap_or_else(|| format!("global_{}", global.0).into());
                    return self.clean_internal_name(&raw_name);
                }
                hlbc::types::Type::Enum { name, constructs, .. } => {
                    // For enum globals, look up the constructor from the constants table
                    let enum_name = self.code.strings.get(name.0)
                        .cloned()
                        .unwrap_or_else(|| "Enum".into());

                    // Check if we have constant initializer data for this global
                    if let Some(&const_idx) = self.code.globals_initializers.get(&global) {
                        if let Some(constants) = &self.code.constants {
                            if let Some(const_def) = constants.get(const_idx) {
                                // First field is the constructor index
                                if let Some(&construct_idx) = const_def.fields.first() {
                                    if let Some(construct) = constructs.get(construct_idx) {
                                        let construct_name = self.code.strings.get(construct.name.0)
                                            .cloned()
                                            .unwrap_or_else(|| format!("Construct{}", construct_idx).into());
                                        return format!("{}.{}", enum_name, construct_name).into();
                                    }
                                }
                            }
                        }
                    }

                    // Check if we have enum_global_map entry (from init function analysis)
                    if let Some((_enum_type, construct_idx)) = self.enum_global_map.get(&global) {
                        if let Some(construct) = constructs.get(*construct_idx) {
                            let construct_name = self.code.strings.get(construct.name.0)
                                .cloned()
                                .unwrap_or_else(|| format!("Construct{}", construct_idx).into());
                            return format!("{}.{}", enum_name, construct_name).into();
                        }
                    }

                    // Fallback: just use enum name
                    return enum_name.into();
                }
                _ => {}
            }
        }
        format!("global_{}", global.0).into()
    }

    /// Clean internal names by stripping $ prefix from components.
    /// e.g., "haxe.$Log" -> "haxe.Log", "$Counter" -> "Counter"
    pub(super) fn clean_internal_name(&self, name: &str) -> Str {
        if name.contains(".$") {
            // Replace .$X with .X
            name.replace(".$", ".").into()
        } else if name.starts_with('$') {
            // Strip leading $
            name[1..].into()
        } else {
            name.into()
        }
    }

    /// Check if a global is a string constant and return its string reference
    pub(super) fn get_global_string_value(&self, global: hlbc::types::RefGlobal) -> Option<hlbc::types::RefString> {
        // Check if global has a constant initializer
        let &const_idx = self.code.globals_initializers.get(&global)?;
        let constants = self.code.constants.as_ref()?;
        let constant_def = constants.get(const_idx)?;

        // Check if the global's type is String (an Obj type named "String")
        let type_ref = self.code.globals.get(global.0)?;
        if let hlbc::types::Type::Obj(obj) = &self.code.types[type_ref.0] {
            let name = self.code.strings.get(obj.name.0)?;
            if name.as_ref() == "String" {
                // For String type, fields[0] is the string pool index
                let string_idx = constant_def.fields.first()?;
                return Some(hlbc::types::RefString(*string_idx));
            }
        }
        None
    }

    /// Convert a global reference to an expression.
    /// Checks for string constants first, otherwise returns an identifier.
    pub(super) fn global_to_expr(&self, global: hlbc::types::RefGlobal) -> Expr {
        if let Some(string_ref) = self.get_global_string_value(global) {
            Expr::Constant(Constant::String(string_ref))
        } else {
            Expr::Ident(self.get_global_name(global))
        }
    }

    /// Get the type reference for a register.
    pub(super) fn get_type_ref(&self, reg: Reg) -> RefType {
        let reg_idx = reg.0 as usize;
        if reg_idx < self.func.regs.len() {
            self.func.regs[reg_idx]
        } else {
            RefType(0) // Fallback to void type
        }
    }

    /// Try to create a method call from a function reference and arguments.
    /// If the function is a method (first arg is `this` of the owner type),
    /// returns a Call with obj.method(rest_args) syntax.
    /// Otherwise returns None and the caller should use normal function call syntax.
    pub(super) fn try_make_method_call(&self, fun: RefFun, args: &[Reg]) -> Option<Call> {
        // Check if this function is a method
        let (owner_type, method_name) = self.method_info.get(&fun)?;

        // Must have at least one argument (the object)
        if args.is_empty() {
            return None;
        }

        // Check if the first argument's type is compatible with the owner type
        // (either the same type or a subtype that inherits from it)
        let first_arg_type = self.get_type_ref(args[0]);
        if !self.is_subtype_of(first_arg_type, *owner_type) {
            return None;
        }

        // Create method call: obj.method(rest_args)
        let obj = self.reg_to_expr(args[0]);
        let method = Expr::Field(Box::new(obj), method_name.clone());
        let arg_exprs: Vec<_> = args[1..].iter().map(|r| self.reg_to_expr(*r)).collect();

        Some(Call { fun: method, args: arg_exprs })
    }

    /// Check if `derived` type is the same as or inherits from `base` type.
    /// Walks the inheritance chain via TypeObj.super_.
    pub(super) fn is_subtype_of(&self, derived: RefType, base: RefType) -> bool {
        if derived == base {
            return true;
        }

        // Walk up the inheritance chain
        let mut current = derived;
        loop {
            if let Some(Type::Obj(obj)) = self.code.types.get(current.0) {
                if let Some(parent) = obj.super_ {
                    if parent == base {
                        return true;
                    }
                    current = parent;
                } else {
                    // No more parents
                    break;
                }
            } else {
                // Not an object type
                break;
            }
        }
        false
    }

    /// Check if a register has void type (used to skip assignments of void-returning calls)
    pub(super) fn is_void_type(&self, reg: Reg) -> bool {
        let type_ref = self.get_type_ref(reg);
        matches!(&self.code.types[type_ref.0], hlbc::types::Type::Void)
    }

    /// Check if a register holds an array type (hl.types.ArrayBytes_*, etc.)
    pub(super) fn is_array_type(&self, reg: Reg) -> bool {
        let reg_idx = reg.0 as usize;
        if reg_idx < self.func.regs.len() {
            let type_ref = self.func.regs[reg_idx];
            if let Some(ty) = self.code.types.get(type_ref.0) {
                if let hlbc::types::Type::Obj(obj) = ty {
                    if let Some(name) = self.code.strings.get(obj.name.0) {
                        // HashLink array types have names like "hl.types.ArrayBytes_Int"
                        return name.contains("Array");
                    }
                }
            }
        }
        false
    }

    /// Check if a type register (from alloc_array arg0) holds a nullable element type
    /// by looking back at the Type opcode that set it
    pub(super) fn is_nullable_element_type(&self, type_reg: Reg) -> bool {
        // Look back at instructions to find the Type opcode that set this register
        for idx in (0..self.current_op).rev() {
            if let Some(Opcode::Type { dst, ty }) = self.func.ops.get(idx) {
                if *dst == type_reg {
                    // Found the Type opcode that set this register
                    // Check if the type is nullable (null<T>) or dynamic
                    if let Some(element_type) = self.code.types.get(ty.0) {
                        return matches!(element_type, Type::Null(_) | Type::Dyn | Type::DynObj);
                    }
                }
            }
        }
        false
    }

    /// Check if the current function being decompiled is a constructor
    pub(super) fn is_current_function_constructor(&self) -> bool {
        self.code.strings.get(self.func.name.0)
            .map(|s| s.starts_with("__constructor__"))
            .unwrap_or(false)
    }

    /// Check if calling a function with this as first arg is a super method call.
    /// This is true when:
    /// 1. Current function has the same name as the target function
    /// 2. Current function is an override (has parent)
    /// 3. Target function belongs to the parent class
    pub(super) fn is_super_method_call(&self, fun: RefFun) -> bool {
        // Get current function name
        let current_name = match self.code.strings.get(self.func.name.0) {
            Some(n) => n.clone(),
            None => return false,
        };

        // Get target function
        let target_func = match fun.as_fn(self.code) {
            Some(f) => f,
            None => return false,
        };

        // Get target function name
        let target_name = match self.code.strings.get(target_func.name.0) {
            Some(n) => n.clone(),
            None => return false,
        };

        // Names must match
        if current_name != target_name {
            return false;
        }

        // Current function must have a parent (be a method)
        let current_parent = match self.func.parent {
            Some(p) => p,
            None => return false,
        };

        // Target function must also have a parent
        let target_parent = match target_func.parent {
            Some(p) => p,
            None => return false,
        };

        // Target parent must be different from current parent (i.e., it's the base class)
        current_parent != target_parent
    }

    /// Get the name of a function
    pub(super) fn get_function_name(&self, fun: RefFun) -> Option<Str> {
        fun.as_fn(self.code)
            .and_then(|f| self.code.strings.get(f.name.0))
            .cloned()
    }

    /// Convert opcode to statements.
    /// May return multiple statements if inline expressions need to be materialized
    /// due to memory conflicts.
    pub(super) fn opcode_to_statements(&mut self, op_idx: usize) -> Vec<Statement> {
        // Check if this opcode was consumed by another construct (e.g., EnumIndex for switch)
        if self.suppressed_ops.contains(&op_idx) {
            return vec![];
        }

        let op = &self.func.ops[op_idx];

        // Check if this is a call opcode - calls need special handling for inlining
        let is_call = matches!(
            op,
            Opcode::Call0 { .. }
            | Opcode::Call1 { .. }
            | Opcode::Call2 { .. }
            | Opcode::Call3 { .. }
            | Opcode::Call4 { .. }
            | Opcode::CallN { .. }
            | Opcode::CallMethod { .. }
            | Opcode::CallThis { .. }
            | Opcode::CallClosure { .. }
        );

        // Invalidate any pending inline expressions that conflict with this opcode
        // (e.g., if this opcode writes to a field that a pending inline reads from)
        // These must be emitted as statements since we suppressed their definitions.
        //
        // IMPORTANT: For calls, we defer invalidation until AFTER building arguments.
        // This is because call arguments are evaluated BEFORE the call executes,
        // so pending inlines used as arguments are safe to inline.
        let mut stmts = if is_call {
            Vec::new() // Defer invalidation for calls
        } else {
            self.invalidate_conflicting_inlines(op)
        };

        // Set SSA context for this opcode - enables SSA-versioned naming
        if let Some((ssa_dst, ssa_uses)) = self.ssa.get_instr_for_op(op_idx) {
            self.current_ssa_dst = ssa_dst;
            self.current_ssa_uses = ssa_uses.clone();
        } else {
            self.current_ssa_dst = None;
            self.current_ssa_uses.clear();
        }

        let stmt = match op {
            Opcode::Label | Opcode::Nop => None,
            // Control flow opcodes - handled by structuring, not statement generation
            Opcode::JTrue { .. } | Opcode::JFalse { .. } | Opcode::JNull { .. }
            | Opcode::JNotNull { .. } | Opcode::JAlways { .. } | Opcode::Ret { .. }
            | Opcode::JSLt { .. } | Opcode::JSGte { .. } | Opcode::JSLte { .. }
            | Opcode::JSGt { .. } | Opcode::JEq { .. } | Opcode::JNotEq { .. }
            | Opcode::JULt { .. } | Opcode::JUGte { .. } | Opcode::JNotLt { .. }
            | Opcode::JNotGte { .. } => None,

            // Exception handling opcodes are control flow - handled by structure_block_range
            Opcode::Trap { .. } | Opcode::EndTrap { .. } => None,

            // Throw should be emitted (e.g., in try body)
            Opcode::Throw { exc } => Some(Statement::Throw(self.reg_to_expr(*exc))),

            // Assert is used for runtime type checking - throws "assert" if reached
            // It's typically preceded by a conditional jump that skips it when the check passes
            Opcode::Assert => Some(Statement::Comment("assert".into())),

            Opcode::Mov { dst, src } => {
                // Suppress self-assignments (b = b) that arise from default parameter handling
                if dst == src {
                    return stmts;
                }
                // If the destination held an iterator and we're assigning from a non-iterator,
                // switch to raw register names to avoid type conflicts
                let dst_was_iterator = self.iterator_regs.contains(dst);
                let src_is_iterator = self.iterator_regs.contains(src);
                if dst_was_iterator && !src_is_iterator {
                    // Use raw register name for this and future uses
                    self.iterator_regs.remove(dst);
                    self.use_raw_name_regs.insert(*dst);
                    let raw_name: Str = format!("r{}", dst.0).into();
                    let var = Expr::Variable(*dst, Some(raw_name.clone()));
                    let expr = self.reg_to_expr(*src);
                    // Hoist to function level so it's available outside the loop
                    self.hoisted_vars.insert(raw_name.clone());
                    self.declared_vars.insert(raw_name.clone());
                    // Track type for hoisted var
                    if let Some(tr) = self.func.regs.get(dst.0 as usize).copied() {
                        self.hoisted_var_types.insert(raw_name, tr);
                    }
                    stmts.push(Statement::Assign {
                        declaration: false, // Declaration is hoisted to function level
                        variable: var,
                        assign: expr,
                    });
                    return stmts;
                }
                let expr = self.reg_to_expr(*src);
                // Use try_inline_or_assign for potential inlining of moves
                self.try_inline_or_assign(*dst, expr)
            }

            Opcode::Int { dst, ptr } => {
                // If the destination held an iterator and we're assigning a constant,
                // switch to raw register names to avoid type conflicts
                if self.iterator_regs.contains(dst) {
                    self.iterator_regs.remove(dst);
                    self.use_raw_name_regs.insert(*dst);
                    let raw_name: Str = format!("r{}", dst.0).into();
                    let var = Expr::Variable(*dst, Some(raw_name.clone()));
                    let val = Expr::Constant(Constant::Int(*ptr));
                    // Hoist to function level so it's available outside the loop
                    self.hoisted_vars.insert(raw_name.clone());
                    self.declared_vars.insert(raw_name.clone());
                    // Track type for hoisted var
                    if let Some(tr) = self.func.regs.get(dst.0 as usize).copied() {
                        self.hoisted_var_types.insert(raw_name, tr);
                    }
                    stmts.push(Statement::Assign {
                        declaration: false, // Declaration is hoisted to function level
                        variable: var,
                        assign: val,
                    });
                    return stmts;
                }
                let val = Expr::Constant(Constant::Int(*ptr));
                self.try_inline_or_assign(*dst, val)
            }

            Opcode::Float { dst, ptr } => {
                let val = Expr::Constant(Constant::Float(*ptr));
                self.try_inline_or_assign(*dst, val)
            }

            Opcode::Bool { dst, value } => {
                let val = Expr::Constant(Constant::Bool(*value));
                self.try_inline_or_assign(*dst, val)
            }

            Opcode::String { dst, ptr } => {
                let val = Expr::Constant(Constant::String(*ptr));
                self.try_inline_or_assign(*dst, val)
            }

            Opcode::Null { dst } => {
                let val = Expr::Constant(Constant::Null);
                self.try_inline_or_assign(*dst, val)
            }

            Opcode::Add { dst, a, b } => {
                let expr = Expr::Op(Operation::Add(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                self.try_inline_or_assign(*dst, expr)
            }

            Opcode::Sub { dst, a, b } => {
                let expr = Expr::Op(Operation::Sub(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                self.try_inline_or_assign(*dst, expr)
            }

            Opcode::Mul { dst, a, b } => {
                let expr = Expr::Op(Operation::Mul(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                self.try_inline_or_assign(*dst, expr)
            }

            Opcode::Incr { dst } => {
                // Incr reads from dst (source SSA version) and writes to dst (destination SSA version)
                // In SSA form: r1_2 = r1_1 + 1  (not r1_2++)
                let dst_var = self.reg_to_expr_dst(*dst);
                let src_expr = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Add(
                    Box::new(src_expr),
                    Box::new(Expr::Constant(Constant::InlineInt(1))),
                ));
                Some(self.make_assign(dst_var, expr))
            }

            Opcode::Decr { dst } => {
                // Decr reads from dst (source SSA version) and writes to dst (destination SSA version)
                // In SSA form: r1_2 = r1_1 - 1  (not r1_2--)
                let dst_var = self.reg_to_expr_dst(*dst);
                let src_expr = self.reg_to_expr(*dst);
                let expr = Expr::Op(Operation::Sub(
                    Box::new(src_expr),
                    Box::new(Expr::Constant(Constant::InlineInt(1))),
                ));
                Some(self.make_assign(dst_var, expr))
            }

            Opcode::Field { dst, obj, field } => {
                // Check if this is an interface cache field (empty name)
                // These are internal HashLink fields - emit null to initialize the register
                // (the real value will come from ToVirtual, but we need the register initialized
                // for the subsequent JNotNull check)
                if self.is_interface_cache_field(*obj, *field) {
                    let var = self.reg_to_expr_dst(*dst);
                    stmts.push(self.make_assign(var, Expr::Constant(Constant::Null)));
                    return stmts;
                }

                let field_name = self.get_field_name(*obj, *field);

                // Check if this is a .bytes access on an array type
                // Instead of emitting (which would fail in Haxe), track the source
                // and reconstruct proper array access in GetMem/SetMem
                if field_name == "bytes" && self.is_array_type(*obj) {
                    // Track: bytes register came from this array expression
                    // We store the Expr to capture the correct SSA version now
                    let array_expr = self.reg_to_expr(*obj);
                    self.array_bytes_source.insert(*dst, array_expr);
                    // Don't emit any statement - the access will be reconstructed later
                    return stmts;
                }

                // Check if this is a .array access on ArrayObj/ArrayDyn
                // This is internal structure access - just pass through the array itself
                if field_name == "array" && self.is_array_type(*obj) {
                    // Track: dst register maps to the array expression
                    // We store the Expr to capture the correct SSA version now
                    let array_expr = self.reg_to_expr(*obj);
                    self.array_bytes_source.insert(*dst, array_expr);
                    return stmts;
                }

                let obj_expr = self.reg_to_expr(*obj);
                let expr = Expr::Field(Box::new(obj_expr), field_name);
                // Use try_inline_or_assign for potential inlining of field accesses
                self.try_inline_or_assign(*dst, expr)
            }

            Opcode::Call0 { dst, fun } => {
                let call = Call::new_fun(*fun, vec![]);
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::Call1 { dst, fun, arg0 } => {
                // Check for constructor call: Call1 following New with same register
                // Pattern: New reg0 = new Type; Call1 void = Constructor(reg0)
                // Skip since the New already creates the object
                if op_idx > 0 {
                    if let Some(Opcode::New { dst: new_dst }) = self.func.ops.get(op_idx - 1) {
                        if *new_dst == *arg0 {
                            // This is a constructor call following New - skip it
                            return stmts;
                        }
                    }
                }

                // Check for super method call: calling parent's method with same name, this as arg
                if *arg0 == Reg(0) && self.is_super_method_call(*fun) {
                    if let Some(method_name) = self.get_function_name(*fun) {
                        let call = Call::new_super_method(method_name, vec![]);
                        stmts.push(self.make_call_stmt(*dst, call));
                        return stmts;
                    }
                }

                // Track iterator-returning methods
                let fun_name = fun.name(self.code);
                if matches!(fun_name.as_ref(), "keys" | "iterator" | "keyValueIterator") {
                    self.iterator_regs.insert(*dst);
                }

                // Check for array wrapper functions: TypeName(array) -> ArrayObj
                // These are generated functions that wrap native arrays into typed ArrayObj
                // The function name matches a type (String, Int, etc.) and takes array, returns ArrayObj
                // We can just pass through the array since it's already been assigned
                if let Some(func) = fun.as_fn(self.code) {
                    // Check if return type is ArrayObj or ArrayDyn
                    if let Some(ret_type) = self.code.types.get(func.ty(self.code).ret.0) {
                        if let Type::Obj(obj) = ret_type {
                            if let Some(ret_name) = self.code.strings.get(obj.name.0) {
                                if ret_name == "hl.types.ArrayObj" || ret_name == "hl.types.ArrayDyn" {
                                    // Check if first arg is array type
                                    if !func.ty(self.code).args.is_empty() {
                                        if let Some(Type::Array) = self.code.types.get(func.ty(self.code).args[0].0) {
                                            // This is an array wrapper - just pass through the array
                                            let var = self.reg_to_expr_dst(*dst);
                                            let arr = self.reg_to_expr(*arg0);
                                            stmts.push(self.make_assign(var, arr));
                                            return stmts;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Check if this is an internal array property accessor (get_length -> .length)
                let fun_name = fun.name(self.code);
                if fun_name.as_ref() == "get_length" {
                    if let Some((owner_type, _)) = self.method_info.get(fun) {
                        if let Some(hlbc::types::Type::Obj(owner_obj)) = self.code.types.get(owner_type.0) {
                            let owner_name = self.code.get(owner_obj.name);
                            if owner_name.contains("hl.types.") && owner_name.contains("Array") {
                                // Emit as .length property access instead of method call
                                let var = self.reg_to_expr_dst(*dst);
                                let obj = self.reg_to_expr(*arg0);
                                let field_access = Expr::Field(Box::new(obj), "length".into());
                                stmts.push(self.make_assign(var, field_access));
                                return stmts;
                            }
                        }
                    }
                }

                let args = [*arg0];
                let call = self.try_make_method_call(*fun, &args)
                    .unwrap_or_else(|| Call::new_fun(*fun, vec![self.reg_to_expr(*arg0)]));
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::Call2 { dst, fun, arg0, arg1 } => {
                let name = fun.name(self.code);

                // Handle alloc_array(type, size) -> output as empty array literal []
                // This native allocates a raw array that gets filled by SetArray ops
                if name.as_ref() == "alloc_array" {
                    // Check if the element type (arg0) is a nullable type from a preceding Type opcode
                    // If so, we need to emit a type hint to allow null values
                    let needs_dynamic_hint = self.is_nullable_element_type(*arg0);

                    let var = self.reg_to_expr_dst(*dst);
                    if needs_dynamic_hint {
                        // For nullable element types, emit with Dynamic hint
                        // This allows pushing both Int and null
                        let typed_var = Expr::TypeAnnotated(
                            Box::new(var),
                            "Array<Dynamic>".into()
                        );
                        stmts.push(self.make_assign(typed_var, Expr::ArrayLiteral(vec![])));
                    } else {
                        stmts.push(self.make_assign(var, Expr::ArrayLiteral(vec![])));
                    }
                    return stmts;
                }

                // Skip itos/ftos/dtos - these are internal string conversion functions
                // The actual string is created by __alloc__ which follows
                if matches!(name.as_ref(), "itos" | "ftos" | "dtos") {
                    // Track the conversion source:
                    // ftos(original_value, ref_out) where ref_out points to length_reg
                    // We want to record: length_reg -> original_value expression
                    // Store the Expr (not Reg) to capture the SSA-versioned name now
                    if let Some(&target_reg) = self.ref_targets.get(arg1) {
                        let original_expr = self.reg_to_expr(*arg0);
                        self.string_conversion_source.insert(target_reg, original_expr);
                    }
                    return stmts;
                }

                // Handle __alloc__ - create Std.string(original_value) if we tracked the source
                if name.as_ref() == "__alloc__" {
                    // __alloc__(bytes, length_reg) where length_reg came from ftos/itos/dtos
                    if let Some(original_expr) = self.string_conversion_source.get(arg1).cloned() {
                        // Create Std.string(original_value)
                        let std_string = Expr::Field(
                            Box::new(Expr::Ident("Std".into())),
                            "string".into()
                        );
                        let call = Call::new(std_string, vec![original_expr]);
                        let var = self.reg_to_expr_dst(*dst);
                        stmts.push(self.make_assign(var, Expr::Call(Box::new(call))));
                        return stmts;
                    }
                }

                // Check if this is a call to an internal HL type allocator function
                // (e.g., hl.types.ArrayDyn.alloc) - these just wrap arrays so pass through arg0
                if name.as_ref() == "alloc" {
                    if let hlbc::types::FunPtr::Fun(func) = self.code.get(*fun) {
                        if let Some(parent_ref) = func.parent {
                            if let Some(hlbc::types::Type::Obj(parent_obj)) = self.code.types.get(parent_ref.0) {
                                let parent_name = self.code.get(parent_obj.name);
                                // Check if parent is an internal HL Array type
                                // Static types look like "hl.types.$ArrayDyn"
                                // Instance types look like "hl.types.ArrayDyn"
                                if parent_name.contains("hl.types.") && parent_name.contains("Array") {
                                    // Internal allocator - just pass through the first argument
                                    let var = self.reg_to_expr_dst(*dst);
                                    let source = self.reg_to_expr(*arg0);
                                    stmts.push(self.make_assign(var, source));
                                    return stmts;
                                }
                            }
                        }
                    }
                }

                let args = [*arg0, *arg1];
                let call = self.try_make_method_call(*fun, &args)
                    .unwrap_or_else(|| Call::new_fun(*fun, vec![self.reg_to_expr(*arg0), self.reg_to_expr(*arg1)]));
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::Call3 { dst, fun, arg0, arg1, arg2 } => {
                let args = [*arg0, *arg1, *arg2];
                let call = self.try_make_method_call(*fun, &args)
                    .unwrap_or_else(|| Call::new_fun(*fun, vec![
                        self.reg_to_expr(*arg0),
                        self.reg_to_expr(*arg1),
                        self.reg_to_expr(*arg2),
                    ]));
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::Call4 { dst, fun, arg0, arg1, arg2, arg3 } => {
                let args = [*arg0, *arg1, *arg2, *arg3];
                let call = self.try_make_method_call(*fun, &args)
                    .unwrap_or_else(|| Call::new_fun(*fun, vec![
                        self.reg_to_expr(*arg0),
                        self.reg_to_expr(*arg1),
                        self.reg_to_expr(*arg2),
                        self.reg_to_expr(*arg3),
                    ]));
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::CallN { dst, fun, args } => {
                // Check for super constructor call: inside constructor, calling parent's __constructor__
                if self.is_current_function_constructor() && self.is_constructor_function(*fun) {
                    if !args.is_empty() && args[0] == Reg(0) {
                        // This is super(args...) - skip first arg (this)
                        let super_args: Vec<_> = args[1..].iter().map(|r| self.reg_to_expr(*r)).collect();
                        let call = Call::new_super(super_args);
                        stmts.push(self.make_call_stmt(*dst, call));
                        return stmts;
                    }
                }

                let call = self.try_make_method_call(*fun, args)
                    .unwrap_or_else(|| {
                        let arg_exprs: Vec<_> = args.iter().map(|r| self.reg_to_expr(*r)).collect();
                        Call::new_fun(*fun, arg_exprs)
                    });
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::CallMethod { dst, field, args } => {
                if args.is_empty() {
                    stmts.push(Statement::Comment("callmethod with no args".into()));
                    return stmts;
                }
                let obj = self.reg_to_expr(args[0]);
                // For CallMethod, 'field' is a proto array index (NOT a pindex or field index)
                let method_name = self.get_proto_name(args[0], *field);

                // When calling .next() on an iterator, the result might get the same debug name
                // as the iterator itself (e.g., `key = key.next()`). This causes type errors
                // because `key` would need to be both Iterator<T> and T.
                // Detect this pattern and give the result a unique name.
                let use_unique_name = method_name.as_ref() == "next"
                    && self.reg_name(args[0]) == self.reg_name(*dst);

                let method = Expr::Field(Box::new(obj), method_name);
                let arg_exprs: Vec<_> = args[1..].iter().map(|r| self.reg_to_expr(*r)).collect();
                let call = Call { fun: method, args: arg_exprs };

                if use_unique_name {
                    // Use raw register name to avoid conflict with iterator variable
                    let raw_name: Str = format!("r{}", dst.0).into();
                    let var = Expr::Variable(*dst, Some(raw_name.clone()));
                    // Track this register to use raw name for all subsequent reads
                    self.use_raw_name_regs.insert(*dst);
                    // Hoist to function level so it's available outside the loop
                    self.hoisted_vars.insert(raw_name.clone());
                    self.declared_vars.insert(raw_name.clone());
                    // Track type for hoisted var
                    if let Some(tr) = self.func.regs.get(dst.0 as usize).copied() {
                        self.hoisted_var_types.insert(raw_name, tr);
                    }
                    Some(Statement::Assign {
                        declaration: false, // Declaration is hoisted to function level
                        variable: var,
                        assign: Expr::Call(Box::new(call)),
                    })
                } else {
                    Some(self.make_call_stmt(*dst, call))
                }
            }

            Opcode::CallThis { dst, field, args } => {
                let this = Expr::Variable(Reg(0), Some("this".into()));
                let method_name = self.get_proto_name(Reg(0), *field);
                let method = Expr::Field(Box::new(this), method_name);
                let arg_exprs: Vec<_> = args.iter().map(|r| self.reg_to_expr(*r)).collect();
                let call = Call { fun: method, args: arg_exprs };
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::CallClosure { dst, fun, args } => {
                let fun_expr = self.reg_to_expr(*fun);
                let arg_exprs: Vec<_> = args.iter().map(|r| self.reg_to_expr(*r)).collect();
                let call = Call { fun: fun_expr, args: arg_exprs };
                Some(self.make_call_stmt(*dst, call))
            }

            Opcode::GetGlobal { dst, global } => {
                // Check if this is a string constant global
                if let Some(string_ref) = self.get_global_string_value(*global) {
                    let expr = Expr::Constant(Constant::String(string_ref));
                    self.try_inline_or_assign(*dst, expr)
                } else {
                    let global_name = self.get_global_name(*global);
                    let expr = Expr::Ident(global_name);
                    // Use try_inline_or_assign to allow inlining of global references
                    // This is important for static method calls: haxe.Log.trace(...)
                    self.try_inline_or_assign(*dst, expr)
                }
            }

            Opcode::SetGlobal { global, src } => {
                let global_name = self.get_global_name(*global);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(Expr::Ident(global_name), expr))
            }

            Opcode::SetField { obj, field, src } => {
                // Check if this is an interface cache field (empty name)
                // These are internal HashLink fields - suppress them
                if self.is_interface_cache_field(*obj, *field) {
                    return stmts;
                }

                let obj_expr = self.reg_to_expr(*obj);
                let field_name = self.get_field_name(*obj, *field);
                let target = Expr::Field(Box::new(obj_expr), field_name);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(target, expr))
            }

            Opcode::New { dst } => {
                let var = self.reg_to_expr_dst(*dst);
                let type_ref = self.get_type_ref(*dst);
                // For Virtual types and DynObj (anonymous objects), use empty object literal
                // DynObj is used when fields are set dynamically via DynSet, then cast to Virtual
                if matches!(
                    &self.code.types[type_ref.0],
                    hlbc::types::Type::Virtual { .. } | hlbc::types::Type::DynObj
                ) {
                    Some(self.make_assign(var, Expr::Anonymous(type_ref, HashMap::new())))
                } else {
                    // Look ahead for __constructor__ call to get constructor arguments
                    let (ctor_args, ctor_op_idx, consumed_ops) = self.find_constructor_args(*dst, op_idx);
                    // Only suppress opcodes if we actually found a constructor call
                    // Otherwise, consumed_ops may contain unrelated ops that shouldn't be suppressed
                    if let Some(idx) = ctor_op_idx {
                        // Suppress the constructor call opcode
                        self.suppressed_ops.insert(idx);
                        // Suppress opcodes that contributed to constructor arguments (e.g., Float, Ref)
                        for consumed_idx in consumed_ops {
                            self.suppressed_ops.insert(consumed_idx);
                        }
                    }
                    let ctor = ConstructorCall::new(type_ref, ctor_args);
                    Some(self.make_assign(var, Expr::Constructor(ctor)))
                }
            }

            Opcode::NullCheck { .. } => {
                // NullCheck is implicit in Haxe field access - skip emitting
                None
            }

            Opcode::ToVirtual { dst, src } => {
                // ToVirtual is often just a cast, emit as assignment
                let var = self.reg_to_expr_dst(*dst);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(var, expr))
            }

            Opcode::ToSFloat { dst, src } | Opcode::ToUFloat { dst, src } => {
                // Convert int to float - emit explicit cast to preserve type across inlining
                let var = self.reg_to_expr_dst(*dst);
                let src_expr = self.reg_to_expr(*src);
                // Wrap in cast: cast(src, Float) or (src : Float)
                let cast_expr = Expr::Cast(Box::new(src_expr), "Float".into());
                Some(self.make_assign(var, cast_expr))
            }

            Opcode::ToInt { dst, src } => {
                // Convert float to int - emit as Std.int(src) call
                let var = self.reg_to_expr_dst(*dst);
                let src_expr = self.reg_to_expr(*src);
                let call = Expr::Call(Box::new(Call {
                    fun: Expr::Field(Box::new(Expr::Ident("Std".into())), "int".into()),
                    args: vec![src_expr],
                }));
                Some(self.make_assign(var, call))
            }

            Opcode::ToDyn { dst, src } => {
                // Convert to Dynamic - emit as simple assignment
                let var = self.reg_to_expr_dst(*dst);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(var, expr))
            }

            Opcode::SafeCast { dst, src } | Opcode::UnsafeCast { dst, src } => {
                // Cast to destination type - emit as simple assignment for now
                let var = self.reg_to_expr_dst(*dst);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(var, expr))
            }

            Opcode::Rethrow { exc } => {
                Some(Statement::Throw(self.reg_to_expr(*exc)))
            }

            Opcode::SDiv { dst, a, b } | Opcode::UDiv { dst, a, b } => {
                let var = self.reg_to_expr_dst(*dst);
                let expr = Expr::Op(Operation::Div(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::SMod { dst, a, b } | Opcode::UMod { dst, a, b } => {
                let var = self.reg_to_expr_dst(*dst);
                let expr = Expr::Op(Operation::Mod(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::And { dst, a, b } => {
                let var = self.reg_to_expr_dst(*dst);
                let expr = Expr::Op(Operation::And(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::Or { dst, a, b } => {
                let var = self.reg_to_expr_dst(*dst);
                let expr = Expr::Op(Operation::Or(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::Xor { dst, a, b } => {
                let var = self.reg_to_expr_dst(*dst);
                let expr = Expr::Op(Operation::Xor(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::Shl { dst, a, b } => {
                // Check if this is a shift by constant 2 or 3 (array index * 4 or * 8)
                // If so, track it for array access reconstruction and suppress the statement
                // Use SSA to find the value of b at this point
                let shift_amount = self.get_ssa_constant_value(*b, self.current_op);
                if let Some(shift) = shift_amount {
                    if shift == 2 || shift == 3 {
                        // Track: shifted index came from original index with this shift
                        // Use SSA to get the actual value if it's a constant
                        // For non-constants, use the correct SSA name via SSA lookup to avoid
                        // getting names from other control flow paths (bounds check branches)
                        let index_expr = if let Some(const_val) = self.get_ssa_constant_value(*a, self.current_op) {
                            Expr::Constant(Constant::InlineInt(const_val as usize))
                        } else {
                            // Get the correct SSA variable name by looking at SSA uses
                            // This avoids the issue where debug names from other branches
                            // incorrectly apply to this control flow path
                            self.get_ssa_based_expr(*a, self.current_op)
                        };
                        self.shifted_indices.insert(*dst, (index_expr, shift));
                        // Don't emit the shift statement - it will be absorbed by array access
                        return stmts;
                    }
                }
                let var = self.reg_to_expr_dst(*dst);
                let a_expr = self.reg_to_expr(*a);
                let b_expr = self.reg_to_expr(*b);
                let expr = Expr::Op(Operation::Shl(
                    Box::new(a_expr),
                    Box::new(b_expr),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::SShr { dst, a, b } => {
                let var = self.reg_to_expr_dst(*dst);
                let expr = Expr::Op(Operation::Shr(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::UShr { dst, a, b } => {
                let var = self.reg_to_expr_dst(*dst);
                // UShr is unsigned shift right, displayed as >>> in Haxe
                let expr = Expr::Op(Operation::Shr(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                Some(self.make_assign(var, expr))
            }

            Opcode::Neg { dst, src } => {
                let var = self.reg_to_expr_dst(*dst);
                let expr = Expr::Op(Operation::Neg(Box::new(self.reg_to_expr(*src))));
                Some(self.make_assign(var, expr))
            }

            Opcode::Not { dst, src } => {
                let var = self.reg_to_expr_dst(*dst);
                let expr = Expr::Op(Operation::Not(Box::new(self.reg_to_expr(*src))));
                Some(self.make_assign(var, expr))
            }

            Opcode::GetArray { dst, array, index } => {
                let var = self.reg_to_expr_dst(*dst);
                // Check if array register came from .array field access (ArrayObj internal structure)
                // Use the stored expression which has the correct SSA version
                let arr = if let Some(array_expr) = self.array_bytes_source.get(array).cloned() {
                    array_expr
                } else {
                    self.reg_to_expr(*array)
                };
                let idx = self.reg_to_expr(*index);
                let expr = Expr::Array(Box::new(arr), Box::new(idx));
                Some(self.make_assign(var, expr))
            }

            Opcode::SetArray { array, index, src } => {
                // Check if array register came from .array field access (ArrayObj internal structure)
                // Use the stored expression which has the correct SSA version
                let arr = if let Some(array_expr) = self.array_bytes_source.get(array).cloned() {
                    array_expr
                } else {
                    self.reg_to_expr(*array)
                };
                let idx = self.reg_to_expr(*index);
                let target = Expr::Array(Box::new(arr), Box::new(idx));
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(target, expr))
            }

            Opcode::ArraySize { dst, array } => {
                let var = self.reg_to_expr_dst(*dst);
                let arr = self.reg_to_expr(*array);
                let expr = Expr::Field(Box::new(arr), "length".into());
                Some(self.make_assign(var, expr))
            }

            Opcode::GetThis { dst, field } => {
                let this = Expr::Variable(Reg(0), Some("this".into()));
                let field_name = self.get_field_name(Reg(0), *field);
                let expr = Expr::Field(Box::new(this), field_name);
                // Use try_inline_or_assign for potential inlining of this.field accesses
                self.try_inline_or_assign(*dst, expr)
            }

            Opcode::SetThis { field, src } => {
                let this = Expr::Variable(Reg(0), Some("this".into()));
                let field_name = self.get_field_name(Reg(0), *field);
                let target = Expr::Field(Box::new(this), field_name);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(target, expr))
            }

            Opcode::Bytes { dst, ptr } => {
                // Load bytes constant from the bytes pool
                let var = self.reg_to_expr_dst(*dst);
                let expr = Expr::Constant(Constant::Bytes(*ptr));
                Some(self.make_assign(var, expr))
            }

            Opcode::GetMem { dst, bytes, index } => {
                let var = self.reg_to_expr_dst(*dst);

                // Determine the target (array or bytes)
                // Use the stored expression which has the correct SSA version
                let target_expr = if let Some(array_expr) = self.array_bytes_source.get(bytes).cloned() {
                    // Bytes came from an array - use the stored array expression
                    array_expr
                } else {
                    // Raw bytes access
                    self.reg_to_expr(*bytes)
                };

                // Always unshift the index if it was tracked
                // Use the stored Expr directly to preserve the correct SSA version
                let index_expr = if let Some((orig_expr, _shift)) = self.shifted_indices.get(index).cloned() {
                    orig_expr
                } else {
                    self.reg_to_expr(*index)
                };

                let expr = Expr::Array(Box::new(target_expr), Box::new(index_expr));
                Some(self.make_assign(var, expr))
            }

            Opcode::SetMem { bytes, index, src } => {
                // Always unshift the index if it was tracked
                let index_expr = if let Some((orig_expr, _shift)) = self.shifted_indices.get(index).cloned() {
                    orig_expr
                } else {
                    self.reg_to_expr(*index)
                };

                let value_expr = self.reg_to_expr(*src);

                // Determine the target (array or raw bytes)
                // Use the stored expression which has the correct SSA version
                if let Some(array_expr) = self.array_bytes_source.get(bytes).cloned() {
                    // Bytes came from an array - use array[index] = value syntax
                    let target = Expr::Array(Box::new(array_expr), Box::new(index_expr));
                    Some(self.make_assign(target, value_expr))
                } else {
                    // Raw bytes - use bytes.set(index, value) syntax
                    let bytes_expr = self.reg_to_expr(*bytes);
                    let method = Expr::Field(Box::new(bytes_expr), "set".into());
                    let call = Call::new(method, vec![index_expr, value_expr]);
                    Some(Statement::ExprStatement(Expr::Call(Box::new(call))))
                }
            }

            // GetI8: Read 8-bit integer from bytes
            // haxe.io.Bytes uses get(pos), hl.Bytes uses getUI8(pos)
            Opcode::GetI8 { dst, bytes, index } => {
                let var = self.reg_to_expr_dst(*dst);
                let bytes_expr = self.reg_to_expr(*bytes);
                let index_expr = self.reg_to_expr(*index);
                // Use haxe.io.Bytes.get() for compatibility
                let method = Expr::Field(Box::new(bytes_expr), "get".into());
                let call = Expr::Call(Box::new(Call::new(method, vec![index_expr])));
                Some(self.make_assign(var, call))
            }

            // GetI16: Read 16-bit integer from bytes
            // haxe.io.Bytes uses getUInt16(pos), hl.Bytes uses getUI16(pos)
            Opcode::GetI16 { dst, bytes, index } => {
                let var = self.reg_to_expr_dst(*dst);
                let bytes_expr = self.reg_to_expr(*bytes);
                let index_expr = self.reg_to_expr(*index);
                // Use haxe.io.Bytes.getUInt16() for compatibility
                let method = Expr::Field(Box::new(bytes_expr), "getUInt16".into());
                let call = Expr::Call(Box::new(Call::new(method, vec![index_expr])));
                Some(self.make_assign(var, call))
            }

            // SetI8: Write 8-bit integer to bytes
            // haxe.io.Bytes uses set(pos, value), hl.Bytes uses setUI8(pos, value)
            Opcode::SetI8 { bytes, index, src } => {
                let bytes_expr = self.reg_to_expr(*bytes);
                let index_expr = self.reg_to_expr(*index);
                let value_expr = self.reg_to_expr(*src);
                // Use haxe.io.Bytes.set() for compatibility
                let method = Expr::Field(Box::new(bytes_expr), "set".into());
                let call = Call::new(method, vec![index_expr, value_expr]);
                Some(Statement::ExprStatement(Expr::Call(Box::new(call))))
            }

            // SetI16: Write 16-bit integer to bytes
            // haxe.io.Bytes uses setUInt16(pos, value), hl.Bytes uses setUI16(pos, value)
            Opcode::SetI16 { bytes, index, src } => {
                let bytes_expr = self.reg_to_expr(*bytes);
                let index_expr = self.reg_to_expr(*index);
                let value_expr = self.reg_to_expr(*src);
                // Use haxe.io.Bytes.setUInt16() for compatibility
                let method = Expr::Field(Box::new(bytes_expr), "setUInt16".into());
                let call = Call::new(method, vec![index_expr, value_expr]);
                Some(Statement::ExprStatement(Expr::Call(Box::new(call))))
            }

            Opcode::Ref { dst, src } => {
                // Reference - creates a pointer to a value
                // Two patterns:
                // 1. OUTPUT ref: Ref reg10 = &reg9; ftos(float, reg10) - reg10 is output param
                //    The ftos function writes through reg10 to set reg9. Skip this Ref.
                // 2. INPUT ref: Ref reg4 = &reg9; Call(reg4) - passes nullable value
                //    We need to emit dst = src so the value flows through.
                //
                // Track ref target for string conversion pattern detection
                self.ref_targets.insert(*dst, *src);

                // Detect OUTPUT pattern by looking at next opcode for ftos/itos/dtos calls
                let is_output_ref = if let Some(next_op) = self.func.ops.get(op_idx + 1) {
                    match next_op {
                        Opcode::Call2 { fun, arg1, .. } if *arg1 == *dst => {
                            // Check if it's a string conversion function (works for natives too)
                            let name = fun.name(self.code);
                            matches!(name.as_ref(), "ftos" | "itos" | "dtos")
                        }
                        _ => false,
                    }
                } else {
                    false
                };

                if is_output_ref {
                    // Skip - the ftos/itos/dtos call handles this
                    None
                } else {
                    // Nullable input parameter - emit assignment
                    let var = self.reg_to_expr_dst(*dst);
                    let expr = self.reg_to_expr(*src);
                    Some(self.make_assign(var, expr))
                }
            }

            Opcode::Unref { dst, src } => {
                // Dereference - reads from a pointer
                // For nullable default parameters, this is used to "unwrap" the value
                // When the names match (e.g., b = *b), it's a no-op for the source code
                let var = self.reg_to_expr_dst(*dst);
                let expr = self.reg_to_expr(*src);
                // Suppress if this would generate a self-assignment (same variable name)
                if let (Expr::Ident(dst_name) | Expr::Variable(_, Some(dst_name)),
                        Expr::Ident(src_name) | Expr::Variable(_, Some(src_name))) = (&var, &expr) {
                    if dst_name == src_name {
                        return stmts;
                    }
                }
                Some(self.make_assign(var, expr))
            }

            Opcode::Type { dst, ty } => {
                // Check if this type is only used for array allocation (element type metadata)
                // Pattern: Type reg = SomeType; Call2 alloc_array(reg, size)
                if self.is_type_only_for_array_alloc(*dst, op_idx) {
                    // Suppress - it's just array element type metadata, not a real value
                    None
                } else {
                    // Emit as type reference (for reflection, switches, etc.)
                    let var = self.reg_to_expr_dst(*dst);
                    let val = Expr::Constant(Constant::TypeRef(*ty));
                    Some(self.make_assign(var, val))
                }
            }

            Opcode::DynGet { dst, obj, field } => {
                let var = self.reg_to_expr_dst(*dst);
                let obj_expr = self.reg_to_expr(*obj);
                let field_name = self.code.strings.get(field.0)
                    .cloned()
                    .unwrap_or_else(|| format!("dyn_{}", field.0).into());
                let expr = Expr::Field(Box::new(obj_expr), field_name);
                Some(self.make_assign(var, expr))
            }

            Opcode::DynSet { obj, field, src } => {
                let obj_expr = self.reg_to_expr(*obj);
                let field_name = self.code.strings.get(field.0)
                    .cloned()
                    .unwrap_or_else(|| format!("dyn_{}", field.0).into());
                let target = Expr::Field(Box::new(obj_expr), field_name);
                let expr = self.reg_to_expr(*src);
                Some(self.make_assign(target, expr))
            }

            Opcode::StaticClosure { dst, fun } => {
                let var = self.reg_to_expr_dst(*dst);

                // Check for self-referencing closure (recursive function that passes itself)
                // or mutual recursion (A references B which references A)
                // In these cases, emit a function reference instead of trying to inline
                if *fun == self.func.findex || crate::is_currently_decompiling(fun.0) {
                    let expr = Expr::FunRef(*fun);
                    stmts.push(self.make_assign(var, expr));
                    return stmts;
                }

                // StaticClosure has no captured variables, just inline the function body
                if let Some(inner_func) = fun.as_fn(self.code) {
                    // Decompile the inner function (no closure analysis needed since no captures)
                    let inner_stmts = crate::decompile_code_with_closures(
                        self.code,
                        inner_func,
                        self.closure_analysis,
                    );
                    let expr = Expr::Closure(*fun, inner_stmts);
                    stmts.push(self.make_assign(var, expr));
                    return stmts;
                }

                // Native function used as callback - emit function reference
                let expr = Expr::FunRef(*fun);
                stmts.push(self.make_assign(var, expr));
                return stmts;
            }

            Opcode::InstanceClosure { dst, fun, obj } => {
                let var = self.reg_to_expr_dst(*dst);

                // Check for self-referencing closure or mutual recursion
                if *fun == self.func.findex || crate::is_currently_decompiling(fun.0) {
                    let expr = Expr::FunRef(*fun);
                    stmts.push(self.make_assign(var, expr));
                    return stmts;
                }

                // Check if this is a detected closure that we should inline
                if let Some(_capture_info) = self.get_closure_at_current_op() {
                    // This is a closure with captured variables
                    // Get the inner function and decompile it
                    if let Some(inner_func) = fun.as_fn(self.code) {
                        // Decompile the inner function with closure context
                        let inner_stmts = crate::decompile_code_with_closures(
                            self.code,
                            inner_func,
                            self.closure_analysis,
                        );

                        // Create a lambda expression with the decompiled body
                        let expr = Expr::Closure(*fun, inner_stmts);
                        stmts.push(self.make_assign(var, expr));
                        return stmts;
                    }
                }

                // Fallback: Method reference syntax (obj.methodName)
                let obj_expr = self.reg_to_expr(*obj);
                // Method reference: obj.methodName
                let method_name = self.get_function_name(*fun)
                    .unwrap_or_else(|| format!("method_{}", fun.0).into());
                let expr = Expr::Field(Box::new(obj_expr), method_name);
                Some(self.make_assign(var, expr))
            }

            // Enum opcodes for closure support
            Opcode::EnumAlloc { dst, construct } => {
                // Suppress closure context allocations
                if self.is_closure_context_reg(*dst) {
                    None // Don't emit - this is part of closure machinery
                } else {
                    let var = self.reg_to_expr_dst(*dst);
                    let type_ref = self.get_type_ref(*dst);
                    // Create new enum variant instance
                    let expr = Expr::EnumConstr(type_ref, *construct, vec![]);
                    Some(self.make_assign(var, expr))
                }
            }

            Opcode::MakeEnum { dst, construct, args } => {
                // Create an enum variant with arguments
                // e.g., Option.Some(42)
                let var = self.reg_to_expr_dst(*dst);
                let type_ref = self.get_type_ref(*dst);
                // Convert args to expressions
                let arg_exprs: Vec<Expr> = args.iter().map(|r| self.reg_to_expr(*r)).collect();
                let expr = Expr::EnumConstr(type_ref, *construct, arg_exprs);
                Some(self.make_assign(var, expr))
            }

            Opcode::EnumIndex { dst, value } => {
                // Get the constructor index of an enum value (for switch statements)
                // This is used internally by switch on enum, typically followed by Switch opcode
                // We emit the assignment so the switch can use it
                let var = self.reg_to_expr_dst(*dst);
                let val_expr = self.reg_to_expr(*value);
                // Use Type.enumIndex(val) which gets the constructor ordinal
                let type_expr = Expr::Ident("Type".into());
                let method_expr = Expr::Field(Box::new(type_expr), "enumIndex".into());
                let expr = Expr::Call(Box::new(Call::new(method_expr, vec![val_expr])));
                Some(self.make_assign(var, expr))
            }

            Opcode::SetEnumField { value, field, src } => {
                // Suppress closure context field writes
                if self.is_closure_context_reg(*value) {
                    None // Don't emit - captured variable is stored in closure machinery
                } else {
                    let obj_expr = self.reg_to_expr(*value);
                    let field_name = format!("field_{}", field.0);
                    let target = Expr::Field(Box::new(obj_expr), field_name.into());
                    let expr = self.reg_to_expr(*src);
                    Some(self.make_assign(target, expr))
                }
            }

            Opcode::EnumField { dst, value, construct, field } => {
                // Check if we're in a closure reading from the capture context (reg 0)
                if self.is_current_function_closure() && *value == Reg(0) {
                    // Try to get the captured variable name
                    if let Some(captured_name) = self.get_captured_var_name(*field) {
                        // Emit assignment using the captured variable name directly
                        let var = self.reg_to_expr_dst(*dst);
                        let expr = Expr::Ident(captured_name);
                        stmts.push(self.make_assign(var, expr));
                        return stmts;
                    }
                }

                // Check if this field access has a bound param name from switch pattern
                let binding_key = (*value, construct.0, field.0);
                if let Some(param_name) = self.enum_param_bindings.get(&binding_key).cloned() {
                    let var = self.reg_to_expr_dst(*dst);
                    let expr = Expr::Ident(param_name.into());
                    stmts.push(self.make_assign(var, expr));
                    return stmts;
                }

                // Normal enum field access - use Type.enumParameters(value)[index]
                // This is the proper Haxe way to extract enum parameters dynamically
                let var = self.reg_to_expr_dst(*dst);
                let val_expr = self.reg_to_expr(*value);
                // Type.enumParameters(val)[field_index]
                let type_expr = Expr::Ident("Type".into());
                let method_expr = Expr::Field(Box::new(type_expr), "enumParameters".into());
                let params_call = Expr::Call(Box::new(Call::new(method_expr, vec![val_expr])));
                let field_idx = Expr::Constant(Constant::InlineInt(field.0));
                let expr = Expr::Array(Box::new(params_call), Box::new(field_idx));
                let _ = construct; // suppress unused warning - construct info not needed for dynamic access
                Some(self.make_assign(var, expr))
            }

            Opcode::VirtualClosure { dst, obj, field } => {
                let var = self.reg_to_expr_dst(*dst);
                let obj_expr = self.reg_to_expr(*obj);
                let field_expr = self.reg_to_expr(*field);
                // Dynamic method lookup: obj[field] or obj.getMethod(field)
                let expr = Expr::Array(Box::new(obj_expr), Box::new(field_expr));
                Some(self.make_assign(var, expr))
            }

            Opcode::GetTID { dst, .. } => {
                // GetTID returns a type ID - just emit as unknown for now
                let var = self.reg_to_expr_dst(*dst);
                Some(self.make_assign(var, Expr::Unknown("tid".into())))
            }

            Opcode::Switch { .. } => {
                // Switch is handled by structure_switch, not opcode_to_statements
                None
            }

            _ => panic!("Unhandled opcode: {:?}", op),
        };

        if let Some(s) = stmt {
            stmts.push(s);
        }

        // For calls, run deferred invalidation AFTER building the call expression
        // This allows inline expressions to be consumed as arguments before invalidation
        if is_call {
            let mut invalidated = self.invalidate_conflicting_inlines(op);
            stmts.append(&mut invalidated);
        }

        stmts
    }
}

/// Simplify a list of statements by:
/// 1. Merging consecutive assignments (r3 = expr; r0 = r3; → r0 = expr;)
/// 2. Recursively simplifying nested blocks
pub fn simplify_statements(stmts: Vec<Statement>) -> Vec<Statement> {
    // First, collect all statements into a vec so we can scan ahead
    let stmts: Vec<_> = stmts.into_iter().collect();

    let mut result = Vec::new();
    let mut i = 0;

    while i < stmts.len() {
        let stmt = &stmts[i];

        // Try to merge with next statement
        if let Statement::Assign {
            declaration: decl1,
            variable: var1,
            assign: assign1,
        } = stmt
        {
            // Check if next statement is an assignment that uses our variable
            if i + 1 < stmts.len() {
                if let Statement::Assign {
                    declaration: decl2,
                    variable: var2,
                    assign: assign2,
                } = &stmts[i + 1]
                {
                    // Pattern: r3 = expr; r0 = r3; → r0 = expr;
                    // BUT: Don't merge if first statement is a declaration, as this would
                    // lose the variable declaration and leave it undeclared for later uses.
                    // ALSO: Don't merge if var1 is used later (other than in assign2)!
                    let var1_used_later = is_var_used_in_stmts(var1, &stmts[i + 2..]);
                    if is_same_expr(var1, assign2) && !is_same_expr(var1, var2) && !*decl1 && !var1_used_later {
                        // Merge: replace with var2 = assign1
                        // Field and Array accesses can NEVER be declarations
                        let is_field_or_array = matches!(var2, Expr::Field(_, _) | Expr::Array(_, _));
                        let merged = Statement::Assign {
                            declaration: if is_field_or_array { false } else { *decl2 },
                            variable: var2.clone(),
                            assign: assign1.clone(),
                        };
                        i += 2; // skip both statements
                        result.push(merged);
                        continue;
                    }
                }
            }
        }

        // Recursively simplify nested structures
        let simplified = match stmt.clone() {
            Statement::While { cond, stmts } => Statement::While {
                cond,
                stmts: simplify_statements(stmts),
            },
            Statement::IfElse { cond, if_, else_ } => Statement::IfElse {
                cond,
                if_: simplify_statements(if_),
                else_: simplify_statements(else_),
            },
            Statement::Block { stmts } => Statement::Block {
                stmts: simplify_statements(stmts),
            },
            Statement::Sequence { stmts } => Statement::Sequence {
                stmts: simplify_statements(stmts),
            },
            Statement::TryCatch {
                try_stmts,
                catch_stmts,
                catch_var,
            } => Statement::TryCatch {
                try_stmts: simplify_statements(try_stmts),
                catch_stmts: simplify_statements(catch_stmts),
                catch_var,
            },
            other => other,
        };
        result.push(simplified);
        i += 1;
    }

    result
}

/// Check if two expressions refer to the same variable
pub fn is_same_expr(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Variable(reg_a, _), Expr::Variable(reg_b, _)) => reg_a == reg_b,
        _ => false,
    }
}

/// Check if a variable is used anywhere in a list of statements
pub fn is_var_used_in_stmts(var: &Expr, stmts: &[Statement]) -> bool {
    let Expr::Variable(target_reg, _) = var else {
        return false;
    };

    for stmt in stmts {
        if is_var_used_in_stmt(target_reg, stmt) {
            return true;
        }
    }
    false
}

/// Check if a register is used in a statement (as a read, not just assignment target)
pub fn is_var_used_in_stmt(reg: &Reg, stmt: &Statement) -> bool {
    match stmt {
        Statement::Assign { variable: _, assign, .. } => {
            // Check if the expression uses this register
            is_var_used_in_expr(reg, assign)
        }
        Statement::ExprStatement(expr) => is_var_used_in_expr(reg, expr),
        Statement::Return(Some(expr)) => is_var_used_in_expr(reg, expr),
        Statement::Return(None) => false,
        Statement::While { cond, stmts } => {
            is_var_used_in_expr(reg, cond) || stmts.iter().any(|s| is_var_used_in_stmt(reg, s))
        }
        Statement::IfElse { cond, if_, else_ } => {
            is_var_used_in_expr(reg, cond)
                || if_.iter().any(|s| is_var_used_in_stmt(reg, s))
                || else_.iter().any(|s| is_var_used_in_stmt(reg, s))
        }
        Statement::Block { stmts } | Statement::Sequence { stmts } => {
            stmts.iter().any(|s| is_var_used_in_stmt(reg, s))
        }
        Statement::TryCatch { try_stmts, catch_stmts, .. } => {
            try_stmts.iter().any(|s| is_var_used_in_stmt(reg, s))
                || catch_stmts.iter().any(|s| is_var_used_in_stmt(reg, s))
        }
        Statement::Throw(expr) => is_var_used_in_expr(reg, expr),
        Statement::Switch { arg, default, cases, .. } => {
            is_var_used_in_expr(reg, arg)
                || default.iter().any(|s| is_var_used_in_stmt(reg, s))
                || cases.iter().any(|(_, body)| body.iter().any(|s| is_var_used_in_stmt(reg, s)))
        }
        Statement::IfElseChain { branches, else_ } => {
            branches.iter().any(|(cond, body)| {
                is_var_used_in_expr(reg, cond) || body.iter().any(|s| is_var_used_in_stmt(reg, s))
            }) || else_.iter().any(|s| is_var_used_in_stmt(reg, s))
        }
        Statement::Comment(_) | Statement::Break | Statement::Continue | Statement::VarDecl { .. } => false,
    }
}

/// Check if a register is used in an expression
pub fn is_var_used_in_expr(reg: &Reg, expr: &Expr) -> bool {
    match expr {
        Expr::Variable(r, _) => r == reg,
        Expr::Field(base, _) => is_var_used_in_expr(reg, base),
        Expr::Array(base, idx) => is_var_used_in_expr(reg, base) || is_var_used_in_expr(reg, idx),
        Expr::Call(call) => {
            is_var_used_in_expr(reg, &call.fun)
                || call.args.iter().any(|a| is_var_used_in_expr(reg, a))
        }
        Expr::Op(op) => is_var_used_in_operation(reg, op),
        Expr::IfElse { cond, if_, else_ } => {
            is_var_used_in_expr(reg, cond)
                || if_.iter().any(|s| is_var_used_in_stmt(reg, s))
                || else_.iter().any(|s| is_var_used_in_stmt(reg, s))
        }
        Expr::Constructor(ctor) => ctor.args.iter().any(|a| is_var_used_in_expr(reg, a)),
        Expr::Anonymous(_, fields) => fields.values().any(|v| is_var_used_in_expr(reg, v)),
        Expr::ArrayLiteral(elems) => elems.iter().any(|e| is_var_used_in_expr(reg, e)),
        Expr::EnumConstr(_, _, args) => args.iter().any(|a| is_var_used_in_expr(reg, a)),
        Expr::Closure(_, stmts) => stmts.iter().any(|s| is_var_used_in_stmt(reg, s)),
        Expr::Cast(inner, _) | Expr::TypeAnnotated(inner, _) => is_var_used_in_expr(reg, inner),
        // These don't contain variable references
        Expr::Constant(_) | Expr::Ident(_) | Expr::FunRef(_) | Expr::Unknown(_) => false,
    }
}

/// Check if a register is used in an operation
pub fn is_var_used_in_operation(reg: &Reg, op: &Operation) -> bool {
    use Operation::*;
    match op {
        Add(l, r) | Sub(l, r) | Mul(l, r) | Div(l, r) | Mod(l, r) |
        Shl(l, r) | Shr(l, r) | And(l, r) | Or(l, r) | Xor(l, r) |
        Eq(l, r) | NotEq(l, r) | Gt(l, r) | Gte(l, r) | Lt(l, r) | Lte(l, r) => {
            is_var_used_in_expr(reg, l) || is_var_used_in_expr(reg, r)
        }
        Neg(e) | Not(e) | Incr(e) | Decr(e) => is_var_used_in_expr(reg, e),
    }
}
