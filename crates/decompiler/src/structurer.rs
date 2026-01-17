//! Pass 5: Structurer - Convert SSA-CFG to structured AST
//!
//! This module transforms an SSA-annotated CFG back into structured code:
//! - Uses loop info from Analyzer to emit `while` loops
//! - Uses dominator tree to structure if/else
//! - Converts φ-functions to variable assignments at branch ends
//!
//! This is a simplified implementation focused on correctness over optimization.

use petgraph::graph::NodeIndex;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, Reg, RefFun, RefField, RefString, RefEnumConstruct, Type};
use hlbc::{Bytecode, Resolve, Str};

use crate::analyzer::{CfgAnalysis, NaturalLoop};
use crate::ast::{not, Call, Constant, ConstructorCall, Expr, Operation, Statement};
use crate::lifter::{BasicBlock, Cfg};
use hlbc::types::RefType;
use crate::ssa::{SsaCfg, SsaInstr, SsaVar, get_dst_reg as get_opcode_dst};
use crate::type_prop::TypeInfo;

use crate::ssa::UseDefInfo;

use crate::closure_analysis::ClosureAnalysis;
use crate::exception_analysis::{ExceptionAnalysis, TryRegion};

/// Tracks memory dependencies for SSA inline expressions.
/// Used to determine when an inline expression must be invalidated
/// because its source memory has been modified.
#[derive(Clone, Debug)]
enum MemoryDep {
    /// No memory dependency - constants, arithmetic results.
    /// These are always safe to inline.
    None,
    /// Depends on reading a specific field of an object.
    /// Invalidated when SetField writes to the same (obj_reg, field_idx).
    Field { obj: Reg, field: usize },
    /// Depends on reading a specific global variable.
    /// Invalidated when SetGlobal writes to the same global.
    Global { global: hlbc::types::RefGlobal },
    /// Conservative dependency - could read any memory.
    /// Invalidated by any memory write or call.
    AnyMemory,
}

/// A detected string switch case
#[derive(Debug, Clone)]
struct StringSwitchCase {
    /// The string literal for this case
    string_ref: RefString,
    /// Opcode index of the handler (where JEq jumps to)
    handler_op: usize,
}

/// A detected string switch region in bytecode
#[derive(Debug, Clone)]
struct StringSwitchRegion {
    /// First opcode of the switch (the first JNull)
    start_op: usize,
    /// Last opcode of the switch pattern (before handlers/default)
    end_op: usize,
    /// The register holding the string being switched on
    switch_arg_reg: Reg,
    /// All detected cases with their string values and handler targets
    cases: Vec<StringSwitchCase>,
    /// Opcode index of the default case handler
    default_op: usize,
}

/// Context for structuring
pub struct Structurer<'a> {
    code: &'a Bytecode,
    func: &'a Function,
    cfg: &'a Cfg,
    analysis: &'a CfgAnalysis,
    ssa: &'a SsaCfg,
    _type_info: &'a TypeInfo,
    /// Closure analysis for detecting/handling closures
    closure_analysis: Option<&'a ClosureAnalysis>,

    /// Processed blocks (to avoid re-processing)
    processed: HashSet<NodeIndex>,
    /// Use-def info for inlining decisions (ILSpy-style)
    use_info: HashMap<SsaVar, UseDefInfo>,
    /// Variable names that have been declared (for declaration tracking)
    declared_vars: HashSet<Str>,
    /// Current opcode index being processed (for debug name lookup)
    current_op: usize,
    /// Method info: maps function references to (owner_type, method_name)
    /// Used to convert f(obj, args) to obj.f(args) syntax
    method_info: HashMap<RefFun, (RefType, Str)>,
    /// Current scope depth (0 = function level, >0 = inside loop/if/switch)
    scope_depth: u32,
    /// Variables that need hoisting to function scope (declared inside nested scope)
    hoisted_vars: HashSet<Str>,
    /// Hoisted vars that need :Dynamic type (assigned empty anonymous objects)
    needs_dynamic_type: HashSet<Str>,
    /// Hoisted vars -> their types (for type hints in declarations)
    hoisted_var_types: HashMap<Str, RefType>,
    /// Array bytes tracking: maps bytes register -> array expression
    /// Used to reconstruct arr[i] from bytes[shifted_i] pattern
    /// We store the Expr (not Reg) to capture the correct SSA version at field access time
    array_bytes_source: HashMap<Reg, Expr>,
    /// Shifted index tracking: maps shifted reg -> (original index expression, shift amount)
    /// Used to reverse index * 4 back to original index for array access
    /// We store the Expr (not Reg) to capture the correct SSA version at shift time
    shifted_indices: HashMap<Reg, (Expr, i32)>,
    /// Exception region analysis for try/catch structuring
    exception_analysis: ExceptionAnalysis,
    /// Enum global -> (enum type, constructor index) mapping
    /// Built by analyzing the entry point function's enum initialization pattern
    enum_global_map: HashMap<hlbc::types::RefGlobal, (RefType, usize)>,
    /// String conversion sources: maps length output reg -> original value expression
    /// Used to track ftos/itos/dtos(value, ref_out) where ref_out points to length_reg
    /// When __alloc__(bytes, length_reg) is seen, we can use the original value expression
    /// We store Expr (not Reg) to capture the correct SSA-versioned name at conversion time
    string_conversion_source: HashMap<Reg, Expr>,
    /// Ref targets: maps ref reg -> target reg (for Ref dst = &src)
    ref_targets: HashMap<Reg, Reg>,
    /// Registers that should use raw names (rN) instead of debug names
    /// to avoid type conflicts (e.g., iterator vs iteration value)
    use_raw_name_regs: HashSet<Reg>,
    /// Registers that hold iterators (from .keys() or .iterator() calls)
    /// Used to detect when these are later reassigned to non-iterator values
    iterator_regs: HashSet<Reg>,
    /// Current SSA destination variable (set when processing each opcode)
    /// Used for SSA-versioned naming
    current_ssa_dst: Option<SsaVar>,
    /// Current SSA source variables (set when processing each opcode)
    /// Used for SSA-versioned naming of source operands
    current_ssa_uses: Vec<SsaVar>,
    /// Detected string switch regions (from bytecode pattern analysis)
    string_switches: Vec<StringSwitchRegion>,
    /// Recursion depth counter to prevent stack overflow
    recursion_depth: u32,
    /// Current loop header (if any) - used to distinguish continue from switch fall-through
    current_loop_header: Option<NodeIndex>,
    /// Expressions available for inlining (SSA var -> (expression, memory dependency))
    /// Single-use, pure expressions are stored here instead of emitting a statement.
    /// When the variable is referenced, the stored expression is inlined at the use site.
    /// The memory dependency tracks what memory the expression reads, allowing smart
    /// invalidation when conflicting writes occur.
    /// Uses RefCell for interior mutability so we can remove entries when inlined.
    inline_exprs: RefCell<HashMap<SsaVar, (Expr, MemoryDep)>>,
    /// Opcodes to suppress (not emit as statements)
    /// Used when an opcode's result is consumed by another construct (e.g., EnumIndex for switch)
    suppressed_ops: HashSet<usize>,
    /// Enum pattern bindings: maps (switch_reg, construct_idx, field_idx) -> bound param name
    /// When set, EnumField opcodes matching these keys emit the param name
    /// instead of Type.enumParameters(...). Used to generate cleaner switch case patterns.
    enum_param_bindings: HashMap<(Reg, usize, usize), String>,
}

impl<'a> Structurer<'a> {
    pub fn new(
        code: &'a Bytecode,
        func: &'a Function,
        cfg: &'a Cfg,
        analysis: &'a CfgAnalysis,
        ssa: &'a SsaCfg,
        type_info: &'a TypeInfo,
    ) -> Self {
        Self::new_with_closures(code, func, cfg, analysis, ssa, type_info, None)
    }

    pub fn new_with_closures(
        code: &'a Bytecode,
        func: &'a Function,
        cfg: &'a Cfg,
        analysis: &'a CfgAnalysis,
        ssa: &'a SsaCfg,
        type_info: &'a TypeInfo,
        closure_analysis: Option<&'a ClosureAnalysis>,
    ) -> Self {
        // Compute use counts for inlining decisions (needs func for purity info)
        let use_info = ssa.compute_use_counts(func);

        // Build method info map from all types' protos
        let method_info = Self::build_method_info(code);

        // Pre-populate declared_vars with parameter names so we don't hoist them
        let mut declared_vars = HashSet::new();
        if let Some(Type::Fun(fun_type) | Type::Method(fun_type)) = code.types.get(func.t.0) {
            let num_args = fun_type.args.len();
            for i in 0..num_args {
                if let Some(param_name) = func.arg_name(code, i) {
                    declared_vars.insert(param_name.into());
                }
            }
        }

        // Analyze exception regions for try/catch structuring
        let exception_analysis = ExceptionAnalysis::analyze(func);

        Structurer {
            code,
            func,
            cfg,
            analysis,
            ssa,
            _type_info: type_info,
            closure_analysis,
            processed: HashSet::new(),
            use_info,
            exception_analysis,
            declared_vars,
            current_op: 0,
            method_info,
            scope_depth: 0,
            hoisted_vars: HashSet::new(),
            needs_dynamic_type: HashSet::new(),
            hoisted_var_types: HashMap::new(),
            array_bytes_source: HashMap::new(),
            shifted_indices: HashMap::new(),
            enum_global_map: Self::build_enum_global_map(code),
            string_conversion_source: HashMap::new(),
            ref_targets: HashMap::new(),
            use_raw_name_regs: HashSet::new(),
            iterator_regs: HashSet::new(),
            current_ssa_dst: None,
            current_ssa_uses: Vec::new(),
            string_switches: Self::detect_string_switches(code, func),
            recursion_depth: 0,
            current_loop_header: None,
            inline_exprs: RefCell::new(HashMap::new()),
            suppressed_ops: HashSet::new(),
            enum_param_bindings: HashMap::new(),
        }
    }

