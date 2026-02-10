use std::collections::{HashMap, HashSet, VecDeque};

use hlbc::opcodes::Opcode;
use hlbc::types::{
    ConstantDef, Function, ObjField, ObjProto, RefBytes, RefField, RefFloat, RefFun, RefGlobal,
    RefInt, RefString, RefType, Type,
};
use hlbc::{Bytecode, Resolve};

use crate::remap::IndexRemap;

/// Known stdlib function renames between Haxe versions.
/// Maps (parent_type, source_name) -> target_name
/// These allow code compiled with newer Haxe to run on older runtimes (and vice versa).
const STDLIB_FUNCTION_REMAPS: &[(&str, &str, &str)] = &[
    // Haxe 4.2+ renamed Std.is to Std.isOfType
    // When source has isOfType but target has is, remap the call
    ("$Std", "isOfType", "is"),
    // Add more remaps here as needed for other Haxe version differences
];

/// Get the initialized string value for a String-type global, if any
fn get_global_string_value<'a>(code: &'a Bytecode, global: RefGlobal) -> Option<&'a str> {
    // Check if global has a constant initializer
    let &const_idx = code.globals_initializers.get(&global)?;
    let constants = code.constants.as_ref()?;
    let constant_def = constants.get(const_idx)?;
    // For strings, fields[0] is the string pool index
    let string_idx = *constant_def.fields.first()?;
    code.strings.get(string_idx).map(|s| s.as_ref())
}

/// Build a map of global_idx -> enum construct index by scanning the entrypoint function.
///
/// The Haxe compiler initializes enum globals via entrypoint code like:
///   Int reg = construct_index
///   GetArray reg2 = evalues[reg]
///   SafeCast reg3 = cast reg2
///   SetGlobal global = reg3
///
/// This is the only path Haxe uses for enum globals (they never appear in the constants table).
fn build_enum_construct_map(code: &Bytecode) -> HashMap<usize, usize> {
    let mut map = HashMap::new();
    let ops = &code.entrypoint().ops;

    for i in 0..ops.len().saturating_sub(3) {
        // Match: Int { dst: idx_reg, ptr } -> GetArray { index: idx_reg } -> SafeCast { dst: cast_reg } -> SetGlobal { src: cast_reg }
        if let Opcode::Int { dst: idx_reg, ptr } = &ops[i] {
            if let Opcode::GetArray { dst: _, array: _, index: ga_idx_reg } = &ops[i + 1] {
                if ga_idx_reg == idx_reg {
                    if let Opcode::SafeCast { dst: cast_reg, src: _ } = &ops[i + 2] {
                        if let Opcode::SetGlobal { global, src: sg_reg } = &ops[i + 3] {
                            if sg_reg == cast_reg {
                                let construct_index = code.ints[ptr.0] as usize;
                                map.insert(global.0, construct_index);
                            }
                        }
                    }
                }
            }
        }
    }

    map
}

/// Get the enum construct index for an enum-type global, if any.
/// First tries the constants table (for non-Haxe or future compilers),
/// then falls back to the entrypoint scan map (the path Haxe actually uses).
fn get_global_enum_construct(
    code: &Bytecode,
    global: RefGlobal,
    _enum_type: RefType,
    enum_construct_map: &HashMap<usize, usize>,
) -> Option<usize> {
    // First try: constant initializer (for non-Haxe or future compilers)
    if let Some(&const_idx) = code.globals_initializers.get(&global) {
        if let Some(constants) = code.constants.as_ref() {
            if let Some(constant_def) = constants.get(const_idx) {
                if let Some(&construct) = constant_def.fields.first() {
                    return Some(construct);
                }
            }
        }
    }
    // Second: entrypoint scan (the path Haxe actually uses)
    enum_construct_map.get(&global.0).copied()
}

/// Get the construct name for an enum global given its construct index
fn get_enum_construct_name(
    code: &Bytecode,
    enum_type: RefType,
    construct_idx: usize,
) -> Option<String> {
    if let Type::Enum { constructs, .. } = code.get(enum_type) {
        constructs
            .get(construct_idx)
            .map(|c| code.get(c.name).to_string())
    } else {
        None
    }
}

/// Find construct index by name in an enum type
fn find_construct_index_by_name(code: &Bytecode, enum_type: RefType, name: &str) -> Option<usize> {
    if let Type::Enum { constructs, .. } = code.get(enum_type) {
        constructs.iter().position(|c| code.get(c.name) == name)
    } else {
        None
    }
}

/// Check if two types are both virtuals with equivalent fields (same names, ignoring order)
/// Virtual types are looked up by field name, so order doesn't matter for compatibility
pub fn virtuals_equivalent(
    src_code: &Bytecode,
    src_type: RefType,
    target_code: &Bytecode,
    target_type: RefType,
) -> bool {
    match (src_code.get(src_type), target_code.get(target_type)) {
        (Type::Virtual { fields: src_fields }, Type::Virtual { fields: target_fields }) => {
            // Collect field names as sets
            let src_names: std::collections::HashSet<_> = src_fields
                .iter()
                .map(|f| src_code.get(f.name).to_string())
                .collect();
            let target_names: std::collections::HashSet<_> = target_fields
                .iter()
                .map(|f| target_code.get(f.name).to_string())
                .collect();
            src_names == target_names
        }
        _ => false,
    }
}

/// Format a type as a human-readable string (e.g. "null(f64)", "obj(GlDriver)")
pub fn format_type(code: &Bytecode, t: RefType) -> String {
    match code.get(t) {
        Type::Void => "void".to_string(),
        Type::UI8 => "ui8".to_string(),
        Type::UI16 => "ui16".to_string(),
        Type::I32 => "i32".to_string(),
        Type::I64 => "i64".to_string(),
        Type::F32 => "f32".to_string(),
        Type::F64 => "f64".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Bytes => "bytes".to_string(),
        Type::Dyn => "dyn".to_string(),
        Type::Array => "array".to_string(),
        Type::Type => "type".to_string(),
        Type::DynObj => "dynobj".to_string(),
        Type::Null(inner) => format!("null({})", format_type(code, *inner)),
        Type::Ref(inner) => format!("ref({})", format_type(code, *inner)),
        Type::Packed(inner) => format!("packed({})", format_type(code, *inner)),
        Type::Obj(obj) => format!("obj({})", code.get(obj.name)),
        Type::Struct(obj) => format!("struct({})", code.get(obj.name)),
        Type::Enum { name, .. } => format!("enum({})", code.get(*name)),
        Type::Abstract { name } => format!("abstract({})", code.get(*name)),
        Type::Fun(fun) => {
            let args: Vec<_> = fun.args.iter().map(|a| format_type(code, *a)).collect();
            format!("fun({})->{}", args.join(","), format_type(code, fun.ret))
        }
        Type::Method(fun) => {
            let args: Vec<_> = fun.args.iter().map(|a| format_type(code, *a)).collect();
            format!("method({})->{}", args.join(","), format_type(code, fun.ret))
        }
        Type::Virtual { fields } => {
            let field_names: Vec<_> = fields.iter().map(|f| code.get(f.name).to_string()).collect();
            format!("virtual({})", field_names.join(","))
        }
        Type::Guid => "guid".to_string(),
    }
}

/// Information about a field type mismatch (same name, different type)
#[derive(Debug, Clone)]
pub struct FieldTypeMismatch {
    pub field_name: String,
    pub source_type: String, // e.g. "null(f64)"
    pub target_type: String, // e.g. "f64"
}

/// Information about a field order mismatch (same field count, different field at same position)
#[derive(Debug, Clone)]
pub struct FieldOrderMismatch {
    pub position: usize,           // 0-based field index
    pub source_field: String,      // "fieldName:Type"
    pub target_field: String,      // "fieldName:Type"
}

/// Method signature mismatch: same name as parent method, different signature
/// This causes vtable dispatch to fail silently - the child method won't be called.
#[derive(Debug, Clone)]
pub struct MethodSignatureMismatch {
    pub method_name: String,
    pub parent_class: String,
    pub source_signature: String,
    pub target_signature: String,
}

/// Information about an enum construct parameter type mismatch
#[derive(Debug, Clone)]
pub struct EnumConstructParamMismatch {
    pub construct_name: String,
    pub param_index: usize,
    pub source_type: String,
    pub target_type: String,
}

/// Information about enum construct mismatches between source and target
#[derive(Debug, Clone)]
pub struct EnumMismatch {
    pub enum_name: String,
    pub construct_param_mismatches: Vec<EnumConstructParamMismatch>,
}

/// Information about type layout mismatches between source and target
#[derive(Debug, Clone)]
pub struct TypeMismatch {
    pub type_name: String,
    pub target_field_count: usize,
    pub source_field_count: usize,
    pub missing_in_target: Vec<String>,            // fields in source but not target
    pub extra_in_target: Vec<String>,              // fields in target but not source (less common)
    pub field_type_mismatches: Vec<FieldTypeMismatch>, // same name, different type
    pub field_order_mismatches: Vec<FieldOrderMismatch>, // different field at same position
    pub method_signature_mismatches: Vec<MethodSignatureMismatch>, // override signature mismatch
}

/// Merges pools from source bytecode into target bytecode, building an IndexRemap
pub struct PoolMerger<'a> {
    pub target: &'a mut Bytecode,
    pub source: &'a Bytecode,
    pub remap: IndexRemap,
    pub warnings: Vec<String>,
    /// Whether to inject missing function dependencies
    pub inject_missing_functions: bool,
    /// Whether to inject missing native declarations into target
    pub inject_missing_natives: bool,
    /// Functions that were injected from source
    pub injected_functions: Vec<String>,
    /// Native functions that were injected into target
    pub injected_natives: Vec<String>,
    /// Native functions that couldn't be resolved (natives can't be injected)
    pub unresolvable_natives: Vec<String>,
    /// Type layout mismatches detected (source type index -> mismatch info)
    pub type_mismatches: HashMap<usize, TypeMismatch>,
    /// Enum construct param type mismatches detected (source type index -> mismatch info)
    pub enum_mismatches: HashMap<usize, EnumMismatch>,
    /// Fields in source types that don't exist in target: type_idx -> set of field indices
    /// Used for validation during opcode remapping
    pub missing_fields: HashMap<usize, HashSet<usize>>,
    /// Stdlib functions that couldn't be injected due to version mismatch
    /// (qualified_name, reason)
    pub stdlib_mismatches: Vec<(String, String)>,
    /// Source global indices that resulted in NEW globals being created in target
    /// (not matched to existing). Maps source_global_idx -> target_global_idx.
    /// These are the globals that need initialization code injection.
    pub injected_globals: HashMap<usize, usize>,
    /// Whether to inject new types when their parent exists in target.
    /// This enables adding new subclasses like `shader.UberSprite extends hxsl.Shader`.
    pub inject_new_types: bool,
    /// Types that were fully injected - maps src_type_idx -> target_type_idx
    pub injected_types: HashMap<usize, usize>,
    /// Pending protos to add after functions are injected.
    /// Maps target_type_idx -> (parent_ref, Vec<(proto_name, src_findex, pindex)>)
    pub pending_protos: HashMap<usize, (Option<RefType>, Vec<(String, RefFun, i32)>)>,
    /// Pending bindings (field -> function mappings like __constructor__)
    /// Maps target_type_idx -> Vec<(field_idx, src_findex)>
    pub pending_bindings: HashMap<usize, Vec<(usize, RefFun)>>,
    /// Enum global -> construct index map for source bytecode (from entrypoint scan)
    source_enum_constructs: HashMap<usize, usize>,
    /// Enum global -> construct index map for target bytecode (from entrypoint scan)
    target_enum_constructs: HashMap<usize, usize>,
    /// Reverse lookup: target int value -> index (avoids O(n) scan in ensure_int)
    target_int_lookup: HashMap<i32, usize>,
    /// Reverse lookup: target float bits -> index (avoids O(n) scan in ensure_float)
    target_float_lookup: HashMap<u64, usize>,
    /// Reverse lookup: target string -> index (avoids O(n) scan in ensure_string)
    target_string_lookup: HashMap<String, usize>,
}

impl<'a> PoolMerger<'a> {
    /// Build reverse-lookup HashMaps for target pools (int/float/string).
    fn build_pool_lookups(target: &Bytecode) -> (HashMap<i32, usize>, HashMap<u64, usize>, HashMap<String, usize>) {
        let int_lookup: HashMap<i32, usize> = target.ints.iter()
            .enumerate().map(|(i, &v)| (v, i)).collect();
        let float_lookup: HashMap<u64, usize> = target.floats.iter()
            .enumerate().map(|(i, &v)| (v.to_bits(), i)).collect();
        let string_lookup: HashMap<String, usize> = target.strings.iter()
            .enumerate().map(|(i, s)| (s.to_string(), i)).collect();
        (int_lookup, float_lookup, string_lookup)
    }

    pub fn new(target: &'a mut Bytecode, source: &'a Bytecode, inject_deps: bool) -> Self {
        let source_enum_constructs = build_enum_construct_map(source);
        let target_enum_constructs = build_enum_construct_map(target);
        let (target_int_lookup, target_float_lookup, target_string_lookup) = Self::build_pool_lookups(target);
        let mut merger = Self {
            target,
            source,
            remap: IndexRemap::new(),
            warnings: Vec::new(),
            inject_missing_functions: inject_deps,
            inject_missing_natives: false, // Must be explicitly enabled
            injected_functions: Vec::new(),
            injected_natives: Vec::new(),
            unresolvable_natives: Vec::new(),
            type_mismatches: HashMap::new(),
            enum_mismatches: HashMap::new(),
            missing_fields: HashMap::new(),
            stdlib_mismatches: Vec::new(),
            injected_globals: HashMap::new(),
            inject_new_types: false,
            injected_types: HashMap::new(),
            pending_protos: HashMap::new(),
            pending_bindings: HashMap::new(),
            source_enum_constructs,
            target_enum_constructs,
            target_int_lookup,
            target_float_lookup,
            target_string_lookup,
        };
        merger.build_virtual_method_map();
        merger
    }

    /// Create a merger with native injection enabled
    pub fn with_native_injection(target: &'a mut Bytecode, source: &'a Bytecode, inject_deps: bool) -> Self {
        let source_enum_constructs = build_enum_construct_map(source);
        let target_enum_constructs = build_enum_construct_map(target);
        let (target_int_lookup, target_float_lookup, target_string_lookup) = Self::build_pool_lookups(target);
        let mut merger = Self {
            target,
            source,
            remap: IndexRemap::new(),
            warnings: Vec::new(),
            inject_missing_functions: inject_deps,
            inject_missing_natives: true,
            injected_functions: Vec::new(),
            injected_natives: Vec::new(),
            unresolvable_natives: Vec::new(),
            type_mismatches: HashMap::new(),
            enum_mismatches: HashMap::new(),
            missing_fields: HashMap::new(),
            stdlib_mismatches: Vec::new(),
            injected_globals: HashMap::new(),
            inject_new_types: false,
            injected_types: HashMap::new(),
            pending_protos: HashMap::new(),
            pending_bindings: HashMap::new(),
            source_enum_constructs,
            target_enum_constructs,
            target_int_lookup,
            target_float_lookup,
            target_string_lookup,
        };
        merger.build_virtual_method_map();
        merger
    }

