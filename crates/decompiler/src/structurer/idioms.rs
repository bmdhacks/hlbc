//! Haxe/HashLink bytecode pattern detection.
//!
//! This module handles detection of Haxe-specific patterns in HashLink bytecode:
//! - String switch detection (9-opcode pattern per case)
//! - Enum switch pattern detection and unwrapping
//! - Internal function suppression (__expand, __construct, etc.)
//! - Interface cache field detection
//!
//! These patterns are "idioms" that aren't pure control flow, but rather
//! implementation details of how Haxe compiles to HashLink bytecode.

use std::collections::{HashMap, HashSet};

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, Reg, RefFun, RefField, RefString, RefType, RefGlobal, Type};
use hlbc::{Bytecode, Resolve};

use crate::ast::{Constant, Expr};
use crate::ssa::get_dst_reg as get_opcode_dst;
use hlbc::types::RefInt;

use super::{StringSwitchCase, StringSwitchRegion, Structurer};

impl<'a> Structurer<'a> {
    /// Detect internal function calls (__expand, __construct) and suppress them.
    /// These are runtime implementation details that shouldn't appear in decompiled output.
    /// NOTE: __constructor__ calls are handled in the Call handlers (stmts.rs), not here.
    pub fn detect_internal_function_calls(&mut self) {
        let ops = &self.func.ops;

        for (i, op) in ops.iter().enumerate() {
            // Extract function ref from call opcodes
            let fun = match op {
                Opcode::Call2 { fun, .. } => Some(*fun),
                Opcode::Call3 { fun, .. } => Some(*fun),
                Opcode::Call4 { fun, .. } => Some(*fun),
                Opcode::CallN { fun, .. } => Some(*fun),
                _ => None,
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
                        // NOTE: __constructor__ calls are NOT suppressed here.
                        // They are handled in the Call handlers (Call2/Call3/Call4/CallN)
                        // which emit proper `new Type(args)` syntax when there's a pending
                        // constructor, or fall through to emit the super() call.
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
    /// Used by legacy structurer - to be implemented in new path for string switch support.
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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

// ============================================================================
// Inline Expansion Detection
// ============================================================================

/// A detected inlined stdlib method expansion.
/// These represent compiler-generated inline expansions of methods like
/// BytesBuffer.addByte() that should be lifted back to method calls.
#[derive(Debug, Clone)]
pub(crate) struct InlineExpansion {
    /// First opcode index of the expansion
    pub start_op: usize,
    /// Last opcode index (inclusive)
    pub end_op: usize,
    /// The kind of inlined method
    pub kind: InlineExpansionKind,
    /// Register holding the buffer/object
    pub obj_reg: Reg,
    /// Register holding the value being written (or result for downcast)
    pub value_reg: Reg,
    /// If the value is a constant loaded inside the expansion (Int opcode),
    /// store the RefInt here so we can emit it directly.
    pub value_const: Option<RefInt>,
}

/// The kind of inlined expansion detected.
#[derive(Debug, Clone)]
pub(crate) enum InlineExpansionKind {
    /// BytesBuffer.addByte(value) — ~12 ops with JNotEq capacity check
    AddByte,
    /// BytesBuffer.addInt32(value) — ~16 ops with JSGte capacity check
    AddInt32,
    /// Std.downcast(value, Class) — 7 ops with check + conditional cast
    Downcast { class_global: RefGlobal, result_reg: Reg },
}

/// Detect inlined stdlib method expansions in a function's bytecode.
///
/// Scans the opcode stream for known patterns emitted by the Haxe compiler
/// when inlining BytesBuffer.addByte(), addInt32(), Std.downcast(), etc.
///
/// Returns detected expansions sorted by start_op (ascending).
pub(crate) fn detect_inline_expansions(code: &Bytecode, func: &Function) -> Vec<InlineExpansion> {
    let mut expansions = Vec::new();
    let ops = &func.ops;

    let mut i = 0;
    while i < ops.len() {
        // Try each pattern in order. On match, skip past the expansion.
        if let Some(exp) = try_detect_add_byte_at(code, func, i) {
            let end = exp.end_op;
            expansions.push(exp);
            i = end + 1;
        } else if let Some(exp) = try_detect_add_int32_at(code, func, i) {
            let end = exp.end_op;
            expansions.push(exp);
            i = end + 1;
        } else if let Some(exp) = try_detect_downcast_at(code, func, i) {
            let end = exp.end_op;
            expansions.push(exp);
            i = end + 1;
        } else {
            i += 1;
        }
    }

    expansions
}

/// Check if a register's type is haxe.io.BytesBuffer.
fn is_bytes_buffer_type(code: &Bytecode, func: &Function, reg: Reg) -> bool {
    let reg_idx = reg.0 as usize;
    if reg_idx >= func.regs.len() {
        return false;
    }
    let type_ref = func.regs[reg_idx];
    if let Some(Type::Obj(obj)) = code.types.get(type_ref.0) {
        let name = code.get(obj.name);
        name.as_ref() == "haxe.io.BytesBuffer"
    } else {
        false
    }
}

/// Check if a function reference is named "__expand".
fn is_expand_fn(code: &Bytecode, fun: RefFun) -> bool {
    if let Some(func) = fun.as_fn(code) {
        if let Some(name) = code.strings.get(func.name.0) {
            return name == "__expand";
        }
    }
    false
}

/// Try to detect BytesBuffer.addByte(value) inline expansion at position `i`.
///
/// Pattern (12 ops, optional NullCheck before):
/// ```text
/// [NullCheck  bufReg]                    ; optional, included if present
/// Field       posReg = bufReg.pos
/// Field       sizeReg = bufReg.size
/// JNotEq      if posReg != sizeReg → SKIP
/// Int         tmpReg = 0
/// Call2       _ = __expand(bufReg, tmpReg)
/// SKIP:
/// Field       bytesReg = bufReg.b
/// Field       posReg2 = bufReg.pos
/// Mov         idxReg = posReg2
/// Incr        posReg2++
/// SetField    bufReg.pos = posReg2
/// SetI8       bytesReg[idxReg] = valueReg
/// ```
fn try_detect_add_byte_at(code: &Bytecode, func: &Function, i: usize) -> Option<InlineExpansion> {
    let ops = &func.ops;

    // Check for optional leading NullCheck
    let (start, j) = if let Some(Opcode::NullCheck { reg }) = ops.get(i) {
        // NullCheck must target a BytesBuffer register
        if is_bytes_buffer_type(code, func, *reg) {
            (i, i + 1)
        } else {
            (i, i) // Not our NullCheck, start pattern at i
        }
    } else {
        (i, i)
    };

    // Need at least 12 ops from j
    if j + 11 >= ops.len() {
        return None;
    }

    // Op j+0: Field posReg = bufReg.pos
    let (pos_reg, buf_reg) = match &ops[j] {
        Opcode::Field { dst, obj, .. } => (*dst, *obj),
        _ => return None,
    };

    // Verify bufReg is BytesBuffer
    if !is_bytes_buffer_type(code, func, buf_reg) {
        return None;
    }

    // Op j+1: Field sizeReg = bufReg.size (same buf)
    let size_reg = match &ops[j + 1] {
        Opcode::Field { dst, obj, .. } if *obj == buf_reg => *dst,
        _ => return None,
    };

    // Op j+2: JNotEq posReg != sizeReg → skip over Int+Call2 (offset=2, target = j+2+2+1 = j+5)
    match &ops[j + 2] {
        Opcode::JNotEq { a, b, offset } => {
            if !({*a == pos_reg && *b == size_reg} || {*a == size_reg && *b == pos_reg}) {
                return None;
            }
            if *offset != 2 {
                return None; // Unexpected jump distance
            }
        }
        _ => return None,
    }

    // Op j+3: Int tmpReg = 0
    match &ops[j + 3] {
        Opcode::Int { .. } => {} // Value must be 0, but we trust the pattern shape
        _ => return None,
    }

    // Op j+4: Call2 _ = __expand(bufReg, tmpReg)
    match &ops[j + 4] {
        Opcode::Call2 { fun, arg0, .. } if *arg0 == buf_reg && is_expand_fn(code, *fun) => {}
        _ => return None,
    }

    // Op j+5: Field bytesReg = bufReg.b
    let bytes_reg = match &ops[j + 5] {
        Opcode::Field { dst, obj, .. } if *obj == buf_reg => *dst,
        _ => return None,
    };

    // Op j+6: Field posReg2 = bufReg.pos
    let pos_reg2 = match &ops[j + 6] {
        Opcode::Field { dst, obj, .. } if *obj == buf_reg => *dst,
        _ => return None,
    };

    // Op j+7: Mov idxReg = posReg2
    let idx_reg = match &ops[j + 7] {
        Opcode::Mov { dst, src } if *src == pos_reg2 => *dst,
        _ => return None,
    };

    // Op j+8: Incr posReg2++
    match &ops[j + 8] {
        Opcode::Incr { dst } if *dst == pos_reg2 => {}
        _ => return None,
    }

    // Op j+9: SetField bufReg.pos = posReg2
    match &ops[j + 9] {
        Opcode::SetField { obj, src, .. } if *obj == buf_reg && *src == pos_reg2 => {}
        _ => return None,
    }

    // Op j+10: SetI8 directly, or Int (value load) followed by SetI8 at j+11.
    // The compiler may load a constant value right before the SetI8.
    if let Some(Opcode::SetI8 { bytes, index, src }) = ops.get(j + 10) {
        if *bytes == bytes_reg && *index == idx_reg {
            return Some(InlineExpansion {
                start_op: start,
                end_op: j + 10,
                kind: InlineExpansionKind::AddByte,
                obj_reg: buf_reg,
                value_reg: *src,
                value_const: None,
            });
        }
    }

    // Try j+10: Int (value constant), j+11: SetI8
    if j + 11 < ops.len() {
        if let Opcode::Int { dst: _, ptr } = &ops[j + 10] {
            if let Some(Opcode::SetI8 { bytes, index, src }) = ops.get(j + 11) {
                if *bytes == bytes_reg && *index == idx_reg {
                    return Some(InlineExpansion {
                        start_op: start,
                        end_op: j + 11,
                        kind: InlineExpansionKind::AddByte,
                        obj_reg: buf_reg,
                        value_reg: *src,
                        value_const: Some(*ptr),
                    });
                }
            }
        }
    }

    None
}

/// Try to detect BytesBuffer.addInt32(value) inline expansion at position `i`.
///
/// Pattern (~16 ops):
/// ```text
/// [NullCheck  bufReg]
/// Field       posReg = bufReg.pos
/// Int         fourReg = 4
/// Add         sumReg = posReg + fourReg
/// Field       sizeReg = bufReg.size
/// JSGte       if sizeReg >= sumReg → SKIP
/// Int         tmpReg = 0
/// Call2       _ = __expand(bufReg, tmpReg)
/// SKIP:
/// Field       bytesReg = bufReg.b
/// Field       posReg2 = bufReg.pos
/// SetMem      bytesReg[posReg2] = valueReg
/// Field       posReg3 = bufReg.pos
/// Int         fourReg2 = 4
/// Add         newPosReg = posReg3 + fourReg2
/// SetField    bufReg.pos = newPosReg
/// ```
fn try_detect_add_int32_at(code: &Bytecode, func: &Function, i: usize) -> Option<InlineExpansion> {
    let ops = &func.ops;

    // Check for optional leading NullCheck
    let (start, j) = if let Some(Opcode::NullCheck { reg }) = ops.get(i) {
        if is_bytes_buffer_type(code, func, *reg) {
            (i, i + 1)
        } else {
            (i, i)
        }
    } else {
        (i, i)
    };

    // Need at least 15 ops from j
    if j + 14 >= ops.len() {
        return None;
    }

    // Op j+0: Field posReg = bufReg.pos
    let (pos_reg, buf_reg) = match &ops[j] {
        Opcode::Field { dst, obj, .. } => (*dst, *obj),
        _ => return None,
    };

    if !is_bytes_buffer_type(code, func, buf_reg) {
        return None;
    }

    // Op j+1: Int fourReg = 4
    let four_reg = match &ops[j + 1] {
        Opcode::Int { dst, .. } => *dst, // Trust the value is 4
        _ => return None,
    };

    // Op j+2: Add sumReg = posReg + fourReg
    let sum_reg = match &ops[j + 2] {
        Opcode::Add { dst, a, b } if *a == pos_reg && *b == four_reg => *dst,
        _ => return None,
    };

    // Op j+3: Field sizeReg = bufReg.size
    let size_reg = match &ops[j + 3] {
        Opcode::Field { dst, obj, .. } if *obj == buf_reg => *dst,
        _ => return None,
    };

    // Op j+4: JSGte if sizeReg >= sumReg → SKIP (offset=2, target = j+4+2+1 = j+7)
    match &ops[j + 4] {
        Opcode::JSGte { a, b, offset } if *a == size_reg && *b == sum_reg && *offset == 2 => {}
        _ => return None,
    }

    // Op j+5: Int tmpReg = 0
    match &ops[j + 5] {
        Opcode::Int { .. } => {}
        _ => return None,
    }

    // Op j+6: Call2 _ = __expand(bufReg, _)
    match &ops[j + 6] {
        Opcode::Call2 { fun, arg0, .. } if *arg0 == buf_reg && is_expand_fn(code, *fun) => {}
        _ => return None,
    }

    // Op j+7: Field bytesReg = bufReg.b
    let bytes_reg = match &ops[j + 7] {
        Opcode::Field { dst, obj, .. } if *obj == buf_reg => *dst,
        _ => return None,
    };

    // Op j+8: Field posReg2 = bufReg.pos
    let pos_reg2 = match &ops[j + 8] {
        Opcode::Field { dst, obj, .. } if *obj == buf_reg => *dst,
        _ => return None,
    };

    // Op j+9: SetMem bytesReg[posReg2] = valueReg
    let value_reg = match &ops[j + 9] {
        Opcode::SetMem { bytes, index, src } if *bytes == bytes_reg && *index == pos_reg2 => *src,
        _ => return None,
    };

    // Op j+10: Field posReg3 = bufReg.pos
    match &ops[j + 10] {
        Opcode::Field { obj, .. } if *obj == buf_reg => {}
        _ => return None,
    }

    // Op j+11: Int _ = 4
    match &ops[j + 11] {
        Opcode::Int { .. } => {}
        _ => return None,
    }

    // Op j+12: Add newPos = pos + 4
    let new_pos_reg = match &ops[j + 12] {
        Opcode::Add { dst, .. } => *dst,
        _ => return None,
    };

    // Op j+13: SetField bufReg.pos = newPosReg
    match &ops[j + 13] {
        Opcode::SetField { obj, src, .. } if *obj == buf_reg && *src == new_pos_reg => {}
        _ => return None,
    }

    Some(InlineExpansion {
        start_op: start,
        end_op: j + 13,
        kind: InlineExpansionKind::AddInt32,
        obj_reg: buf_reg,
        value_reg,
        value_const: None,
    })
}

/// Try to detect Std.downcast(value, Class) inline expansion at position `i`.
///
/// Pattern (7 ops):
/// ```text
/// GetGlobal   classReg = global@N
/// Call2       boolReg = check(classReg, valueReg)
/// JFalse      if boolReg == false → NULL_LABEL (+3)
/// ToVirtual   resultReg = cast valueReg   (or SafeCast/UnsafeCast)
/// JAlways     → END_LABEL (+1)
/// Null        resultReg = null
/// ```
fn try_detect_downcast_at(code: &Bytecode, func: &Function, i: usize) -> Option<InlineExpansion> {
    let ops = &func.ops;

    // Need at least 6 ops from i
    if i + 5 >= ops.len() {
        return None;
    }

    // Op i+0: GetGlobal classReg = global@N
    let (class_reg, class_global) = match &ops[i] {
        Opcode::GetGlobal { dst, global } => (*dst, *global),
        _ => return None,
    };

    // Verify the global is a $-prefixed class companion type
    // The global's type should be an Obj type with a $ prefix in its name
    let global_type_ref = if let Some(ty) = code.globals.get(class_global.0) {
        *ty
    } else {
        return None;
    };
    if let Some(Type::Obj(obj)) = code.types.get(global_type_ref.0) {
        let name = code.get(obj.name);
        if !name.contains(".$") && !name.starts_with('$') {
            return None; // Not a class companion
        }
    } else {
        return None;
    }

    // Op i+1: Call2 boolReg = check(classReg, valueReg)
    let (bool_reg, value_reg) = match &ops[i + 1] {
        Opcode::Call2 { dst, fun, arg0, arg1 } if *arg0 == class_reg => {
            // Verify function is named "check" (hl.BaseType.check)
            if let Some(f) = fun.as_fn(code) {
                if let Some(name) = code.strings.get(f.name.0) {
                    if name != "check" {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                return None;
            }
            (*dst, *arg1)
        }
        _ => return None,
    };

    // Op i+2: JFalse if boolReg == false → NULL_LABEL (offset=2, target = i+2+2+1 = i+5)
    match &ops[i + 2] {
        Opcode::JFalse { cond, offset } if *cond == bool_reg && *offset == 2 => {}
        _ => return None,
    }

    // Op i+3: ToVirtual or SafeCast or UnsafeCast
    let result_reg = match &ops[i + 3] {
        Opcode::ToVirtual { dst, src } if *src == value_reg => *dst,
        Opcode::SafeCast { dst, src } if *src == value_reg => *dst,
        Opcode::UnsafeCast { dst, src } if *src == value_reg => *dst,
        _ => return None,
    };

    // Op i+4: JAlways → END_LABEL (offset=1, target = i+4+1+1 = i+6, past the Null at i+5)
    match &ops[i + 4] {
        Opcode::JAlways { offset } if *offset == 1 => {}
        _ => return None,
    }

    // Op i+5: Null resultReg = null
    match &ops[i + 5] {
        Opcode::Null { dst } if *dst == result_reg => {}
        _ => return None,
    }

    Some(InlineExpansion {
        start_op: i,
        end_op: i + 5,
        kind: InlineExpansionKind::Downcast {
            class_global,
            result_reg,
        },
        obj_reg: class_reg, // The class companion register
        value_reg,
        value_const: None,
    })
}
