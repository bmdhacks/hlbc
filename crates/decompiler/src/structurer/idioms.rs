//! Haxe/HashLink bytecode pattern detection.
//!
//! This module handles detection of Haxe-specific patterns in HashLink bytecode:
//! - String switch detection (9-opcode pattern per case)
//! - Enum switch pattern detection and unwrapping
//! - Constructor argument collection
//! - Internal function suppression (__expand, __construct, etc.)
//! - Interface cache field detection
//!
//! These patterns are "idioms" that aren't pure control flow, but rather
//! implementation details of how Haxe compiles to HashLink bytecode.

use std::collections::{HashMap, HashSet};

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, Reg, RefFun, RefField, RefString, RefType, Type};
use hlbc::{Bytecode, Resolve};

use crate::ast::{Call, Constant, ConstructorCall, Expr, Operation};
use crate::ssa::get_dst_reg as get_opcode_dst;

use super::{StringSwitchCase, StringSwitchRegion, Structurer};

impl<'a> Structurer<'a> {
    /// Detect EnumIndex → Switch patterns and mark EnumIndex for suppression.
    /// Pattern:
    ///   EnumIndex dst = value
    ///   ... (0 or more ops)
    ///   Switch dst
    /// When detected, the EnumIndex opcode is suppressed and the switch uses the original enum.
    pub(super) fn detect_enum_switch_patterns(&mut self) {
        let ops = &self.func.ops;

        for (i, op) in ops.iter().enumerate() {
            if let Opcode::EnumIndex { dst, value: _ } = op {
                // Look for a Switch that uses this dst register
                // Search forward (within reasonable distance)
                for j in (i + 1)..ops.len().min(i + 20) {
                    if let Opcode::Switch { reg, .. } = &ops[j] {
                        if reg == dst {
                            // Found EnumIndex → Switch pattern
                            self.suppressed_ops.insert(i);
                            break;
                        }
                    }
                    // Stop if dst is overwritten
                    if let Some(def_dst) = get_opcode_dst(&ops[j]) {
                        if def_dst == *dst {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Detect internal function calls (__expand, __construct, __constructor__) and suppress them.
    /// These are runtime implementation details that shouldn't appear in decompiled output.
    pub(super) fn detect_internal_function_calls(&mut self) {
        let ops = &self.func.ops;

        for (i, op) in ops.iter().enumerate() {
            // Extract function ref and first argument (if any) from call opcodes
            let (fun, first_arg) = match op {
                Opcode::Call2 { fun, arg0, .. } => (Some(*fun), Some(*arg0)),
                Opcode::Call3 { fun, arg0, .. } => (Some(*fun), Some(*arg0)),
                Opcode::Call4 { fun, arg0, .. } => (Some(*fun), Some(*arg0)),
                Opcode::CallN { fun, args, .. } => (Some(*fun), args.first().copied()),
                _ => (None, None),
            };

            if let Some(fun) = fun {
                if let Some(func) = fun.as_fn(self.code) {
                    if let Some(name) = self.code.strings.get(func.name.0) {
                        // Suppress internal functions:
                        // - __expand: array growth
                        // - __construct: object construction helper
                        if name == "__expand" || name == "__construct" {
                            self.suppressed_ops.insert(i);
                        }
                        // - __constructor__: constructor call (folded into `new Type(...)`)
                        //   BUT NOT super constructor calls (where first arg is `this`, i.e., reg0)
                        else if name.starts_with("__constructor__") {
                            // Don't suppress if first arg is reg0 (this) - that's a super() call
                            let is_super_call = first_arg == Some(Reg(0));
                            if !is_super_call {
                                self.suppressed_ops.insert(i);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Detect string switch patterns in bytecode.
    /// Pattern per case (9 ops):
    ///   JNull reg0 -> next_case        // null check
    ///   Field reg2 = reg0.length
    ///   Int reg3 = N                   // expected length
    ///   JNotEq reg2 reg3 -> next_case  // length check
    ///   Field reg4 = reg0.bytes
    ///   String reg5 = "literal"        // case string
    ///   Call3 reg2 = string_compare(...)
    ///   Int reg3 = 0
    ///   JEq reg2 reg3 -> handler       // match check
    pub(super) fn detect_string_switches(code: &Bytecode, func: &Function) -> Vec<StringSwitchRegion> {
        let mut switches = Vec::new();
        let ops = &func.ops;

        if ops.len() < 9 {
            return switches;
        }

        let mut i = 0;
        while i + 8 < ops.len() {
            // Try to detect a string switch starting at position i
            if let Some(region) = Self::try_detect_string_switch_at(code, func, i) {
                let end = region.end_op;
                switches.push(region);
                // Skip past this switch
                i = end + 1;
            } else {
                i += 1;
            }
        }

        switches
    }

    /// Try to detect a string switch starting at the given opcode index.
    /// Returns None if the pattern doesn't match.
    fn try_detect_string_switch_at(code: &Bytecode, func: &Function, start: usize) -> Option<StringSwitchRegion> {
        let ops = &func.ops;

        // First case must start with JNull
        let switch_arg_reg = match &ops[start] {
            Opcode::JNull { reg, .. } => *reg,
            _ => return None,
        };

        let mut cases = Vec::new();
        let mut pos = start;

        // Collect cases
        loop {
            if pos + 8 >= ops.len() {
                break;
            }

            // Check for the 9-opcode pattern
            let case_info = Self::try_parse_string_case(code, func, pos, switch_arg_reg)?;

            cases.push(StringSwitchCase {
                string_ref: case_info.0,
                handler_op: case_info.1,
            });

            // Next case starts at the JNull jump target (or after the JEq)
            let next_case_start = case_info.2;

            // Check if there's another case starting at next_case_start
            if next_case_start >= ops.len() {
                break;
            }

            // If the next position doesn't start with JNull for the same register, we're done
            match &ops[next_case_start] {
                Opcode::JNull { reg, .. } if *reg == switch_arg_reg => {
                    pos = next_case_start;
                }
                _ => {
                    // This is the default case position
                    break;
                }
            }
        }

        // Need at least 2 cases to be considered a switch
        if cases.len() < 2 {
            return None;
        }

        // Find the end_op and default_op
        // The last case's "next case start" points to the default handler
        let last_case_end = pos + 8; // After the last JEq
        let default_op = if let Opcode::JNull { offset, .. } = &ops[pos] {
            // The JNull jumps to the next case or default
            (pos as isize + *offset as isize + 1) as usize
        } else {
            last_case_end + 1
        };

        Some(StringSwitchRegion {
            start_op: start,
            end_op: last_case_end,
            switch_arg_reg,
            cases,
            default_op,
        })
    }

    /// Try to parse a single string case at the given position.
    /// Returns Some((string_ref, handler_op, next_case_start)) if successful.
    fn try_parse_string_case(
        code: &Bytecode,
        func: &Function,
        pos: usize,
        expected_arg_reg: Reg,
    ) -> Option<(RefString, usize, usize)> {
        let ops = &func.ops;

        if pos + 8 >= ops.len() {
            return None;
        }

        // Op 0: JNull reg0 -> next_case
        let next_case_from_null = match &ops[pos] {
            Opcode::JNull { reg, offset } if *reg == expected_arg_reg => {
                (pos as isize + *offset as isize + 1) as usize
            }
            _ => return None,
        };

        // Op 1: Field reg2 = reg0.length
        match &ops[pos + 1] {
            Opcode::Field { obj, .. } if *obj == expected_arg_reg => {}
            _ => return None,
        }

        // Op 2: Int reg3 = N (expected length)
        match &ops[pos + 2] {
            Opcode::Int { .. } => {}
            _ => return None,
        }

        // Op 3: JNotEq reg2 reg3 -> next_case
        match &ops[pos + 3] {
            Opcode::JNotEq { .. } => {}
            _ => return None,
        }

        // Op 4: Field reg4 = reg0.bytes
        match &ops[pos + 4] {
            Opcode::Field { obj, .. } if *obj == expected_arg_reg => {}
            _ => return None,
        }

        // Op 5: String reg5 = "literal"
        let string_ref = match &ops[pos + 5] {
            Opcode::String { ptr, .. } => *ptr,
            _ => return None,
        };

        // Op 6: Call3 to string_compare
        match &ops[pos + 6] {
            Opcode::Call3 { fun, .. } => {
                // Verify it's string_compare
                use hlbc::types::FunPtr;
                if let FunPtr::Native(native) = code.get(*fun) {
                    let lib = code.get(native.lib);
                    let name = code.get(native.name);
                    if lib != "std" || name != "string_compare" {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            _ => return None,
        }

        // Op 7: Int reg3 = 0
        match &ops[pos + 7] {
            Opcode::Int { ptr, .. } => {
                if code.ints[ptr.0] != 0 {
                    return None;
                }
            }
            _ => return None,
        }

        // Op 8: JEq reg2 reg3 -> handler
        let handler_op = match &ops[pos + 8] {
            Opcode::JEq { offset, .. } => {
                (pos as isize + 8 + *offset as isize + 1) as usize
            }
            _ => return None,
        };

        Some((string_ref, handler_op, next_case_from_null))
    }

    /// Build a map from enum globals to their (enum type, constructor index).
    /// This analyzes the entry point function to find the pattern:
    ///   Type reg1 = enum<Color>
    ///   Call2 result = initEnum(_, reg1)
    ///   Field evalues_reg = result.__evalues__
    ///   Int index_reg = N
    ///   GetArray value_reg = evalues_reg[index_reg]
    ///   SetGlobal global@X = value_reg   // global X is constructor N
    pub(super) fn build_enum_global_map(code: &Bytecode) -> HashMap<hlbc::types::RefGlobal, (RefType, usize)> {
        use hlbc::types::FunPtr;

        let mut map = HashMap::new();

        // Skip if no functions are registered (e.g., in tests with mock bytecode)
        if code.findex_max() == 0 {
            return map;
        }

        // Get the entry point function using the proper lookup
        let entry_func = code.entrypoint();

        // Track register -> enum type mapping (from Type opcodes)
        let mut reg_to_enum_type: HashMap<Reg, RefType> = HashMap::new();

        // Track the current initialization state
        let mut current_index: Option<usize> = None;
        let mut init_enum_result_reg: Option<Reg> = None;
        let mut evalues_reg: Option<Reg> = None;
        let mut current_enum_type: Option<RefType> = None;

        for op in entry_func.ops.iter() {
            match op {
                // Track: Type reg = enum<...>
                Opcode::Type { dst, ty } => {
                    if let Type::Enum { .. } = &code[*ty] {
                        reg_to_enum_type.insert(*dst, *ty);
                    }
                }
                // Track: Call2 result = initEnum(_, enumTypeReg)
                Opcode::Call2 { dst, fun, arg0: _, arg1 } => {
                    // Check if this is a call to initEnum (could be native or user function)
                    let is_init_enum = match code.get(*fun) {
                        FunPtr::Native(native) => code.get(native.name) == "initEnum",
                        FunPtr::Fun(func) => func.name(code).as_ref() == "initEnum",
                    };
                    if is_init_enum {
                        init_enum_result_reg = Some(*dst);
                        // arg1 is the register holding the enum type
                        current_enum_type = reg_to_enum_type.get(arg1).copied();
                    }
                }
                // Track: Field evalues_reg = init_result.__evalues__
                // After initEnum, the next Field access on that register is __evalues__
                Opcode::Field { dst, obj, .. } => {
                    if init_enum_result_reg == Some(*obj) {
                        evalues_reg = Some(*dst);
                        init_enum_result_reg = None; // Only capture the first field access
                    }
                }
                // Track: Int reg = N (array index)
                Opcode::Int { dst: _, ptr } => {
                    if evalues_reg.is_some() {
                        current_index = code.ints.get(ptr.0).map(|&v| v as usize);
                    }
                }
                // Track: SetGlobal global = value
                Opcode::SetGlobal { global, src: _ } => {
                    if let (Some(idx), Some(ty)) = (current_index, current_enum_type) {
                        map.insert(*global, (ty, idx));
                    }
                }
                // Reset on control flow
                Opcode::JAlways { .. } | Opcode::Ret { .. } => {
                    init_enum_result_reg = None;
                    evalues_reg = None;
                    current_index = None;
                    current_enum_type = None;
                }
                _ => {}
            }
        }

        map
    }

    /// Try to extract a string comparison pattern from two register references.
    /// Returns Some((switch_arg, string_constant)) if reg_result was defined by
    /// string_compare(bytes, string_literal, len) and reg_zero was defined as 0.
    pub(super) fn try_extract_string_compare_pattern(&self, reg_result: Reg, reg_zero: Reg) -> Option<(Expr, Expr)> {
        // Find SSA variables for these registers
        let result_var = self.find_ssa_use(reg_result)?;
        let zero_var = self.find_ssa_use(reg_zero)?;

        // Find defining opcodes
        let result_def_idx = self.ssa.find_def(result_var)?;
        let zero_def_idx = self.ssa.find_def(zero_var)?;

        // Check if zero_def is Int with value 0
        let zero_op = &self.func.ops[zero_def_idx];
        let is_zero = if let Opcode::Int { ptr, .. } = zero_op {
            // Look up the actual integer value
            self.code.ints[ptr.0] == 0
        } else {
            false
        };
        if !is_zero {
            return None;
        }

        // Check if result_def is Call3 to string_compare
        let result_op = &self.func.ops[result_def_idx];
        if let Opcode::Call3 { fun, arg1, .. } = result_op {
            // Check if this is a call to string_compare
            if self.is_string_compare_function(*fun) {
                // arg1 is the string literal bytes - look up what it was assigned from
                // The pattern is: String reg = "literal"; then reg.bytes is passed
                // We need to find the String opcode that defined the value in arg1
                if let Some(string_val) = self.find_string_literal_for_bytes(*arg1, result_def_idx) {
                    // The switch argument is the original string being compared
                    // (typically from a field access like obj.id)
                    // For now, just return a placeholder - the actual switch arg detection
                    // would need to trace back further
                    return Some((
                        Expr::Variable(reg_result, Some("stringArg".into())),
                        Expr::Constant(Constant::String(string_val)),
                    ));
                }
            }
        }

        None
    }

    /// Check if a function reference is for string_compare
    pub(super) fn is_string_compare_function(&self, fun: RefFun) -> bool {
        use hlbc::types::FunPtr;
        if let FunPtr::Native(native) = self.code.get(fun) {
            // Check for std.string_compare native
            let lib = self.code.get(native.lib);
            let name = self.code.get(native.name);
            lib == "std" && name == "string_compare"
        } else {
            false
        }
    }

    /// Find the string literal that was passed to string_compare.
    /// Searches backwards from call_idx to find the String opcode that defined bytes_reg.
    pub(super) fn find_string_literal_for_bytes(&self, bytes_reg: Reg, call_idx: usize) -> Option<RefString> {
        // Search backwards from the call to find where bytes_reg was defined
        for i in (0..call_idx).rev() {
            let op = &self.func.ops[i];
            if let Opcode::String { dst, ptr } = op {
                if *dst == bytes_reg {
                    return Some(*ptr);
                }
            }
            // If we see bytes_reg being written to by something else, stop
            if get_opcode_dst(op).map_or(false, |d| d == bytes_reg) {
                break;
            }
        }
        None
    }

    /// Check if a field is an interface implementation cache field (has empty name).
    /// These are internal HashLink fields used to cache interface vtable lookups.
    pub(super) fn is_interface_cache_field(&self, obj_reg: Reg, field: RefField) -> bool {
        let reg_idx = obj_reg.0 as usize;
        if reg_idx < self.func.regs.len() {
            let type_ref = self.func.regs[reg_idx];
            if let Some(ty) = self.code.types.get(type_ref.0) {
                let fields: Option<&[hlbc::types::ObjField]> = match ty {
                    hlbc::types::Type::Obj(obj) => Some(&obj.fields),
                    _ => None,
                };
                if let Some(fields) = fields {
                    if let Some(f) = fields.get(field.0) {
                        if let Some(name) = self.code.strings.get(f.name.0) {
                            return name.is_empty();
                        }
                    }
                }
            }
        }
        false
    }

    /// Check if switch_arg is Type.enumIndex(x) and unwrap to just x
    /// Also check if switch_reg was produced by EnumIndex opcode
    /// Returns (unwrapped_arg, optional_enum_type, optional_enum_value_reg)
    /// The enum_value_reg is the register holding the actual enum value (for pattern binding)
    pub(super) fn unwrap_enum_index_switch(
        &mut self,
        switch_arg: Expr,
        switch_reg: Reg,
    ) -> (Expr, Option<RefType>, Option<Reg>) {
        // Check if switch_arg is Type.enumIndex(x) (already inlined)
        if let Expr::Call(call) = &switch_arg {
            if let Expr::Field(base, method) = &call.fun {
                if method.as_ref() == "enumIndex" {
                    if let Expr::Ident(name) = base.as_ref() {
                        if name.as_ref() == "Type" {
                            if let Some(inner_arg) = call.args.first() {
                                // Try to get enum type and register from the inner argument
                                let enum_type = self.get_enum_type_from_expr(inner_arg);
                                let enum_reg = if let Expr::Variable(reg, _) = inner_arg {
                                    Some(*reg)
                                } else {
                                    None
                                };
                                return (inner_arg.clone(), enum_type, enum_reg);
                            }
                        }
                    }
                }
            }
        }

        // Check if switch_reg was produced by EnumIndex opcode
        // Find the SSA variable for switch_reg in the current uses
        let ssa_var = self.current_ssa_uses.iter()
            .find(|&&v| v.reg == switch_reg)
            .copied();

        if let Some(var) = ssa_var {
            if let Some(def_op_idx) = self.ssa.find_def(var) {
                if let Opcode::EnumIndex { dst: _, value } = &self.func.ops[def_op_idx] {
                    // The switch should use the original enum value
                    let enum_reg = *value;

                    // Set SSA context to the EnumIndex opcode to get correct variable names
                    let saved_op = self.current_op;
                    let saved_uses = self.current_ssa_uses.clone();
                    self.current_op = def_op_idx;
                    if let Some((_, ssa_uses)) = self.ssa.get_instr_for_op(def_op_idx) {
                        self.current_ssa_uses = ssa_uses.clone();
                    }

                    let enum_expr = self.reg_to_expr(enum_reg);

                    // Restore SSA context
                    self.current_op = saved_op;
                    self.current_ssa_uses = saved_uses;

                    let enum_type = self.get_enum_type_for_reg(enum_reg);
                    // Suppress the EnumIndex opcode since we're using the enum directly
                    self.suppressed_ops.insert(def_op_idx);
                    return (enum_expr, enum_type, Some(enum_reg));
                }
            }
        }

        // Not a Type.enumIndex call - try to get enum type from the register's type
        let enum_type = self.get_enum_type_for_reg(switch_reg);
        (switch_arg, enum_type, None)
    }

    /// Scan opcodes in a case body to find which enum fields are accessed.
    /// Returns a set of (construct_idx, field_idx) pairs.
    pub(super) fn scan_enum_field_accesses(
        &self,
        enum_value_reg: Reg,
        start_op: usize,
        end_op: usize,
    ) -> HashSet<(usize, usize)> {
        let mut accessed = HashSet::new();
        for idx in start_op..end_op {
            if idx >= self.func.ops.len() {
                break;
            }
            if let Opcode::EnumField { value, construct, field, .. } = &self.func.ops[idx] {
                if *value == enum_value_reg {
                    accessed.insert((construct.0, field.0));
                }
            }
        }
        accessed
    }

    /// Try to get the enum type from an expression
    pub(super) fn get_enum_type_from_expr(&self, expr: &Expr) -> Option<RefType> {
        match expr {
            Expr::Variable(reg, _) => self.get_enum_type_for_reg(*reg),
            _ => None,
        }
    }

    /// Try to get the enum type for a register
    pub(super) fn get_enum_type_for_reg(&self, reg: Reg) -> Option<RefType> {
        let reg_type = self.func.regs.get(reg.0 as usize)?;
        if let Type::Enum { .. } = &self.code[*reg_type] {
            Some(*reg_type)
        } else {
            None
        }
    }

    /// Look ahead from a New opcode to find the __constructor__ call arguments.
    /// In HashLink bytecode, object construction is split:
    ///   New reg0 = new Type
    ///   GetGlobal reg1 = global@5  // "Hello World"
    ///   Call2 __constructor__(reg0, reg1)
    /// We need to combine these into: new Type("Hello World")
    /// Returns (constructor_args, constructor_call_opcode_index, consumed_op_indices)
    pub(super) fn find_constructor_args(&self, new_dst: Reg, new_op_idx: usize) -> (Vec<Expr>, Option<usize>, Vec<usize>) {
        // Search forward within the same basic block for a constructor call
        let block = self.cfg.op_to_block.get(&new_op_idx);
        let search_end = block
            .and_then(|b| Some(self.cfg.graph[*b].end))
            .unwrap_or(self.func.ops.len().saturating_sub(1));

        // Track register values from intermediate opcodes
        let mut reg_values: HashMap<Reg, Expr> = HashMap::new();

        // Track intermediate New opcodes (for nested constructors like `new Point(new Point(1,2).x, ...)`)
        // Maps register -> type reference from the New opcode
        let mut pending_new: HashMap<Reg, RefType> = HashMap::new();

        // Track which opcodes contribute to constructor arguments (to suppress them)
        let mut consumed_ops: Vec<usize> = Vec::new();

        for idx in (new_op_idx + 1)..=search_end {
            // Search within the same basic block for constructor call
            if idx >= self.func.ops.len() {
                break;
            }

            let op = &self.func.ops[idx];

            // Helper to get expression for a register - used during tracking
            let get_val = |reg: Reg, regs: &HashMap<Reg, Expr>| -> Expr {
                regs.get(&reg).cloned().unwrap_or_else(|| self.reg_to_expr(reg))
            };

            // Track constant/global/computed assignments
            // Also mark opcodes as consumed so they can be suppressed
            match op {
                Opcode::String { dst, ptr } => {
                    reg_values.insert(*dst, Expr::Constant(Constant::String(*ptr)));
                    consumed_ops.push(idx);
                }
                Opcode::Int { dst, ptr } => {
                    reg_values.insert(*dst, Expr::Constant(Constant::Int(*ptr)));
                    consumed_ops.push(idx);
                }
                Opcode::Float { dst, ptr } => {
                    reg_values.insert(*dst, Expr::Constant(Constant::Float(*ptr)));
                    consumed_ops.push(idx);
                }
                Opcode::Bool { dst, value } => {
                    reg_values.insert(*dst, Expr::Constant(Constant::Bool(*value)));
                    consumed_ops.push(idx);
                }
                Opcode::Null { dst } => {
                    reg_values.insert(*dst, Expr::Constant(Constant::Null));
                    consumed_ops.push(idx);
                }
                // Track field access
                Opcode::Field { dst, obj, field } => {
                    let obj_expr = get_val(*obj, &reg_values);
                    let field_name = self.get_field_name(*obj, *field);
                    reg_values.insert(*dst, Expr::Field(Box::new(obj_expr), field_name));
                    consumed_ops.push(idx);
                }
                // Track this.field access
                Opcode::GetThis { dst, field } => {
                    let this = Expr::Variable(Reg(0), Some("this".into()));
                    let field_name = self.get_field_name(Reg(0), *field);
                    reg_values.insert(*dst, Expr::Field(Box::new(this), field_name));
                    consumed_ops.push(idx);
                }
                // Track arithmetic operations
                Opcode::Add { dst, a, b } => {
                    let a_expr = get_val(*a, &reg_values);
                    let b_expr = get_val(*b, &reg_values);
                    reg_values.insert(*dst, Expr::Op(Operation::Add(Box::new(a_expr), Box::new(b_expr))));
                    consumed_ops.push(idx);
                }
                Opcode::Sub { dst, a, b } => {
                    let a_expr = get_val(*a, &reg_values);
                    let b_expr = get_val(*b, &reg_values);
                    reg_values.insert(*dst, Expr::Op(Operation::Sub(Box::new(a_expr), Box::new(b_expr))));
                    consumed_ops.push(idx);
                }
                Opcode::Mul { dst, a, b } => {
                    let a_expr = get_val(*a, &reg_values);
                    let b_expr = get_val(*b, &reg_values);
                    reg_values.insert(*dst, Expr::Op(Operation::Mul(Box::new(a_expr), Box::new(b_expr))));
                    consumed_ops.push(idx);
                }
                Opcode::SDiv { dst, a, b } | Opcode::UDiv { dst, a, b } => {
                    let a_expr = get_val(*a, &reg_values);
                    let b_expr = get_val(*b, &reg_values);
                    reg_values.insert(*dst, Expr::Op(Operation::Div(Box::new(a_expr), Box::new(b_expr))));
                    consumed_ops.push(idx);
                }
                Opcode::Neg { dst, src } => {
                    let src_expr = get_val(*src, &reg_values);
                    reg_values.insert(*dst, Expr::Op(Operation::Neg(Box::new(src_expr))));
                    consumed_ops.push(idx);
                }
                // Track moves
                Opcode::Mov { dst, src } => {
                    let src_expr = get_val(*src, &reg_values);
                    reg_values.insert(*dst, src_expr);
                    consumed_ops.push(idx);
                }
                // Track refs - pass through the underlying value
                Opcode::Ref { dst, src } => {
                    let src_expr = get_val(*src, &reg_values);
                    reg_values.insert(*dst, src_expr);
                    consumed_ops.push(idx);
                }
                // Track casts
                Opcode::ToSFloat { dst, src } | Opcode::ToUFloat { dst, src } => {
                    let src_expr = get_val(*src, &reg_values);
                    reg_values.insert(*dst, Expr::Cast(Box::new(src_expr), "Float".into()));
                    consumed_ops.push(idx);
                }
                Opcode::ToInt { dst, src } => {
                    let src_expr = get_val(*src, &reg_values);
                    let call = Expr::Call(Box::new(Call {
                        fun: Expr::Field(Box::new(Expr::Ident("Std".into())), "int".into()),
                        args: vec![src_expr],
                    }));
                    reg_values.insert(*dst, call);
                    consumed_ops.push(idx);
                }
                // Track function calls (for things like Math.cos, Math.sin, computeX(), etc.)
                // NOTE: We track calls into reg_values for value propagation, but do NOT
                // add them to consumed_ops because calls have side effects and must still
                // emit their statements. The constructor arg will reference the result variable.
                Opcode::Call0 { dst, fun } => {
                    let call = Expr::Call(Box::new(Call::new_fun(*fun, vec![])));
                    reg_values.insert(*dst, call);
                    // Don't suppress - calls have side effects
                }
                Opcode::Call1 { dst, fun, arg0 } => {
                    let arg = get_val(*arg0, &reg_values);
                    let call = Expr::Call(Box::new(Call::new_fun(*fun, vec![arg])));
                    reg_values.insert(*dst, call);
                    // Don't suppress - calls have side effects
                }
                // Track Call2 but skip if it's the constructor call we're looking for
                Opcode::Call2 { dst, fun, arg0, arg1 } if *arg0 != new_dst => {
                    let a0 = get_val(*arg0, &reg_values);
                    let a1 = get_val(*arg1, &reg_values);
                    let call = Expr::Call(Box::new(Call::new_fun(*fun, vec![a0, a1])));
                    reg_values.insert(*dst, call);
                    // Don't suppress - calls have side effects
                }
                // Track Call3 but skip if it's the constructor call
                Opcode::Call3 { dst, fun, arg0, arg1, arg2 } if *arg0 != new_dst => {
                    let a0 = get_val(*arg0, &reg_values);
                    let a1 = get_val(*arg1, &reg_values);
                    let a2 = get_val(*arg2, &reg_values);
                    let call = Expr::Call(Box::new(Call::new_fun(*fun, vec![a0, a1, a2])));
                    reg_values.insert(*dst, call);
                    // Don't suppress - calls have side effects
                }
                // Track Call4 but skip if it's the constructor call
                Opcode::Call4 { dst, fun, arg0, arg1, arg2, arg3 } if *arg0 != new_dst => {
                    let a0 = get_val(*arg0, &reg_values);
                    let a1 = get_val(*arg1, &reg_values);
                    let a2 = get_val(*arg2, &reg_values);
                    let a3 = get_val(*arg3, &reg_values);
                    let call = Expr::Call(Box::new(Call::new_fun(*fun, vec![a0, a1, a2, a3])));
                    reg_values.insert(*dst, call);
                    // Don't suppress - calls have side effects
                }
                // Track CallN but skip if it's the constructor call
                Opcode::CallN { dst, fun, args } if args.first() != Some(&new_dst) => {
                    let call_args: Vec<_> = args.iter().map(|r| get_val(*r, &reg_values)).collect();
                    let call = Expr::Call(Box::new(Call::new_fun(*fun, call_args)));
                    reg_values.insert(*dst, call);
                    // Don't suppress - calls have side effects
                }
                // GetGlobal is pure (just reads a global) - safe to suppress
                Opcode::GetGlobal { dst, global } => {
                    reg_values.insert(*dst, self.global_to_expr(*global));
                    consumed_ops.push(idx);
                }
                // Track intermediate New opcodes (for nested constructors)
                // New itself is not suppressed - it needs the constructor call tracking
                Opcode::New { dst } if *dst != new_dst => {
                    // Get type from the register's declared type
                    let type_ref = self.func.regs.get(dst.0 as usize).copied().unwrap_or(RefType(0));
                    pending_new.insert(*dst, type_ref);
                    // Don't suppress - nested New needs its own constructor handling
                }
                _ => {}
            }

            // Helper function to get expression for a register (not a closure to avoid borrow issues)
            fn get_expr(reg: Reg, reg_values: &HashMap<Reg, Expr>, structurer: &Structurer) -> Expr {
                reg_values.get(&reg).cloned().unwrap_or_else(|| structurer.reg_to_expr(reg))
            }

            // Check for intermediate __constructor__ calls (for nested constructors)
            // These are constructor calls on registers OTHER than our target new_dst
            match op {
                Opcode::Call2 { fun, arg0, arg1, .. }
                    if *arg0 != new_dst && self.is_constructor_function(*fun) => {
                    // Build the complete constructor expression for this intermediate object
                    if let Some(ty_ref) = pending_new.remove(arg0) {
                        let args = vec![get_expr(*arg1, &reg_values, self)];
                        let ctor = Expr::Constructor(ConstructorCall::new(ty_ref, args));
                        reg_values.insert(*arg0, ctor);
                    }
                }
                Opcode::Call3 { fun, arg0, arg1, arg2, .. }
                    if *arg0 != new_dst && self.is_constructor_function(*fun) => {
                    if let Some(ty_ref) = pending_new.remove(arg0) {
                        let args = vec![
                            get_expr(*arg1, &reg_values, self),
                            get_expr(*arg2, &reg_values, self)
                        ];
                        let ctor = Expr::Constructor(ConstructorCall::new(ty_ref, args));
                        reg_values.insert(*arg0, ctor);
                    }
                }
                Opcode::Call4 { fun, arg0, arg1, arg2, arg3, .. }
                    if *arg0 != new_dst && self.is_constructor_function(*fun) => {
                    if let Some(ty_ref) = pending_new.remove(arg0) {
                        let args = vec![
                            get_expr(*arg1, &reg_values, self),
                            get_expr(*arg2, &reg_values, self),
                            get_expr(*arg3, &reg_values, self),
                        ];
                        let ctor = Expr::Constructor(ConstructorCall::new(ty_ref, args));
                        reg_values.insert(*arg0, ctor);
                    }
                }
                Opcode::CallN { fun, args, .. }
                    if !args.is_empty() && args[0] != new_dst && self.is_constructor_function(*fun) => {
                    if let Some(ty_ref) = pending_new.remove(&args[0]) {
                        let ctor_args: Vec<_> = args[1..].iter()
                            .map(|r| get_expr(*r, &reg_values, self))
                            .collect();
                        let ctor = Expr::Constructor(ConstructorCall::new(ty_ref, ctor_args));
                        reg_values.insert(args[0], ctor);
                    }
                }
                _ => {}
            }

            // Check for the target constructor call
            match op {
                // Call2 __constructor__(obj, arg1)
                Opcode::Call2 { fun, arg0, arg1, .. } if *arg0 == new_dst => {
                    if self.is_constructor_function(*fun) {
                        return (vec![get_expr(*arg1, &reg_values, self)], Some(idx), consumed_ops);
                    }
                }
                // Call3 __constructor__(obj, arg1, arg2)
                Opcode::Call3 { fun, arg0, arg1, arg2, .. } if *arg0 == new_dst => {
                    if self.is_constructor_function(*fun) {
                        return (vec![
                            get_expr(*arg1, &reg_values, self),
                            get_expr(*arg2, &reg_values, self)
                        ], Some(idx), consumed_ops);
                    }
                }
                // Call4 __constructor__(obj, arg1, arg2, arg3)
                Opcode::Call4 { fun, arg0, arg1, arg2, arg3, .. } if *arg0 == new_dst => {
                    if self.is_constructor_function(*fun) {
                        return (vec![
                            get_expr(*arg1, &reg_values, self),
                            get_expr(*arg2, &reg_values, self),
                            get_expr(*arg3, &reg_values, self),
                        ], Some(idx), consumed_ops);
                    }
                }
                // CallN __constructor__(obj, args...)
                Opcode::CallN { fun, args, .. } if !args.is_empty() && args[0] == new_dst => {
                    if self.is_constructor_function(*fun) {
                        return (args[1..].iter()
                            .map(|r| get_expr(*r, &reg_values, self))
                            .collect(), Some(idx), consumed_ops);
                    }
                }
                _ => {}
            }
        }
        (vec![], None, consumed_ops) // No constructor call found
    }

    /// Check if a function is a __constructor__
    pub(super) fn is_constructor_function(&self, fun: RefFun) -> bool {
        if let Some(func) = fun.as_fn(self.code) {
            self.code.strings.get(func.name.0)
                .map(|s| s.starts_with("__constructor__"))
                .unwrap_or(false)
        } else {
            false
        }
    }

    /// Check if a Type opcode's result is only used for array allocation (alloc_array/alloc_dynarray).
    /// The pattern is:
    ///   Type reg2 = SomeType
    ///   Int reg3 = 0
    ///   Call2 reg1 = alloc_array(reg2, reg3)
    /// In this case, the type is just array element type metadata and should be suppressed.
    pub(super) fn is_type_only_for_array_alloc(&self, type_dst: Reg, type_op_idx: usize) -> bool {
        use hlbc::types::FunPtr;

        // Look ahead for uses of the type register
        let search_limit = (type_op_idx + 10).min(self.func.ops.len());

        for idx in (type_op_idx + 1)..search_limit {
            let op = &self.func.ops[idx];

            match op {
                // The expected pattern: Call2 where first arg is the type register
                Opcode::Call2 { fun, arg0, .. } if *arg0 == type_dst => {
                    // Check if this is specifically the std library's alloc_array or alloc_dynarray
                    // We need to verify it's a native function from "std" to avoid false positives
                    // with user-defined functions that happen to have the same name
                    if let FunPtr::Native(native) = self.code.get(*fun) {
                        let lib = native.lib(self.code);
                        let name = native.name(self.code);
                        if lib == "std" && (name == "alloc_array" || name == "alloc_dynarray") {
                            return true;
                        }
                    }
                    // It's used for something else (not std alloc)
                    return false;
                }
                // Any other use of the register means it's not just for array alloc
                Opcode::Call1 { arg0, .. } if *arg0 == type_dst => return false,
                Opcode::Call2 { arg1, .. } if *arg1 == type_dst => return false, // second arg
                Opcode::Call3 { arg0, arg1, arg2, .. }
                    if *arg0 == type_dst || *arg1 == type_dst || *arg2 == type_dst => return false,
                Opcode::Mov { src, .. } if *src == type_dst => return false,
                Opcode::SetField { src, .. } if *src == type_dst => return false,
                Opcode::SetArray { src, .. } if *src == type_dst => return false,
                // If the register is overwritten before being used, it's safe to suppress
                Opcode::Type { dst, .. } if *dst == type_dst => return true,
                Opcode::Mov { dst, .. } if *dst == type_dst => return true,
                _ => {}
            }
        }

        // If we didn't find any use within the search range, it's probably safe
        false
    }
}