    /// Create a merger with type injection enabled.
    /// This enables adding new subclasses like `shader.UberSprite extends hxsl.Shader`.
    pub fn with_type_injection(target: &'a mut Bytecode, source: &'a Bytecode, inject_deps: bool) -> Self {
        let source_enum_constructs = build_enum_construct_map(source);
        let target_enum_constructs = build_enum_construct_map(target);
        let (target_int_lookup, target_float_lookup, target_string_lookup) = Self::build_pool_lookups(target);
        let mut merger = Self {
            target,
            source,
            remap: IndexRemap::new(),
            warnings: Vec::new(),
            inject_missing_functions: inject_deps,
            inject_missing_natives: true, // Type injection requires native injection
            injected_functions: Vec::new(),
            injected_natives: Vec::new(),
            unresolvable_natives: Vec::new(),
            type_mismatches: HashMap::new(),
            enum_mismatches: HashMap::new(),
            missing_fields: HashMap::new(),
            stdlib_mismatches: Vec::new(),
            injected_globals: HashMap::new(),
            inject_new_types: true,
            injected_types: HashMap::new(),
            pending_protos: HashMap::new(),
            pending_bindings: HashMap::new(),
            source_enum_constructs,
            target_enum_constructs,
            target_int_lookup,
            target_float_lookup,
            target_string_lookup,
        };
        merger.build_virtual_method_map();
        merger
    }

    /// Ensure an int constant exists in target, return the remapped RefInt
    pub fn ensure_int(&mut self, src_ref: RefInt) -> RefInt {
        if let Some(&target_idx) = self.remap.ints.get(&src_ref.0) {
            return RefInt(target_idx);
        }

        let src_val = self.source.ints[src_ref.0];

        // O(1) lookup via HashMap
        if let Some(&target_idx) = self.target_int_lookup.get(&src_val) {
            self.remap.ints.insert(src_ref.0, target_idx);
            return RefInt(target_idx);
        }

        // Add new int
        let target_idx = self.target.ints.len();
        self.target.ints.push(src_val);
        self.target_int_lookup.insert(src_val, target_idx);
        self.remap.ints.insert(src_ref.0, target_idx);
        RefInt(target_idx)
    }

    /// Ensure a float constant exists in target, return the remapped RefFloat
    pub fn ensure_float(&mut self, src_ref: RefFloat) -> RefFloat {
        if let Some(&target_idx) = self.remap.floats.get(&src_ref.0) {
            return RefFloat(target_idx);
        }

        let src_val = self.source.floats[src_ref.0];

        // O(1) lookup via HashMap (bitwise comparison for floats)
        if let Some(&target_idx) = self.target_float_lookup.get(&src_val.to_bits()) {
            self.remap.floats.insert(src_ref.0, target_idx);
            return RefFloat(target_idx);
        }

        // Add new float
        let target_idx = self.target.floats.len();
        self.target.floats.push(src_val);
        self.target_float_lookup.insert(src_val.to_bits(), target_idx);
        self.remap.floats.insert(src_ref.0, target_idx);
        RefFloat(target_idx)
    }

    /// Ensure a string exists in target, return the remapped RefString
    pub fn ensure_string(&mut self, src_ref: RefString) -> RefString {
        if let Some(&target_idx) = self.remap.strings.get(&src_ref.0) {
            return RefString(target_idx);
        }

        let src_str = &self.source.strings[src_ref.0];

        // O(1) lookup via HashMap
        if let Some(&target_idx) = self.target_string_lookup.get(src_str.as_ref()) {
            self.remap.strings.insert(src_ref.0, target_idx);
            return RefString(target_idx);
        }

        // Add new string
        let target_idx = self.target.strings.len();
        self.target.strings.push(src_str.clone());
        self.target_string_lookup.insert(src_str.to_string(), target_idx);
        self.remap.strings.insert(src_ref.0, target_idx);
        RefString(target_idx)
    }

    /// Ensure a debug file exists in target, return the remapped file index.
    /// Adds ".substituted" suffix to distinguish injected code from original.
    pub fn ensure_debug_file(&mut self, src_file_idx: usize) -> usize {
        if let Some(&target_idx) = self.remap.debug_files.get(&src_file_idx) {
            return target_idx;
        }

        let Some(src_debug_files) = &self.source.debug_files else {
            // Source has no debug files, return 0 as fallback
            return 0;
        };

        let Some(src_file) = src_debug_files.get(src_file_idx) else {
            // Invalid source index, return 0 as fallback
            return 0;
        };

        // Create the substituted filename: "Foo.hx" -> "Foo.hx.substituted"
        let substituted_name: hlbc::Str = format!("{}.substituted", src_file).into();

        // Ensure target has debug_files vec
        let debug_files = self.target.debug_files.get_or_insert_with(Vec::new);

        // Check if this substituted filename already exists
        if let Some(target_idx) = debug_files.iter().position(|s| s == &substituted_name) {
            self.remap.debug_files.insert(src_file_idx, target_idx);
            return target_idx;
        }

        // Add new debug file
        let target_idx = debug_files.len();
        debug_files.push(substituted_name);
        self.remap.debug_files.insert(src_file_idx, target_idx);
        target_idx
    }

    /// Ensure all debug files referenced in a function's debug info exist in target,
    /// and return the remapped debug info.
    pub fn remap_function_debug_info(
        &mut self,
        debug_info: &[(usize, usize)],
    ) -> Vec<(usize, usize)> {
        debug_info
            .iter()
            .map(|(file_idx, line_num)| {
                let target_file_idx = self.ensure_debug_file(*file_idx);
                (target_file_idx, *line_num)
            })
            .collect()
    }

    /// Ensure bytes exist in target, return the remapped RefBytes
    pub fn ensure_bytes(&mut self, src_ref: RefBytes) -> RefBytes {
        if let Some(&target_idx) = self.remap.bytes.get(&src_ref.0) {
            return RefBytes(target_idx);
        }

        // Bytes are stored as (Vec<u8>, Vec<usize>) where the Vec<usize> contains offsets
        // This is more complex - for now just warn if bytes are used
        if self.source.bytes.is_some() {
            self.warnings.push(format!(
                "Bytes constant pool remapping not fully implemented (ref {})",
                src_ref.0
            ));
        }

        // Return as-is for now
        self.remap.bytes.insert(src_ref.0, src_ref.0);
        src_ref
    }

    /// Ensure a type exists in target, return the remapped RefType
    /// Types are matched by name for objects/structs, or structurally for others
    pub fn ensure_type(&mut self, src_ref: RefType) -> RefType {
        // Known types (primitives) don't need remapping - they're at fixed positions
        if src_ref.is_known() {
            return src_ref;
        }

        if let Some(&target_idx) = self.remap.types.get(&src_ref.0) {
            return RefType(target_idx);
        }

        let src_type = self.source.get(src_ref);

        // Try to find matching type in target
        match src_type {
            Type::Obj(obj) | Type::Struct(obj) => {
                let src_name = self.source.get(obj.name);
                for (i, t) in self.target.types.iter().enumerate() {
                    if let Some(target_obj) = t.get_type_obj() {
                        if self.target.get(target_obj.name) == src_name {
                            self.remap.types.insert(src_ref.0, i);
                            let target_type = RefType(i);
                            // Process field remapping and injection
                            self.process_type_fields(src_ref, target_type);
                            return target_type;
                        }
                    }
                }
                // Type not found - create it in target
                self.warnings.push(format!(
                    "Creating missing type '{}' in target",
                    src_name
                ));
                return self.create_obj_type(src_ref);
            }
            Type::Enum {
                name,
                global,
                constructs,
            } => {
                let src_name = self.source.get(*name);

                // Collect source construct info for matching
                let src_construct_info: Vec<(String, Vec<RefType>)> = constructs
                    .iter()
                    .map(|c| {
                        (
                            self.source.get(c.name).to_string(),
                            c.params.clone(),
                        )
                    })
                    .collect();

                // Try to find matching enum in target - must match structurally for anonymous enums
                for (i, t) in self.target.types.iter().enumerate() {
                    if let Type::Enum {
                        name: target_name,
                        constructs: target_constructs,
                        ..
                    } = t
                    {
                        if self.target.get(*target_name) == src_name {
                            // Check if constructs match structurally
                            // For anonymous enums (closures), we need exact structural match
                            // For named enums, we allow source to have more constructs (superset)
                            // and tolerate param type mismatches with warnings
                            let (matched, param_mismatches) = self.enum_constructs_match(&src_name, constructs, target_constructs);
                            if !matched {
                                continue; // Try next enum with same name
                            }
                            // Store param type mismatches if any (e.g. Dynamic vs concrete)
                            if !param_mismatches.is_empty() {
                                self.enum_mismatches.insert(src_ref.0, EnumMismatch {
                                    enum_name: src_name.to_string(),
                                    construct_param_mismatches: param_mismatches,
                                });
                            }

                            self.remap.types.insert(src_ref.0, i);
                            let target_type = RefType(i);

                            // Build construct name -> (index, param_count) map for target
                            let target_construct_info: HashMap<String, (usize, usize)> =
                                target_constructs
                                    .iter()
                                    .enumerate()
                                    .map(|(idx, c)| {
                                        (
                                            self.target.get(c.name).to_string(),
                                            (idx, c.params.len()),
                                        )
                                    })
                                    .collect();

                            // Save target construct count before mutable borrows
                            let original_target_construct_count = target_constructs.len();

                            // Build construct remap and find missing/extended constructs
                            let mut construct_map = HashMap::new();
                            let mut missing_constructs: Vec<(usize, String, Vec<RefType>)> =
                                Vec::new();
                            // Constructs that exist but need more params: (target_idx, src_params)
                            let mut extend_constructs: Vec<(usize, Vec<RefType>)> = Vec::new();

                            for (src_idx, (construct_name, src_params)) in
                                src_construct_info.iter().enumerate()
                            {
                                if let Some(&(target_idx, target_param_count)) =
                                    target_construct_info.get(construct_name)
                                {
                                    // Check param count compatibility
                                    if src_params.len() > target_param_count {
                                        // Source has more params - extend target construct
                                        extend_constructs
                                            .push((target_idx, src_params.clone()));
                                        // Record remap
                                        if src_idx != target_idx {
                                            construct_map.insert(src_idx, target_idx);
                                        }
                                    } else if src_params.len() < target_param_count {
                                        // Source has fewer params - can't use target construct
                                        // Need to inject source construct as new
                                        self.warnings.push(format!(
                                            "Enum '{}' construct '{}': source has {} params but target has {} - injecting as new construct",
                                            src_name, construct_name, src_params.len(), target_param_count
                                        ));
                                        missing_constructs.push((
                                            src_idx,
                                            construct_name.clone(),
                                            src_params.clone(),
                                        ));
                                    } else {
                                        // Same param count - just record remap if different
                                        if src_idx != target_idx {
                                            construct_map.insert(src_idx, target_idx);
                                        }
                                    }
                                } else {
                                    // Missing - need to inject
                                    missing_constructs.push((
                                        src_idx,
                                        construct_name.clone(),
                                        src_params.clone(),
                                    ));
                                }
                            }

                            // Extend construct params where needed
                            if !extend_constructs.is_empty() {
                                self.extend_enum_construct_params(
                                    target_type,
                                    src_ref,
                                    &extend_constructs,
                                );
                            }

                            // Inject missing constructs
                            if !missing_constructs.is_empty() {
                                self.inject_enum_constructs(
                                    target_type,
                                    src_ref,
                                    &missing_constructs,
                                    &mut construct_map,
                                );
                            }

                            // Store construct remap if any
                            if !construct_map.is_empty() {
                                self.remap
                                    .enum_type_constructs
                                    .insert(src_ref.0, construct_map);

                                // Store target construct count for Switch remapping
                                // After injection, target has original + injected constructs
                                let final_target_count =
                                    original_target_construct_count + missing_constructs.len();
                                self.remap
                                    .enum_type_target_counts
                                    .insert(src_ref.0, final_target_count);
                            }

                            return target_type;
                        }
                    }
                }

                // Not found - create the enum type in target
                self.warnings.push(format!(
                    "Creating missing enum '{}' in target",
                    src_name
                ));

                // Reserve index to prevent recursion
                let new_type_idx = self.target.types.len();
                self.remap.types.insert(src_ref.0, new_type_idx);
                self.target.types.push(Type::Void); // placeholder

                // Ensure name string exists
                let name_ref = RefString(self.ensure_string_value(src_name.as_ref()));

                // Ensure global type exists (if valid)
                let global_ref = if global.0 < self.source.globals.len() {
                    self.ensure_global(*global)
                } else {
                    RefGlobal(0)
                };

                // Remap enum constructs
                let new_constructs: Vec<hlbc::types::EnumConstruct> = constructs
                    .iter()
                    .map(|c| {
                        let construct_name =
                            RefString(self.ensure_string_value(self.source.get(c.name).as_ref()));
                        let params: Vec<RefType> =
                            c.params.iter().map(|&p| self.ensure_type(p)).collect();
                        hlbc::types::EnumConstruct {
                            name: construct_name,
                            params,
                        }
                    })
                    .collect();

                let new_type = Type::Enum {
                    name: name_ref,
                    global: global_ref,
                    constructs: new_constructs,
                };
                self.target.types[new_type_idx] = new_type;

                return RefType(new_type_idx);
            }
            Type::Abstract { name } => {
                let src_name = self.source.get(*name);
                for (i, t) in self.target.types.iter().enumerate() {
                    if let Type::Abstract { name: target_name } = t {
                        if self.target.get(*target_name) == src_name {
                            self.remap.types.insert(src_ref.0, i);
                            return RefType(i);
                        }
                    }
                }

                // Not found - create the abstract type in target
                self.warnings.push(format!(
                    "Creating missing abstract '{}' in target",
                    src_name
                ));
                let name_ref = RefString(self.ensure_string_value(src_name.as_ref()));
                let new_type = Type::Abstract { name: name_ref };
                let new_type_idx = self.target.types.len();
                self.target.types.push(new_type);
                self.remap.types.insert(src_ref.0, new_type_idx);
                return RefType(new_type_idx);
            }
            Type::Fun(fun) | Type::Method(fun) => {
                let is_method = matches!(src_type, Type::Method(_));

                // Function types are structural - try to find exact structural match
                for (i, t) in self.target.types.iter().enumerate() {
                    if self.types_structurally_equal(src_type, t) {
                        self.remap.types.insert(src_ref.0, i);
                        return RefType(i);
                    }
                }

                // Not found - create the function type in target
                // First collect arg types and return type
                let args: Vec<RefType> = fun.args.clone();
                let ret = fun.ret;

                // Reserve index to prevent recursion
                let new_type_idx = self.target.types.len();
                self.remap.types.insert(src_ref.0, new_type_idx);
                self.target.types.push(Type::Void); // placeholder

                // Now remap the args and return type
                let remapped_args: Vec<RefType> = args.iter().map(|&a| self.ensure_type(a)).collect();
                let remapped_ret = self.ensure_type(ret);

                // Create the new function type
                let new_fun = hlbc::types::TypeFun {
                    args: remapped_args,
                    ret: remapped_ret,
                };
                let new_type = if is_method {
                    Type::Method(new_fun)
                } else {
                    Type::Fun(new_fun)
                };
                self.target.types[new_type_idx] = new_type;

                return RefType(new_type_idx);
            }
            Type::Ref(inner) | Type::Null(inner) | Type::Packed(inner) => {
                // Wrapper types - need to match inner type first
                let remapped_inner = self.ensure_type(*inner);
                for (i, t) in self.target.types.iter().enumerate() {
                    match (src_type, t) {
                        (Type::Ref(_), Type::Ref(target_inner))
                        | (Type::Null(_), Type::Null(target_inner))
                        | (Type::Packed(_), Type::Packed(target_inner)) => {
                            if *target_inner == remapped_inner {
                                self.remap.types.insert(src_ref.0, i);
                                return RefType(i);
                            }
                        }
                        _ => {}
                    }
                }

                // Not found - create the wrapper type in target
                let new_type = match src_type {
                    Type::Ref(_) => Type::Ref(remapped_inner),
                    Type::Null(_) => Type::Null(remapped_inner),
                    Type::Packed(_) => Type::Packed(remapped_inner),
                    _ => unreachable!(),
                };
                let new_type_idx = self.target.types.len();
                self.target.types.push(new_type);
                self.remap.types.insert(src_ref.0, new_type_idx);
                return RefType(new_type_idx);
            }
            Type::Virtual { fields: src_fields } => {
                // Virtual types are structural - match by field names
                // First collect source field info
                let src_field_info: Vec<(String, RefType)> = src_fields
                    .iter()
                    .map(|f| (self.source.get(f.name).to_string(), f.t))
                    .collect();

                let src_field_names: HashSet<String> =
                    src_field_info.iter().map(|(n, _)| n.clone()).collect();

                // Try to find a matching Virtual in target - collect info without borrowing self
                let matching_virtual: Option<(usize, Vec<(String, usize)>)> = self
                    .target
                    .types
                    .iter()
                    .enumerate()
                    .find_map(|(i, t)| {
                        if let Type::Virtual { fields: target_fields } = t {
                            let target_field_info: Vec<(String, usize)> = target_fields
                                .iter()
                                .enumerate()
                                .map(|(idx, f)| (self.target.get(f.name).to_string(), idx))
                                .collect();

                            let target_field_names: HashSet<String> =
                                target_field_info.iter().map(|(n, _)| n.clone()).collect();

                            // If target has all source fields, we can use it
                            if src_field_names.is_subset(&target_field_names) {
                                return Some((i, target_field_info));
                            }
                        }
                        None
                    });

                if let Some((target_idx, target_field_info)) = matching_virtual {
                    self.remap.types.insert(src_ref.0, target_idx);

                    // Build field remap - map source field names to target indices
                    let target_field_indices: HashMap<String, usize> =
                        target_field_info.into_iter().collect();

                    let mut field_map = HashMap::new();
                    for (src_idx, (field_name, _)) in src_field_info.iter().enumerate() {
                        if let Some(&target_field_idx) = target_field_indices.get(field_name) {
                            if src_idx != target_field_idx {
                                field_map.insert(src_idx, target_field_idx);
                            }
                        }
                    }
                    if !field_map.is_empty() {
                        self.remap.type_fields.insert(src_ref.0, field_map);
                    }

                    return RefType(target_idx);
                }

                // No matching Virtual found - create a new one with all source fields
                // Reserve index BEFORE recursing to prevent infinite recursion on cyclic types
                let new_type_idx = self.target.types.len();
                self.remap.types.insert(src_ref.0, new_type_idx);
                self.target.types.push(Type::Void); // placeholder

                let new_fields: Vec<ObjField> = src_field_info
                    .iter()
                    .map(|(name, src_type)| {
                        let name_idx = self.ensure_string_value(name);
                        let type_ref = self.ensure_type(*src_type);
                        ObjField {
                            name: RefString(name_idx),
                            t: type_ref,
                        }
                    })
                    .collect();

                self.target.types[new_type_idx] = Type::Virtual { fields: new_fields };
                // No field remap needed - indices will match
                return RefType(new_type_idx);
            }
            Type::DynObj => {
                // DynObj is a singleton type - find it in target
                for (i, t) in self.target.types.iter().enumerate() {
                    if matches!(t, Type::DynObj) {
                        self.remap.types.insert(src_ref.0, i);
                        return RefType(i);
                    }
                }
                // If not found, create one (shouldn't happen)
                let new_type_idx = self.target.types.len();
                self.target.types.push(Type::DynObj);
                self.remap.types.insert(src_ref.0, new_type_idx);
                return RefType(new_type_idx);
            }
            Type::Array => {
                // Array is also a singleton type
                for (i, t) in self.target.types.iter().enumerate() {
                    if matches!(t, Type::Array) {
                        self.remap.types.insert(src_ref.0, i);
                        return RefType(i);
                    }
                }
                let new_type_idx = self.target.types.len();
                self.target.types.push(Type::Array);
                self.remap.types.insert(src_ref.0, new_type_idx);
                return RefType(new_type_idx);
            }
            Type::Type => {
                // Type is also a singleton type
                for (i, t) in self.target.types.iter().enumerate() {
                    if matches!(t, Type::Type) {
                        self.remap.types.insert(src_ref.0, i);
                        return RefType(i);
                    }
                }
                let new_type_idx = self.target.types.len();
                self.target.types.push(Type::Type);
                self.remap.types.insert(src_ref.0, new_type_idx);
                return RefType(new_type_idx);
            }
            Type::Dyn => {
                // Dyn is also a singleton type
                for (i, t) in self.target.types.iter().enumerate() {
                    if matches!(t, Type::Dyn) {
                        self.remap.types.insert(src_ref.0, i);
                        return RefType(i);
                    }
                }
                let new_type_idx = self.target.types.len();
                self.target.types.push(Type::Dyn);
                self.remap.types.insert(src_ref.0, new_type_idx);
                return RefType(new_type_idx);
            }
            Type::Bytes => {
                // Bytes is also a singleton type
                for (i, t) in self.target.types.iter().enumerate() {
                    if matches!(t, Type::Bytes) {
                        self.remap.types.insert(src_ref.0, i);
                        return RefType(i);
                    }
                }
                let new_type_idx = self.target.types.len();
                self.target.types.push(Type::Bytes);
                self.remap.types.insert(src_ref.0, new_type_idx);
                return RefType(new_type_idx);
            }
            _ => {
                // Other primitive-like types should already be handled by is_known()
            }
        }

        // Return original if not found (will likely cause issues at runtime)
        self.warnings.push(format!(
            "Type {} not mapped (variant: {:?})",
            src_ref.0,
            std::mem::discriminant(src_type)
        ));
        src_ref
    }

