pub mod matching;
pub mod merge;
pub mod remap;

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, RefType};
use hlbc::{Bytecode, Resolve};

use matching::{matches_pattern, is_type_injection_pattern, strip_type_prefix, FunctionIndex};
use merge::PoolMerger;

/// Get the source file path for a function (from its first opcode's debug info)
pub fn get_function_source_file<'a>(code: &'a Bytecode, func: &Function) -> Option<&'a str> {
    let debug_info = func.debug_info.as_ref()?;
    let debug_files = code.debug_files.as_ref()?;
    let (file_idx, _line) = debug_info.first()?;
    debug_files.get(*file_idx).map(|s| s.as_ref())
}

/// Check if a source file path matches any of the given prefixes
pub fn matches_source_prefix(source_file: Option<&str>, prefixes: &[&str]) -> bool {
    match source_file {
        Some(path) => prefixes.iter().any(|prefix| path.starts_with(prefix)),
        None => false, // No debug info means we can't verify, so exclude
    }
}

/// Check if ANY opcode in the function has a source file matching the given prefixes
/// This handles cases where the first opcode is from inline code (like Debug.hx)
pub fn function_has_source_prefix(code: &Bytecode, func: &Function, prefixes: &[&str]) -> bool {
    let Some(debug_info) = func.debug_info.as_ref() else {
        return false;
    };
    let Some(debug_files) = code.debug_files.as_ref() else {
        return false;
    };

    for (file_idx, _line) in debug_info {
        if let Some(path) = debug_files.get(*file_idx) {
            if prefixes.iter().any(|prefix| path.starts_with(prefix)) {
                return true;
            }
        }
    }
    false
}

/// Result of the substitution operation
#[derive(Debug, Default)]
pub struct SubstitutionResult {
    /// Functions successfully replaced
    pub replaced: Vec<String>,
    /// Functions from source not found in target
    pub not_found: Vec<String>,
    /// Errors encountered during replacement
    pub errors: Vec<String>,
    /// Warnings (non-fatal issues)
    pub warnings: Vec<String>,
    /// Functions injected as dependencies
    pub injected_functions: Vec<String>,
    /// Native functions that were injected into target
    pub injected_natives: Vec<String>,
    /// Native functions that couldn't be resolved (can't inject natives)
    pub unresolvable_natives: Vec<String>,
    /// Functions skipped due to type layout mismatches (func_name, reason)
    pub skipped_type_mismatch: Vec<(String, String)>,
    /// Type layout mismatches detected
    pub type_mismatches: Vec<TypeMismatchInfo>,
    /// Stdlib functions that couldn't be injected due to Haxe version mismatch
    /// (qualified_name, reason)
    pub stdlib_mismatches: Vec<(String, String)>,
    /// Number of initialization opcodes injected into entry point
    /// for initializing static fields of injected types
    pub injected_init_count: usize,
    /// Types that were injected as new types (e.g., new subclasses)
    pub injected_types: Vec<String>,
}

/// Information about a field type mismatch (same name, different type)
#[derive(Debug, Clone)]
pub struct FieldTypeMismatchInfo {
    pub field_name: String,
    pub source_type: String, // e.g. "null(f64)"
    pub target_type: String, // e.g. "f64"
}

/// Information about a field order mismatch (different field at same position)
#[derive(Debug, Clone)]
pub struct FieldOrderMismatchInfo {
    pub position: usize,       // 0-based field index
    pub source_field: String,  // "fieldName:Type"
    pub target_field: String,  // "fieldName:Type"
}

/// Information about a method signature mismatch with parent class
#[derive(Debug, Clone)]
pub struct MethodSignatureMismatchInfo {
    pub method_name: String,
    pub parent_class: String,
    pub source_signature: String,
    pub target_signature: String,
}

/// Information about a type layout mismatch
#[derive(Debug, Clone)]
pub struct TypeMismatchInfo {
    pub type_name: String,
    pub target_fields: usize,
    pub source_fields: usize,
    pub missing_fields: Vec<String>,
    pub field_type_mismatches: Vec<FieldTypeMismatchInfo>,
    pub field_order_mismatches: Vec<FieldOrderMismatchInfo>,
    pub method_signature_mismatches: Vec<MethodSignatureMismatchInfo>,
}

