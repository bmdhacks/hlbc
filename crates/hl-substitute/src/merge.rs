use std::collections::{HashMap, HashSet, VecDeque};

use hlbc::opcodes::Opcode;
use hlbc::types::{
    ConstantDef, Function, ObjField, RefBytes, RefFloat, RefFun, RefGlobal, RefInt, RefString,
    RefType, Type,
};
use hlbc::{Bytecode, Resolve};

use crate::remap::IndexRemap;

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

/// Information about type layout mismatches between source and target
#[derive(Debug, Clone)]
pub struct TypeMismatch {
    pub type_name: String,
    pub target_field_count: usize,
    pub source_field_count: usize,
    pub missing_in_target: Vec<String>, // fields in source but not target
    pub extra_in_target: Vec<String>,   // fields in target but not source (less common)
}

/// Merges pools from source bytecode into target bytecode, building an IndexRemap
pub struct PoolMerger<'a> {
    pub target: &'a mut Bytecode,
    pub source: &'a Bytecode,
    pub remap: IndexRemap,
    pub warnings: Vec<String>,
    /// Whether to inject missing function dependencies
    pub inject_missing_functions: bool,
    /// Functions that were injected from source
    pub injected_functions: Vec<String>,
    /// Native functions that couldn't be resolved (natives can't be injected)
    pub unresolvable_natives: Vec<String>,
    /// Type layout mismatches detected (source type index -> mismatch info)
    pub type_mismatches: HashMap<usize, TypeMismatch>,
}

impl<'a> PoolMerger<'a> {
    pub fn new(target: &'a mut Bytecode, source: &'a Bytecode, inject_deps: bool) -> Self {
        Self {
            target,
            source,
            remap: IndexRemap::new(),
            warnings: Vec::new(),
            inject_missing_functions: inject_deps,
            injected_functions: Vec::new(),
            unresolvable_natives: Vec::new(),
            type_mismatches: HashMap::new(),
        }
    }

    /// Ensure an int constant exists in target, return the remapped RefInt
    pub fn ensure_int(&mut self, src_ref: RefInt) -> RefInt {
        if let Some(&target_idx) = self.remap.ints.get(&src_ref.0) {
            return RefInt(target_idx);
        }

        let src_val = self.source.ints[src_ref.0];

        // Check if already exists in target
        if let Some(target_idx) = self.target.ints.iter().position(|&v| v == src_val) {
            self.remap.ints.insert(src_ref.0, target_idx);
            return RefInt(target_idx);
        }

        // Add new int
        let target_idx = self.target.ints.len();
        self.target.ints.push(src_val);
        self.remap.ints.insert(src_ref.0, target_idx);
        RefInt(target_idx)
    }

    /// Ensure a float constant exists in target, return the remapped RefFloat
    pub fn ensure_float(&mut self, src_ref: RefFloat) -> RefFloat {
        if let Some(&target_idx) = self.remap.floats.get(&src_ref.0) {
            return RefFloat(target_idx);
        }

        let src_val = self.source.floats[src_ref.0];

        // Check if already exists in target (use bitwise comparison for floats)
        if let Some(target_idx) = self
            .target
            .floats
            .iter()
            .position(|&v| v.to_bits() == src_val.to_bits())
        {
            self.remap.floats.insert(src_ref.0, target_idx);
            return RefFloat(target_idx);
        }

        // Add new float
        let target_idx = self.target.floats.len();
        self.target.floats.push(src_val);
        self.remap.floats.insert(src_ref.0, target_idx);
        RefFloat(target_idx)
    }