    /// Check if a type is the String class (an Obj type with name "String")
    fn is_string_type(&self, code: &Bytecode, type_ref: RefType) -> bool {
        match code.get(type_ref) {
            Type::Obj(obj) | Type::Struct(obj) => {
                // Use direct array access instead of code.get() since get() may have
                // special handling for certain string indices
                code.strings[obj.name.0].as_ref() == "String"
            }
            _ => false,
        }
    }

    /// Find the String type index in target bytecode
    fn find_string_type(&self) -> Option<RefType> {
        for (i, t) in self.target.types.iter().enumerate() {
            if let Type::Obj(obj) = t {
                if self.target.strings[obj.name.0].as_ref() == "String" {
                    return Some(RefType(i));
                }
            }
        }
        None
    }

    /// Ensure a string value exists in target pool, return the index
    fn ensure_string_value(&mut self, value: &str) -> usize {
        // Check if string already exists
        if let Some(idx) = self.target.strings.iter().position(|s| s.as_ref() == value) {
            return idx;
        }
        // Add new string
        self.target.strings.push(value.into());
        self.target.strings.len() - 1
    }

    /// Ensure an int value exists in target pool, return the index
    fn ensure_int_value(&mut self, value: i32) -> usize {
        // Check if int already exists
        if let Some(idx) = self.target.ints.iter().position(|&v| v == value) {
            return idx;
        }
        // Add new int
        self.target.ints.push(value);
        self.target.ints.len() - 1
    }

    /// Create a new String global initialized with the given value
    fn create_string_global(&mut self, src_ref: RefGlobal, value: &str) -> RefGlobal {
        // 1. Ensure string exists in target string pool
        let string_idx = self.ensure_string_value(value);

        // 2. String type has 2 fields: bytes (string data) and length (i32)
        //    We need to add the length to the ints pool
        let length_idx = self.ensure_int_value(value.len() as i32);

        // 3. Find the String type index in target
        let string_type = self.find_string_type().expect("String type must exist in target");

        // 4. Add new global of String type
        self.target.globals.push(string_type);
        let new_global = RefGlobal(self.target.globals.len() - 1);

        // 5. Create ConstantDef to initialize the global
        //    fields[0] = string pool index (for 'bytes' field)
        //    fields[1] = ints pool index (for 'length' field)
        let constant_def = ConstantDef {
            global: new_global,
            fields: vec![string_idx, length_idx],
        };

        // 6. Add to constants pool and update accelerator
        if let Some(constants) = &mut self.target.constants {
            let const_idx = constants.len();
            constants.push(constant_def);
            self.target.globals_initializers.insert(new_global, const_idx);
        }

        // 7. Record the remap
        self.remap.globals.insert(src_ref.0, new_global.0);

        // Note: String globals have ConstantDefs, so they don't need entry point init.
        // We don't add them to injected_globals.

        new_global
    }

    /// Ensure a global exists in target, return the remapped RefGlobal
    pub fn ensure_global(&mut self, src_ref: RefGlobal) -> RefGlobal {
        if let Some(&target_idx) = self.remap.globals.get(&src_ref.0) {
            return RefGlobal(target_idx);
        }

        let src_type = self.source.globals[src_ref.0];

        // Special handling for String-type globals - match by initialized value
        if self.is_string_type(self.source, src_type) {
            if let Some(src_str_value) = get_global_string_value(self.source, src_ref) {
                // Try to find a target global with the same string value
                for (i, &target_type) in self.target.globals.iter().enumerate() {
                    if self.is_string_type(self.target, target_type) {
                        if let Some(target_str_value) =
                            get_global_string_value(self.target, RefGlobal(i))
                        {
                            if src_str_value == target_str_value {
                                self.remap.globals.insert(src_ref.0, i);
                                return RefGlobal(i);
                            }
                        }
                    }
                }
                // Not found - create a new global with the string value
                return self.create_string_global(src_ref, src_str_value);
            }
        }

        // Special handling for Enum-type globals - match by construct NAME
        if let Type::Enum { name, .. } = self.source.get(src_type) {
            let enum_name = self.source.get(*name).to_string();
            if let Some(src_construct) = get_global_enum_construct(self.source, src_ref, src_type, &self.source_enum_constructs) {
                let remapped_type = self.ensure_type(src_type);

                // Get the source construct NAME - this is stable across compilations
                if let Some(src_construct_name) =
                    get_enum_construct_name(self.source, src_type, src_construct)
                {
                    // Find the target construct index with the same NAME
                    if let Some(target_construct_idx) =
                        find_construct_index_by_name(self.target, remapped_type, &src_construct_name)
                    {
                        // Now find target global with this construct index
                        for (i, &target_type) in self.target.globals.iter().enumerate() {
                            if target_type == remapped_type {
                                if let Some(actual_target_construct) =
                                    get_global_enum_construct(self.target, RefGlobal(i), remapped_type, &self.target_enum_constructs)
                                {
                                    if actual_target_construct == target_construct_idx {
                                        self.remap.globals.insert(src_ref.0, i);
                                        return RefGlobal(i);
                                    }
                                }
                            }
                        }
                    }
                }

                // Not found with matching construct - warn and try fallback
                self.warnings.push(format!(
                    "Enum global {} ({} construct {}) not found in target",
                    src_ref.0, enum_name, src_construct
                ));
                // Not found in target - create a new global with the enum value
                // This is unusual for enums since they should exist in both
                let new_global_idx = self.target.globals.len();
                self.target.globals.push(remapped_type);
                self.remap.globals.insert(src_ref.0, new_global_idx);

                // Track this as an injected global (needs initialization code)
                self.injected_globals.insert(src_ref.0, new_global_idx);

                // Copy the constant definition to initialize the enum
                self.copy_global_constant(src_ref, RefGlobal(new_global_idx));

                let type_name = match self.target.get(remapped_type) {
                    Type::Enum { name, .. } => self.target.get(*name).to_string(),
                    _ => format!("type@{}", remapped_type.0),
                };
                self.warnings.push(format!(
                    "Created enum global {} for type '{}' construct {} (was source global {})",
                    new_global_idx, type_name, src_construct, src_ref.0
                ));

                return RefGlobal(new_global_idx);
            }
        }

        // Non-String/Enum globals: match by type (original behavior)
        let remapped_type = self.ensure_type(src_type);

        // Try to find a matching global in target with the same type
        // This is imprecise - there could be multiple globals of the same type
        // A better approach would be to match by associated class name if it's a static class
        for (i, &target_type) in self.target.globals.iter().enumerate() {
            if target_type == remapped_type {
                // Check if this global is already mapped
                if !self.remap.globals.values().any(|&v| v == i) {
                    self.remap.globals.insert(src_ref.0, i);
                    return RefGlobal(i);
                }
            }
        }

        // Global not found - create a new one with the remapped type
        let new_global_idx = self.target.globals.len();
        self.target.globals.push(remapped_type);
        self.remap.globals.insert(src_ref.0, new_global_idx);

        // Track this as an injected global (needs initialization code)
        self.injected_globals.insert(src_ref.0, new_global_idx);

        // Get type name for logging
        let type_name = match self.target.get(remapped_type) {
            Type::Obj(obj) | Type::Struct(obj) => self.target.get(obj.name).to_string(),
            Type::Enum { name, .. } => self.target.get(*name).to_string(),
            Type::Abstract { name } => self.target.get(*name).to_string(),
            _ => format!("type@{}", remapped_type.0),
        };
        self.warnings.push(format!(
            "Created global {} for type '{}' (was source global {})",
            new_global_idx, type_name, src_ref.0
        ));

        RefGlobal(new_global_idx)
    }

    /// Copy a ConstantDef from source to target for the given global.
    /// Returns true if a constant was copied.
    fn copy_global_constant(&mut self, src_global: RefGlobal, target_global: RefGlobal) -> bool {
        let src_const_idx = match self.source.globals_initializers.get(&src_global) {
            Some(&idx) => idx,
            None => return false,
        };

        let src_const = match self.source.constants.as_ref().and_then(|c| c.get(src_const_idx)) {
            Some(c) => c,
            None => return false,
        };

        // Remap the field values - each field[i] is a pool index
        // whose interpretation depends on the i-th field's type
        let remapped_fields = self.remap_constant_fields(src_global, &src_const.fields);

        let new_const = ConstantDef {
            global: target_global,
            fields: remapped_fields,
        };

        let constants = self.target.constants.get_or_insert_with(Vec::new);
        let new_const_idx = constants.len();
        constants.push(new_const);
        self.target.globals_initializers.insert(target_global, new_const_idx);

        true
    }

    /// Remap ConstantDef field values based on field types.
    fn remap_constant_fields(&mut self, src_global: RefGlobal, src_fields: &[usize]) -> Vec<usize> {
        let src_type = self.source.globals[src_global.0];

        // Get field types from the object
        let field_types: Vec<RefType> = match self.source.get(src_type) {
            Type::Obj(obj) | Type::Struct(obj) => obj.fields.iter().map(|f| f.t).collect(),
            Type::Enum { .. } => return src_fields.to_vec(), // Enum construct index, no remap
            _ => return src_fields.to_vec(),
        };

        src_fields
            .iter()
            .enumerate()
            .map(|(i, &value)| {
                if i >= field_types.len() {
                    return value;
                }
                match self.source.get(field_types[i]) {
                    Type::I32 | Type::I64 => self.ensure_int(RefInt(value)).0,
                    Type::F32 | Type::F64 => self.ensure_float(RefFloat(value)).0,
                    Type::Bytes => self.ensure_string(RefString(value)).0,
                    _ => value, // Other types: return as-is for now
                }
            })
            .collect()
    }