/// Substitute functions from source bytecode into target bytecode
///
/// # Arguments
/// * `target` - The bytecode to modify
/// * `source` - The bytecode containing replacement functions
/// * `function_names` - Optional list of specific function names to replace.
///                      If None, all matching functions are replaced.
/// * `source_prefixes` - Optional list of source file prefixes to filter by.
///                       Only functions from matching source files are replaced.
/// * `inject_deps` - If true, inject missing function dependencies from source.
///                   If false, log warnings for missing functions (legacy behavior).
///
/// # Returns
/// A result containing lists of replaced, not found, and error functions
pub fn substitute_functions(
    target: &mut Bytecode,
    source: &Bytecode,
    function_names: Option<&[String]>,
    source_prefixes: Option<&[&str]>,
    inject_deps: bool,
) -> SubstitutionResult {
    let mut result = SubstitutionResult::default();

    // Build indexes
    let target_index = FunctionIndex::build(target);
    let source_index = FunctionIndex::build(source);

    // Determine which functions to replace
    let to_replace: Vec<(String, usize)> = if let Some(names) = function_names {
        names
            .iter()
            .filter_map(|name| source_index.find(name).map(|idx| (name.clone(), idx)))
            .collect()
    } else {
        source_index
            .iter()
            .filter(|(_name, idx)| {
                // If source prefixes specified, filter by source file
                // Use function_has_source_prefix to check ANY opcode, not just first
                // (handles cases where first opcode is from inline code like Debug.hx)
                if let Some(prefixes) = source_prefixes {
                    let func = &source.functions[*idx];
                    function_has_source_prefix(source, func, prefixes)
                } else {
                    true
                }
            })
            .map(|(name, idx)| (name.to_string(), idx))
            .collect()
    };

    // Resolve target function indices and filter out not-found functions
    let to_replace: Vec<(String, usize, usize)> = to_replace
        .into_iter()
        .filter_map(|(name, src_func_idx)| {
            match target_index.find(&name) {
                Some(target_func_idx) => Some((name, src_func_idx, target_func_idx)),
                None => {
                    result.not_found.push(name);
                    None
                }
            }
        })
        .collect();

    // Create a single pool merger for all substitutions to share type/global mappings
    let mut merger = PoolMerger::new(target, source, inject_deps);

    // First pass: scan all functions to build complete remap
    for (_name, src_func_idx, _target_func_idx) in &to_replace {
        let src_func = &source.functions[*src_func_idx];
        scan_and_ensure_refs(&mut merger, src_func);
    }

    // Get the remap and collect results
    let mut remap = merger.remap.clone();
    // Copy missing_fields to remap for validation during opcode remapping
    remap.missing_fields = merger.missing_fields.clone();
    let warnings = std::mem::take(&mut merger.warnings);
    let injected_functions = std::mem::take(&mut merger.injected_functions);
    let unresolvable_natives = std::mem::take(&mut merger.unresolvable_natives);
    result.warnings.extend(warnings);
    result.injected_functions.extend(injected_functions);
    result.unresolvable_natives.extend(unresolvable_natives);

    // Collect type mismatches
    let type_mismatches = std::mem::take(&mut merger.type_mismatches);
    for (_src_type_idx, mismatch) in &type_mismatches {
        result.type_mismatches.push(TypeMismatchInfo {
            type_name: mismatch.type_name.clone(),
            target_fields: mismatch.target_field_count,
            source_fields: mismatch.source_field_count,
            missing_fields: mismatch.missing_in_target.clone(),
            field_type_mismatches: mismatch
                .field_type_mismatches
                .iter()
                .map(|ftm| FieldTypeMismatchInfo {
                    field_name: ftm.field_name.clone(),
                    source_type: ftm.source_type.clone(),
                    target_type: ftm.target_type.clone(),
                })
                .collect(),
            field_order_mismatches: mismatch
                .field_order_mismatches
                .iter()
                .map(|fom| FieldOrderMismatchInfo {
                    position: fom.position,
                    source_field: fom.source_field.clone(),
                    target_field: fom.target_field.clone(),
                })
                .collect(),
            method_signature_mismatches: mismatch
                .method_signature_mismatches
                .iter()
                .map(|msm| MethodSignatureMismatchInfo {
                    method_name: msm.method_name.clone(),
                    parent_class: msm.parent_class.clone(),
                    source_signature: msm.source_signature.clone(),
                    target_signature: msm.target_signature.clone(),
                })
                .collect(),
        });
    }

    // Collect stdlib mismatches (fatal errors)
    let stdlib_mismatches = std::mem::take(&mut merger.stdlib_mismatches);
    result.stdlib_mismatches.extend(stdlib_mismatches);

    // Drop the merger to release the mutable borrow on target
    drop(merger);

    // Second pass: apply remaps to each function
    for (name, src_func_idx, target_func_idx) in to_replace {
        let src_func = &source.functions[src_func_idx];

        // Remap the function opcodes (with field index remapping based on register types)
        let remapped_ops: Vec<Opcode> = src_func
            .ops
            .iter()
            .map(|op| remap.remap_opcode_with_regs(op, &src_func.regs))
            .collect();

        let remapped_regs: Vec<RefType> = src_func
            .regs
            .iter()
            .map(|&r| remap.remap_type(r))
            .collect();

        let remapped_assigns = src_func.assigns.as_ref().map(|assigns| {
            assigns
                .iter()
                .map(|(s, p)| (remap.remap_string(*s), *p))
                .collect()
        });

        // Get mutable reference to target function and update it
        let target_func = &mut target.functions[target_func_idx];

        // Keep original findex, name, parent - just replace the body
        target_func.t = remap.remap_type(src_func.t);
        target_func.regs = remapped_regs;
        // Remap debug info: preserve source file/line with ".substituted" suffix on filename
        target_func.debug_info = src_func
            .debug_info
            .as_ref()
            .map(|di| remap.remap_debug_info(di));
        target_func.ops = remapped_ops;
        target_func.assigns = remapped_assigns;

        result.replaced.push(name);
    }

    result
}