    /// Detect EnumIndex → Switch patterns and mark EnumIndex for suppression.
    /// Pattern:
    ///   EnumIndex dst = value
    ///   ... (0 or more ops)
    ///   Switch dst
    /// When detected, the EnumIndex opcode is suppressed and the switch uses the original enum.
    fn detect_enum_switch_patterns(&mut self) {
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
                    if let Some(def_dst) = crate::ssa::get_dst_reg(&ops[j]) {
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
    fn detect_internal_function_calls(&mut self) {
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
    fn detect_string_switches(code: &Bytecode, func: &Function) -> Vec<StringSwitchRegion> {
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
    ///   initEnum(type, enumType) -> enum_obj
    ///   enum_obj.__evalues__ -> evalues_array
    ///   evalues_array[N] -> value
    ///   global@X = value   // global X is constructor N
    fn build_enum_global_map(code: &Bytecode) -> HashMap<hlbc::types::RefGlobal, (RefType, usize)> {
        let mut map = HashMap::new();

        // Skip if no functions are registered (e.g., in tests with mock bytecode)
        if code.findex_max() == 0 {
            return map;
        }

        // Get the entry point function using the proper lookup
        let entry_func = code.entrypoint();

        // Track the last known array index and the enum type being initialized
        let mut current_index: Option<usize> = None;
        let mut enum_type_reg: Option<Reg> = None;
        let mut evalues_reg: Option<Reg> = None;
        let mut current_enum_type: Option<RefType> = None;

        for op in entry_func.ops.iter() {
            match op {
                // Track: Type reg = enum<...>
                Opcode::Type { dst, ty } => {
                    if let Some(Type::Enum { .. }) = code.types.get(ty.0) {
                        enum_type_reg = Some(*dst);
                        current_enum_type = Some(*ty);
                    }
                }
                // Track: Call2 initEnum(_, enumType) -> reg
                // The result has __evalues__ field
                Opcode::Call2 { dst: _, fun, arg0: _, arg1 } => {
                    // Check if this is initEnum by looking at the function name
                    if let Some(f) = code.functions.get(fun.0) {
                        let name = f.name(code);
                        if name.as_ref() == "initEnum" {
                            // arg1 is the enum type register
                            if enum_type_reg == Some(*arg1) {
                                // dst now holds the hl.Enum object - track for Field access
                                evalues_reg = None; // Reset until we see Field
                            }
                        }
                    }
                }
                // Track: Field reg = enumObj.__evalues__
                Opcode::Field { dst, obj: _, field } => {
                    if let Some(field_name) = code.strings.get(field.0) {
                        if field_name.as_ref() == "__evalues__" {
                            evalues_reg = Some(*dst);
                        }
                    }
                }
                // Track: Int reg = N (the constructor index)
                Opcode::Int { dst: _, ptr } => {
                    if let Some(val) = code.ints.get(ptr.0) {
                        current_index = Some(*val as usize);
                    }
                }
                // Track: GetArray result = evalues[index]
                Opcode::GetArray { dst: _, array, index: _ } => {
                    if evalues_reg == Some(*array) {
                        // Good - we're reading from __evalues__ with current_index
                    }
                }
                // Track: SetGlobal global = value
                Opcode::SetGlobal { global, src: _ } => {
                    // If we have a valid enum type and index, record the mapping
                    if let (Some(idx), Some(enum_t)) = (current_index, current_enum_type) {
                        // Verify this global has the enum type
                        if let Some(global_type) = code.globals.get(global.0) {
                            if *global_type == enum_t {
                                map.insert(*global, (enum_t, idx));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        map
    }

    /// Build a map from function references to their owner types and method names.
    /// This is used to convert f(obj, args) to obj.f(args) syntax.
    fn build_method_info(code: &Bytecode) -> HashMap<RefFun, (RefType, Str)> {
        let mut info = HashMap::new();

        for (type_idx, ty) in code.types.iter().enumerate() {
            if let Type::Obj(obj) = ty {
                for proto in &obj.protos {
                    let method_name: Str = code.strings.get(proto.name.0)
                        .cloned()
                        .unwrap_or_else(|| format!("method_{}", proto.findex.0).into());
                    info.insert(proto.findex, (RefType(type_idx), method_name));
                }
            }
        }

        info
    }

    /// Create an assignment statement, tracking declaration status.
    /// Returns a Statement::Assign with declaration=true if this is the first
    /// assignment to this variable name.
    ///
    /// IMPORTANT: Variables are only declared (with `var`) at scope depth 0 to avoid
    /// scoping issues where a variable declared inside a loop is not visible outside.
    fn make_assign(&mut self, variable: Expr, assign: Expr) -> Statement {
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
    fn is_empty_anonymous(expr: &Expr) -> bool {
        matches!(expr, Expr::Anonymous(_, fields) if fields.is_empty())
    }

    /// Create a call statement, handling void return types correctly.
    /// For void functions, we emit just the call as an expression statement.
    /// For non-void functions, we assign the result to a variable.
    fn make_call_stmt(&mut self, dst: Reg, call: Call) -> Statement {
        if self.is_void_type(dst) {
            Statement::ExprStatement(Expr::Call(Box::new(call)))
        } else {
            let var = self.reg_to_expr_dst(dst);
            self.make_assign(var, Expr::Call(Box::new(call)))
        }
    }

    /// Get the fully qualified class name of this function's containing class, if any.
    fn get_current_class_name(&self) -> Option<Str> {
        self.func.parent.map(|parent_ref| {
            self.code[parent_ref].get_type_obj()
                .map(|obj| obj.name(self.code))
                .unwrap_or_else(|| Str::from(""))
        })
    }

    /// Structure the entire function into statements
    pub fn structure(&mut self) -> Vec<Statement> {
        // Pre-process: detect patterns that should be suppressed
        self.detect_enum_switch_patterns();
        self.detect_internal_function_calls();

        // For functions with exception regions, use opcode-range-based structuring
        // which handles nested Trap/EndTrap correctly without CFG edge interference.
        // For functions without exceptions, use CFG-based structuring.
        let stmts = if self.exception_analysis.has_exceptions() {
            self.structure_block_range(0, self.func.ops.len())
        } else {
            self.structure_from(self.cfg.entry, None)
        };

        // Prepend hoisted variable declarations (for vars first assigned inside scopes)
        let mut result = Vec::new();
        let current_class = self.get_current_class_name();
        let current_class_str = current_class.as_ref().map(|s| s.as_ref());
        for name in &self.hoisted_vars {
            // Add type hint based on:
            // 1. :Dynamic for vars that will hold empty anonymous objects
            // 2. Type from hoisted_var_types if available
            // 3. None if no type info
            let type_hint = if self.needs_dynamic_type.contains(name) {
                Some("Dynamic".into())
            } else if let Some(type_ref) = self.hoisted_var_types.get(name) {
                let ty = &self.code.types[type_ref.0];
                // Use context-aware type formatting to simplify nested types
                let type_str = crate::fmt::to_haxe_type_in_context(ty, self.code, current_class_str);
                // Don't emit Void type hints - use Dynamic instead
                // (Void variables are not valid in Haxe)
                // Also use Dynamic for haxe.Exception since catch blocks
                // can catch any type, not just Exception
                if type_str == "Void" || type_str == "haxe.Exception" {
                    Some("Dynamic".into())
                } else {
                    Some(type_str)
                }
            } else {
                None
            };
            result.push(Statement::VarDecl { name: name.clone(), type_hint });
        }
        result.extend(stmts);

        // Post-process to simplify
        simplify_statements(result)
    }

    /// Check if an SSA variable is dead (defined but never used)
    fn is_dead_var(&self, var: SsaVar) -> bool {
        self.use_info.get(&var).map_or(false, |info| info.is_dead())
    }

    /// Check if a register is a closure context (EnumAlloc result for a closure)
    fn is_closure_context_reg(&self, reg: Reg) -> bool {
        if let Some(analysis) = self.closure_analysis {
            analysis.is_context_reg(self.func.findex, reg)
        } else {
            false
        }
    }

    /// Get closure info if the current opcode is an InstanceClosure that we should inline
    fn get_closure_at_current_op(&self) -> Option<&crate::closure_analysis::CaptureInfo> {
        if let Some(analysis) = self.closure_analysis {
            analysis.get_closure_at(self.func.findex, self.current_op)
        } else {
            None
        }
    }

    /// Check if the current function is a closure (inner function with capture context)
    fn is_current_function_closure(&self) -> bool {
        if let Some(analysis) = self.closure_analysis {
            analysis.is_closure(self.func.findex)
        } else {
            false
        }
    }

    /// Get capture info for the current function if it's a closure
    fn get_current_capture_info(&self) -> Option<&crate::closure_analysis::CaptureInfo> {
        if let Some(analysis) = self.closure_analysis {
            analysis.get_capture_info(self.func.findex)
        } else {
            None
        }
    }

    /// Get captured variable name from an EnumField access in a closure body
    fn get_captured_var_name(&self, field: RefField) -> Option<Str> {
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

    /// Structure code starting from a given block
    fn structure_from(&mut self, start: NodeIndex, stop_at: Option<NodeIndex>) -> Vec<Statement> {
        if Some(start) == stop_at || self.processed.contains(&start) {
            return vec![];
        }

        // Prevent stack overflow from deep recursion
        const MAX_RECURSION_DEPTH: u32 = 500;
        if self.recursion_depth >= MAX_RECURSION_DEPTH {
            return vec![Statement::Comment(format!(
                "// [deep nesting truncated at depth {} - remaining {} blocks]",
                MAX_RECURSION_DEPTH,
                self.cfg.graph.node_count() - self.processed.len()
            ))];
        }
        self.recursion_depth += 1;

        let result = self.structure_from_inner(start, stop_at);

        self.recursion_depth -= 1;
        result
    }

    /// Inner implementation of structure_from (separated for recursion depth tracking)
    /// Uses iterative processing for sequential blocks to avoid deep recursion
    fn structure_from_inner(&mut self, start: NodeIndex, stop_at: Option<NodeIndex>) -> Vec<Statement> {
        let mut all_stmts = Vec::new();
        let mut current = Some(start);

        while let Some(block) = current {
            // Check stop conditions
            if Some(block) == stop_at || self.processed.contains(&block) {
                break;
            }

            let block_start_op = self.cfg.graph[block].start;
            let block_end_op = self.cfg.graph[block].end;

            // Check if this block CONTAINS a string switch (may start mid-block after setup ops)
            if let Some(switch_region) = self.string_switches.iter()
                .find(|s| s.start_op >= block_start_op && s.start_op <= block_end_op)
                .cloned()
            {
                // Structure any ops before the switch starts (e.g., NullCheck, Field ops)
                if switch_region.start_op > block_start_op {
                    for op_idx in block_start_op..switch_region.start_op {
                        self.current_op = op_idx;
                        all_stmts.extend(self.opcode_to_statements(op_idx));
                    }
                }
                all_stmts.extend(self.structure_string_switch(&switch_region, stop_at));
                break; // String switch handles its own continuation
            }

            // Check if this is a loop header
            if let Some(loop_info) = self.analysis.loops.iter().find(|l| l.header == block).cloned() {
                all_stmts.extend(self.structure_loop(&loop_info, stop_at));
                break; // Loop handles its own continuation
            }

            self.processed.insert(block);
            let stmts = self.structure_block(block);
            all_stmts.extend(stmts);

            // Get successors
            let succs: Vec<NodeIndex> = self.cfg.successors(block);

            match succs.len() {
                0 => {
                    // Terminal block - done
                    break;
                }
                1 => {
                    // Single successor - check for continue/break, then iterate
                    let is_in_loop = self.current_loop_header.is_some()
                        && Some(succs[0]) == self.current_loop_header;
                    if is_in_loop && Some(succs[0]) == stop_at {
                        let block_data = &self.cfg.graph[block];
                        let is_pure_jump = block_data.start == block_data.end
                            && matches!(self.func.ops.get(block_data.start), Some(Opcode::JAlways { .. } | Opcode::Label));
                        if is_pure_jump || block_data.end == block_data.start {
                            all_stmts.push(Statement::Continue);
                            break;
                        }
                    }

                    if let Some(header) = self.current_loop_header {
                        if let Some(loop_info) = self.analysis.loops.iter().find(|l| l.header == header) {
                            if !loop_info.body.contains(&succs[0]) {
                                let block_data = &self.cfg.graph[block];
                                let is_pure_jump = block_data.start == block_data.end
                                    && matches!(self.func.ops.get(block_data.start), Some(Opcode::JAlways { .. } | Opcode::Label));
                                if is_pure_jump || block_data.end == block_data.start {
                                    all_stmts.push(Statement::Break);
                                    break;
                                }
                            }
                        }
                    }

                    // Continue to next block iteratively
                    current = Some(succs[0]);
                }
                2 => {
                    // Conditional - structure it and get the continuation point
                    let (cond_stmts, continuation) = self.structure_conditional_with_continuation(block, &succs, stop_at);
                    all_stmts.extend(cond_stmts);
                    current = continuation;
                }
                _ => {
                    // Switch statement - structure it and get the continuation point
                    let (switch_stmts, continuation) = self.structure_switch_with_continuation(block, &succs, stop_at);
                    all_stmts.extend(switch_stmts);
                    current = continuation;
                }
            }
        }

        all_stmts
    }

    /// Structure a detected string switch pattern
    fn structure_string_switch(
        &mut self,
        region: &StringSwitchRegion,
        _stop_at: Option<NodeIndex>,
    ) -> Vec<Statement> {
        // Mark all blocks that contain the switch pattern as processed
        // But NOT the handler blocks (which might share a block with pattern ops)
        let handler_ops: std::collections::HashSet<usize> = region.cases.iter()
            .map(|c| c.handler_op)
            .chain(std::iter::once(region.default_op))
            .collect();

        for op_idx in region.start_op..=region.end_op {
            if let Some(&block) = self.cfg.op_to_block.get(&op_idx) {
                let block_start = self.cfg.graph[block].start;
                // Only mark as processed if this block doesn't start a handler
                if !handler_ops.contains(&block_start) {
                    self.processed.insert(block);
                }
            }
        }

        // Build the switch argument expression (the string being compared)
        let switch_arg = self.reg_to_expr(region.switch_arg_reg);

        // Build cases
        let mut cases = Vec::new();
        for case in &region.cases {
            let case_value = Expr::Constant(Constant::String(case.string_ref));

            // Structure the case handler
            // Find the block that contains the handler opcode
            let handler_body = if let Some(&handler_block) = self.cfg.op_to_block.get(&case.handler_op) {
                if !self.processed.contains(&handler_block) {
                    // Note: structure_from will mark the block as processed
                    self.structure_from(handler_block, None)
                } else {
                    vec![]
                }
            } else {
                // Fallback: structure the handler as a block range
                self.structure_block_range(case.handler_op, self.func.ops.len())
            };

            cases.push((vec![case_value], handler_body));
        }

        // Structure the default case
        let default_body = if let Some(&default_block) = self.cfg.op_to_block.get(&region.default_op) {
            if !self.processed.contains(&default_block) {
                // Note: structure_from will mark the block as processed
                self.structure_from(default_block, None)
            } else {
                vec![]
            }
        } else {
            self.structure_block_range(region.default_op, self.func.ops.len())
        };

        vec![Statement::Switch {
            arg: switch_arg,
            default: default_body,
            cases,
            enum_type: None,
        }]
    }

    /// Structure a loop
    fn structure_loop(&mut self, loop_info: &NaturalLoop, stop_at: Option<NodeIndex>) -> Vec<Statement> {
        let header = loop_info.header;
        self.processed.insert(header);

        // IMPORTANT: Process header block statements FIRST before extracting condition.
        // This populates inline_exprs so that the loop condition can use inlined expressions.
        // Otherwise, variables marked for inlining won't have their declarations emitted,
        // but the condition extraction won't find them in inline_exprs.
        let header_block = &self.cfg.graph[header];
        self.scope_depth += 1;
        let header_stmts = self.structure_block_range(header_block.start, header_block.end);
        self.scope_depth -= 1;

        // Extract condition and body start (now inline_exprs is populated)
        let (condition, body_start, exit_target) = self.extract_loop_condition(loop_info);

        // Structure body (increment scope depth to avoid declaring vars inside loop)
        let body = if let Some(body_node) = body_start {
            // Temporarily allow processing body nodes
            let old_processed = self.processed.clone();
            for &node in &loop_info.body {
                if node != header {
                    self.processed.remove(&node);
                }
            }
            // NOTE: We no longer mark exit targets as processed here.
            // Instead, we rely on break detection in structure_from to handle
            // blocks whose successor is outside the loop.
            // This allows break blocks to be properly structured and emit Break statements.
            self.scope_depth += 1;
            let old_loop_header = self.current_loop_header;
            self.current_loop_header = Some(header);
            let body_stmts = self.structure_from(body_node, Some(header));
            self.current_loop_header = old_loop_header;
            self.scope_depth -= 1;
            self.processed = old_processed;
            for &node in &loop_info.body {
                self.processed.insert(node);
            }
            body_stmts
        } else {
            vec![]
        };

        // Build the loop structure
        let loop_stmt = if header_stmts.is_empty() {
            // No header statements - simple while(condition)
            Statement::While {
                cond: condition,
                stmts: body,
            }
        } else {
            // Header has statements that compute the condition
            // Use while(true) { header_stmts; if (!cond) break; body; }
            // The condition is the "continue condition" so we break when it's false
            let break_cond = Expr::Op(Operation::Not(Box::new(condition)));
            let break_stmt = Statement::IfElse {
                cond: break_cond,
                if_: vec![Statement::Break],
                else_: vec![],
            };
            let mut loop_body = header_stmts;
            loop_body.push(break_stmt);
            loop_body.extend(body);
            Statement::While {
                cond: Expr::Constant(Constant::Bool(true)),
                stmts: loop_body,
            }
        };

        let mut result = vec![loop_stmt];

        // Continue after loop
        if let Some(exit) = exit_target {
            if Some(exit) != stop_at && !self.processed.contains(&exit) {
                result.extend(self.structure_from(exit, stop_at));
            }
        }

        result
    }

    /// Structure a range of opcodes into statements (for header blocks)
    fn structure_block_range(&mut self, start: usize, end: usize) -> Vec<Statement> {
        let mut stmts = Vec::new();
        let mut op_idx = start;

        while op_idx < end {
            self.current_op = op_idx;

            // Check if this opcode starts an exception region
            // Clone the region to avoid borrow checker issues
            if let Some(region) = self.exception_analysis.region_starting_at(op_idx).cloned() {
                // Compute catch_end: find next sequential region or use end
                let catch_end = self.find_catch_end(region.handler_op, end);

                // Structure the try/catch and skip past the entire region
                let try_catch = self.structure_exception_region(&region, catch_end);
                stmts.push(try_catch);
                // Skip past the catch body to continue after the try/catch
                op_idx = catch_end;
                continue;
            }

            // Check if this is a Switch opcode - needs special handling
            if let Opcode::Switch { .. } = &self.func.ops[op_idx] {
                if let Some(block) = self.cfg.op_to_block.get(&op_idx).copied() {
                    // Use CFG-based switch structuring
                    let succs = self.cfg.successors(block);
                    let switch_stmts = self.structure_switch(block, &succs, None);
                    stmts.extend(switch_stmts);
                    // Skip to merge point or continue after switch
                    if let Some(merge) = self.find_switch_merge_point(block) {
                        let merge_block = &self.cfg.graph[merge];
                        op_idx = merge_block.start;
                    } else {
                        // No merge point found, just continue past the switch opcode
                        op_idx += 1;
                    }
                    continue;
                } else {
                    // No CFG block for this switch - emit a comment and continue
                    stmts.push(Statement::Comment(format!(
                        "switch at {} (no CFG block)",
                        op_idx
                    )));
                    op_idx += 1;
                    continue;
                }
            }

            stmts.extend(self.opcode_to_statements(op_idx));
            op_idx += 1;
        }
        stmts
    }

    /// Find where a catch body ends
    /// Returns the first opcode after the catch body
    fn find_catch_end(&self, handler_start: usize, outer_end: usize) -> usize {
        // Look for the next top-level try region that starts after handler_start
        // That would be the start of the next sequential try/catch
        let mut catch_end = outer_end;

        for region in self.exception_analysis.top_level_regions() {
            if region.trap_op > handler_start && region.trap_op < catch_end {
                catch_end = region.trap_op;
            }
        }

        catch_end
    }

    /// Structure a try/catch region using exception analysis
    fn structure_exception_region(&mut self, region: &TryRegion, outer_end: usize) -> Statement {
        // Get the exception variable name from the Trap opcode
        let catch_var = self.reg_name(region.exc_reg).to_string();

        // Try body: from trap+1 to end_trap (excluding EndTrap itself which is control flow)
        // The EndTrap marks the end of the try body's normal exit path
        self.scope_depth += 1;
        let try_stmts = self.structure_opcode_range(
            region.trap_op + 1,
            region.end_trap_op, // End before EndTrap
            &region.nested,
        );
        self.scope_depth -= 1;

        // Catch body: from handler to computed end
        // Catch body ends at: outer_end (for top-level), or function end
        // For nested regions, catch body is inside outer region's try body
        let catch_end = outer_end;

        // Mark the exception register to use raw names in the catch body
        // This ensures uses of the caught exception match the catch parameter name
        // (prevents SSA versioning from creating mismatched names like `catch (r0)` vs `r0_1`)
        self.use_raw_name_regs.insert(region.exc_reg);

        self.scope_depth += 1;
        let catch_stmts = self.structure_opcode_range(
            region.handler_op,
            catch_end,
            &[], // Nested regions in catch body would need separate tracking
        );
        self.scope_depth -= 1;

        Statement::TryCatch {
            try_stmts,
            catch_var,
            catch_stmts,
        }
    }

    /// Structure a range of opcodes, handling nested exception regions
    fn structure_opcode_range(
        &mut self,
        start: usize,
        end: usize,
        nested_regions: &[TryRegion],
    ) -> Vec<Statement> {
        let mut stmts = Vec::new();
        let mut op_idx = start;

        while op_idx < end {
            self.current_op = op_idx;

            // Check if this opcode starts a nested exception region
            if let Some(region) = nested_regions.iter().find(|r| r.trap_op == op_idx) {
                let try_catch = self.structure_exception_region(region, end);
                stmts.push(try_catch);
                // Skip past the entire nested try/catch including its catch body
                // The catch body was processed from handler_op to 'end', so skip there
                op_idx = end;
                continue;
            }

            // Check if this is a Switch opcode - needs special handling
            if let Opcode::Switch { .. } = &self.func.ops[op_idx] {
                if let Some(block) = self.cfg.op_to_block.get(&op_idx).copied() {
                    // Use CFG-based switch structuring
                    let succs = self.cfg.successors(block);
                    let switch_stmts = self.structure_switch(block, &succs, None);
                    stmts.extend(switch_stmts);
                    // Skip to merge point or continue after switch
                    if let Some(merge) = self.find_switch_merge_point(block) {
                        let merge_block = &self.cfg.graph[merge];
                        op_idx = merge_block.start;
                    } else {
                        op_idx += 1;
                    }
                    continue;
                } else {
                    stmts.push(Statement::Comment(format!(
                        "switch at {} (no CFG block)",
                        op_idx
                    )));
                    op_idx += 1;
                    continue;
                }
            }

            stmts.extend(self.opcode_to_statements(op_idx));
            op_idx += 1;
        }
        stmts
    }

    /// Extract loop condition
    fn extract_loop_condition(
        &self,
        loop_info: &NaturalLoop,
    ) -> (Expr, Option<NodeIndex>, Option<NodeIndex>) {
        let header = loop_info.header;
        let block = &self.cfg.graph[header];
        let last_op = &self.func.ops[block.end];

        match last_op {
            Opcode::JNull { reg, offset } => {
                let target = self.compute_target(block.end, *offset);
                let cond_var = self.reg_to_expr_in_block(*reg, header);
                let null_expr = Expr::Constant(Constant::Null);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop, so continue while != null
                    (
                        Expr::Op(Operation::NotEq(Box::new(cond_var), Box::new(null_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        Expr::Op(Operation::Eq(Box::new(cond_var), Box::new(null_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JSLt { a, b, offset } => {
                let target = self.compute_target(block.end, *offset);
                let a_expr = self.reg_to_expr_in_block(*a, header);
                let b_expr = self.reg_to_expr_in_block(*b, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    (
                        Expr::Op(Operation::Gte(Box::new(a_expr), Box::new(b_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        Expr::Op(Operation::Lt(Box::new(a_expr), Box::new(b_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JSGte { a, b, offset } => {
                let target = self.compute_target(block.end, *offset);
                let a_expr = self.reg_to_expr_in_block(*a, header);
                let b_expr = self.reg_to_expr_in_block(*b, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when a >= b, so continue while a < b
                    (
                        Expr::Op(Operation::Lt(Box::new(a_expr), Box::new(b_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        Expr::Op(Operation::Gte(Box::new(a_expr), Box::new(b_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JNotNull { reg, offset } => {
                let target = self.compute_target(block.end, *offset);
                let cond_var = self.reg_to_expr_in_block(*reg, header);
                let null_expr = Expr::Constant(Constant::Null);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when not null, so continue while null
                    (
                        Expr::Op(Operation::Eq(Box::new(cond_var), Box::new(null_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        Expr::Op(Operation::NotEq(Box::new(cond_var), Box::new(null_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JTrue { cond, offset } => {
                let target = self.compute_target(block.end, *offset);
                let cond_expr = self.reg_to_expr_in_block(*cond, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when true, so continue while not true
                    (
                        Expr::Op(Operation::Not(Box::new(cond_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        cond_expr,
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JFalse { cond, offset } => {
                let target = self.compute_target(block.end, *offset);
                let cond_expr = self.reg_to_expr_in_block(*cond, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when false, so continue while true
                    (
                        cond_expr,
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    (
                        Expr::Op(Operation::Not(Box::new(cond_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JNotLt { a, b, offset } => {
                // JNotLt: jump if NOT (a < b), i.e., jump if a >= b
                let target = self.compute_target(block.end, *offset);
                let a_expr = self.reg_to_expr_in_block(*a, header);
                let b_expr = self.reg_to_expr_in_block(*b, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when a >= b, so continue while a < b
                    (
                        Expr::Op(Operation::Lt(Box::new(a_expr), Box::new(b_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    // Jump stays in loop when a >= b
                    (
                        Expr::Op(Operation::Gte(Box::new(a_expr), Box::new(b_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            Opcode::JNotGte { a, b, offset } => {
                // JNotGte: jump if NOT (a >= b), i.e., jump if a < b
                let target = self.compute_target(block.end, *offset);
                let a_expr = self.reg_to_expr_in_block(*a, header);
                let b_expr = self.reg_to_expr_in_block(*b, header);

                if target.map_or(false, |t| !loop_info.body.contains(&t)) {
                    // Jump exits loop when a < b, so continue while a >= b
                    (
                        Expr::Op(Operation::Gte(Box::new(a_expr), Box::new(b_expr))),
                        self.cfg.block_for_op(block.end + 1),
                        target,
                    )
                } else {
                    // Jump stays in loop when a < b
                    (
                        Expr::Op(Operation::Lt(Box::new(a_expr), Box::new(b_expr))),
                        target,
                        self.cfg.block_for_op(block.end + 1),
                    )
                }
            }
            _ => {
                // Default: while(true)
                let exit = loop_info.exit_nodes.first().and_then(|&n| {
                    self.cfg.successors(n).into_iter().find(|s| !loop_info.body.contains(s))
                });
                (
                    Expr::Constant(Constant::Bool(true)),
                    self.cfg.block_for_op(block.end + 1),
                    exit,
                )
            }
        }
    }

    /// Check if a block is a conditional that can be collected into a chain.
    /// We allow blocks with preambles - they'll be handled when building the result.
    fn is_chain_candidate(&self, block: NodeIndex) -> bool {
        let succs = self.cfg.successors(block);
        if succs.len() != 2 {
            return false;
        }
        // Avoid loop headers - they need special handling
        if self.analysis.loops.iter().any(|l| l.header == block) {
            return false;
        }
        !self.processed.contains(&block)
    }

    /// Structure a branch target, returning statements
    fn structure_branch(&mut self, target: Option<NodeIndex>, stop_at: Option<NodeIndex>) -> Vec<Statement> {
        if let Some(t) = target {
            if Some(t) != stop_at && !self.processed.contains(&t) {
                self.structure_from(t, stop_at)
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    }

    /// Structure a conditional and return (statements, continuation_point)
    /// This version doesn't process the continuation - caller handles it iteratively
    fn structure_conditional_with_continuation(
        &mut self,
        block: NodeIndex,
        succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
    ) -> (Vec<Statement>, Option<NodeIndex>) {
        // Collect the if-else-if chain iteratively
        let mut chain: Vec<(Vec<Statement>, Expr, Vec<Statement>)> = vec![];
        let mut current_block = Some(block);
        let mut final_merge: Option<NodeIndex> = None;
        let mut is_first = true;
        let mut has_preambles = false;

        self.scope_depth += 1;

        while let Some(blk) = current_block {
            if !is_first && self.processed.contains(&blk) {
                break;
            }

            let preamble = if is_first {
                is_first = false;
                vec![]
            } else {
                self.processed.insert(blk);
                let p = self.structure_block(blk);
                if !p.is_empty() {
                    has_preambles = true;
                }
                p
            };

            let block_data = &self.cfg.graph[blk];
            let last_op = &self.func.ops[block_data.end];

            self.current_op = block_data.end;
            if let Some((ssa_dst, ssa_uses)) = self.ssa.get_instr_for_op(block_data.end) {
                self.current_ssa_dst = ssa_dst;
                self.current_ssa_uses = ssa_uses.clone();
            } else {
                self.current_ssa_dst = None;
                self.current_ssa_uses.clear();
            }

            let (condition, then_target, else_target) = self.extract_condition(block_data.end, last_op);

            let merge = self.find_merge_point(then_target, else_target);
            if merge.is_some() {
                final_merge = merge;
            }

            self.processed.insert(blk);

            let is_not_eq = matches!(&condition, Expr::Op(Operation::NotEq(_, _)));

            if is_not_eq {
                let eq_condition = if let Expr::Op(Operation::NotEq(a, b)) = &condition {
                    Expr::Op(Operation::Eq(a.clone(), b.clone()))
                } else {
                    condition.clone()
                };

                let chain_merge = if let Some(e) = else_target {
                    let case_succs = self.cfg.successors(e);
                    if case_succs.len() == 1 { Some(case_succs[0]) } else { merge }
                } else {
                    merge
                };

                if chain_merge.is_some() {
                    final_merge = chain_merge;
                }

                let effective_stop = chain_merge.or(stop_at);
                let case_stmts = self.structure_branch(else_target, effective_stop);
                chain.push((preamble, eq_condition, case_stmts));

                if let Some(t) = then_target {
                    if Some(t) != chain_merge && self.is_chain_candidate(t) {
                        current_block = Some(t);
                        continue;
                    }
                }

                let else_stmts = self.structure_branch(then_target, effective_stop);
                let stmts = self.build_conditional_result_no_continuation(chain, else_stmts, has_preambles);
                return (stmts, self.get_unprocessed_continuation(final_merge, stop_at));
            }

            let effective_stop = merge.or(stop_at);
            let then_stmts = self.structure_branch(then_target, effective_stop);
            chain.push((preamble, condition, then_stmts));

            if let Some(e) = else_target {
                if Some(e) != merge && self.is_chain_candidate(e) {
                    current_block = Some(e);
                    continue;
                }
            }

            let else_stmts = self.structure_branch(else_target, effective_stop);
            let stmts = self.build_conditional_result_no_continuation(chain, else_stmts, has_preambles);
            return (stmts, self.get_unprocessed_continuation(final_merge, stop_at));
        }

        self.scope_depth -= 1;

        if chain.is_empty() {
            return (vec![], None);
        }

        let stmts = self.build_conditional_result_no_continuation(chain, vec![], has_preambles);
        (stmts, self.get_unprocessed_continuation(final_merge, stop_at))
    }

    /// Build conditional result without processing continuation
    fn build_conditional_result_no_continuation(
        &mut self,
        chain: Vec<(Vec<Statement>, Expr, Vec<Statement>)>,
        else_stmts: Vec<Statement>,
        has_preambles: bool,
    ) -> Vec<Statement> {
        self.scope_depth -= 1;

        if has_preambles {
            self.build_nested_if_else(chain, else_stmts)
        } else {
            let flat_chain: Vec<(Expr, Vec<Statement>)> = chain.into_iter()
                .map(|(_, cond, body)| (cond, body))
                .collect();

            if flat_chain.len() > 1 {
                vec![Statement::IfElseChain {
                    branches: flat_chain,
                    else_: else_stmts,
                }]
            } else if flat_chain.len() == 1 {
                let (cond, then_stmts) = flat_chain.into_iter().next().unwrap();
                if then_stmts.is_empty() && else_stmts.is_empty() {
                    vec![]
                } else {
                    vec![Statement::IfElse {
                        cond,
                        if_: then_stmts,
                        else_: else_stmts,
                    }]
                }
            } else {
                else_stmts
            }
        }
    }

    /// Get the continuation point if it's unprocessed and not at stop_at
    fn get_unprocessed_continuation(&self, merge: Option<NodeIndex>, stop_at: Option<NodeIndex>) -> Option<NodeIndex> {
        if let Some(m) = merge {
            if Some(m) != stop_at && !self.processed.contains(&m) {
                return Some(m);
            }
        }
        None
    }

    /// Structure a conditional - collects if-else-if chains iteratively
    /// Chain entry: (preamble_statements, condition, then_body)
    fn structure_conditional(
        &mut self,
        block: NodeIndex,
        _succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
    ) -> Vec<Statement> {
        // Collect the if-else-if chain iteratively
        // Each entry: (preamble, condition, then_body)
        let mut chain: Vec<(Vec<Statement>, Expr, Vec<Statement>)> = vec![];
        let mut current_block = Some(block);
        let mut final_merge: Option<NodeIndex> = None;
        let mut is_first = true;
        let mut has_preambles = false;

        self.scope_depth += 1;

        while let Some(blk) = current_block {
            // Skip processed check for first block - structure_from already marked it
            if !is_first && self.processed.contains(&blk) {
                break;
            }

            // Structure preamble for non-first blocks (first block's preamble is in structure_from)
            let preamble = if is_first {
                is_first = false;
                vec![]  // First block's preamble already handled by structure_from
            } else {
                self.processed.insert(blk);
                let p = self.structure_block(blk);
                if !p.is_empty() {
                    has_preambles = true;
                }
                p
            };

            let block_data = &self.cfg.graph[blk];
            let last_op = &self.func.ops[block_data.end];

            // Set SSA context for the conditional jump opcode
            self.current_op = block_data.end;
            if let Some((ssa_dst, ssa_uses)) = self.ssa.get_instr_for_op(block_data.end) {
                self.current_ssa_dst = ssa_dst;
                self.current_ssa_uses = ssa_uses.clone();
            } else {
                self.current_ssa_dst = None;
                self.current_ssa_uses.clear();
            }

            let (condition, then_target, else_target) = self.extract_condition(block_data.end, last_op);

            // Find merge point for this conditional
            // Update final_merge to track the continuation after the chain
            // (should be the LAST entry's merge point, not the first)
            let merge = self.find_merge_point(then_target, else_target);
            if merge.is_some() {
                final_merge = merge;
            }

            // Mark this block as processed (if not already)
            self.processed.insert(blk);

            // Check if this is a NotEq pattern (used in if-else-if chains compiled from Haxe)
            let is_not_eq = matches!(&condition, Expr::Op(Operation::NotEq(_, _)));

            if is_not_eq {
                // NotEq pattern: case body is in else branch, next case is in then branch
                let eq_condition = if let Expr::Op(Operation::NotEq(a, b)) = &condition {
                    Expr::Op(Operation::Eq(a.clone(), b.clone()))
                } else {
                    condition.clone()
                };

                let chain_merge = if let Some(e) = else_target {
                    let case_succs = self.cfg.successors(e);
                    if case_succs.len() == 1 { Some(case_succs[0]) } else { merge }
                } else {
                    merge
                };

                // Update final_merge to track the continuation after the chain
                // (should be the LAST entry's merge point, not the first)
                if chain_merge.is_some() {
                    final_merge = chain_merge;
                }

                // Use stop_at as fallback if no merge point found (e.g., inside loops)
                let effective_stop = chain_merge.or(stop_at);
                let case_stmts = self.structure_branch(else_target, effective_stop);
                chain.push((preamble, eq_condition, case_stmts));

                // Check if the then branch can continue the chain
                if let Some(t) = then_target {
                    if Some(t) != chain_merge && self.is_chain_candidate(t) {
                        current_block = Some(t);
                        continue;
                    }
                }

                // End of chain
                let else_stmts = self.structure_branch(then_target, effective_stop);
                return self.build_conditional_result(chain, else_stmts, has_preambles, final_merge, stop_at);
            }

            // Standard pattern: case body is in then branch
            // Use stop_at as fallback if no merge point found (e.g., inside loops)
            let effective_stop = merge.or(stop_at);
            let then_stmts = self.structure_branch(then_target, effective_stop);
            chain.push((preamble, condition, then_stmts));

            // Check if the else branch can continue the chain
            if let Some(e) = else_target {
                if Some(e) != merge && self.is_chain_candidate(e) {
                    current_block = Some(e);
                    continue;
                }
            }

            // End of chain
            let else_stmts = self.structure_branch(else_target, effective_stop);
            return self.build_conditional_result(chain, else_stmts, has_preambles, final_merge, stop_at);
        }

        self.scope_depth -= 1;

        // Edge case: loop ended without producing result
        if chain.is_empty() {
            return vec![];
        }

        // Build result from accumulated chain (no else)
        self.build_conditional_result(chain, vec![], has_preambles, final_merge, stop_at)
    }

    /// Build the final result from a collected conditional chain
    fn build_conditional_result(
        &mut self,
        chain: Vec<(Vec<Statement>, Expr, Vec<Statement>)>,
        else_stmts: Vec<Statement>,
        has_preambles: bool,
        final_merge: Option<NodeIndex>,
        stop_at: Option<NodeIndex>,
    ) -> Vec<Statement> {
        self.scope_depth -= 1;

        let result = if has_preambles {
            // Build nested IfElse iteratively (from last to first) to include preambles
            self.build_nested_if_else(chain, else_stmts)
        } else {
            // No preambles - can use flat representation
            // Convert chain to (condition, body) format for analysis
            let flat_chain: Vec<(Expr, Vec<Statement>)> = chain.into_iter()
                .map(|(_, cond, body)| (cond, body))
                .collect();

            if let Some((switch_arg, cases)) = self.analyze_for_switch(&flat_chain) {
                vec![Statement::Switch {
                    arg: switch_arg,
                    default: else_stmts,
                    cases,
                    enum_type: None,
                }]
            } else if flat_chain.len() > 1 {
                vec![Statement::IfElseChain {
                    branches: flat_chain,
                    else_: else_stmts,
                }]
            } else if flat_chain.len() == 1 {
                let (cond, then_stmts) = flat_chain.into_iter().next().unwrap();
                // Skip if-else entirely if both branches are empty (e.g., suppressed internal ops)
                if then_stmts.is_empty() && else_stmts.is_empty() {
                    vec![]
                }
                // Try phi-return optimization: both branches assign, merge returns the phi var
                else if let Some(optimized) = self.try_simplify_phi_return(&cond, &then_stmts, &else_stmts, final_merge) {
                    return optimized;
                }
                // Try ternary condensation: both branches return directly
                else if let Some(ternary) = self.try_condense_ternary_return(&cond, &then_stmts, &else_stmts) {
                    vec![ternary]
                }
                // Try ternary assignment: both branches assign to same variable
                else if let Some(ternary) = self.try_condense_ternary_assign(&cond, &then_stmts, &else_stmts) {
                    vec![ternary]
                } else {
                    vec![Statement::IfElse {
                        cond,
                        if_: then_stmts,
                        else_: else_stmts,
                    }]
                }
            } else {
                else_stmts
            }
        };

        // Continue after merge
        let mut full_result = result;
        if let Some(m) = final_merge {
            if Some(m) != stop_at && !self.processed.contains(&m) {
                full_result.extend(self.structure_from(m, stop_at));
            }
        }

        full_result
    }

    /// Build nested IfElse structure iteratively from a chain with preambles
    /// This avoids recursion by building from last to first
    fn build_nested_if_else(
        &self,
        chain: Vec<(Vec<Statement>, Expr, Vec<Statement>)>,
        final_else: Vec<Statement>,
    ) -> Vec<Statement> {
        // Build from last to first: each iteration wraps the previous result as the else branch
        let mut current_else = final_else;

        for (preamble, cond, then_body) in chain.into_iter().rev() {
            let if_else = Statement::IfElse {
                cond,
                if_: then_body,
                else_: current_else,
            };
            // Preamble goes before the if-else
            let mut block = preamble;
            block.push(if_else);
            current_else = block;
        }

        current_else
    }

    /// Extract condition from a conditional jump
    fn extract_condition(
        &self,
        op_idx: usize,
        op: &Opcode,
    ) -> (Expr, Option<NodeIndex>, Option<NodeIndex>) {
        let target = |offset: i32| self.compute_target(op_idx, offset);
        let fall = self.cfg.block_for_op(op_idx + 1);

        match op {
            Opcode::JNull { reg, offset } => {
                let var = self.reg_to_expr(*reg);
                let null = Expr::Constant(Constant::Null);
                (Expr::Op(Operation::Eq(Box::new(var), Box::new(null))), target(*offset), fall)
            }
            Opcode::JNotNull { reg, offset } => {
                let var = self.reg_to_expr(*reg);
                let null = Expr::Constant(Constant::Null);
                (Expr::Op(Operation::NotEq(Box::new(var), Box::new(null))), target(*offset), fall)
            }
            Opcode::JTrue { cond, offset } => {
                (self.reg_to_expr(*cond), target(*offset), fall)
            }
            Opcode::JFalse { cond, offset } => {
                let var = self.reg_to_expr(*cond);
                (Expr::Op(Operation::Not(Box::new(var))), target(*offset), fall)
            }
            Opcode::JSLt { a, b, offset } => {
                let cond = Expr::Op(Operation::Lt(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JSGte { a, b, offset } => {
                let cond = Expr::Op(Operation::Gte(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JEq { a, b, offset } => {
                let cond = Expr::Op(Operation::Eq(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JNotEq { a, b, offset } => {
                let cond = Expr::Op(Operation::NotEq(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            // Unsigned comparisons (used for array bounds checking)
            Opcode::JULt { a, b, offset } => {
                let cond = Expr::Op(Operation::Lt(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JUGte { a, b, offset } => {
                let cond = Expr::Op(Operation::Gte(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JSLte { a, b, offset } => {
                let cond = Expr::Op(Operation::Lte(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JNotLt { a, b, offset } => {
                // not(a < b) is equivalent to a >= b
                let cond = Expr::Op(Operation::Gte(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            Opcode::JNotGte { a, b, offset } => {
                // not(a >= b) is equivalent to a < b
                let cond = Expr::Op(Operation::Lt(
                    Box::new(self.reg_to_expr(*a)),
                    Box::new(self.reg_to_expr(*b)),
                ));
                (cond, target(*offset), fall)
            }
            _ => (Expr::Constant(Constant::Bool(true)), fall, None),
        }
    }

    /// Check if a block terminates (ends with Ret, Throw, or Rethrow)
    fn block_terminates(&self, block: NodeIndex) -> bool {
        let block_data = &self.cfg.graph[block];
        matches!(
            &self.func.ops[block_data.end],
            Opcode::Ret { .. } | Opcode::Throw { .. } | Opcode::Rethrow { .. }
        )
    }

    /// Check if a branch terminates (the target block ends in return/throw)
    fn branch_terminates(&self, target: Option<NodeIndex>) -> bool {
        match target {
            Some(block) => {
                // Check if the block itself terminates
                self.block_terminates(block) || self.cfg.successors(block).is_empty()
            }
            None => true, // No target = terminates
        }
    }

    /// Find merge point of two branches.
    /// If one branch terminates (return/throw) and the other branch doesn't eventually
    /// reach the same terminator, the non-terminating branch IS the merge point.
    fn find_merge_point(&self, a: Option<NodeIndex>, b: Option<NodeIndex>) -> Option<NodeIndex> {
        let a = a?;
        let b = b?;

        // Standard merge point detection: find common successor
        let a_succs: HashSet<_> = self.cfg.successors(a).into_iter().collect();
        let b_succs: HashSet<_> = self.cfg.successors(b).into_iter().collect();

        for s in &a_succs {
            if b_succs.contains(s) {
                return Some(*s);
            }
        }
        if a_succs.contains(&b) { return Some(b); }
        if b_succs.contains(&a) { return Some(a); }

        // If one branch terminates AND the other doesn't reach it,
        // the non-terminating branch is where control "continues" after the if.
        // But only apply this if the other branch doesn't eventually flow into
        // the terminating branch (which would mean they share a merge point).
        let a_terminates = self.branch_terminates(Some(a));
        let b_terminates = self.branch_terminates(Some(b));

        // If BOTH branches terminate, there is no merge point
        if a_terminates && b_terminates {
            return None;
        }

        if a_terminates && !b_succs.contains(&a) {
            // Branch 'a' returns, branch 'b' doesn't flow into 'a'
            // So 'b' is the continuation point
            return Some(b);
        }
        if b_terminates && !a_succs.contains(&b) {
            // Branch 'b' returns, branch 'a' doesn't flow into 'b'
            // So 'a' is the continuation point
            return Some(a);
        }

        None
    }

    /// Analyze an if-else-if chain to see if it can be converted to a switch statement.
    /// Returns Some((switch_arg, cases)) if the chain matches the switch pattern.
    ///
    /// Switch pattern criteria:
    /// - All conditions are equality tests: Eq(X, C) or Eq(C, X)
    /// - X is the same expression in all conditions (the switch argument)
    /// - C is a constant (Int, String, Bool, or enum constructor)
    fn analyze_for_switch(
        &self,
        chain: &[(Expr, Vec<Statement>)],
    ) -> Option<(Expr, Vec<(Vec<Expr>, Vec<Statement>)>)> {
        // Need at least 3 conditions to make a switch worthwhile
        if chain.len() < 3 {
            return None;
        }

        let mut switch_arg: Option<Expr> = None;
        let mut cases = Vec::new();

        for (cond, body) in chain {
            let (arg, case_val) = self.extract_equality_pattern(cond)?;

            if let Some(ref existing) = switch_arg {
                // Check if the argument matches the existing switch argument
                if !self.exprs_structurally_equal(existing, &arg) {
                    return None;
                }
            } else {
                switch_arg = Some(arg);
            }

            cases.push((vec![case_val], body.clone()));
        }

        Some((switch_arg?, cases))
    }

    /// Extract an equality pattern from a condition expression.
    /// Returns Some((arg, case_value)) where case_value is a constant.
    /// Note: NotEq patterns are converted to Eq during chain collection.
    fn extract_equality_pattern(&self, cond: &Expr) -> Option<(Expr, Expr)> {
        match cond {
            Expr::Op(Operation::Eq(a, b)) => {
                // First, check if one side is a simple constant (integer, string literal, etc.)
                if self.is_switch_case_constant(b) {
                    return Some((*a.clone(), *b.clone()));
                } else if self.is_switch_case_constant(a) {
                    return Some((*b.clone(), *a.clone()));
                }

                // If not a simple constant, try string comparison pattern:
                // Eq(Variable(result), Variable(zero))
                // where result = string_compare(bytes, string_literal, len) and zero = 0
                if let (Expr::Variable(reg_a, _), Expr::Variable(reg_b, _)) = (a.as_ref(), b.as_ref()) {
                    if let Some(result) = self.try_extract_string_compare_pattern(*reg_a, *reg_b) {
                        return Some(result);
                    }
                    if let Some(result) = self.try_extract_string_compare_pattern(*reg_b, *reg_a) {
                        return Some(result);
                    }
                }

                None
            }
            _ => None,
        }
    }

    /// Try to extract a string comparison pattern from two register references.
    /// Returns Some((switch_arg, string_constant)) if reg_result was defined by
    /// string_compare(bytes, string_literal, len) and reg_zero was defined as 0.
    fn try_extract_string_compare_pattern(&self, reg_result: Reg, reg_zero: Reg) -> Option<(Expr, Expr)> {
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
    fn is_string_compare_function(&self, fun: RefFun) -> bool {
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
    fn find_string_literal_for_bytes(&self, bytes_reg: Reg, call_idx: usize) -> Option<RefString> {
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

    /// Check if an expression is a valid switch case constant
    fn is_switch_case_constant(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Constant(c) => matches!(
                c,
                Constant::Int(_)
                | Constant::InlineInt(_)
                | Constant::String(_)
                | Constant::Bool(_)
            ),
            _ => false,
        }
    }

    /// Check if two expressions are structurally equal (for switch arg comparison)
    fn exprs_structurally_equal(&self, a: &Expr, b: &Expr) -> bool {
        match (a, b) {
            (Expr::Variable(ra, _), Expr::Variable(rb, _)) => ra == rb,
            (Expr::Constant(ca), Expr::Constant(cb)) => self.constants_equal(ca, cb),
            (Expr::Field(obj_a, field_a), Expr::Field(obj_b, field_b)) => {
                field_a == field_b && self.exprs_structurally_equal(obj_a, obj_b)
            }
            (Expr::Call(call_a), Expr::Call(call_b)) => {
                // For calls, compare function and arguments
                if !self.exprs_structurally_equal(&call_a.fun, &call_b.fun)
                    || call_a.args.len() != call_b.args.len()
                {
                    return false;
                }
                call_a.args.iter().zip(call_b.args.iter())
                    .all(|(a, b)| self.exprs_structurally_equal(a, b))
            }
            (Expr::Op(op_a), Expr::Op(op_b)) => {
                // Simple operation comparison - just check if they're the same variant
                // This is a conservative check
                std::mem::discriminant(op_a) == std::mem::discriminant(op_b)
            }
            _ => false,
        }
    }

    /// Check if two constants are equal
    fn constants_equal(&self, a: &Constant, b: &Constant) -> bool {
        match (a, b) {
            (Constant::Int(ia), Constant::Int(ib)) => ia == ib,
            (Constant::InlineInt(ia), Constant::InlineInt(ib)) => ia == ib,
            (Constant::Float(fa), Constant::Float(fb)) => fa == fb,
            (Constant::String(sa), Constant::String(sb)) => sa == sb,
            (Constant::Bool(ba), Constant::Bool(bb)) => ba == bb,
            (Constant::Null, Constant::Null) => true,
            (Constant::This, Constant::This) => true,
            _ => false,
        }
    }

    /// Try to condense an if/else with returns into a ternary return.
    /// Returns Some(statement) if the pattern matches, None otherwise.
    ///
    /// Supported patterns:
    /// - Pattern 1: if (cond) { return a; } else { return b; } → return cond ? a : b
    /// - Pattern 2: if (cond) { r = a; return r; } else { r = b; return r; } → return cond ? a : b
    /// - Pattern 3: if (cond) { r = a; return r; } else { return b; } → return cond ? a : b
    /// - Pattern 4: if (cond) { return a; } else { r = b; return r; } → return cond ? a : b
    fn try_condense_ternary_return(
        &self,
        cond: &Expr,
        then_stmts: &[Statement],
        else_stmts: &[Statement],
    ) -> Option<Statement> {
        // Pattern 1: Simple returns in both branches
        // if (cond) { return a; } else { return b; }
        if then_stmts.len() == 1 && else_stmts.len() == 1 {
            if let (Statement::Return(Some(if_expr)), Statement::Return(Some(else_expr))) =
                (&then_stmts[0], &else_stmts[0])
            {
                return Some(Statement::Return(Some(Expr::IfElse {
                    cond: Box::new(cond.clone()),
                    if_: vec![Statement::ExprStatement(if_expr.clone())],
                    else_: vec![Statement::ExprStatement(else_expr.clone())],
                })));
            }
        }

        // Pattern 2: Assignment + return in both branches
        // if (cond) { r = a; return r; } else { r = b; return r; }
        if then_stmts.len() == 2 && else_stmts.len() == 2 {
            if let (
                Statement::Assign { variable: if_var, assign: if_assign, .. },
                Statement::Return(Some(if_ret)),
            ) = (&then_stmts[0], &then_stmts[1])
            {
                if let (
                    Statement::Assign { variable: else_var, assign: else_assign, .. },
                    Statement::Return(Some(else_ret)),
                ) = (&else_stmts[0], &else_stmts[1])
                {
                    // Check that both assign to the same register and return that register
                    if let (Expr::Variable(if_reg, _), Expr::Variable(else_reg, _)) =
                        (if_var, else_var)
                    {
                        if if_reg == else_reg {
                            if let (
                                Expr::Variable(if_ret_reg, _),
                                Expr::Variable(else_ret_reg, _),
                            ) = (if_ret, else_ret)
                            {
                                if if_ret_reg == if_reg && else_ret_reg == else_reg {
                                    return Some(Statement::Return(Some(Expr::IfElse {
                                        cond: Box::new(cond.clone()),
                                        if_: vec![Statement::ExprStatement(if_assign.clone())],
                                        else_: vec![Statement::ExprStatement(else_assign.clone())],
                                    })));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Pattern 3: Asymmetric - if has assign+return, else has direct return
        // if (cond) { r = a; return r; } else { return b; }
        if then_stmts.len() == 2 && else_stmts.len() == 1 {
            if let (
                Statement::Assign { variable: if_var, assign: if_assign, .. },
                Statement::Return(Some(if_ret)),
            ) = (&then_stmts[0], &then_stmts[1])
            {
                if let Statement::Return(Some(else_expr)) = &else_stmts[0] {
                    if let (Expr::Variable(if_reg, _), Expr::Variable(if_ret_reg, _)) =
                        (if_var, if_ret)
                    {
                        if if_reg == if_ret_reg {
                            return Some(Statement::Return(Some(Expr::IfElse {
                                cond: Box::new(cond.clone()),
                                if_: vec![Statement::ExprStatement(if_assign.clone())],
                                else_: vec![Statement::ExprStatement(else_expr.clone())],
                            })));
                        }
                    }
                }
            }
        }

        // Pattern 4: Asymmetric (reversed) - if has direct return, else has assign+return
        // if (cond) { return a; } else { r = b; return r; }
        if then_stmts.len() == 1 && else_stmts.len() == 2 {
            if let Statement::Return(Some(if_expr)) = &then_stmts[0] {
                if let (
                    Statement::Assign { variable: else_var, assign: else_assign, .. },
                    Statement::Return(Some(else_ret)),
                ) = (&else_stmts[0], &else_stmts[1])
                {
                    if let (Expr::Variable(else_reg, _), Expr::Variable(else_ret_reg, _)) =
                        (else_var, else_ret)
                    {
                        if else_reg == else_ret_reg {
                            return Some(Statement::Return(Some(Expr::IfElse {
                                cond: Box::new(cond.clone()),
                                if_: vec![Statement::ExprStatement(if_expr.clone())],
                                else_: vec![Statement::ExprStatement(else_assign.clone())],
                            })));
                        }
                    }
                }
            }
        }

        None
    }

    /// Try to condense if/else assignment pattern into ternary assignment.
    /// Pattern: if (cond) { x = a; } else { x = b; } → x = cond ? a : b
    fn try_condense_ternary_assign(
        &self,
        cond: &Expr,
        then_stmts: &[Statement],
        else_stmts: &[Statement],
    ) -> Option<Statement> {
        // Both branches must have exactly one statement
        if then_stmts.len() != 1 || else_stmts.len() != 1 {
            return None;
        }

        // Both must be assignments
        let (if_decl, if_var, if_assign) = match &then_stmts[0] {
            Statement::Assign { declaration, variable, assign, .. } => {
                (declaration, variable, assign)
            }
            _ => return None,
        };

        let (else_decl, else_var, else_assign) = match &else_stmts[0] {
            Statement::Assign { declaration, variable, assign, .. } => {
                (declaration, variable, assign)
            }
            _ => return None,
        };

        // Both must assign to the same register
        let (if_reg, if_name) = match if_var {
            Expr::Variable(reg, name) => (reg, name),
            _ => return None,
        };

        let else_reg = match else_var {
            Expr::Variable(reg, _) => reg,
            _ => return None,
        };

        if if_reg != else_reg {
            return None;
        }

        // Create ternary assignment: x = cond ? a : b
        // Use declaration from first branch (the one that declares the variable)
        Some(Statement::Assign {
            declaration: *if_decl || *else_decl,
            variable: Expr::Variable(*if_reg, if_name.clone()),
            assign: Expr::IfElse {
                cond: Box::new(cond.clone()),
                if_: vec![Statement::ExprStatement(if_assign.clone())],
                else_: vec![Statement::ExprStatement(else_assign.clone())],
            },
        })
    }

    /// Check if branches form a phi pattern where:
    /// - Both branches assign to the same register
    /// - The merge block immediately returns that register
    /// Returns Some(optimized_statements) if pattern matched
    fn try_simplify_phi_return(
        &mut self,
        cond: &Expr,
        if_stmts: &[Statement],
        else_stmts: &[Statement],
        merge: Option<NodeIndex>,
    ) -> Option<Vec<Statement>> {
        let merge_node = merge?;
        if self.processed.contains(&merge_node) {
            return None;
        }

        // Get the return register from merge block
        let merge_block = &self.cfg.graph[merge_node];
        let ret_reg = self.get_merge_return_reg(merge_block)?;

        // Check what each branch assigns to the return register
        let if_assign = self.extract_assign_to_reg(if_stmts, ret_reg);
        let else_assign = self.extract_assign_to_reg(else_stmts, ret_reg);

        match (if_assign, else_assign) {
            // Pattern 1: Both branches assign → ternary return
            (Some(if_val), Some(else_val)) => {
                self.processed.insert(merge_node);
                Some(vec![self.make_ternary_return(cond, if_val, else_val)])
            }
            _ => None,
        }
    }

    /// Check if merge block's first (and only) instruction is Ret, return the register
    fn get_merge_return_reg(&self, block: &BasicBlock) -> Option<Reg> {
        // Block must be a single instruction (just the Ret)
        if block.start != block.end {
            return None;
        }
        let op = self.func.ops.get(block.start)?;
        if let &Opcode::Ret { ret } = op {
            Some(ret)
        } else {
            None
        }
    }

    /// Extract the assigned value if stmts is a single assignment to the given register
    fn extract_assign_to_reg(&self, stmts: &[Statement], reg: Reg) -> Option<Expr> {
        if stmts.len() != 1 {
            return None;
        }
        if let Statement::Assign { variable, assign, .. } = &stmts[0] {
            if let Expr::Variable(r, _) = variable {
                if *r == reg {
                    return Some(assign.clone());
                }
            }
        }
        None
    }

    /// Create optimized ternary return, with boolean simplification
    fn make_ternary_return(&self, cond: &Expr, if_val: Expr, else_val: Expr) -> Statement {
        // Boolean simplification: if both values are boolean constants
        if let (Expr::Constant(Constant::Bool(if_b)), Expr::Constant(Constant::Bool(else_b))) =
            (&if_val, &else_val)
        {
            let result = match (if_b, else_b) {
                (true, false) => cond.clone(),
                (false, true) => not(cond.clone()),
                // Both same value - just return one
                _ => return Statement::Return(Some(if_val)),
            };
            return Statement::Return(Some(result));
        }

        // General case: return cond ? if_val : else_val
        Statement::Return(Some(Expr::IfElse {
            cond: Box::new(cond.clone()),
            if_: vec![Statement::ExprStatement(if_val)],
            else_: vec![Statement::ExprStatement(else_val)],
        }))
    }

    /// Structure a switch statement and return (statements, continuation_point)
    /// This version doesn't process the continuation - caller handles it iteratively
    fn structure_switch_with_continuation(
        &mut self,
        block: NodeIndex,
        succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
    ) -> (Vec<Statement>, Option<NodeIndex>) {
        // Use the core switch logic but capture the merge point
        let merge_point = self.find_switch_merge_point(block);

        // Structure the switch without continuation
        let stmts = self.structure_switch_core(block, succs, stop_at, false);

        // Return unprocessed continuation
        let continuation = if let Some(m) = merge_point {
            if Some(m) != stop_at && !self.processed.contains(&m) {
                Some(m)
            } else {
                None
            }
        } else {
            None
        };

        (stmts, continuation)
    }

    /// Structure a switch statement
    fn structure_switch(
        &mut self,
        block: NodeIndex,
        _succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
    ) -> Vec<Statement> {
        self.structure_switch_core(block, _succs, stop_at, true)
    }

    /// Core switch structuring logic
    fn structure_switch_core(
        &mut self,
        block: NodeIndex,
        _succs: &[NodeIndex],
        stop_at: Option<NodeIndex>,
        do_continuation: bool,
    ) -> Vec<Statement> {
        let block_data = &self.cfg.graph[block];
        let last_op = &self.func.ops[block_data.end];

        // Extract switch info
        let (switch_reg, offsets) = match last_op {
            Opcode::Switch { reg, offsets, .. } => (*reg, offsets.clone()),
            _ => {
                // Fallback: emit comment
                return vec![Statement::Comment("switch statement".to_string())];
            }
        };

        let switch_op_idx = block_data.end;
        // Set SSA context for the switch opcode so reg_to_expr uses SSA-versioned names
        self.current_op = switch_op_idx;
        if let Some((ssa_dst, ssa_uses)) = self.ssa.get_instr_for_op(switch_op_idx) {
            self.current_ssa_dst = ssa_dst;
            self.current_ssa_uses = ssa_uses.clone();
        } else {
            self.current_ssa_dst = None;
            self.current_ssa_uses.clear();
        }
        let switch_arg = self.reg_to_expr(switch_reg);

        // Check if switch_arg is Type.enumIndex(x) and unwrap to just x
        // Also extract the enum type for proper case pattern formatting
        // enum_value_reg is the register holding the actual enum value (for pattern binding)
        let (switch_arg, enum_type, enum_value_reg) = self.unwrap_enum_index_switch(switch_arg, switch_reg);

        // Group cases by their target opcode
        // offsets[i] = offset for case value i, target = switch_op_idx + 1 + offset
        let mut target_to_cases: HashMap<usize, Vec<usize>> = HashMap::new();
        for (case_val, &offset) in offsets.iter().enumerate() {
            let target_op = (switch_op_idx as i64 + 1 + offset as i64) as usize;
            target_to_cases
                .entry(target_op)
                .or_default()
                .push(case_val);
        }

        // Default case is fall-through (switch_op_idx + 1)
        let default_op = switch_op_idx + 1;

        // Find the merge point - look for where all branches converge
        // This is typically the op after the last JAlways in any case
        let merge_point = self.find_switch_merge_point(block);

        // Structure default case (fall-through) - increment scope depth for case bodies
        self.scope_depth += 1;
        let default_stmts = if let Some(default_block) = self.cfg.block_for_op(default_op) {
            if !target_to_cases.contains_key(&default_op) && !self.processed.contains(&default_block) {
                self.structure_from(default_block, merge_point)
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        // Structure each case target
        let mut cases: Vec<(Vec<Expr>, Vec<Statement>)> = Vec::new();
        let mut processed_targets: HashSet<usize> = HashSet::new();

        // Sort case values for deterministic output
        let mut sorted_targets: Vec<_> = target_to_cases.into_iter().collect();
        sorted_targets.sort_by_key(|(_, vals)| vals.iter().min().copied().unwrap_or(0));

        // Determine the merge_op for scanning case body ranges
        let merge_op = merge_point.and_then(|mp| {
            let mp_block = &self.cfg.graph[mp];
            Some(mp_block.start)
        }).unwrap_or(self.func.ops.len());

        // Collect sorted target ops for determining case body end points
        let all_target_ops: Vec<usize> = sorted_targets.iter().map(|(op, _)| *op).collect();

        for (idx, (target_op, case_vals)) in sorted_targets.iter().enumerate() {
            if processed_targets.contains(target_op) {
                continue;
            }
            processed_targets.insert(*target_op);

            // Determine the end of this case's body (next case target or merge)
            let case_end_op = all_target_ops.get(idx + 1).copied().unwrap_or(merge_op);

            // Convert case values to expressions
            // If we have an enum type, use constructor names instead of integers
            let case_exprs: Vec<Expr> = case_vals
                .iter()
                .map(|&v| {
                    if let Some(ref_type) = enum_type {
                        // Try to get enum constructor name for this index
                        if let Type::Enum { constructs, .. } = &self.code[ref_type] {
                            if let Some(construct) = constructs.get(v) {
                                let name = self.code.strings.get(construct.name.0)
                                    .cloned()
                                    .unwrap_or_else(|| format!("_{}", v).into());
                                // If constructor has parameters, check if we can bind them
                                if !construct.params.is_empty() {
                                    // Scan the case body for EnumField accesses on this construct
                                    let accessed_fields = if let Some(enum_reg) = enum_value_reg {
                                        self.scan_enum_field_accesses(enum_reg, *target_op, case_end_op)
                                    } else {
                                        HashSet::new()
                                    };

                                    // Generate bindings for accessed fields, wildcards for others
                                    let bindings: Vec<Expr> = construct.params.iter().enumerate()
                                        .map(|(i, _)| {
                                            if accessed_fields.contains(&(v, i)) {
                                                let param_name = format!("param{}", i);
                                                // Register this binding for EnumField handler
                                                if let Some(enum_reg) = enum_value_reg {
                                                    self.enum_param_bindings.insert(
                                                        (enum_reg, v, i),
                                                        param_name.clone()
                                                    );
                                                }
                                                Expr::Ident(param_name.into())
                                            } else {
                                                Expr::Ident("_".into())
                                            }
                                        })
                                        .collect();
                                    return Expr::EnumConstr(ref_type, RefEnumConstruct(v), bindings);
                                }
                                return Expr::Ident(name);
                            }
                        }
                    }
                    // Fallback to integer
                    Expr::Constant(Constant::InlineInt(v))
                })
                .collect();

            // Skip default case target if it's also a case target
            if *target_op == default_op {
                // Add to cases instead of default
                let stmts = if let Some(target_block) = self.cfg.block_for_op(*target_op) {
                    if !self.processed.contains(&target_block) {
                        self.structure_from(target_block, merge_point)
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                };
                cases.push((case_exprs, stmts));
                continue;
            }

            let stmts = if let Some(target_block) = self.cfg.block_for_op(*target_op) {
                if !self.processed.contains(&target_block) {
                    self.structure_from(target_block, merge_point)
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            cases.push((case_exprs, stmts));
        }
        self.scope_depth -= 1;

        // Hoist variable declarations from cases to before the switch
        // This is needed because Haxe switch cases share scope but VarDecl
        // in one case doesn't make the variable visible in other cases
        let mut hoisted_decls: Vec<Statement> = Vec::new();
        let mut hoisted_names: HashSet<Str> = HashSet::new(); // Track what we've hoisted

        // Helper to extract var name from an expression
        fn extract_var_name(expr: &Expr) -> Option<Str> {
            match expr {
                Expr::Variable(_, Some(name)) => Some(name.clone()),
                _ => None
            }
        }

        // Process case statements: hoist var declarations and convert to assignments
        fn process_case_stmts(
            stmts: Vec<Statement>,
            hoisted: &mut HashSet<Str>,
            decls: &mut Vec<Statement>,
        ) -> Vec<Statement> {
            stmts.into_iter().map(|stmt| {
                match stmt {
                    Statement::Assign { declaration: true, variable, assign } => {
                        // This is a "var x = ..." statement
                        // Hoist the declaration and convert to simple assignment
                        if let Some(name) = extract_var_name(&variable) {
                            if !hoisted.contains(&name) {
                                hoisted.insert(name.clone());
                                // Check if assign is empty anonymous object - needs :Dynamic
                                let type_hint = if matches!(&assign, Expr::Anonymous(_, fields) if fields.is_empty()) {
                                    Some("Dynamic".into())
                                } else {
                                    None
                                };
                                // Create a declaration without initialization: "var x;" or "var x:Dynamic;"
                                decls.push(Statement::VarDecl { name, type_hint });
                            }
                        }
                        // Convert to non-declaration assignment
                        Statement::Assign { declaration: false, variable, assign }
                    }
                    other => other
                }
            }).collect()
        }

        // Process all case statements (process default FIRST since it has the declarations)
        let default_stmts = process_case_stmts(default_stmts, &mut hoisted_names, &mut hoisted_decls);

        let cases: Vec<(Vec<Expr>, Vec<Statement>)> = cases
            .into_iter()
            .map(|(vals, stmts)| (vals, process_case_stmts(stmts, &mut hoisted_names, &mut hoisted_decls)))
            .collect();

        // Add hoisted names to declared_vars so merge point won't re-declare them
        for name in &hoisted_names {
            self.declared_vars.insert(name.clone());
        }

        // Build result: hoisted declarations, then switch
        let mut result = hoisted_decls;
        result.push(Statement::Switch {
            arg: switch_arg,
            default: default_stmts,
            cases,
            enum_type,
        });

        // Continue after merge (only if requested)
        if do_continuation {
            if let Some(m) = merge_point {
                if Some(m) != stop_at && !self.processed.contains(&m) {
                    result.extend(self.structure_from(m, stop_at));
                }
            }
        }

        // Clear enum param bindings after processing switch
        self.enum_param_bindings.clear();

        result
    }

    /// Find the merge point for a switch statement
    fn find_switch_merge_point(&self, switch_block: NodeIndex) -> Option<NodeIndex> {
        // Get all successors of the switch block
        let succs = self.cfg.successors(switch_block);

        if succs.is_empty() {
            return None;
        }

        // Look for a common successor among all case branches
        // Each case typically ends with JAlways to the merge point
        let mut candidate_merges: HashMap<NodeIndex, usize> = HashMap::new();

        for succ in &succs {
            // Follow each case to find where it jumps to
            let case_succs = self.cfg.successors(*succ);
            for case_succ in case_succs {
                *candidate_merges.entry(case_succ).or_default() += 1;
            }
        }

        // The merge point must be reachable from at least 2 cases to be valid.
        // If only one case reaches a node, it's part of that case's control flow,
        // not a merge point. This prevents incorrectly skipping branches.
        candidate_merges
            .into_iter()
            .filter(|(_, count)| *count >= 2)
            .max_by_key(|(_, count)| *count)
            .map(|(node, _)| node)
    }

    /// Check if switch_arg is Type.enumIndex(x) and unwrap to just x
    /// Also check if switch_reg was produced by EnumIndex opcode
    /// Returns (unwrapped_arg, optional_enum_type, optional_enum_value_reg)
    /// The enum_value_reg is the register holding the actual enum value (for pattern binding)
    fn unwrap_enum_index_switch(
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
    fn scan_enum_field_accesses(
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
    fn get_enum_type_from_expr(&self, expr: &Expr) -> Option<RefType> {
        match expr {
            Expr::Variable(reg, _) => self.get_enum_type_for_reg(*reg),
            _ => None,
        }
    }

    /// Try to get the enum type for a register
    fn get_enum_type_for_reg(&self, reg: Reg) -> Option<RefType> {
        let reg_type = self.func.regs.get(reg.0 as usize)?;
        if let Type::Enum { .. } = &self.code[*reg_type] {
            Some(*reg_type)
        } else {
            None
        }
    }

    /// Structure a single basic block into statements
    fn structure_block(&mut self, node: NodeIndex) -> Vec<Statement> {
        let block = &self.cfg.graph[node];
        let mut stmts = Vec::new();

        // Build mapping from op_idx to SSA destination for this block
        let op_to_ssa: HashMap<usize, SsaVar> = if let Some(ssa_block) = self.ssa.blocks.get(&node) {
            ssa_block.ops.iter().filter_map(|op| {
                if let SsaInstr::Op { op_idx, dst: Some(dst), .. } = op {
                    Some((*op_idx, *dst))
                } else {
                    None
                }
            }).collect()
        } else {
            HashMap::new()
        };

        // Note: φ-functions are handled via φ-elimination in structure_conditional
        // We don't emit separate variable declarations for φ destinations here because:
        // 1. Same-register phis (different versions of same reg) don't need declarations
        // 2. Different-register phis get assignments emitted by structure_conditional
        // 3. The register will be declared when first assigned in regular code

        // Process opcodes
        // Skip control flow ops at block.end - they're handled by structuring logic
        let end = if self.is_control_flow_op(block.end) {
            // If block.end is a control flow op, process up to (but not including) it
            // Special case: if block is just one control flow op (start == end), skip entirely
            if block.start >= block.end {
                // No non-control-flow ops to process
                return stmts;
            }
            block.end - 1
        } else {
            block.end
        };

        let func_end = self.func.ops.len();
        let mut op_idx = block.start;
        while op_idx <= end {
            // Track current opcode for debug name lookup
            self.current_op = op_idx;

            // Check if this opcode starts an exception region
            if let Some(region) = self.exception_analysis.region_starting_at(op_idx).cloned() {
                // Compute catch_end: find next sequential region or use function end
                let catch_end = self.find_catch_end(region.handler_op, func_end);

                // Structure the try/catch and skip past the entire region
                let try_catch = self.structure_exception_region(&region, catch_end);
                stmts.push(try_catch);
                // Skip past the catch body to continue after the try/catch
                op_idx = catch_end;
                continue;
            }

            // Check if this op has an SSA destination we can analyze
            let is_dead = if let Some(&ssa_dst) = op_to_ssa.get(&op_idx) {
                self.is_dead_var(ssa_dst)
            } else {
                false
            };

            // Only skip dead variables for opcodes that have no side effects.
            // Function calls always have side effects and should be emitted.
            let op = &self.func.ops[op_idx];
            let has_side_effects = matches!(
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
                    | Opcode::SetField { .. }
                    | Opcode::SetArray { .. }
                    | Opcode::SetMem { .. }
                    | Opcode::SetI8 { .. }
                    | Opcode::SetI16 { .. }
                    | Opcode::SetGlobal { .. }
                    | Opcode::Throw { .. }
            );

            if is_dead && !has_side_effects {
                op_idx += 1;
                continue;
            }

            // Emit statement for this opcode
            if self.is_control_flow_op(op_idx) {
                eprintln!("BUG: Control flow op {} in opcode_to_statements: {:?}", op_idx, &self.func.ops[op_idx]);
                eprintln!("  block.start={}, block.end={}, end={}", block.start, block.end, end);
            }
            stmts.extend(self.opcode_to_statements(op_idx));
            op_idx += 1;
        }

        // Check for return/throw
        self.current_op = block.end;
        if let Some(ret) = self.check_terminal(block.end) {
            stmts.push(ret);
        }

        stmts
    }

    fn is_control_flow_op(&self, op_idx: usize) -> bool {
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

    fn check_terminal(&self, op_idx: usize) -> Option<Statement> {
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

    /// Convert opcode to statements.
    /// May return multiple statements if inline expressions need to be materialized
    /// due to memory conflicts.
    fn opcode_to_statements(&mut self, op_idx: usize) -> Vec<Statement> {
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
                // For Virtual types (anonymous objects), use empty object literal
                if matches!(&self.code.types[type_ref.0], hlbc::types::Type::Virtual { .. }) {
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

    fn get_global_name(&self, global: hlbc::types::RefGlobal) -> Str {
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
    fn clean_internal_name(&self, name: &str) -> Str {
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
    fn get_global_string_value(&self, global: hlbc::types::RefGlobal) -> Option<hlbc::types::RefString> {
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
    fn global_to_expr(&self, global: hlbc::types::RefGlobal) -> Expr {
        if let Some(string_ref) = self.get_global_string_value(global) {
            Expr::Constant(Constant::String(string_ref))
        } else {
            Expr::Ident(self.get_global_name(global))
        }
    }

    fn get_type_ref(&self, reg: Reg) -> RefType {
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
    fn try_make_method_call(&self, fun: RefFun, args: &[Reg]) -> Option<Call> {
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
    fn is_subtype_of(&self, derived: RefType, base: RefType) -> bool {
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
    fn is_void_type(&self, reg: Reg) -> bool {
        let type_ref = self.get_type_ref(reg);
        matches!(&self.code.types[type_ref.0], hlbc::types::Type::Void)
    }

    /// Check if a register holds an array type (hl.types.ArrayBytes_*, etc.)
    fn is_array_type(&self, reg: Reg) -> bool {
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
    fn is_nullable_element_type(&self, type_reg: Reg) -> bool {
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

    fn reg_name(&self, reg: Reg) -> Str {
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
    fn reg_name_for_source(&self, reg: Reg) -> Str {
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
    fn reg_name_at(&self, reg: Reg, at_op: usize) -> Str {
        self.get_debug_name_at(reg, at_op, true)
            .unwrap_or_else(|| format!("r{}", reg.0))
            .into()
    }

    /// Get SSA-versioned variable name for a destination register.
    /// If a debug name exists, use it (preserves user variable names).
    /// For same-register phi destinations/sources, use non-SSA name (allows value flow-through).
    /// Otherwise, use SSA-versioned name like "r3_1" to enable inlining.
    fn ssa_var_name_dst(&self, var: SsaVar) -> Str {
        // Check for raw name override first (set by CallMethod for .next() results)
        // Use non-versioned name for consistency
        if self.use_raw_name_regs.contains(&var.reg) {
            return format!("r{}", var.reg.0).into();
        }
        // If debug name exists, use it (preserve user variable names)
        if let Some(debug_name) = self.get_debug_name(var.reg, false) {
            return debug_name.into();
        }
        // For same-register phi destinations or sources, use non-SSA name
        // This allows the value to flow through if/else branches without explicit phi assignments
        if self.ssa.is_same_register_phi(var) || self.ssa.is_same_register_phi_source(var) {
            return format!("r{}", var.reg.0).into();
        }
        // No debug name → use SSA-versioned name
        format!("r{}_{}", var.reg.0, var.version).into()
    }

    /// Get SSA-versioned variable name for a source register.
    /// Uses source context (only debug names assigned BEFORE current_op).
    fn ssa_var_name_src(&self, var: SsaVar) -> Str {
        self.ssa_var_name_src_at(var, self.current_op)
    }

    /// Get SSA-versioned variable name for a source register at a specific opcode position.
    /// Uses source context (only debug names assigned BEFORE at_op).
    fn ssa_var_name_src_at(&self, var: SsaVar, at_op: usize) -> Str {
        // Check for raw name override first (set by CallMethod for .next() results)
        // Use non-versioned name to match the destination
        if self.use_raw_name_regs.contains(&var.reg) {
            return format!("r{}", var.reg.0).into();
        }
        // If debug name exists, use it (preserve user variable names)
        if let Some(debug_name) = self.get_debug_name_at(var.reg, at_op, true) {
            return debug_name.into();
        }
        // For same-register phi results or sources, use non-SSA name
        // This ensures consistency with phi destinations (allows value flow-through)
        if self.ssa.is_same_register_phi(var) || self.ssa.is_same_register_phi_source(var) {
            return format!("r{}", var.reg.0).into();
        }
        // No debug name → use SSA-versioned name
        format!("r{}_{}", var.reg.0, var.version).into()
    }

    /// Create expression for destination register using SSA-versioned name.
    fn reg_to_expr_ssa_dst(&self, reg: Reg, ssa_var: SsaVar) -> Expr {
        let name = self.ssa_var_name_dst(ssa_var);
        Expr::Variable(reg, Some(name))
    }

    /// Create expression for source register using SSA-versioned name.
    fn reg_to_expr_ssa_src(&self, reg: Reg, ssa_var: SsaVar) -> Expr {
        let name = self.ssa_var_name_src(ssa_var);
        Expr::Variable(reg, Some(name))
    }

    /// Find the SSA variable for a source register in the current instruction's uses.
    fn find_ssa_use(&self, reg: Reg) -> Option<SsaVar> {
        self.current_ssa_uses.iter().find(|v| v.reg == reg).copied()
    }

    /// Check if an SSA variable can be inlined and return its stored expression.
    /// Returns None if the variable shouldn't be inlined (multi-use, impure, has debug name, etc.)
    /// IMPORTANT: For non-constants, removes the expression from inline_exprs after returning it,
    /// since once inlined, it should not be invalidated or emitted as a separate statement.
    /// For constants, clones instead of removing since constants can be used multiple times.
    fn try_get_inline_expr(&self, var: SsaVar) -> Option<Expr> {
        // First check if it's a constant - constants can be used multiple times
        {
            let inline_exprs = self.inline_exprs.borrow();
            if let Some((expr, _)) = inline_exprs.get(&var) {
                if matches!(expr, Expr::Constant(_)) {
                    return Some(expr.clone());
                }
            }
        }

        // For non-constants, remove after retrieval (single use)
        self.inline_exprs.borrow_mut().remove(&var).map(|(expr, _)| expr)
    }

    /// Check if a variable should be inlined based on use-def info and debug names.
    /// This implements the ILSpy-style inlining safety guards.
    fn can_inline_var(&self, var: SsaVar) -> bool {
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
    ///   - Stores the expression with its memory dependency and returns None
    /// Otherwise:
    ///   - Returns Some(assignment statement)
    fn try_inline_or_assign(&mut self, dst: Reg, expr: Expr) -> Option<Statement> {
        // Check if we have an SSA destination that can be inlined
        if let Some(ssa_var) = self.current_ssa_dst {
            if ssa_var.reg == dst && self.can_inline_var(ssa_var) {
                // Compute memory dependency for this expression based on source opcode
                let mem_dep = self.compute_memory_dep();
                // Store for inlining - don't emit statement
                self.inline_exprs.borrow_mut().insert(ssa_var, (expr, mem_dep));
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
    fn compute_memory_dep(&self) -> MemoryDep {
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
    /// Called before processing each opcode to ensure we don't inline expressions
    /// whose source memory has been modified.
    ///
    /// Returns statements for any invalidated expressions (they need to be emitted
    /// since we originally suppressed their definition statements).
    fn invalidate_conflicting_inlines(&mut self, op: &Opcode) -> Vec<Statement> {
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
            for (ssa_var, (expr, dep)) in inline_exprs.iter() {
                if conflicts_with(dep) {
                    // Create assignment statement for the invalidated expression
                    let var_name: Str = ssa_var.name().into();
                    let var_expr = Expr::Variable(ssa_var.reg, Some(var_name.clone()));
                    // Ensure variable is declared
                    if !self.declared_vars.contains(&var_name) {
                        self.declared_vars.insert(var_name.clone());
                        self.hoisted_vars.insert(var_name.clone());
                        // Track type for hoisted var
                        if let Some(tr) = self.func.regs.get(ssa_var.reg.0 as usize).copied() {
                            self.hoisted_var_types.insert(var_name, tr);
                        }
                    }
                    stmts.push(Statement::Assign {
                        declaration: false,
                        variable: var_expr,
                        assign: expr.clone(),
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

    /// Get debug name for a register, optionally for source context.
    /// When `for_source` is true, only returns names assigned BEFORE current_op.
    /// This prevents using a name before it's been assigned (e.g., `var dx = dx - r3`).
    fn get_debug_name(&self, reg: Reg, for_source: bool) -> Option<String> {
        self.get_debug_name_at(reg, self.current_op, for_source)
    }

    /// Get debug name for a register at a specific opcode position.
    fn get_debug_name_at(&self, reg: Reg, at_op: usize, for_source: bool) -> Option<String> {
        let reg_idx = reg.0 as usize;

        // First, check if this is a function parameter
        // Parameters are the first N registers where N = number of function args
        if let Some(Type::Fun(fun_type) | Type::Method(fun_type)) = self.code.types.get(self.func.t.0) {
            let num_args = fun_type.args.len();
            if reg_idx < num_args {
                // For instance methods, reg0 is 'this' and doesn't have an assign entry.
                // The assigns at op_idx 0 start from the first explicit parameter (reg1).
                // So we need to adjust: arg_name_pos = reg_idx - 1 for instance methods.
                //
                // Instance methods are detected by:
                // 1. Constructors (name starts with "__constructor__")
                // 2. Methods where first arg type matches parent type
                //
                // Note: Just having parent.is_some() is NOT enough - static methods
                // also have parent set (they belong to a class).
                let func_name = self.code.strings.get(self.func.name.0)
                    .map(|s| s.as_ref())
                    .unwrap_or("");
                let is_constructor = func_name.starts_with("__constructor__");

                // Check if first arg type matches parent type (indicates instance method)
                let first_arg_is_self = if let Some(parent_type) = self.func.parent {
                    !fun_type.args.is_empty() && fun_type.args[0] == parent_type
                } else {
                    false
                };

                let is_instance_method = is_constructor || first_arg_is_self;

                if reg_idx == 0 && is_instance_method {
                    // reg0 is 'this' for instance methods
                    return Some("this".to_string());
                }

                // For explicit parameters, adjust the position for arg_name
                let arg_name_pos = if is_instance_method { reg_idx - 1 } else { reg_idx };
                if let Some(name) = self.func.arg_name(self.code, arg_name_pos) {
                    if self.is_valid_identifier(&name) {
                        return Some(name.to_string());
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
    fn is_valid_identifier(&self, name: &str) -> bool {
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
    fn name_conflicts_with_param(&self, name: &str) -> bool {
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

    /// Check if a field is an interface implementation cache field (has empty name).
    /// These are internal HashLink fields used to cache interface vtable lookups.
    fn is_interface_cache_field(&self, obj_reg: Reg, field: hlbc::types::RefField) -> bool {
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

    fn get_field_name(&self, obj_reg: Reg, field: hlbc::types::RefField) -> Str {
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
    fn get_proto_name(&self, obj_reg: Reg, proto_idx: hlbc::types::RefField) -> Str {
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

    /// Look ahead from a New opcode to find the __constructor__ call arguments.
    /// In HashLink bytecode, object construction is split:
    ///   New reg0 = new Type
    ///   GetGlobal reg1 = global@5  // "Hello World"
    ///   Call2 __constructor__(reg0, reg1)
    /// We need to combine these into: new Type("Hello World")
    /// Returns (constructor_args, constructor_call_opcode_index)
    fn find_constructor_args(&self, new_dst: Reg, new_op_idx: usize) -> (Vec<Expr>, Option<usize>, Vec<usize>) {
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
                    let call = Expr::Call(Box::new(crate::ast::Call {
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
                    let call = Expr::Call(Box::new(crate::ast::Call::new_fun(*fun, vec![])));
                    reg_values.insert(*dst, call);
                    // Don't suppress - calls have side effects
                }
                Opcode::Call1 { dst, fun, arg0 } => {
                    let arg = get_val(*arg0, &reg_values);
                    let call = Expr::Call(Box::new(crate::ast::Call::new_fun(*fun, vec![arg])));
                    reg_values.insert(*dst, call);
                    // Don't suppress - calls have side effects
                }
                // Track Call2 but skip if it's the constructor call we're looking for
                Opcode::Call2 { dst, fun, arg0, arg1 } if *arg0 != new_dst => {
                    let a0 = get_val(*arg0, &reg_values);
                    let a1 = get_val(*arg1, &reg_values);
                    let call = Expr::Call(Box::new(crate::ast::Call::new_fun(*fun, vec![a0, a1])));
                    reg_values.insert(*dst, call);
                    // Don't suppress - calls have side effects
                }
                // Track Call3 but skip if it's the constructor call
                Opcode::Call3 { dst, fun, arg0, arg1, arg2 } if *arg0 != new_dst => {
                    let a0 = get_val(*arg0, &reg_values);
                    let a1 = get_val(*arg1, &reg_values);
                    let a2 = get_val(*arg2, &reg_values);
                    let call = Expr::Call(Box::new(crate::ast::Call::new_fun(*fun, vec![a0, a1, a2])));
                    reg_values.insert(*dst, call);
                    // Don't suppress - calls have side effects
                }
                // Track Call4 but skip if it's the constructor call
                Opcode::Call4 { dst, fun, arg0, arg1, arg2, arg3 } if *arg0 != new_dst => {
                    let a0 = get_val(*arg0, &reg_values);
                    let a1 = get_val(*arg1, &reg_values);
                    let a2 = get_val(*arg2, &reg_values);
                    let a3 = get_val(*arg3, &reg_values);
                    let call = Expr::Call(Box::new(crate::ast::Call::new_fun(*fun, vec![a0, a1, a2, a3])));
                    reg_values.insert(*dst, call);
                    // Don't suppress - calls have side effects
                }
                // Track CallN but skip if it's the constructor call
                Opcode::CallN { dst, fun, args } if args.first() != Some(&new_dst) => {
                    let call_args: Vec<_> = args.iter().map(|r| get_val(*r, &reg_values)).collect();
                    let call = Expr::Call(Box::new(crate::ast::Call::new_fun(*fun, call_args)));
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
    fn is_constructor_function(&self, fun: RefFun) -> bool {
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
    fn is_type_only_for_array_alloc(&self, type_dst: Reg, type_op_idx: usize) -> bool {
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

    /// Check if the current function being decompiled is a constructor
    fn is_current_function_constructor(&self) -> bool {
        self.code.strings.get(self.func.name.0)
            .map(|s| s.starts_with("__constructor__"))
            .unwrap_or(false)
    }

    /// Check if calling a function with this as first arg is a super method call.
    /// This is true when:
    /// 1. Current function has the same name as the target function
    /// 2. Current function is an override (has parent)
    /// 3. Target function belongs to the parent class
    fn is_super_method_call(&self, fun: RefFun) -> bool {
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
    fn get_function_name(&self, fun: RefFun) -> Option<Str> {
        fun.as_fn(self.code)
            .and_then(|f| self.code.strings.get(f.name.0))
            .cloned()
    }

    /// Get expression for reading from a register (source context).
    /// Uses SSA-versioned names when available to enable inlining.
    /// Falls back to debug names assigned BEFORE current_op.
    fn reg_to_expr(&self, reg: Reg) -> Expr {
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
    fn reg_to_expr_dst(&self, reg: Reg) -> Expr {
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
    fn reg_to_expr_in_block(&self, reg: Reg, block: NodeIndex) -> Expr {
        let blk = &self.cfg.graph[block];

        // Use SSA: look up which version of this register is used at block.end
        // The conditional jump is at block.end, so we want the SSA variable used there
        if let Some((_dst, uses)) = self.ssa.get_instr_for_op(blk.end) {
            for ssa_var in uses {
                if ssa_var.reg == reg {
                    // IMPORTANT: Check if this variable has an inlined expression first.
                    // If the variable was marked for inlining, its defining statement was
                    // suppressed, so we must return the stored expression, not a variable name.
                    if let Some(inline_expr) = self.try_get_inline_expr(*ssa_var) {
                        return inline_expr;
                    }

                    // Found the SSA variable for this register at block end
                    // Now find its definition to see if it's a constant
                    let def = self.ssa.find_def(*ssa_var);
                    if let Some(def_op_idx) = def {
                        match &self.func.ops[def_op_idx] {
                            Opcode::Int { ptr, .. } => {
                                return Expr::Constant(Constant::Int(*ptr));
                            }
                            Opcode::Float { ptr, .. } => {
                                return Expr::Constant(Constant::Float(*ptr));
                            }
                            Opcode::Bool { value, .. } => {
                                return Expr::Constant(Constant::Bool(*value));
                            }
                            Opcode::String { ptr, .. } => {
                                return Expr::Constant(Constant::String(*ptr));
                            }
                            Opcode::Null { .. } => {
                                return Expr::Constant(Constant::Null);
                            }
                            _ => {
                                // Not a constant - return as variable with SSA-versioned name
                                // Use block.end as the position for debug name lookup
                                let name = self.ssa_var_name_src_at(*ssa_var, blk.end);
                                return Expr::Variable(reg, Some(name));
                            }
                        }
                    }
                    // No def found (phi function) - use SSA-versioned name
                    // Use block.end as the position for debug name lookup
                    let name = self.ssa_var_name_src_at(*ssa_var, blk.end);
                    return Expr::Variable(reg, Some(name));
                }
            }
        }

        // Fallback: return as variable using non-SSA naming
        let name = self.reg_name_at(reg, blk.end);
        Expr::Variable(reg, Some(name))
    }

    fn compute_target(&self, op_idx: usize, offset: i32) -> Option<NodeIndex> {
        let target_idx = (op_idx as i64 + offset as i64 + 1) as usize;
        self.cfg.block_for_op(target_idx)
    }

    /// Get the constant value of a register at a specific opcode using SSA information.
    /// Looks up which SSA variable is used for this register at this op,
    /// then finds its definition and checks if it's a constant.
    fn get_ssa_constant_value(&self, reg: Reg, at_op: usize) -> Option<i32> {
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
    fn get_ssa_based_expr(&self, reg: Reg, at_op: usize) -> Expr {
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
                    let is_phi_involved = self.ssa.is_same_register_phi(*ssa_var) || self.ssa.is_same_register_phi_source(*ssa_var);

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
                        // EXCEPTION: Function parameters (version 0) are always safe since
                        // they're defined at function entry before any branches.
                        if ssa_var.version == 0 {
                            // Check if this is a function parameter
                            if let Some(debug_name) = self.get_debug_name_at(reg, at_op, true) {
                                debug_name.into()
                            } else {
                                format!("r{}_{}", reg.0, ssa_var.version).into()
                            }
                        } else {
                            // Non-parameter, non-phi: use raw SSA-versioned name
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

/// Simplify a list of statements by:
/// 1. Merging consecutive assignments (r3 = expr; r0 = r3; → r0 = expr;)
/// 2. Recursively simplifying nested blocks
fn simplify_statements(stmts: Vec<Statement>) -> Vec<Statement> {
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
fn is_same_expr(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Variable(reg_a, _), Expr::Variable(reg_b, _)) => reg_a == reg_b,
        _ => false,
    }
}

/// Check if a variable is used anywhere in a list of statements
fn is_var_used_in_stmts(var: &Expr, stmts: &[Statement]) -> bool {
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
fn is_var_used_in_stmt(reg: &Reg, stmt: &Statement) -> bool {
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
fn is_var_used_in_expr(reg: &Reg, expr: &Expr) -> bool {
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
fn is_var_used_in_operation(reg: &Reg, op: &Operation) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::CfgAnalysis;
    use crate::lifter::Cfg;
    use crate::ssa::SsaCfg;
    use crate::type_prop::TypePropagator;
    use hlbc::types::{RefFun, RefInt, RefString, RefType};

    fn create_mock_bytecode() -> Bytecode {
        let mut code = Bytecode::default();
        code.ints = vec![0, 1, 10, 42];
        code.strings = vec!["test".into()];
        code.types = vec![hlbc::types::Type::Void, hlbc::types::Type::I32];
        code
    }

    fn create_mock_function(ops: &[Opcode], num_regs: usize) -> Function {
        Function {
            name: RefString(0),
            t: RefType(0),
            findex: RefFun(0),
            regs: vec![RefType(1); num_regs],
            ops: ops.to_vec(),
            debug_info: None,
            assigns: None,
            parent: None,
        }
    }

    #[test]
    fn test_simple_linear() {
        let code = create_mock_bytecode();
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(3) },
            Opcode::Ret { ret: Reg(0) },
        ];

        let func = create_mock_function(&ops, 1);
        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        let ssa = SsaCfg::build(&func, &cfg, &analysis);
        let type_info = TypePropagator::new(&code, &func, &cfg, &ssa).propagate();

        let mut structurer = Structurer::new(&code, &func, &cfg, &analysis, &ssa, &type_info);
        let stmts = structurer.structure();

        assert!(!stmts.is_empty());
    }

    #[test]
    fn test_conditional() {
        let code = create_mock_bytecode();
        let ops = vec![
            Opcode::Int { dst: Reg(0), ptr: RefInt(0) },
            Opcode::JNull { reg: Reg(0), offset: 2 },
            Opcode::Int { dst: Reg(1), ptr: RefInt(1) },
            Opcode::JAlways { offset: 1 },
            Opcode::Int { dst: Reg(1), ptr: RefInt(2) },
            Opcode::Ret { ret: Reg(1) },
        ];

        let func = create_mock_function(&ops, 2);
        let cfg = Cfg::from_ops(&ops);
        let analysis = CfgAnalysis::analyze(&cfg);
        let ssa = SsaCfg::build(&func, &cfg, &analysis);
        let type_info = TypePropagator::new(&code, &func, &cfg, &ssa).propagate();

        let mut structurer = Structurer::new(&code, &func, &cfg, &analysis, &ssa, &type_info);
        let stmts = structurer.structure();

        assert!(!stmts.is_empty());
    }
}