    /// Ensure a string exists in target, return the remapped RefString
    pub fn ensure_string(&mut self, src_ref: RefString) -> RefString {
        if let Some(&target_idx) = self.remap.strings.get(&src_ref.0) {
            return RefString(target_idx);
        }

        let src_str = &self.source.strings[src_ref.0];

        // Check if already exists in target
        if let Some(target_idx) = self.target.strings.iter().position(|s| s == src_str) {
            self.remap.strings.insert(src_ref.0, target_idx);
            return RefString(target_idx);
        }

        // Add new string
        let target_idx = self.target.strings.len();
        self.target.strings.push(src_str.clone());
        self.remap.strings.insert(src_ref.0, target_idx);
        RefString(target_idx)
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
                            if !self.enum_constructs_match(constructs, target_constructs) {
                                continue; // Try next enum with same name
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

        // 6. Record the remap
        self.remap.globals.insert(src_ref.0, new_global.0);

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

        // Non-String globals: match by type (original behavior)
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
                    for target_func in &self.target.functions {
                        let target_name = self.target.get(target_func.name).to_string();
                        let target_parent_name = target_func.parent.map(|p| {
                            self.target.get(p).get_type_obj()
                                .map(|obj| self.target.get(obj.name).to_string())
                        }).flatten();

                        if src_name == target_name && src_parent_name == target_parent_name {
                            self.remap.funs.insert(src_ref.0, target_func.findex.0);
                            return target_func.findex;
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

                // Native not found - cannot inject natives, they're external bindings
                let qualified_name = format!("{}::{}", src_lib, src_name);
                self.unresolvable_natives.push(qualified_name.clone());
                self.warnings.push(format!(
                    "Native '{}' from source not found in target (natives cannot be injected)",
                    qualified_name
                ));
            }
        }

        src_ref
    }

    /// Inject a function from source into target
    /// This is called when a function is not found in target and injection is enabled
    fn inject_function(
        &mut self,
        src_ref: RefFun,
        func_name: &str,
        parent_name: Option<&str>,
    ) -> RefFun {
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
        let nops = new_ops.len();

        // Create the new function
        let new_func = Function {
            t: new_type,
            findex: new_findex,
            regs: new_regs,
            ops: new_ops,
            // Create dummy debug_info - source file indices aren't valid in target
            // Each opcode needs an entry (file_idx, line_num), use (0, 0) as placeholder
            // TODO: remap debug file indices properly
            debug_info: Some(vec![(0, 0); nops]),
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
    fn enum_constructs_match(
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

    /// Process field remapping for an Obj/Struct type
    /// Called by ensure_type after matching a type by name
    ///
    /// IMPORTANT: This does NOT inject fields into existing types.
    /// If source has fields that target doesn't, we record a mismatch.
    /// Injecting fields would corrupt the target's memory layout.
    fn process_type_fields(&mut self, src_type: RefType, target_type: RefType) {
        // Detect field mismatches (don't inject - that corrupts memory layout)
        self.detect_field_mismatches(src_type, target_type);

        // Build the field index remap for fields that exist in both
        self.build_field_remap(src_type, target_type);
    }

    /// Detect fields that exist in source but not in target
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

        // Find extra fields in target (target has but source doesn't) - less common
        let extra_in_target: Vec<String> = target_field_names
            .difference(&src_field_names)
            .cloned()
            .collect();

        // Only record if there's a mismatch
        if !missing_in_target.is_empty() || src_obj.fields.len() != target_obj.fields.len() {
            self.type_mismatches.insert(
                src_type.0,
                TypeMismatch {
                    type_name: type_name.clone(),
                    target_field_count: target_obj.fields.len(),
                    source_field_count: src_obj.fields.len(),
                    missing_in_target: missing_in_target.clone(),
                    extra_in_target,
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

        // Create the actual type
        let new_type = if is_struct {
            Type::Struct(hlbc::types::TypeObj {
                name: name_ref,
                super_: super_ref,
                global: RefGlobal(0), // No global for created types
                own_fields: own_fields.clone(),
                protos: Vec::new(),        // No protos for created types
                bindings: HashMap::new(),  // No bindings for created types
                fields: flattened_fields.into(),
            })
        } else {
            Type::Obj(hlbc::types::TypeObj {
                name: name_ref,
                super_: super_ref,
                global: RefGlobal(0), // No global for created types
                own_fields: own_fields.clone(),
                protos: Vec::new(),        // No protos for created types
                bindings: HashMap::new(),  // No bindings for created types
                fields: flattened_fields.into(),
            })
        };

        // Replace the placeholder with the actual type
        self.target.types[new_type_idx] = new_type;

        RefType(new_type_idx)
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
}