/// Scan a function for all pool references and ensure they exist in target
fn scan_and_ensure_refs(merger: &mut PoolMerger, func: &Function) {
    // Ensure function type exists
    merger.ensure_type(func.t);

    // Ensure all register types exist
    for &reg_type in &func.regs {
        merger.ensure_type(reg_type);
    }

    // Ensure all opcode references exist
    for op in &func.ops {
        scan_opcode_refs(merger, op);
    }

    // Ensure assign string references exist
    if let Some(assigns) = &func.assigns {
        for (s, _) in assigns {
            merger.ensure_string(*s);
        }
    }

    // Ensure debug file references exist (for preserving source file info)
    if let Some(debug_info) = &func.debug_info {
        for (file_idx, _line) in debug_info {
            merger.ensure_debug_file(*file_idx);
        }
    }
}

/// Scan an opcode for pool references and ensure they exist in target
fn scan_opcode_refs(merger: &mut PoolMerger, op: &Opcode) {
    match op {
        // Constant pool references
        Opcode::Int { ptr, .. } => {
            merger.ensure_int(*ptr);
        }
        Opcode::Float { ptr, .. } => {
            merger.ensure_float(*ptr);
        }
        Opcode::Bytes { ptr, .. } => {
            merger.ensure_bytes(*ptr);
        }
        Opcode::String { ptr, .. } => {
            merger.ensure_string(*ptr);
        }

        // Function references
        Opcode::Call0 { fun, .. }
        | Opcode::Call1 { fun, .. }
        | Opcode::Call2 { fun, .. }
        | Opcode::Call3 { fun, .. }
        | Opcode::Call4 { fun, .. }
        | Opcode::CallN { fun, .. }
        | Opcode::StaticClosure { fun, .. }
        | Opcode::InstanceClosure { fun, .. } => {
            merger.ensure_fun(*fun);
        }

        // Global references
        Opcode::GetGlobal { global, .. } | Opcode::SetGlobal { global, .. } => {
            merger.ensure_global(*global);
        }

        // Type references
        Opcode::Type { ty, .. } => {
            merger.ensure_type(*ty);
        }

        // Dynamic field access uses strings
        Opcode::DynGet { field, .. } | Opcode::DynSet { field, .. } => {
            merger.ensure_string(*field);
        }

        // Enum construction - enum constructs reference is local to enum type
        // No global pool reference needed for RefEnumConstruct
        Opcode::MakeEnum { .. } | Opcode::EnumAlloc { .. } | Opcode::EnumField { .. } => {
            // These use RefEnumConstruct which is relative to the enum type
            // The type should already be ensured elsewhere
        }

        // All other opcodes don't reference global pools
        _ => {}
    }
}