    /// Ensure a function reference exists in target, return the remapped RefFun
    pub fn ensure_fun(&mut self, src_ref: RefFun) -> RefFun {
        if let Some(&target_idx) = self.remap.funs.get(&src_ref.0) {
            return RefFun(target_idx);
        }

        // Try to match by qualified name using the function's parent type and name
        let src_fun_ptr = self.source.get(src_ref);

        match src_fun_ptr {
            hlbc::types::FunPtr::Fun(src_func) => {
                // Get source function's qualified name
                let src_name = self.source.get(src_func.name).to_string();
                let src_parent_name = src_func.parent.map(|p| {
                    self.source.get(p).get_type_obj()
                        .map(|obj| self.source.get(obj.name).to_string())
                }).flatten();

                // Anonymous closures (name is empty or "<none>" and no parent) should always be injected
                // because they can't be reliably matched - there may be many with the same signature
                let is_anonymous_closure =
                    (src_name.is_empty() || src_name == "<none>") && src_parent_name.is_none();

                if !is_anonymous_closure {
                    // Search target functions for a match
                    let mut name_parent_match: Option<RefFun> = None;
                    for target_func in &self.target.functions {
                        let target_name = self.target.get(target_func.name).to_string();
                        let target_parent_name = target_func.parent.map(|p| {
                            self.target.get(p).get_type_obj()
                                .map(|obj| self.target.get(obj.name).to_string())
                        }).flatten();

                        if src_name == target_name && src_parent_name == target_parent_name {
                            // Also verify signature matches to avoid collisions when
                            // multiple functions share the same name (e.g., "String" wrapper functions)
                            if self.signatures_match(src_func, target_func) {
                                self.remap.funs.insert(src_ref.0, target_func.findex.0);
                                return target_func.findex;
                            }
                            // For class methods (both have parents), record as fallback.
                            // Parentless functions (e.g., "String" wrappers) can collide
                            // by name alone, so we only relax matching for class methods.
                            if src_parent_name.is_some() && target_parent_name.is_some() {
                                name_parent_match = Some(target_func.findex);
                            }
                        }
                    }

                    // If no exact match found but name+parent matched, prefer the
                    // target's existing function. Source stubs often have simplified
                    // signatures that don't match the real target implementation.
                    if let Some(target_findex) = name_parent_match {
                        self.warnings.push(format!(
                            "Function '{}.{}': matched by name+parent but signatures differ. \
                             Using target's version (source has simplified stubs).",
                            src_parent_name.as_deref().unwrap_or("?"), src_name
                        ));
                        self.remap.funs.insert(src_ref.0, target_findex.0);
                        return target_findex;
                    }

                    // Check stdlib remaps - function might have been renamed between Haxe versions
                    if let Some(parent) = &src_parent_name {
                        for &(remap_parent, remap_src, remap_target) in STDLIB_FUNCTION_REMAPS {
                            if parent == remap_parent && src_name == remap_src {
                                // Look for the remapped function name in target
                                for target_func in &self.target.functions {
                                    let target_name = self.target.get(target_func.name).to_string();
                                    let target_parent_name = target_func.parent.map(|p| {
                                        self.target.get(p).get_type_obj()
                                            .map(|obj| self.target.get(obj.name).to_string())
                                    }).flatten();

                                    if target_name == remap_target && target_parent_name.as_deref() == Some(remap_parent) {
                                        // Also verify signature matches for stdlib remaps
                                        if self.signatures_match(src_func, target_func) {
                                            // Found the remapped function - use it instead
                                            self.remap.funs.insert(src_ref.0, target_func.findex.0);
                                            self.warnings.push(format!(
                                                "Stdlib remap: {}.{} -> {}.{} (Haxe version compatibility)",
                                                parent, src_name, remap_parent, remap_target
                                            ));
                                            return target_func.findex;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Function not found (or anonymous closure) - inject if enabled
                if self.inject_missing_functions {
                    return self.inject_function(src_ref, &src_name, src_parent_name.as_deref());
                } else {
                    self.warnings.push(format!(
                        "Function '{}' (parent: {:?}) from source not found in target",
                        src_name, src_parent_name
                    ));
                }
            }
            hlbc::types::FunPtr::Native(src_native) => {
                // Match native by lib + name
                let src_name = self.source.get(src_native.name).to_string();
                let src_lib = self.source.get(src_native.lib).to_string();

                for target_native in &self.target.natives {
                    let target_name = self.target.get(target_native.name).to_string();
                    let target_lib = self.target.get(target_native.lib).to_string();

                    if src_name == target_name && src_lib == target_lib {
                        self.remap.funs.insert(src_ref.0, target_native.findex.0);
                        return target_native.findex;
                    }
                }

                // Native not found - try to inject if enabled
                let qualified_name = format!("{}@{}", src_name, src_lib);

                if self.inject_missing_natives {
                    // Inject the native declaration into target
                    return self.inject_native(src_ref, &src_name, &src_lib, src_native.t);
                } else {
                    // Native injection disabled
                    self.unresolvable_natives.push(qualified_name.clone());
                    self.warnings.push(format!(
                        "Native '{}' from source not found in target (native injection disabled)",
                        qualified_name
                    ));
                }
            }
        }

        src_ref
    }

    /// Compare function signatures between source and target functions.
    /// Returns true if the signatures match structurally (same parameter types and return type).
    fn signatures_match(&self, src_func: &hlbc::types::Function, target_func: &hlbc::types::Function) -> bool {
        let src_sig = format_type(self.source, src_func.t);
        let target_sig = format_type(self.target, target_func.t);
        src_sig == target_sig
    }

    /// Check if a type name indicates a stdlib/runtime type that should not be injected
    /// across different Haxe versions.
    ///
    /// Note: The `$` prefix is added by the Haxe compiler to ALL class types, so we can't
    /// just check for `$` prefix. Instead we check for known stdlib type names.
    fn is_stdlib_type(name: &str) -> bool {
        // Strip the $ prefix if present for matching
        let base_name = name.strip_prefix('$').unwrap_or(name);

        // haxe.* namespace is stdlib
        if base_name.starts_with("haxe.") {
            return true;
        }

        // hl.* namespace is stdlib (HashLink runtime types)
        if base_name.starts_with("hl.") {
            return true;
        }

        // Known stdlib types that vary between Haxe versions
        // These are the types where field layouts or method signatures change
        matches!(base_name,
            "Std" | "Sys" | "String" | "Array" | "Bytes" | "EReg" | "Date" | "Xml"
            | "Math" | "Reflect" | "Type" | "StringBuf" | "StringTools"
            | "Lambda" | "IntIterator" | "DateTools" | "SysError"
        )
    }

    /// Check if a stdlib type exists in target and has compatible structure
    /// Returns None if compatible or not a stdlib type, Some(reason) if incompatible
    fn check_stdlib_compatibility(&self, parent_name: &str) -> Option<String> {
        if !Self::is_stdlib_type(parent_name) {
            return None;
        }

        // Find the source type
        let src_type_idx = self.source.types.iter().enumerate().find_map(|(i, t)| {
            if let Some(obj) = t.get_type_obj() {
                if self.source.get(obj.name) == parent_name {
                    return Some(i);
                }
            }
            None
        });

        // Find the target type with the same name
        let target_type_idx = self.target.types.iter().enumerate().find_map(|(i, t)| {
            if let Some(obj) = t.get_type_obj() {
                if self.target.get(obj.name) == parent_name {
                    return Some(i);
                }
            }
            None
        });

        match (src_type_idx, target_type_idx) {
            (Some(src_idx), Some(target_idx)) => {
                let src_type = &self.source.types[src_idx];
                let target_type = &self.target.types[target_idx];

                // Get field/method names from both
                let src_fields: HashSet<String> = match src_type.get_type_obj() {
                    Some(obj) => obj.fields.iter()
                        .map(|f| self.source.get(f.name).to_string())
                        .collect(),
                    None => HashSet::new(),
                };

                let target_fields: HashSet<String> = match target_type.get_type_obj() {
                    Some(obj) => obj.fields.iter()
                        .map(|f| self.target.get(f.name).to_string())
                        .collect(),
                    None => HashSet::new(),
                };

                // Check if source has fields that target doesn't
                let missing_in_target: Vec<_> = src_fields.difference(&target_fields).collect();
                let extra_in_target: Vec<_> = target_fields.difference(&src_fields).collect();

                if !missing_in_target.is_empty() || !extra_in_target.is_empty() {
                    let mut reason = format!(
                        "stdlib type '{}' has incompatible layout (source: {} fields, target: {} fields)",
                        parent_name, src_fields.len(), target_fields.len()
                    );
                    if !missing_in_target.is_empty() {
                        reason.push_str(&format!(
                            ". Source has: {}",
                            missing_in_target.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                        ));
                    }
                    if !extra_in_target.is_empty() {
                        reason.push_str(&format!(
                            ". Target has: {}",
                            extra_in_target.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                        ));
                    }
                    reason.push_str(". Recompile source with matching Haxe version.");
                    return Some(reason);
                }

                None // Compatible
            }
            (Some(_), None) => {
                // Source has the stdlib type but target doesn't - can't inject
                Some(format!(
                    "stdlib type '{}' exists in source but not in target. \
                     This indicates a Haxe version mismatch.",
                    parent_name
                ))
            }
            _ => None, // No source type or no parent, allow injection
        }
    }

    /// Inject a function from source into target
    /// This is called when a function is not found in target and injection is enabled
    fn inject_function(
        &mut self,
        src_ref: RefFun,
        func_name: &str,
        parent_name: Option<&str>,
    ) -> RefFun {
        // Check if this is a stdlib function with incompatible types
        if let Some(parent) = parent_name {
            if let Some(reason) = self.check_stdlib_compatibility(parent) {
                let qualified_name = format!("{}.{}", parent, func_name);
                self.stdlib_mismatches.push((qualified_name.clone(), reason));
                // Return original ref - can't inject, but don't crash
                // The error will be reported and substitution will fail
                return src_ref;
            }
        }

        // Get the source function (we already know it's a Fun, not Native)
        let src_func = match self.source.get(src_ref) {
            hlbc::types::FunPtr::Fun(f) => f,
            hlbc::types::FunPtr::Native(_) => unreachable!("inject_function called with native"),
        };

        // Allocate new findex BEFORE doing anything else
        // This is the global index used by RefFun
        // Note: can't use findex_max() because it uses findexes.len() which doesn't update
        // when we push functions. Instead, calculate based on actual vectors.
        let new_findex = RefFun(self.target.functions.len() + self.target.natives.len());

        // Record remap FIRST to prevent infinite recursion
        // (function might reference itself or be part of a cycle)
        self.remap.funs.insert(src_ref.0, new_findex.0);

        // IMPORTANT: Push a placeholder function BEFORE ensuring dependencies!
        // Dependencies may recursively inject more functions, and they need to see
        // this function's slot already taken so they get correct findexes.
        // We'll replace the placeholder with the real function at the end.
        let placeholder_idx = self.target.functions.len();
        self.target.functions.push(Function {
            t: RefType(0),
            findex: new_findex,
            regs: vec![],
            ops: vec![],
            debug_info: None,
            assigns: None,
            name: RefString(0),
            parent: None,
        });

        // Collect source data before modifying target
        let src_type = src_func.t;
        let src_regs: Vec<RefType> = src_func.regs.clone();
        let src_ops: Vec<Opcode> = src_func.ops.clone();
        let src_assigns = src_func.assigns.clone();
        let src_debug_info = src_func.debug_info.clone();
        let src_name_ref = src_func.name;
        let src_parent = src_func.parent;

        // Ensure all dependencies exist (may recursively inject more functions)
        // 1. Function type
        let new_type = self.ensure_type(src_type);

        // 2. Register types
        let new_regs: Vec<RefType> = src_regs.iter().map(|&r| self.ensure_type(r)).collect();

        // 3. Scan opcodes for references (this may recursively inject functions)
        for op in &src_ops {
            self.scan_opcode_refs_for_injection(op);
        }

        // 4. Parent type (if any)
        let new_parent = src_parent.map(|p| self.ensure_type(p));

        // 5. Function name string
        let new_name = RefString(self.ensure_string_value(self.source.get(src_name_ref).as_ref()));

        // 6. Assigns (variable names)
        let new_assigns = src_assigns.map(|assigns| {
            assigns
                .iter()
                .map(|(s, p)| {
                    let new_str = RefString(self.ensure_string_value(self.source.get(*s).as_ref()));
                    (new_str, *p)
                })
                .collect()
        });

        // 7. Remap opcodes (now that all dependencies are ensured)
        let new_ops: Vec<Opcode> = src_ops
            .iter()
            .map(|op| self.remap.remap_opcode_with_regs(op, &src_regs))
            .collect();

        // 8. Remap debug info: preserve source file/line with ".substituted" suffix
        let new_debug_info = src_debug_info.map(|di| self.remap_function_debug_info(&di));

        // Create the new function
        let new_func = Function {
            t: new_type,
            findex: new_findex,
            regs: new_regs,
            ops: new_ops,
            debug_info: new_debug_info,
            assigns: new_assigns,
            name: new_name,
            parent: new_parent,
        };

        // Replace the placeholder with the real function
        self.target.functions[placeholder_idx] = new_func;

        // Log the injection
        let qualified_name = match parent_name {
            Some(p) => format!("{}.{}", p, func_name),
            None => func_name.to_string(),
        };
        self.injected_functions.push(qualified_name.clone());

        new_findex
    }

    /// Inject a native function declaration into target bytecode
    /// This adds the native to the target's native table so it can be resolved at runtime
    fn inject_native(
        &mut self,
        src_ref: RefFun,
        native_name: &str,
        native_lib: &str,
        src_type: RefType,
    ) -> RefFun {
        // Allocate new findex - natives come after all functions
        // findex = functions.len() + natives.len()
        let new_findex = RefFun(self.target.functions.len() + self.target.natives.len());

        // Record remap FIRST to prevent issues with recursive type resolution
        self.remap.funs.insert(src_ref.0, new_findex.0);

        // Ensure the native's function type exists in target
        let new_type = self.ensure_type(src_type);

        // Ensure name and lib strings exist in target
        let new_name = RefString(self.ensure_string_value(native_name));
        let new_lib = RefString(self.ensure_string_value(native_lib));

        // Create the new native declaration
        let new_native = hlbc::types::Native {
            name: new_name,
            lib: new_lib,
            t: new_type,
            findex: new_findex,
        };

        // Add to target's native table
        self.target.natives.push(new_native);

        // Log the injection
        let qualified_name = format!("{}@{}", native_name, native_lib);
        self.injected_natives.push(qualified_name);

        new_findex
    }

    /// Scan an opcode for pool references and ensure they exist
    /// This is used during function injection to recursively ensure dependencies
    fn scan_opcode_refs_for_injection(&mut self, op: &Opcode) {
        match op {
            // Constant pool references
            Opcode::Int { ptr, .. } => {
                self.ensure_int(*ptr);
            }
            Opcode::Float { ptr, .. } => {
                self.ensure_float(*ptr);
            }
            Opcode::Bytes { ptr, .. } => {
                self.ensure_bytes(*ptr);
            }
            Opcode::String { ptr, .. } => {
                self.ensure_string(*ptr);
            }

            // Function references - may recursively inject more functions
            Opcode::Call0 { fun, .. }
            | Opcode::Call1 { fun, .. }
            | Opcode::Call2 { fun, .. }
            | Opcode::Call3 { fun, .. }
            | Opcode::Call4 { fun, .. }
            | Opcode::CallN { fun, .. }
            | Opcode::StaticClosure { fun, .. }
            | Opcode::InstanceClosure { fun, .. } => {
                self.ensure_fun(*fun);
            }

            // Global references
            Opcode::GetGlobal { global, .. } | Opcode::SetGlobal { global, .. } => {
                self.ensure_global(*global);
            }

            // Type references
            Opcode::Type { ty, .. } => {
                self.ensure_type(*ty);
            }

            // Dynamic field access uses strings
            Opcode::DynGet { field, .. } | Opcode::DynSet { field, .. } => {
                self.ensure_string(*field);
            }

            // Enum construction - construct reference is relative to enum type
            Opcode::MakeEnum { .. } | Opcode::EnumAlloc { .. } | Opcode::EnumField { .. } => {
                // These use RefEnumConstruct which is relative to the enum type
                // The type should already be ensured elsewhere
            }

            // All other opcodes don't reference global pools
            _ => {}
        }
    }

    /// Check if two types are structurally equal (for function types)
    fn types_structurally_equal(&self, src: &Type, target: &Type) -> bool {
        match (src, target) {
            (Type::Fun(src_fun), Type::Fun(target_fun))
            | (Type::Method(src_fun), Type::Method(target_fun)) => {
                if src_fun.args.len() != target_fun.args.len() {
                    return false;
                }
                // Check return type
                let src_ret = self.source.get(src_fun.ret);
                let target_ret = self.target.get(target_fun.ret);
                if !self.types_match(src_ret, target_ret) {
                    return false;
                }
                // Check all args
                for (src_arg, target_arg) in src_fun.args.iter().zip(target_fun.args.iter()) {
                    let src_arg_type = self.source.get(*src_arg);
                    let target_arg_type = self.target.get(*target_arg);
                    if !self.types_match(src_arg_type, target_arg_type) {
                        return false;
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Check if two types match (by name for objects, structurally for primitives)
    fn types_match(&self, src: &Type, target: &Type) -> bool {
        match (src, target) {
            // Primitive types
            (Type::Void, Type::Void)
            | (Type::UI8, Type::UI8)
            | (Type::UI16, Type::UI16)
            | (Type::I32, Type::I32)
            | (Type::I64, Type::I64)
            | (Type::F32, Type::F32)
            | (Type::F64, Type::F64)
            | (Type::Bool, Type::Bool)
            | (Type::Bytes, Type::Bytes)
            | (Type::Dyn, Type::Dyn)
            | (Type::Array, Type::Array)
            | (Type::Type, Type::Type)
            | (Type::DynObj, Type::DynObj) => true,

            // Named types - match by name
            (Type::Obj(src_obj), Type::Obj(target_obj))
            | (Type::Struct(src_obj), Type::Struct(target_obj)) => {
                self.source.get(src_obj.name) == self.target.get(target_obj.name)
            }
            (Type::Enum { name: src_name, .. }, Type::Enum { name: target_name, .. }) => {
                self.source.get(*src_name) == self.target.get(*target_name)
            }
            (Type::Abstract { name: src_name }, Type::Abstract { name: target_name }) => {
                self.source.get(*src_name) == self.target.get(*target_name)
            }

            // Wrapper types - recurse
            (Type::Ref(src_inner), Type::Ref(target_inner))
            | (Type::Null(src_inner), Type::Null(target_inner))
            | (Type::Packed(src_inner), Type::Packed(target_inner)) => {
                self.types_match(self.source.get(*src_inner), self.target.get(*target_inner))
            }

            // Function types
            (Type::Fun(_), Type::Fun(_)) | (Type::Method(_), Type::Method(_)) => {
                self.types_structurally_equal(src, target)
            }

            // Virtual types - match by field names only (to avoid infinite recursion)
            // Haxe virtuals are structurally typed by field names
            (Type::Virtual { fields: src_fields }, Type::Virtual { fields: target_fields }) => {
                // Must have same number of fields
                if src_fields.len() != target_fields.len() {
                    return false;
                }
                // Each field must match by name
                for (src_f, target_f) in src_fields.iter().zip(target_fields.iter()) {
                    if self.source.get(src_f.name) != self.target.get(target_f.name) {
                        return false;
                    }
                }
                true
            }

            _ => false,
        }
    }

    /// Check if enum constructs match structurally
    /// This is used to find the correct enum type when there are multiple with the same name
    ///
    /// For anonymous enums (closures), requires exact structural match.
    /// For named enums, allows source to have MORE constructs than target (superset matching),
    /// and tolerates param type mismatches (e.g. Dynamic vs concrete) with warnings.
    fn enum_constructs_match(
        &self,
        enum_name: &str,
        src_constructs: &[hlbc::types::EnumConstruct],
        target_constructs: &[hlbc::types::EnumConstruct],
    ) -> (bool, Vec<EnumConstructParamMismatch>) {
        // Anonymous enums (closures) need exact structural matching
        if enum_name == "<none>" {
            return (self.enum_constructs_match_exact(src_constructs, target_constructs), Vec::new());
        }

        // Named enums use relaxed matching: target constructs must exist in source
        self.enum_constructs_match_relaxed(src_constructs, target_constructs)
    }

    /// Exact structural matching for anonymous enums (closures)
    fn enum_constructs_match_exact(
        &self,
        src_constructs: &[hlbc::types::EnumConstruct],
        target_constructs: &[hlbc::types::EnumConstruct],
    ) -> bool {
        // Must have same number of constructs
        if src_constructs.len() != target_constructs.len() {
            return false;
        }

        // Each construct must match by name and param types
        for (src_c, target_c) in src_constructs.iter().zip(target_constructs.iter()) {
            // Names must match
            if self.source.get(src_c.name) != self.target.get(target_c.name) {
                return false;
            }

            // Param counts must match
            if src_c.params.len() != target_c.params.len() {
                return false;
            }

            // Param types must match (by name for named types, structurally for others)
            for (src_param, target_param) in src_c.params.iter().zip(target_c.params.iter()) {
                let src_type = self.source.get(*src_param);
                let target_type = self.target.get(*target_param);
                if !self.types_match(src_type, target_type) {
                    return false;
                }
            }
        }

        true
    }

    /// Relaxed matching for named enums: all TARGET constructs must exist in source
    /// Source can have additional constructs that will be injected into target.
    /// Param type mismatches are recorded as warnings but don't prevent matching —
    /// a named enum where all construct names match is unambiguously the right type.
    fn enum_constructs_match_relaxed(
        &self,
        src_constructs: &[hlbc::types::EnumConstruct],
        target_constructs: &[hlbc::types::EnumConstruct],
    ) -> (bool, Vec<EnumConstructParamMismatch>) {
        let mut mismatches = Vec::new();

        // For each target construct, find a matching source construct by name
        for target_c in target_constructs {
            let target_name = self.target.get(target_c.name);

            // Find source construct with same name
            let src_match = src_constructs.iter().find(|src_c| {
                self.source.get(src_c.name) == target_name
            });

            match src_match {
                None => {
                    // Target construct doesn't exist in source - can't match
                    return (false, mismatches);
                }
                Some(src_c) => {
                    // Check param types match for the minimum common count
                    // Source can have MORE params (will be extended), or EQUAL
                    let min_params = src_c.params.len().min(target_c.params.len());
                    for i in 0..min_params {
                        let src_type = self.source.get(src_c.params[i]);
                        let target_type = self.target.get(target_c.params[i]);
                        if !self.types_match(src_type, target_type) {
                            mismatches.push(EnumConstructParamMismatch {
                                construct_name: target_name.to_string(),
                                param_index: i,
                                source_type: format_type(self.source, src_c.params[i]),
                                target_type: format_type(self.target, target_c.params[i]),
                            });
                        }
                    }
                }
            }
        }

        (true, mismatches)
    }

    /// Build a map of target findex -> pindex for virtual methods.
    /// This allows Call[1-N] opcodes to be converted to CallMethod when the
    /// target function is a virtual method (has a proto entry with pindex >= 0).
    fn build_virtual_method_map(&mut self) {
        for ty in &self.target.types {
            let protos = match ty {
                Type::Obj(obj) => &obj.protos,
                Type::Struct(obj) => &obj.protos,
                _ => continue,
            };
            for proto in protos {
                if proto.pindex >= 0 {
                    self.remap
                        .virtual_funs
                        .insert(proto.findex.0, proto.pindex as usize);
                }
            }
        }
    }

    /// Build a field index remap for a type
    /// Maps source field indices to target field indices by matching field names
    fn build_field_remap(&mut self, src_type: RefType, target_type: RefType) {
        let src_obj = match self.source.get(src_type) {
            Type::Obj(obj) | Type::Struct(obj) => obj,
            _ => return,
        };
        let target_obj = match self.target.get(target_type) {
            Type::Obj(obj) | Type::Struct(obj) => obj,
            _ => return,
        };

        let mut field_map = HashMap::new();

        // Build name→index lookup for target fields (flattened)
        let target_field_indices: HashMap<String, usize> = target_obj
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| (self.target.get(f.name).to_string(), i))
            .collect();

        // Map each source field to target by name
        for (src_idx, src_field) in src_obj.fields.iter().enumerate() {
            let field_name = self.source.get(src_field.name).to_string();
            if let Some(&target_idx) = target_field_indices.get(&field_name) {
                // Only record if indices differ
                if src_idx != target_idx {
                    field_map.insert(src_idx, target_idx);
                }
            }
            // Fields that don't exist in target will be handled by inject_missing_fields
        }

        if !field_map.is_empty() {
            self.remap.type_fields.insert(src_type.0, field_map);
        }
    }

    /// Force-map field types from source to target when both types exist.
    /// This prevents source's field types from being created as new types when
    /// a matching type already exists in both source and target.
    ///
    /// For example, if source's ObjectMap has a Virtual() field type and target's
    /// ObjectMap has a Virtual(toString,set,keys,...) field type, this ensures
    /// the source's field type maps to target's existing field type rather than
    /// creating a new empty Virtual.
    fn force_map_field_types(&mut self, src_type: RefType, target_type: RefType) {
        let src_obj = match self.source.get(src_type) {
            Type::Obj(obj) | Type::Struct(obj) => obj,
            _ => return,
        };
        let target_obj = match self.target.get(target_type) {
            Type::Obj(obj) | Type::Struct(obj) => obj,
            _ => return,
        };

        // Build target field name -> field_type map
        let target_fields: HashMap<String, RefType> = target_obj
            .fields
            .iter()
            .map(|f| (self.target.get(f.name).to_string(), f.t))
            .collect();

        // Collect source field info to avoid borrowing issues
        let src_field_info: Vec<(String, RefType)> = src_obj
            .fields
            .iter()
            .map(|f| (self.source.get(f.name).to_string(), f.t))
            .collect();

        // For each source field with matching target field, force the type mapping
        for (src_field_name, src_field_type) in src_field_info {
            if let Some(&target_field_type) = target_fields.get(&src_field_name) {
                // Only force-map if not already mapped and not a primitive
                if !src_field_type.is_known()
                    && !self.remap.types.contains_key(&src_field_type.0)
                {
                    self.force_map_type_recursive(src_field_type, target_field_type);
                }
            }
        }
    }

    /// Recursively force-map source type to target type.
    /// Handles wrapper types (Null, Ref, Packed) and Virtual types.
    fn force_map_type_recursive(&mut self, src_type: RefType, target_type: RefType) {
        if src_type.is_known() || self.remap.types.contains_key(&src_type.0) {
            return;
        }

        let src = self.source.get(src_type);
        let target = self.target.get(target_type);

        // Must be same variant to force-map
        if std::mem::discriminant(src) != std::mem::discriminant(target) {
            return;
        }

        match (src, target) {
            (
                Type::Virtual { fields: src_fields },
                Type::Virtual {
                    fields: target_fields,
                },
            ) => {
                // Clone the fields to avoid borrow issues
                let src_fields = src_fields.clone();
                let target_fields = target_fields.clone();

                self.remap.types.insert(src_type.0, target_type.0);
                self.build_virtual_field_remap_for_force_map(
                    src_type,
                    &src_fields,
                    &target_fields,
                );
            }

            (Type::Null(src_inner), Type::Null(target_inner))
            | (Type::Ref(src_inner), Type::Ref(target_inner))
            | (Type::Packed(src_inner), Type::Packed(target_inner)) => {
                let src_inner = *src_inner;
                let target_inner = *target_inner;
                self.remap.types.insert(src_type.0, target_type.0);
                self.force_map_type_recursive(src_inner, target_inner);
            }

            (Type::Obj(src_obj), Type::Obj(target_obj))
            | (Type::Struct(src_obj), Type::Struct(target_obj)) => {
                if self.source.get(src_obj.name) == self.target.get(target_obj.name) {
                    self.remap.types.insert(src_type.0, target_type.0);
                }
            }

            (Type::Fun(_), Type::Fun(_)) | (Type::Method(_), Type::Method(_)) => {
                // For function types, we trust that they're compatible if they have
                // the same discriminant. A more thorough check could compare args/ret.
                self.remap.types.insert(src_type.0, target_type.0);
            }

            _ => {}
        }
    }

    /// Build field index remap for a force-mapped Virtual type.
    fn build_virtual_field_remap_for_force_map(
        &mut self,
        src_type: RefType,
        src_fields: &[ObjField],
        target_fields: &[ObjField],
    ) {
        let target_field_indices: HashMap<String, usize> = target_fields
            .iter()
            .enumerate()
            .map(|(i, f)| (self.target.get(f.name).to_string(), i))
            .collect();

        let mut field_map = HashMap::new();

        for (src_idx, src_field) in src_fields.iter().enumerate() {
            let field_name = self.source.get(src_field.name).to_string();
            if let Some(&target_idx) = target_field_indices.get(&field_name) {
                if src_idx != target_idx {
                    field_map.insert(src_idx, target_idx);
                }
            }
        }

        if !field_map.is_empty() {
            self.remap.type_fields.insert(src_type.0, field_map);
        }
    }

    /// Process field remapping for an Obj/Struct type
    /// Called by ensure_type after matching a type by name
    ///
    /// IMPORTANT: This does NOT inject fields into existing types.
    /// If source has fields that target doesn't, we record a mismatch.
    /// Injecting fields would corrupt the target's memory layout.
    fn process_type_fields(&mut self, src_type: RefType, target_type: RefType) {
        // Force-map field types to target's field types BEFORE detecting mismatches
        // This ensures that types appearing in both source and target use the same
        // field type indices (e.g., Virtual types on ObjectMap fields)
        self.force_map_field_types(src_type, target_type);

        // Detect field mismatches (don't inject - that corrupts memory layout)
        self.detect_field_mismatches(src_type, target_type);

        // Build the field index remap for fields that exist in both
        self.build_field_remap(src_type, target_type);

        // Validate method override signatures
        self.validate_method_overrides(src_type, target_type);
    }

    /// Detect fields that exist in source but not in target, and field type mismatches
    /// Records mismatches for later validation
    fn detect_field_mismatches(&mut self, src_type: RefType, target_type: RefType) {
        let src_obj = match self.source.get(src_type) {
            Type::Obj(obj) | Type::Struct(obj) => obj,
            _ => return,
        };
        let target_obj = match self.target.get(target_type) {
            Type::Obj(obj) | Type::Struct(obj) => obj,
            _ => return,
        };

        let type_name = self.source.get(src_obj.name).to_string();

        // Collect source and target field names
        let src_field_names: HashSet<String> = src_obj
            .fields
            .iter()
            .map(|f| self.source.get(f.name).to_string())
            .collect();

        let target_field_names: HashSet<String> = target_obj
            .fields
            .iter()
            .map(|f| self.target.get(f.name).to_string())
            .collect();

        // Find fields missing in target (source has but target doesn't)
        let missing_in_target: Vec<String> = src_field_names
            .difference(&target_field_names)
            .cloned()
            .collect();

        // Track missing field indices for validation during opcode remapping
        if !missing_in_target.is_empty() {
            let missing_indices: HashSet<usize> = src_obj
                .fields
                .iter()
                .enumerate()
                .filter(|(_, f)| {
                    let name = self.source.get(f.name).to_string();
                    missing_in_target.contains(&name)
                })
                .map(|(i, _)| i)
                .collect();

            if !missing_indices.is_empty() {
                self.missing_fields.insert(src_type.0, missing_indices);
            }
        }

        // Find extra fields in target (target has but source doesn't) - less common
        let extra_in_target: Vec<String> = target_field_names
            .difference(&src_field_names)
            .cloned()
            .collect();

        // Check for field TYPE mismatches (same name, different type)
        let mut field_type_mismatches = Vec::new();
        for src_field in &src_obj.fields {
            let src_name = self.source.get(src_field.name).to_string();

            // Find matching field in target by name
            if let Some(target_field) = target_obj
                .fields
                .iter()
                .find(|f| self.target.get(f.name) == src_name)
            {
                // Compare types using format_type for readable output
                let src_type_str = format_type(self.source, src_field.t);
                let target_type_str = format_type(self.target, target_field.t);

                // If formatted types differ, it's a mismatch
                // But skip if both are virtuals with equivalent fields (order doesn't matter for virtuals)
                if src_type_str != target_type_str
                    && !virtuals_equivalent(self.source, src_field.t, self.target, target_field.t)
                {
                    field_type_mismatches.push(FieldTypeMismatch {
                        field_name: src_name,
                        source_type: src_type_str,
                        target_type: target_type_str,
                    });
                }
            }
        }

        // Check for field ORDER mismatches (same count, different field at same position)
        // This is critical for Obj/Struct types where field access is by index
        let mut field_order_mismatches = Vec::new();
        if src_obj.fields.len() == target_obj.fields.len() {
            for (pos, (src_field, target_field)) in
                src_obj.fields.iter().zip(target_obj.fields.iter()).enumerate()
            {
                let src_name = self.source.get(src_field.name).to_string();
                let target_name = self.target.get(target_field.name).to_string();

                // If field names differ at this position, it's an order mismatch
                if src_name != target_name {
                    let src_type_str = format_type(self.source, src_field.t);
                    let target_type_str = format_type(self.target, target_field.t);
                    field_order_mismatches.push(FieldOrderMismatch {
                        position: pos,
                        source_field: format!("{}:{}", src_name, src_type_str),
                        target_field: format!("{}:{}", target_name, target_type_str),
                    });
                }
            }
        }

        // Only record if there's any kind of mismatch
        if !missing_in_target.is_empty()
            || src_obj.fields.len() != target_obj.fields.len()
            || !field_type_mismatches.is_empty()
            || !field_order_mismatches.is_empty()
        {
            self.type_mismatches.insert(
                src_type.0,
                TypeMismatch {
                    type_name: type_name.clone(),
                    target_field_count: target_obj.fields.len(),
                    source_field_count: src_obj.fields.len(),
                    missing_in_target: missing_in_target.clone(),
                    extra_in_target,
                    field_type_mismatches: field_type_mismatches.clone(),
                    field_order_mismatches: field_order_mismatches.clone(),
                    method_signature_mismatches: Vec::new(), // Filled in by validate_method_overrides
                },
            );

            if !missing_in_target.is_empty() {
                self.warnings.push(format!(
                    "Type '{}' has {} fields in source but {} in target (missing: {})",
                    type_name,
                    src_obj.fields.len(),
                    target_obj.fields.len(),
                    missing_in_target.join(", ")
                ));
            }

            // Warn about field type mismatches - these can cause runtime crashes!
            for ftm in &field_type_mismatches {
                self.warnings.push(format!(
                    "Type '{}' field '{}' has different type: source={}, target={}",
                    type_name, ftm.field_name, ftm.source_type, ftm.target_type
                ));
            }

            // Warn about field order mismatches - field indices won't match!
            if !field_order_mismatches.is_empty() {
                self.warnings.push(format!(
                    "Type '{}' has {} fields at wrong positions (source vs target field order differs)",
                    type_name,
                    field_order_mismatches.len()
                ));
            }
        }
    }

    /// Check for methods that appear to override parent methods but have incompatible signatures.
    /// This causes vtable dispatch to fail silently - the child method won't be called.
    fn validate_method_overrides(&mut self, src_type: RefType, target_type: RefType) {
        let src_obj = match self.source.get(src_type) {
            Type::Obj(obj) | Type::Struct(obj) => obj,
            _ => return,
        };
        let target_obj = match self.target.get(target_type) {
            Type::Obj(obj) | Type::Struct(obj) => obj,
            _ => return,
        };

        let type_name = self.source.get(src_obj.name).to_string();
        let mut method_mismatches = Vec::new();

        // For each method (proto) in source type
        for src_proto in &src_obj.protos {
            let method_name = self.source.get(src_proto.name).to_string();

            // Walk parent chain in target to find same-named method
            let mut parent_ref = target_obj.super_;
            while let Some(parent_type) = parent_ref {
                if let Some(parent_obj) = self.target.get(parent_type).get_type_obj() {
                    // Search parent's protos
                    for parent_proto in &parent_obj.protos {
                        let parent_method_name = self.target.get(parent_proto.name).to_string();
                        if parent_method_name == method_name {
                            // Found same-named method - compare signatures
                            let src_sig = self.get_function_signature_str(self.source, src_proto.findex);
                            let target_sig = self.get_function_signature_str(self.target, parent_proto.findex);

                            if src_sig != target_sig {
                                let parent_name = self.target.get(parent_obj.name).to_string();
                                method_mismatches.push(MethodSignatureMismatch {
                                    method_name: method_name.clone(),
                                    parent_class: parent_name,
                                    source_signature: src_sig,
                                    target_signature: target_sig,
                                });
                            }
                            // Found the override target, stop searching parents for this method
                            break;
                        }
                    }
                    parent_ref = parent_obj.super_;
                } else {
                    break;
                }
            }
        }

        // Record mismatches if any
        if !method_mismatches.is_empty() {
            for mismatch in &method_mismatches {
                self.warnings.push(format!(
                    "Method '{}' in '{}' has signature '{}' but parent '{}' has '{}' - \
                     override may not work correctly",
                    mismatch.method_name, type_name, mismatch.source_signature,
                    mismatch.parent_class, mismatch.target_signature
                ));
            }

            // Add to type_mismatches if entry exists, or create new entry
            if let Some(mismatch) = self.type_mismatches.get_mut(&src_type.0) {
                mismatch.method_signature_mismatches = method_mismatches;
            } else {
                // Create a minimal TypeMismatch entry for method-only mismatches
                let target_field_count = target_obj.fields.len();
                let source_field_count = src_obj.fields.len();
                self.type_mismatches.insert(
                    src_type.0,
                    TypeMismatch {
                        type_name,
                        target_field_count,
                        source_field_count,
                        missing_in_target: Vec::new(),
                        extra_in_target: Vec::new(),
                        field_type_mismatches: Vec::new(),
                        field_order_mismatches: Vec::new(),
                        method_signature_mismatches: method_mismatches,
                    },
                );
            }
        }
    }

    /// Get a human-readable signature string for a function
    fn get_function_signature_str(&self, code: &Bytecode, findex: RefFun) -> String {
        match code.get(findex) {
            hlbc::types::FunPtr::Fun(f) => format_type(code, f.t),
            hlbc::types::FunPtr::Native(n) => format_type(code, n.t),
        }
    }

    /// Create a new Obj/Struct type in target from a source type
    /// This is used when a type doesn't exist in target and needs to be created
    fn create_obj_type(&mut self, src_ref: RefType) -> RefType {
        let src_type = self.source.get(src_ref);
        let (src_obj, is_struct) = match src_type {
            Type::Obj(obj) => (obj, false),
            Type::Struct(obj) => (obj, true),
            _ => {
                self.warnings.push(format!(
                    "create_obj_type called with non-Obj/Struct type at index {}",
                    src_ref.0
                ));
                return src_ref;
            }
        };

        // IMPORTANT: Reserve the type index FIRST to prevent infinite recursion
        // when field types reference this type (e.g., a type with a field of its own type)
        let new_type_idx = self.target.types.len();
        self.remap.types.insert(src_ref.0, new_type_idx);

        // Add a placeholder type that we'll update later
        self.target.types.push(Type::Void);

        // Ensure the type name exists in target
        let name_ref = RefString(self.ensure_string_value(self.source.get(src_obj.name).as_ref()));

        // Ensure the parent type exists (if any)
        // This may recurse but won't hit this type since it's already in remap
        let super_ref = src_obj.super_.map(|s| self.ensure_type(s));

        // Collect source field info before creating the type
        // (we need to ensure field types, which may recursively call ensure_type)
        let src_own_fields: Vec<(String, RefType)> = src_obj
            .own_fields
            .iter()
            .map(|f| (self.source.get(f.name).to_string(), f.t))
            .collect();

        // Now ensure all field types exist and build own_fields for target
        // This may recurse but won't hit this type since it's already in remap
        let own_fields: Vec<ObjField> = src_own_fields
            .iter()
            .map(|(name, src_type)| {
                let name_idx = self.ensure_string_value(name);
                let type_ref = self.ensure_type(*src_type);
                ObjField {
                    name: RefString(name_idx),
                    t: type_ref,
                }
            })
            .collect();

        // Build flattened fields by walking parent chain
        let mut flattened_fields = VecDeque::new();
        if let Some(parent_ref) = super_ref {
            if let Some(parent_obj) = self.target.get(parent_ref).get_type_obj() {
                // Copy parent's flattened fields
                for f in &parent_obj.fields {
                    flattened_fields.push_back(f.clone());
                }
            }
        }
        // Add own fields
        for f in &own_fields {
            flattened_fields.push_back(f.clone());
        }

        // Create a global for this type if source has one
        // NOTE: The global field in bytecode is 1-indexed (0 = no global, N = globals[N-1])
        let new_global = if src_obj.global.0 > 0 {
            // Convert from 1-indexed bytecode value to 0-indexed array index
            let src_global_idx = src_obj.global.0 - 1;

            // Check if this source global was already remapped (e.g., when $ClassName
            // and ClassName share the same global and $ClassName was processed first)
            if let Some(&existing_target_idx) = self.remap.globals.get(&src_global_idx) {
                // Return as 1-indexed for the type's global field
                RefGlobal(existing_target_idx + 1)
            } else {
                // Get the source global's type (likely $ClassName, not ClassName)
                let src_global_type = self.source.globals[src_global_idx];
                // Ensure that type exists in target (will copy $ClassName if needed)
                let target_global_type = self.ensure_type(src_global_type);

                // Create new global at the end of the target globals array
                let target_global_idx = self.target.globals.len();
                self.target.globals.push(target_global_type);
                self.remap.globals.insert(src_global_idx, target_global_idx);

                // Track this as an injected global (needs initialization code)
                self.injected_globals.insert(src_global_idx, target_global_idx);

                // Copy the ConstantDef if source has one (using 0-indexed RefGlobals)
                self.copy_global_constant(RefGlobal(src_global_idx), RefGlobal(target_global_idx));

                // Return as 1-indexed for the type's global field
                RefGlobal(target_global_idx + 1)
            }
        } else {
            RefGlobal(0)
        };

        // Record protos for deferred processing when type injection is enabled.
        // The function indices can't be remapped yet because the methods may not
        // have been injected. We'll populate protos in finalize_injected_types().
        if self.inject_new_types && !src_obj.protos.is_empty() {
            let pending: Vec<(String, RefFun, i32)> = src_obj.protos
                .iter()
                .map(|p| (self.source.get(p.name).to_string(), p.findex, p.pindex))
                .collect();
            self.pending_protos.insert(new_type_idx, (super_ref, pending));
            self.injected_types.insert(src_ref.0, new_type_idx);
        }

        // Similarly record bindings for deferred processing
        if self.inject_new_types && !src_obj.bindings.is_empty() {
            let pending: Vec<(usize, RefFun)> = src_obj.bindings
                .iter()
                .map(|(f, fun)| (f.0, *fun))
                .collect();
            self.pending_bindings.insert(new_type_idx, pending);
        }

        // Create the actual type
        let new_type = if is_struct {
            Type::Struct(hlbc::types::TypeObj {
                name: name_ref,
                super_: super_ref,
                global: new_global,
                own_fields: own_fields.clone(),
                protos: Vec::new(),        // Populated later by finalize_injected_types()
                bindings: HashMap::new(),  // Populated later by finalize_injected_types()
                fields: flattened_fields.into(),
            })
        } else {
            Type::Obj(hlbc::types::TypeObj {
                name: name_ref,
                super_: super_ref,
                global: new_global,
                own_fields: own_fields.clone(),
                protos: Vec::new(),        // Populated later by finalize_injected_types()
                bindings: HashMap::new(),  // Populated later by finalize_injected_types()
                fields: flattened_fields.into(),
            })
        };

        // Replace the placeholder with the actual type
        self.target.types[new_type_idx] = new_type;

        RefType(new_type_idx)
    }

    /// Walk a target type's inheritance chain and return the highest pindex used.
    /// Returns -1 if no protos exist in the chain.
    fn get_max_pindex_for_type(&self, type_ref: RefType) -> i32 {
        let mut max_pindex: i32 = -1;
        let mut current = Some(type_ref);
        while let Some(t) = current {
            if let Some(obj) = self.target.get(t).get_type_obj() {
                for proto in &obj.protos {
                    if proto.pindex > max_pindex {
                        max_pindex = proto.pindex;
                    }
                }
                current = obj.super_;
            } else {
                break;
            }
        }
        max_pindex
    }

    /// Search a target parent's inheritance chain for a method by name.
    /// Returns its pindex if found (for override alignment).
    fn find_parent_proto_pindex(&self, parent_ref: RefType, method_name: &str) -> Option<i32> {
        let mut current = Some(parent_ref);
        while let Some(t) = current {
            if let Some(obj) = self.target.get(t).get_type_obj() {
                for proto in &obj.protos {
                    if self.target.get(proto.name) == method_name {
                        return Some(proto.pindex);
                    }
                }
                current = obj.super_;
            } else {
                break;
            }
        }
        None
    }

    /// Finalize injected types by populating protos with remapped function indices.
    /// Must be called AFTER all functions are injected.
    pub fn finalize_injected_types(&mut self) {

        // Process pending protos
        for (target_type_idx, (parent_ref, pending)) in std::mem::take(&mut self.pending_protos) {
            let mut protos = Vec::new();

            // Determine the base pindex from the target parent's inheritance chain
            let parent_max_pindex = parent_ref
                .map(|pr| self.get_max_pindex_for_type(pr))
                .unwrap_or(-1);
            let mut next_new_pindex = parent_max_pindex + 1;

            for (name, src_findex, src_pindex) in &pending {
                let target_findex = match self.remap.funs.get(&src_findex.0) {
                    Some(&idx) => RefFun(idx),
                    None => {
                        self.warnings.push(format!(
                            "Method '{}' not injected (src findex {}), skipping proto",
                            name, src_findex.0
                        ));
                        continue;
                    }
                };

                let name_ref = RefString(self.ensure_string_value(name));

                // Determine correct pindex for this method
                let final_pindex = if *src_pindex < 0 {
                    // Negative pindex means private/static - keep as-is
                    *src_pindex
                } else if let Some(parent) = parent_ref {
                    // Check if this method overrides one in the parent chain
                    if let Some(parent_pindex) = self.find_parent_proto_pindex(parent, name) {
                        if parent_pindex != *src_pindex {
                            self.warnings.push(format!(
                                "Proto '{}': rebased pindex {} -> {} (override alignment with target parent)",
                                name, src_pindex, parent_pindex
                            ));
                        }
                        parent_pindex
                    } else {
                        // New method not in parent chain - assign next available slot
                        let pindex = next_new_pindex;
                        if pindex != *src_pindex {
                            self.warnings.push(format!(
                                "Proto '{}': rebased pindex {} -> {} (new vtable slot after target parent)",
                                name, src_pindex, pindex
                            ));
                        }
                        next_new_pindex += 1;
                        pindex
                    }
                } else {
                    // No parent - keep source pindex
                    *src_pindex
                };

                protos.push(ObjProto {
                    name: name_ref,
                    findex: target_findex,
                    pindex: final_pindex,
                });
            }

            // Update the type with its protos
            if let Type::Obj(obj) | Type::Struct(obj) = &mut self.target.types[target_type_idx] {
                obj.protos = protos;
            }
        }

        // Process pending bindings
        for (target_type_idx, pending) in std::mem::take(&mut self.pending_bindings) {
            let mut bindings = HashMap::new();
            for (field_idx, src_findex) in pending {
                if let Some(&target_idx) = self.remap.funs.get(&src_findex.0) {
                    bindings.insert(RefField(field_idx), RefFun(target_idx));
                } else {
                    self.warnings.push(format!(
                        "Binding function (src findex {}) not injected, skipping",
                        src_findex.0
                    ));
                }
            }
            if let Type::Obj(obj) | Type::Struct(obj) = &mut self.target.types[target_type_idx] {
                obj.bindings = bindings;
            }
        }
    }

    /// Get a list of injected type names for reporting
    pub fn get_injected_type_names(&self) -> Vec<String> {
        self.injected_types
            .keys()
            .filter_map(|&src_idx| {
                match self.source.get(RefType(src_idx)) {
                    Type::Obj(obj) | Type::Struct(obj) => {
                        Some(self.source.get(obj.name).to_string())
                    }
                    _ => None,
                }
            })
            .collect()
    }

    /// Extend enum construct params when source has more params than target
    /// Validates that existing params are compatible before extending
    fn extend_enum_construct_params(
        &mut self,
        target_type: RefType,
        src_type: RefType,
        extensions: &[(usize, Vec<RefType>)], // (target_construct_idx, src_params)
    ) {
        // Get enum name for logging
        let enum_name = match self.source.get(src_type) {
            Type::Enum { name, .. } => self.source.get(*name).to_string(),
            _ => format!("enum@{}", src_type.0),
        };

        for (target_construct_idx, src_params) in extensions {
            // Get current target construct info
            let (current_params, construct_name) = match self.target.get(target_type) {
                Type::Enum { constructs, .. } => {
                    if let Some(c) = constructs.get(*target_construct_idx) {
                        (c.params.clone(), self.target.get(c.name).to_string())
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };

            let current_count = current_params.len();

            // Validate that existing params are compatible
            // First, remap the source params to target types
            let remapped_src_params: Vec<RefType> = src_params
                .iter()
                .map(|&p| self.ensure_type(p))
                .collect();

            // Check that the first N params match
            let mut params_compatible = true;
            for (i, &target_param) in current_params.iter().enumerate() {
                if i >= remapped_src_params.len() {
                    params_compatible = false;
                    break;
                }
                // Compare types - they should be the same after remapping
                if target_param != remapped_src_params[i] {
                    // Types don't match - check if they're compatible by name
                    let target_type_name = self.get_type_name_in_target(target_param);
                    let src_type_name = self.get_type_name_in_target(remapped_src_params[i]);
                    if target_type_name != src_type_name {
                        self.warnings.push(format!(
                            "Enum '{}' construct '{}' param {} type mismatch: target={}, source={}",
                            enum_name, construct_name, i, target_type_name, src_type_name
                        ));
                        params_compatible = false;
                        break;
                    }
                }
            }

            if !params_compatible {
                self.warnings.push(format!(
                    "Skipping extension of enum '{}' construct '{}' due to param mismatch",
                    enum_name, construct_name
                ));
                continue;
            }

            // Get the additional params to add
            let new_params: Vec<RefType> = remapped_src_params
                .into_iter()
                .skip(current_count)
                .collect();

            if new_params.is_empty() {
                continue;
            }

            // Extend the params
            if let Type::Enum { constructs, .. } = &mut self.target.types[target_type.0] {
                if let Some(construct) = constructs.get_mut(*target_construct_idx) {
                    construct.params.extend(new_params.clone());
                }
            }

            self.warnings.push(format!(
                "Extended enum '{}' construct '{}' with {} additional param(s)",
                enum_name,
                construct_name,
                new_params.len()
            ));
        }
    }

    /// Get a readable name for a type in target bytecode
    fn get_type_name_in_target(&self, type_ref: RefType) -> String {
        match self.target.get(type_ref) {
            Type::Void => "Void".to_string(),
            Type::UI8 => "UI8".to_string(),
            Type::UI16 => "UI16".to_string(),
            Type::I32 => "I32".to_string(),
            Type::I64 => "I64".to_string(),
            Type::F32 => "F32".to_string(),
            Type::F64 => "F64".to_string(),
            Type::Bool => "Bool".to_string(),
            Type::Bytes => "Bytes".to_string(),
            Type::Dyn => "Dyn".to_string(),
            Type::Obj(obj) | Type::Struct(obj) => self.target.get(obj.name).to_string(),
            Type::Enum { name, .. } => self.target.get(*name).to_string(),
            Type::Abstract { name } => self.target.get(*name).to_string(),
            Type::Null(inner) => format!("Null<{}>", self.get_type_name_in_target(*inner)),
            Type::Ref(inner) => format!("Ref<{}>", self.get_type_name_in_target(*inner)),
            Type::Fun(f) => format!("Fun({})", f.args.len()),
            Type::Method(f) => format!("Method({})", f.args.len()),
            Type::Virtual { fields } => format!("Virtual({})", fields.len()),
            _ => format!("type@{}", type_ref.0),
        }
    }

    /// Inject missing enum constructs into a target enum type
    /// Updates construct_map with the new mappings (src_idx -> target_idx)
    fn inject_enum_constructs(
        &mut self,
        target_type: RefType,
        src_type: RefType,
        missing: &[(usize, String, Vec<RefType>)],
        construct_map: &mut HashMap<usize, usize>,
    ) {
        // Get source enum name for logging
        let enum_name = match self.source.get(src_type) {
            Type::Enum { name, .. } => self.source.get(*name).to_string(),
            _ => format!("enum@{}", src_type.0),
        };

        // Get current construct count in target
        let target_construct_count = match self.target.get(target_type) {
            Type::Enum { constructs, .. } => constructs.len(),
            _ => return,
        };

        // Collect new constructs to add
        let mut new_constructs: Vec<hlbc::types::EnumConstruct> = Vec::new();
        let mut injected_names: Vec<String> = Vec::new();

        for (src_idx, construct_name, src_params) in missing {
            // Ensure param types exist in target
            let target_params: Vec<RefType> =
                src_params.iter().map(|&p| self.ensure_type(p)).collect();

            // Ensure construct name string exists
            let name_ref = RefString(self.ensure_string_value(construct_name));

            // The new construct will be appended, so its index is current_count + new_constructs.len()
            let new_idx = target_construct_count + new_constructs.len();
            construct_map.insert(*src_idx, new_idx);

            new_constructs.push(hlbc::types::EnumConstruct {
                name: name_ref,
                params: target_params,
            });
            injected_names.push(construct_name.clone());
        }

        // Actually add the constructs to the target enum
        if let Type::Enum { constructs, .. } = &mut self.target.types[target_type.0] {
            constructs.extend(new_constructs);
        }

        self.warnings.push(format!(
            "Injected {} construct(s) into enum '{}': {}",
            injected_names.len(),
            enum_name,
            injected_names.join(", ")
        ));
    }

    /// Extract initialization code for injected types from source entry point.
    ///
    /// This scans the source entry point function for:
    /// 1. initClass() calls for injected $ClassName types
    /// 2. GetGlobal + SetField sequences for static field initialization
    /// 3. New + constructor calls that create values for static fields
    ///
    /// The extracted opcodes are remapped to use target pool indices.
    /// This method MUST be called before dropping the merger, as it needs to
    /// call ensure_fun() and ensure_type() for any references in the extracted code.
    ///
    /// Returns None if no initialization code is needed.
    pub fn extract_init_code_for_injected_types(&mut self) -> Option<ExtractedInitCode> {
        if self.injected_globals.is_empty() {
            return None;
        }

        // Get source entry point function
        let entry_func = self.source.get(self.source.entrypoint);
        let entry_func = match entry_func {
            hlbc::types::FunPtr::Fun(f) => f,
            hlbc::types::FunPtr::Native(_) => return None, // Shouldn't happen
        };

        // Build set of injected source globals
        let injected_src_globals: HashSet<usize> = self.injected_globals.keys().copied().collect();

        // Collect type names of injected $ClassName types for initClass matching
        let mut injected_class_names: HashSet<String> = HashSet::new();
        for &src_global_idx in self.injected_globals.keys() {
            let global_type = self.source.globals[src_global_idx];
            if let Type::Obj(obj) = self.source.get(global_type) {
                let name = self.source.get(obj.name).to_string();
                injected_class_names.insert(name);
            }
        }

        let ops = &entry_func.ops;
        let regs = &entry_func.regs;

        // Track which opcodes to extract
        let mut extracted_indices: HashSet<usize> = HashSet::new();
        // Track registers that hold objects we need to initialize (from New opcodes we include)
        let mut initialized_registers: HashSet<u32> = HashSet::new();

        // Pass 1: Find initClass calls for injected types by looking at the pattern:
        //   Type reg = $ClassName
        //   Type reg = ClassName
        //   String reg = "full.ClassName"
        //   Call3 initClass(reg, reg, reg)
        //
        // We identify these by checking if the String argument matches an injected class name
        for (idx, op) in ops.iter().enumerate() {
            if let Opcode::Call3 { fun, arg0, arg1, arg2, .. } = op {
                let is_init_class = if let hlbc::types::FunPtr::Fun(f) = self.source.get(*fun) {
                    self.source.get(f.name) == "initClass"
                } else {
                    false
                };

                if is_init_class && idx >= 3 {
                    // Verify the pattern: Type, Type, String, Call3
                    let type1_ok = matches!(&ops[idx - 3], Opcode::Type { dst, .. } if dst.0 == arg0.0);
                    let type2_ok = matches!(&ops[idx - 2], Opcode::Type { dst, .. } if dst.0 == arg1.0);
                    let string_ok = matches!(&ops[idx - 1], Opcode::String { dst, .. } if dst.0 == arg2.0);

                    if type1_ok && type2_ok && string_ok {
                        // Get the class name from the String opcode
                        if let Opcode::String { ptr, .. } = &ops[idx - 1] {
                            let class_name = self.source.get(*ptr);
                            // Check if this is an injected class
                            // initClass string is "uber.UberState", but injected_class_names
                            // contains "uber.$UberState" ($ before class name, not package)
                            // Convert "uber.UberState" -> "uber.$UberState"
                            let dollar_name = if let Some(dot_pos) = class_name.rfind('.') {
                                format!("{}.${}",
                                    &class_name[..dot_pos],
                                    &class_name[dot_pos + 1..])
                            } else {
                                format!("${}", class_name)
                            };
                            if injected_class_names.contains(&dollar_name) {
                                // Extract the whole initClass sequence
                                extracted_indices.insert(idx - 3); // Type $ClassName
                                extracted_indices.insert(idx - 2); // Type ClassName
                                extracted_indices.insert(idx - 1); // String "full.ClassName"
                                extracted_indices.insert(idx);     // Call3 initClass
                            }
                        }
                    }
                }
            }
        }

        // Pass 2: Find GetGlobal + SetField sequences for injected globals
        // and track the value definitions we need to include
        for (idx, op) in ops.iter().enumerate() {
            if let Opcode::GetGlobal { global, .. } = op {
                if injected_src_globals.contains(&global.0) {
                    extracted_indices.insert(idx);

                    // Look ahead for SetField operations on this register
                    // The pattern is: GetGlobal reg = global; SetField reg.field = value
                    if idx + 1 < ops.len() {
                        if let Opcode::SetField { src, .. } = &ops[idx + 1] {
                            extracted_indices.insert(idx + 1);

                            // Find the definition of src and include it
                            if let Some(def_idx) = find_def_before(ops, idx + 1, src.0) {
                                include_def_with_constructor(
                                    ops, def_idx, &mut extracted_indices, &mut initialized_registers
                                );
                            }
                        }
                    }
                }
            }
        }

        if extracted_indices.is_empty() {
            return None;
        }

        // Sort indices and extract opcodes in order
        let mut sorted_indices: Vec<usize> = extracted_indices.into_iter().collect();
        sorted_indices.sort();

        // NEW: Pass 3 - Ensure all function and type references in extracted opcodes are mapped
        // This MUST happen before we use the remap, so that functions like ObjectMap.__constructor__
        // get added to the remap tables.
        for &idx in &sorted_indices {
            let op = &ops[idx];
            match op {
                // Function references
                Opcode::Call0 { fun, .. }
                | Opcode::Call1 { fun, .. }
                | Opcode::Call2 { fun, .. }
                | Opcode::Call3 { fun, .. }
                | Opcode::Call4 { fun, .. }
                | Opcode::CallN { fun, .. } => {
                    self.ensure_fun(*fun);
                }
                // Type references
                Opcode::Type { ty, .. } => {
                    self.ensure_type(*ty);
                }
                // New opcodes - type comes from the register
                Opcode::New { dst } => {
                    let reg_type = regs[dst.0 as usize];
                    self.ensure_type(reg_type);
                }
                // String references
                Opcode::String { ptr, .. } => {
                    self.ensure_string(*ptr);
                }
                // Global references
                Opcode::GetGlobal { global, .. } | Opcode::SetGlobal { global, .. } => {
                    self.ensure_global(*global);
                }
                // Int/Float references
                Opcode::Int { ptr, .. } => {
                    self.ensure_int(*ptr);
                }
                Opcode::Float { ptr, .. } => {
                    self.ensure_float(*ptr);
                }
                _ => {}
            }
        }

        // Build register remapping: old register -> new register
        let mut old_to_new_reg: HashMap<u32, u32> = HashMap::new();
        let mut next_reg: u32 = 0;

        // First pass: collect all registers used
        for &idx in &sorted_indices {
            collect_registers(&ops[idx], &mut old_to_new_reg, &mut next_reg);
        }

        // Build source register type map for the registers we're using
        let mut src_reg_types: HashMap<u32, RefType> = HashMap::new();
        for (&old_reg, _) in &old_to_new_reg {
            if (old_reg as usize) < regs.len() {
                src_reg_types.insert(old_reg, regs[old_reg as usize]);
            }
        }

        // Second pass: remap opcodes
        let mut remapped_opcodes = Vec::new();
        let initialized_globals: Vec<usize> = self.injected_globals.keys().copied().collect();

        for &idx in &sorted_indices {
            let op = &ops[idx];
            // First remap pool references using the index remap
            let pool_remapped = self.remap.remap_opcode_with_regs(op, regs);
            // Then remap registers to the new contiguous range
            let fully_remapped = remap_registers(&pool_remapped, &old_to_new_reg);
            remapped_opcodes.push(fully_remapped);
        }

        // Build remapped register types
        let mut remapped_reg_types: Vec<RefType> = vec![RefType(0); next_reg as usize];
        for (&old_reg, &new_reg) in &old_to_new_reg {
            if let Some(&src_type) = src_reg_types.get(&old_reg) {
                // Remap the type reference
                remapped_reg_types[new_reg as usize] = self.remap.remap_type(src_type);
            }
        }

        Some(ExtractedInitCode {
            opcodes: remapped_opcodes,
            register_count: next_reg as usize,
            initialized_globals,
            register_types: remapped_reg_types,
        })
    }
}

/// Information about initialization code extracted from source entry point
#[derive(Debug)]
pub struct ExtractedInitCode {
    /// Opcodes to inject, in order (already remapped)
    pub opcodes: Vec<Opcode>,
    /// Number of registers used by the extracted code
    pub register_count: usize,
    /// Source globals that were initialized
    pub initialized_globals: Vec<usize>,
    /// Register types for the extracted code (already remapped to target types)
    pub register_types: Vec<RefType>,
}

/// Find the opcode that defines a register, looking backwards from idx
fn find_def_before(ops: &[Opcode], idx: usize, reg: u32) -> Option<usize> {
    for i in (0..idx).rev() {
        if opcode_defines_reg(&ops[i], reg) {
            return Some(i);
        }
    }
    None
}

/// Check if an opcode defines (writes to) a register
fn opcode_defines_reg(op: &Opcode, reg: u32) -> bool {
    match op {
        Opcode::Mov { dst, .. } |
        Opcode::New { dst } |
        Opcode::Null { dst } |
        Opcode::Bool { dst, .. } |
        Opcode::Int { dst, .. } |
        Opcode::Float { dst, .. } |
        Opcode::String { dst, .. } |
        Opcode::Type { dst, .. } |
        Opcode::GetGlobal { dst, .. } |
        Opcode::Field { dst, .. } |
        Opcode::Call0 { dst, .. } |
        Opcode::Call1 { dst, .. } |
        Opcode::Call2 { dst, .. } |
        Opcode::Call3 { dst, .. } |
        Opcode::Call4 { dst, .. } |
        Opcode::CallN { dst, .. } => dst.0 == reg,
        _ => false,
    }
}

/// Include an opcode definition and its dependencies, including constructor calls
fn include_def_with_constructor(
    ops: &[Opcode],
    def_idx: usize,
    extracted: &mut HashSet<usize>,
    initialized_regs: &mut HashSet<u32>,
) {
    if !extracted.insert(def_idx) {
        return; // Already included
    }

    let op = &ops[def_idx];

    // If this is a New opcode, look for the constructor call that follows
    if let Opcode::New { dst } = op {
        initialized_regs.insert(dst.0);

        // Look ahead for Call1/CallN __constructor__ that takes this register as first arg
        for i in (def_idx + 1)..ops.len().min(def_idx + 10) {
            match &ops[i] {
                Opcode::Call1 { fun, arg0, .. } if arg0.0 == dst.0 => {
                    // Check if this is a constructor call
                    // Constructor calls have the pattern Call1 __constructor__(obj)
                    extracted.insert(i);
                    break;
                }
                Opcode::CallN { fun: _, args, .. } if !args.is_empty() && args[0].0 == dst.0 => {
                    extracted.insert(i);
                    break;
                }
                _ => {
                    // If we see another definition of dst, stop looking
                    if opcode_defines_reg(&ops[i], dst.0) {
                        break;
                    }
                }
            }
        }
    }

    // Include dependencies for read registers
    let read_regs = get_read_registers(op);
    for reg in read_regs {
        if let Some(dep_idx) = find_def_before(ops, def_idx, reg) {
            include_def_with_constructor(ops, dep_idx, extracted, initialized_regs);
        }
    }
}

/// Get all registers that an opcode reads from
fn get_read_registers(op: &Opcode) -> Vec<u32> {
    match op {
        Opcode::Mov { src, .. } => vec![src.0],
        Opcode::Call0 { .. } => vec![],
        Opcode::Call1 { arg0, .. } => vec![arg0.0],
        Opcode::Call2 { arg0, arg1, .. } => vec![arg0.0, arg1.0],
        Opcode::Call3 { arg0, arg1, arg2, .. } => vec![arg0.0, arg1.0, arg2.0],
        Opcode::Call4 { arg0, arg1, arg2, arg3, .. } => vec![arg0.0, arg1.0, arg2.0, arg3.0],
        Opcode::CallN { args, .. } => args.iter().map(|r| r.0).collect(),
        Opcode::SetField { obj, src, .. } => vec![obj.0, src.0],
        Opcode::GetGlobal { .. } => vec![],
        Opcode::SetGlobal { src, .. } => vec![src.0],
        Opcode::Field { obj, .. } => vec![obj.0],
        Opcode::New { .. } => vec![],
        Opcode::Type { .. } => vec![],
        Opcode::Null { .. } => vec![],
        Opcode::Bool { .. } => vec![],
        Opcode::Int { .. } => vec![],
        Opcode::Float { .. } => vec![],
        Opcode::String { .. } => vec![],
        _ => vec![], // Conservative: assume no reads
    }
}

/// Collect all registers used by an opcode, assigning new indices as needed
fn collect_registers(op: &Opcode, mapping: &mut HashMap<u32, u32>, next_reg: &mut u32) {
    let regs = get_all_registers(op);
    for reg in regs {
        mapping.entry(reg).or_insert_with(|| {
            let new = *next_reg;
            *next_reg += 1;
            new
        });
    }
}

/// Get all registers (read and written) by an opcode
fn get_all_registers(op: &Opcode) -> Vec<u32> {
    match op {
        Opcode::Mov { dst, src } => vec![dst.0 as u32, src.0 as u32],
        Opcode::Call0 { dst, .. } => vec![dst.0 as u32],
        Opcode::Call1 { dst, arg0, .. } => vec![dst.0 as u32, arg0.0 as u32],
        Opcode::Call2 { dst, arg0, arg1, .. } => vec![dst.0 as u32, arg0.0 as u32, arg1.0 as u32],
        Opcode::Call3 { dst, arg0, arg1, arg2, .. } => {
            vec![dst.0 as u32, arg0.0 as u32, arg1.0 as u32, arg2.0 as u32]
        }
        Opcode::Call4 { dst, arg0, arg1, arg2, arg3, .. } => {
            vec![dst.0 as u32, arg0.0 as u32, arg1.0 as u32, arg2.0 as u32, arg3.0 as u32]
        }
        Opcode::CallN { dst, args, .. } => {
            let mut v = vec![dst.0 as u32];
            v.extend(args.iter().map(|r| r.0 as u32));
            v
        }
        Opcode::SetField { obj, src, .. } => vec![obj.0 as u32, src.0 as u32],
        Opcode::GetGlobal { dst, .. } => vec![dst.0 as u32],
        Opcode::SetGlobal { src, .. } => vec![src.0 as u32],
        Opcode::Field { dst, obj, .. } => vec![dst.0 as u32, obj.0 as u32],
        Opcode::New { dst } => vec![dst.0 as u32],
        Opcode::Type { dst, .. } => vec![dst.0 as u32],
        Opcode::Null { dst } => vec![dst.0 as u32],
        Opcode::Bool { dst, .. } => vec![dst.0 as u32],
        Opcode::Int { dst, .. } => vec![dst.0 as u32],
        Opcode::Float { dst, .. } => vec![dst.0 as u32],
        Opcode::String { dst, .. } => vec![dst.0 as u32],
        _ => vec![], // Conservative
    }
}

/// Remap registers in an opcode using the given mapping
fn remap_registers(op: &Opcode, mapping: &HashMap<u32, u32>) -> Opcode {
    use hlbc::types::Reg;

    let remap_reg = |r: Reg| -> Reg {
        Reg(*mapping.get(&r.0).unwrap_or(&r.0))
    };

    match op.clone() {
        Opcode::Mov { dst, src } => Opcode::Mov {
            dst: remap_reg(dst),
            src: remap_reg(src),
        },
        Opcode::Call0 { dst, fun } => Opcode::Call0 {
            dst: remap_reg(dst),
            fun,
        },
        Opcode::Call1 { dst, fun, arg0 } => Opcode::Call1 {
            dst: remap_reg(dst),
            fun,
            arg0: remap_reg(arg0),
        },
        Opcode::Call2 { dst, fun, arg0, arg1 } => Opcode::Call2 {
            dst: remap_reg(dst),
            fun,
            arg0: remap_reg(arg0),
            arg1: remap_reg(arg1),
        },
        Opcode::Call3 { dst, fun, arg0, arg1, arg2 } => Opcode::Call3 {
            dst: remap_reg(dst),
            fun,
            arg0: remap_reg(arg0),
            arg1: remap_reg(arg1),
            arg2: remap_reg(arg2),
        },
        Opcode::Call4 { dst, fun, arg0, arg1, arg2, arg3 } => Opcode::Call4 {
            dst: remap_reg(dst),
            fun,
            arg0: remap_reg(arg0),
            arg1: remap_reg(arg1),
            arg2: remap_reg(arg2),
            arg3: remap_reg(arg3),
        },
        Opcode::CallN { dst, fun, args } => Opcode::CallN {
            dst: remap_reg(dst),
            fun,
            args: args.into_iter().map(remap_reg).collect(),
        },
        Opcode::SetField { obj, field, src } => Opcode::SetField {
            obj: remap_reg(obj),
            field,
            src: remap_reg(src),
        },
        Opcode::GetGlobal { dst, global } => Opcode::GetGlobal {
            dst: remap_reg(dst),
            global,
        },
        Opcode::SetGlobal { global, src } => Opcode::SetGlobal {
            global,
            src: remap_reg(src),
        },
        Opcode::Field { dst, obj, field } => Opcode::Field {
            dst: remap_reg(dst),
            obj: remap_reg(obj),
            field,
        },
        Opcode::New { dst } => Opcode::New {
            dst: remap_reg(dst),
        },
        Opcode::Type { dst, ty } => Opcode::Type {
            dst: remap_reg(dst),
            ty,
        },
        Opcode::Null { dst } => Opcode::Null {
            dst: remap_reg(dst),
        },
        Opcode::Bool { dst, value } => Opcode::Bool {
            dst: remap_reg(dst),
            value,
        },
        Opcode::Int { dst, ptr } => Opcode::Int {
            dst: remap_reg(dst),
            ptr,
        },
        Opcode::Float { dst, ptr } => Opcode::Float {
            dst: remap_reg(dst),
            ptr,
        },
        Opcode::String { dst, ptr } => Opcode::String {
            dst: remap_reg(dst),
            ptr,
        },
        other => other, // Pass through unchanged
    }
}

/// Inject initialization opcodes into target entry point.
///
/// Inserts the extracted initialization code before the main() call in the target
/// entry point function.
///
/// # Arguments
/// * `target` - Target bytecode to modify
/// * `init_code` - The extracted and remapped initialization code
///
/// Returns the number of opcodes injected.
pub fn inject_init_into_entrypoint(
    target: &mut Bytecode,
    init_code: ExtractedInitCode,
) -> usize {
    if init_code.opcodes.is_empty() {
        return 0;
    }

    // Find target entry point function
    let entry_findex = target.entrypoint.0;

    // Find the function in the functions array
    let func_idx = target.functions.iter().position(|f| f.findex.0 == entry_findex);
    let func_idx = match func_idx {
        Some(idx) => idx,
        None => return 0, // Entry point not found
    };

    // First pass: find insertion point while holding immutable borrows
    // We need to find Call0 to "main" function
    let ops_len = target.functions[func_idx].ops.len();
    let mut insert_idx = ops_len; // Default: insert at end (before Ret)

    for (idx, op) in target.functions[func_idx].ops.iter().enumerate().rev() {
        match op {
            Opcode::Call0 { fun, .. } => {
                if let hlbc::types::FunPtr::Fun(f) = target.get(*fun) {
                    if target.get(f.name) == "main" {
                        insert_idx = idx;
                        break;
                    }
                }
            }
            Opcode::Ret { .. } if insert_idx == ops_len => {
                // Found Ret, insert before it if we haven't found main
                insert_idx = idx;
            }
            _ => {}
        }
    }

    // Now get mutable access to the function
    let func = &mut target.functions[func_idx];

    // Calculate register offset: we need to shift all extracted registers
    // to start after the current max register
    let current_max_reg = func.regs.len() as u32;
    let shifted_ops: Vec<Opcode> = init_code.opcodes.into_iter()
        .map(|op| shift_registers(op, current_max_reg))
        .collect();

    let count = shifted_ops.len();

    // Add register types for the new registers (using actual types from source)
    for reg_type in init_code.register_types {
        func.regs.push(reg_type);
    }

    // Insert the opcodes
    for (i, op) in shifted_ops.into_iter().enumerate() {
        func.ops.insert(insert_idx + i, op);
    }

    // Update debug info if present
    if let Some(ref mut debug_info) = func.debug_info {
        // Insert placeholder debug entries for the new opcodes
        for i in 0..count {
            debug_info.insert(insert_idx + i, (0, 0));
        }
    }

    count
}

/// Shift all registers in an opcode by a fixed offset
fn shift_registers(op: Opcode, offset: u32) -> Opcode {
    use hlbc::types::Reg;

    let shift_reg = |r: Reg| -> Reg {
        Reg(r.0.saturating_add(offset))
    };

    match op {
        Opcode::Mov { dst, src } => Opcode::Mov {
            dst: shift_reg(dst),
            src: shift_reg(src),
        },
        Opcode::Call0 { dst, fun } => Opcode::Call0 {
            dst: shift_reg(dst),
            fun,
        },
        Opcode::Call1 { dst, fun, arg0 } => Opcode::Call1 {
            dst: shift_reg(dst),
            fun,
            arg0: shift_reg(arg0),
        },
        Opcode::Call2 { dst, fun, arg0, arg1 } => Opcode::Call2 {
            dst: shift_reg(dst),
            fun,
            arg0: shift_reg(arg0),
            arg1: shift_reg(arg1),
        },
        Opcode::Call3 { dst, fun, arg0, arg1, arg2 } => Opcode::Call3 {
            dst: shift_reg(dst),
            fun,
            arg0: shift_reg(arg0),
            arg1: shift_reg(arg1),
            arg2: shift_reg(arg2),
        },
        Opcode::Call4 { dst, fun, arg0, arg1, arg2, arg3 } => Opcode::Call4 {
            dst: shift_reg(dst),
            fun,
            arg0: shift_reg(arg0),
            arg1: shift_reg(arg1),
            arg2: shift_reg(arg2),
            arg3: shift_reg(arg3),
        },
        Opcode::CallN { dst, fun, args } => Opcode::CallN {
            dst: shift_reg(dst),
            fun,
            args: args.into_iter().map(shift_reg).collect(),
        },
        Opcode::SetField { obj, field, src } => Opcode::SetField {
            obj: shift_reg(obj),
            field,
            src: shift_reg(src),
        },
        Opcode::GetGlobal { dst, global } => Opcode::GetGlobal {
            dst: shift_reg(dst),
            global,
        },
        Opcode::SetGlobal { global, src } => Opcode::SetGlobal {
            global,
            src: shift_reg(src),
        },
        Opcode::Field { dst, obj, field } => Opcode::Field {
            dst: shift_reg(dst),
            obj: shift_reg(obj),
            field,
        },
        Opcode::New { dst } => Opcode::New {
            dst: shift_reg(dst),
        },
        Opcode::Type { dst, ty } => Opcode::Type {
            dst: shift_reg(dst),
            ty,
        },
        Opcode::Null { dst } => Opcode::Null {
            dst: shift_reg(dst),
        },
        Opcode::Bool { dst, value } => Opcode::Bool {
            dst: shift_reg(dst),
            value,
        },
        Opcode::Int { dst, ptr } => Opcode::Int {
            dst: shift_reg(dst),
            ptr,
        },
        Opcode::Float { dst, ptr } => Opcode::Float {
            dst: shift_reg(dst),
            ptr,
        },
        Opcode::String { dst, ptr } => Opcode::String {
            dst: shift_reg(dst),
            ptr,
        },
        other => other, // Pass through unchanged
    }
}