/// List functions that exist in source and could potentially be substituted
#[deprecated(note = "Use list_matching_functions with patterns instead")]
pub fn list_substitutable_functions(
    target: &Bytecode,
    source: &Bytecode,
    source_prefixes: Option<&[&str]>,
) -> Vec<(String, bool)> {
    let target_index = FunctionIndex::build(target);
    let source_index = FunctionIndex::build(source);

    source_index
        .iter()
        .filter(|(_name, idx)| {
            // If source prefixes specified, filter by source file
            // Use function_has_source_prefix to check ANY opcode, not just first
            if let Some(prefixes) = source_prefixes {
                let func = &source.functions[*idx];
                function_has_source_prefix(source, func, prefixes)
            } else {
                true
            }
        })
        .map(|(name, _)| {
            let exists_in_target = target_index.find(name).is_some();
            (name.to_string(), exists_in_target)
        })
        .collect()
}

/// List functions in source that match any of the given patterns
///
/// Returns a list of (function_name, exists_in_target) pairs.
/// If patterns is empty, lists all functions in source.
/// Patterns prefixed with `@` are stripped before matching (@ is a mode indicator).
pub fn list_matching_functions(
    target: &Bytecode,
    source: &Bytecode,
    patterns: &[&str],
) -> Vec<(String, bool)> {
    let target_index = FunctionIndex::build(target);
    let source_index = FunctionIndex::build(source);

    // Strip @ prefix from patterns for matching
    let stripped_patterns: Vec<&str> = patterns
        .iter()
        .map(|p| strip_type_prefix(p))
        .collect();

    source_index
        .iter()
        .filter(|(name, _)| {
            // If no patterns, match all (for --list without patterns)
            if stripped_patterns.is_empty() {
                // Skip anonymous closures
                *name != "<none>" && !name.is_empty()
            } else {
                stripped_patterns.iter().any(|pattern| matches_pattern(name, pattern))
            }
        })
        .map(|(name, _)| {
            let exists_in_target = target_index.find(name).is_some();
            (name.to_string(), exists_in_target)
        })
        .collect()
}

/// Check if a function can be injected (its parent type's parent exists in target)
fn can_inject_function_type(merger: &PoolMerger, func: &hlbc::types::Function) -> bool {
    let Some(parent_type) = func.parent else { return false };

    let src_obj = match merger.source.get(parent_type) {
        hlbc::types::Type::Obj(obj) | hlbc::types::Type::Struct(obj) => obj,
        _ => return false,
    };

    // Check if this type's parent exists in target
    let Some(super_ref) = src_obj.super_ else { return true }; // Root class OK to inject

    let parent_name = match merger.source.get(super_ref) {
        hlbc::types::Type::Obj(obj) | hlbc::types::Type::Struct(obj) => {
            merger.source.get(obj.name).to_string()
        }
        _ => return false,
    };

    // Look for parent in target
    merger.target.types.iter().any(|t| {
        t.get_type_obj()
            .map(|obj| merger.target.get(obj.name) == parent_name)
            .unwrap_or(false)
    })
}

/// Substitute functions from source bytecode into target bytecode using pattern matching.
///
/// Supports per-pattern type injection via the `@` prefix:
/// - Patterns prefixed with `@` (e.g., `@shader.UberSprite.**`) enable type injection
///   for matching functions. New types with vtables will be created.
/// - Regular patterns (e.g., `h3d.impl.GlDriver.resetStream`) only replace existing
///   functions without any type injection logic.
///
/// # Arguments
/// * `target` - The bytecode to modify
/// * `source` - The bytecode containing replacement functions
/// * `patterns` - List of patterns to match (supports `*` and `**` wildcards).
///                Prefix with `@` to enable type injection for that pattern.
/// * `inject_deps` - If true, inject missing function dependencies from source
/// * `inject_natives` - If true, inject missing native declarations into target
pub fn substitute_by_pattern(
    target: &mut Bytecode,
    source: &Bytecode,
    patterns: &[&str],
    inject_deps: bool,
    inject_natives: bool,
) -> SubstitutionResult {
    let mut result = SubstitutionResult::default();

    // Partition patterns into type-injection vs regular
    let (type_inject_patterns, regular_patterns): (Vec<&str>, Vec<&str>) = patterns
        .iter()
        .partition(|p| is_type_injection_pattern(p));

    // Strip @ prefix from type-injection patterns for matching
    let type_inject_patterns: Vec<&str> = type_inject_patterns
        .iter()
        .map(|p| strip_type_prefix(p))
        .collect();

    // Build indexes
    let target_index = FunctionIndex::build(target);
    let source_index = FunctionIndex::build(source);

    // Helper to check if a name matches a type-injection pattern
    let matches_type_inject = |name: &str| -> bool {
        type_inject_patterns.iter().any(|pattern| matches_pattern(name, pattern))
    };

    // Helper to check if a name matches a regular pattern
    let matches_regular = |name: &str| -> bool {
        regular_patterns.iter().any(|pattern| matches_pattern(name, pattern))
    };

    // Find functions matching any pattern (with @ stripped)
    let all_patterns: Vec<&str> = regular_patterns
        .iter()
        .copied()
        .chain(type_inject_patterns.iter().copied())
        .collect();

    let to_replace: Vec<(String, usize)> = source_index
        .iter()
        .filter(|(name, _)| all_patterns.iter().any(|pattern| matches_pattern(name, pattern)))
        .map(|(name, idx)| (name.to_string(), idx))
        .collect();

    // Create a merger - only enable type injection if we have @-prefixed patterns
    let has_type_inject_patterns = !type_inject_patterns.is_empty();
    let mut merger = if has_type_inject_patterns {
        PoolMerger::with_type_injection(target, source, inject_deps)
    } else {
        if inject_natives {
            PoolMerger::with_native_injection(target, source, inject_deps)
        } else {
            PoolMerger::new(target, source, inject_deps)
        }
    };

    if !inject_natives {
        merger.inject_missing_natives = false;
    }

    // Separate functions into those that exist in target vs source-only
    // Only consider type injection for functions matching @-prefixed patterns
    let mut to_inject: Vec<(String, usize)> = Vec::new();
    let to_replace: Vec<(String, usize, usize)> = to_replace
        .into_iter()
        .filter_map(|(name, src_func_idx)| {
            match target_index.find(&name) {
                Some(target_func_idx) => Some((name, src_func_idx, target_func_idx)),
                None => {
                    // Function not in target - only consider injection if it matched a @-pattern
                    if matches_type_inject(&name) {
                        let src_func = &source.functions[src_func_idx];
                        if can_inject_function_type(&merger, src_func) {
                            to_inject.push((name, src_func_idx));
                        } else {
                            result.not_found.push(format!("{} (parent type not in target)", name));
                        }
                    } else if matches_regular(&name) {
                        // Regular pattern - just report not found
                        result.not_found.push(name);
                    } else {
                        // Matched both patterns somehow, treat as not found
                        result.not_found.push(name);
                    }
                    None
                }
            }
        })
        .collect();

    // First pass: scan all replacement functions to build complete remap
    for (_name, src_func_idx, _target_func_idx) in &to_replace {
        let src_func = &source.functions[*src_func_idx];
        scan_and_ensure_refs(&mut merger, src_func);
    }

    // Second pass: inject source-only functions (this creates their parent types too)
    // Only happens for functions matching @-prefixed patterns
    for (name, src_func_idx) in &to_inject {
        let src_func = &source.functions[*src_func_idx];
        // ensure_fun will inject the function and its parent type if needed
        merger.ensure_fun(src_func.findex);
        result.injected_functions.push(name.clone());
    }

    // Finalize protos AFTER all functions are injected (only if we have type injection)
    if has_type_inject_patterns {
        merger.finalize_injected_types();
    }

    // Collect injected type names
    let injected_type_names = merger.get_injected_type_names();

    // Collect results from merger
    let warnings = std::mem::take(&mut merger.warnings);
    let injected_functions = std::mem::take(&mut merger.injected_functions);
    let injected_natives = std::mem::take(&mut merger.injected_natives);
    let unresolvable_natives = std::mem::take(&mut merger.unresolvable_natives);
    result.warnings.extend(warnings);
    // Merge injected_functions from merger with those we tracked ourselves
    for func in injected_functions {
        if !result.injected_functions.contains(&func) {
            result.injected_functions.push(func);
        }
    }
    result.injected_natives.extend(injected_natives);
    result.unresolvable_natives.extend(unresolvable_natives);
    result.injected_types = injected_type_names;

    // Collect type mismatches
    let type_mismatches = std::mem::take(&mut merger.type_mismatches);
    for (_src_type_idx, mismatch) in &type_mismatches {
        result.type_mismatches.push(TypeMismatchInfo {
            type_name: mismatch.type_name.clone(),
            target_fields: mismatch.target_field_count,
            source_fields: mismatch.source_field_count,
            missing_fields: mismatch.missing_in_target.clone(),
            field_type_mismatches: mismatch
                .field_type_mismatches
                .iter()
                .map(|ftm| FieldTypeMismatchInfo {
                    field_name: ftm.field_name.clone(),
                    source_type: ftm.source_type.clone(),
                    target_type: ftm.target_type.clone(),
                })
                .collect(),
            field_order_mismatches: mismatch
                .field_order_mismatches
                .iter()
                .map(|fom| FieldOrderMismatchInfo {
                    position: fom.position,
                    source_field: fom.source_field.clone(),
                    target_field: fom.target_field.clone(),
                })
                .collect(),
            method_signature_mismatches: mismatch
                .method_signature_mismatches
                .iter()
                .map(|msm| MethodSignatureMismatchInfo {
                    method_name: msm.method_name.clone(),
                    parent_class: msm.parent_class.clone(),
                    source_signature: msm.source_signature.clone(),
                    target_signature: msm.target_signature.clone(),
                })
                .collect(),
        });
    }

    // Collect stdlib mismatches (fatal errors)
    let stdlib_mismatches = std::mem::take(&mut merger.stdlib_mismatches);
    result.stdlib_mismatches.extend(stdlib_mismatches);

    // Extract init code BEFORE dropping merger
    let init_code = if !merger.injected_globals.is_empty() {
        merger.extract_init_code_for_injected_types()
    } else {
        None
    };
    let injected_globals_count = merger.injected_globals.len();

    // Get the final remap (after any new functions/types were ensured)
    let mut remap = merger.remap.clone();
    remap.missing_fields = merger.missing_fields.clone();

    // Drop the merger to release the mutable borrow on target
    drop(merger);

    // Third pass: apply remaps to each replacement function
    for (name, src_func_idx, target_func_idx) in to_replace {
        let src_func = &source.functions[src_func_idx];

        // Remap the function opcodes (with field index remapping based on register types)
        let remapped_ops: Vec<Opcode> = src_func
            .ops
            .iter()
            .map(|op| remap.remap_opcode_with_regs(op, &src_func.regs))
            .collect();

        let remapped_regs: Vec<RefType> = src_func
            .regs
            .iter()
            .map(|&r| remap.remap_type(r))
            .collect();

        let remapped_assigns = src_func.assigns.as_ref().map(|assigns| {
            assigns
                .iter()
                .map(|(s, p)| (remap.remap_string(*s), *p))
                .collect()
        });

        // Get mutable reference to target function
        let target_func = &mut target.functions[target_func_idx];

        // Keep original findex, name, parent - just replace the body
        target_func.t = remap.remap_type(src_func.t);
        target_func.regs = remapped_regs;
        // Remap debug info: preserve source file/line with ".substituted" suffix on filename
        target_func.debug_info = src_func
            .debug_info
            .as_ref()
            .map(|di| remap.remap_debug_info(di));
        target_func.ops = remapped_ops;
        target_func.assigns = remapped_assigns;

        result.replaced.push(name);
    }

    // Fourth pass: inject initialization code for injected types into entry point
    if let Some(init_code) = init_code {
        let init_count = merge::inject_init_into_entrypoint(target, init_code);
        if init_count > 0 {
            result.warnings.push(format!(
                "Injected {} initialization opcode(s) into entry point for {} static type(s)",
                init_count, injected_globals_count
            ));
            result.injected_init_count = init_count;
        }
    }

    result
}
