use std::fs;
use std::io::BufWriter;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use hlbc::{Bytecode, Resolve};

use hl_substitute::merge::format_type;
use hl_substitute::{list_matching_functions, substitute_functions_by_pattern_with_options};

#[derive(Parser, Debug)]
#[command(name = "hl-substitute")]
#[command(about = "Substitute functions in HashLink bytecode with replacements from another bytecode file")]
#[command(after_help = r#"PATTERN SYNTAX:
    Patterns use '.' as segment separator and support wildcards:

    *     matches any single segment
    **    matches zero or more segments

EXAMPLES:
    # Replace a specific function
    hl-substitute target.hl source.hl h3d.impl.GlDriver.clear

    # Replace all methods of a class
    hl-substitute target.hl source.hl "h3d.impl.GlDriver.*"

    # Replace all functions in a package (recursive)
    hl-substitute target.hl source.hl "hxsl.**"

    # Replace multiple patterns
    hl-substitute target.hl source.hl "h3d.impl.GlDriver.*" "hxsl.Linker.*"

    # Dump types from target (for analysis/hxml generation)
    hl-substitute target.hl --dump-types h3d

    # Compare types between target and source
    hl-substitute target.hl source.hl --compare-types h2d
"#)]
struct Args {
    /// Path to the target .hl file to patch
    #[arg(value_name = "TARGET")]
    target: PathBuf,

    /// Path to the source .hl file with replacement functions (optional for --dump-types)
    #[arg(value_name = "SOURCE")]
    source: Option<PathBuf>,

    /// Function patterns to substitute (supports * and ** wildcards)
    #[arg(value_name = "PATTERN")]
    patterns: Vec<String>,

    /// Output file path (defaults to target.patched.hl)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Dry run - show what would be replaced without modifying
    #[arg(long)]
    dry_run: bool,

    /// List functions matching patterns and show which exist in target
    #[arg(long)]
    list: bool,

    /// Dump types from target bytecode with optional prefix filter.
    /// Outputs type names and field counts for hxml generation.
    #[arg(long, value_name = "PREFIX")]
    dump_types: Option<Option<String>>,

    /// Compare types between target and source, showing mismatches.
    /// Use with a prefix to filter (e.g., --compare-types h2d)
    #[arg(long, value_name = "PREFIX")]
    compare_types: Option<Option<String>>,

    /// Generate Haxe extern definitions from target bytecode.
    /// These can be used to compile mod code against the game's types.
    #[arg(long)]
    gen_externs: bool,

    /// Output directory for generated extern files (used with --gen-externs or --gen-hxml)
    #[arg(long)]
    externs_output: Option<PathBuf>,

    /// Filter extern generation to specific type patterns (e.g., "h3d.**")
    #[arg(long = "extern-type", value_name = "PATTERN")]
    extern_types: Vec<String>,

    /// Disable automatic injection of missing function dependencies.
    /// By default, functions called by substituted code are automatically injected
    /// from source if they don't exist in target.
    #[arg(long)]
    no_inject_deps: bool,

    /// Disable automatic injection of missing native function declarations.
    /// By default, SDL/GL natives from source that don't exist in target are injected.
    #[arg(long)]
    no_inject_natives: bool,

    /// Inject new types from source when their parent class exists in target.
    /// Enables adding new subclasses like `shader.UberSprite extends hxsl.Shader`.
    /// This allows injecting methods for types that don't exist in target.
    #[arg(long)]
    inject_new_types: bool,

    /// Verbose output - show warnings and additional details
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Read and parse target bytecode (always required)
    let target_data = fs::read(&args.target)
        .with_context(|| format!("Failed to read target file: {}", args.target.display()))?;

    let mut target = Bytecode::deserialize(&mut target_data.as_slice())
        .with_context(|| "Failed to parse target bytecode")?;

    // Handle --dump-types (doesn't require source)
    if let Some(prefix) = &args.dump_types {
        dump_types(&target, prefix.as_deref());
        return Ok(());
    }

    // Handle --compare-types (requires source)
    if let Some(prefix) = &args.compare_types {
        let source_path = args.source.as_ref()
            .ok_or_else(|| anyhow::anyhow!("--compare-types requires a source file"))?;
        let source_data = fs::read(source_path)
            .with_context(|| format!("Failed to read source file: {}", source_path.display()))?;
        let source = Bytecode::deserialize(&mut source_data.as_slice())
            .with_context(|| "Failed to parse source bytecode")?;
        compare_types(&target, &source, prefix.as_deref());
        return Ok(());
    }

    // Handle --gen-externs (generates extern files from target)
    if args.gen_externs {
        use hlbc::extern_gen::{ExternGenOptions, generate_all_externs, write_externs_to_dir};

        let options = ExternGenOptions {
            type_filter: if args.extern_types.is_empty() {
                None
            } else {
                Some(args.extern_types.clone())
            },
            include_internal: false,
            generate_native_meta: true,
            exclude_stdlib: true, // Exclude stdlib types by default
        };

        let result = generate_all_externs(&target, &options);

        println!("Generated {} extern files:", result.files.len());
        println!("  Classes: {}", result.class_count);
        println!("  Enums: {}", result.enum_count);
        println!("  Abstracts: {}", result.abstract_count);

        if let Some(output_dir) = &args.externs_output {
            write_externs_to_dir(&result, output_dir)?;
            println!("\nWritten to: {}", output_dir.display());
        } else {
            // Print to stdout if no output directory specified
            for (path, content) in &result.files {
                println!("\n=== {} ===", path);
                println!("{}", content);
            }
            eprintln!("\nTip: Use --externs-output <dir> to write files to disk");
        }

        return Ok(());
    }

    // For substitution operations, require source and patterns
    let source_path = args.source.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Source file is required for substitution"))?;

    // Require at least one pattern for substitution
    if args.patterns.is_empty() && !args.list {
        eprintln!("Error: At least one pattern is required");
        eprintln!("Usage: hl-substitute <TARGET> <SOURCE> <PATTERN>...");
        eprintln!();
        eprintln!("Examples:");
        eprintln!("  hl-substitute target.hl source.hl \"h3d.impl.GlDriver.*\"");
        eprintln!("  hl-substitute target.hl source.hl \"hxsl.**\"");
        eprintln!("  hl-substitute target.hl --dump-types h3d");
        std::process::exit(1);
    }

    // Read and parse source bytecode
    let source_data = fs::read(source_path)
        .with_context(|| format!("Failed to read source file: {}", source_path.display()))?;

    let source = Bytecode::deserialize(&mut source_data.as_slice())
        .with_context(|| "Failed to parse source bytecode")?;

    println!("HashLink Function Substitution");
    println!("==============================");
    println!("Target: {} (v{})", args.target.display(), target.version);
    println!("Source: {} (v{})", source_path.display(), source.version);
    if !args.patterns.is_empty() {
        println!("Patterns: {}", args.patterns.join(", "));
    }
    println!();

    let patterns: Vec<&str> = args.patterns.iter().map(|s| s.as_str()).collect();

    // List mode - show what matches the patterns
    if args.list {
        let functions = list_matching_functions(&target, &source, &patterns);
        let matching: Vec<_> = functions.iter().filter(|(_, exists)| *exists).collect();
        let non_matching: Vec<_> = functions.iter().filter(|(_, exists)| !*exists).collect();

        println!("Functions matching patterns: {}", functions.len());
        println!("  Exists in target: {}", matching.len());
        println!("  Not in target: {}", non_matching.len());
        println!();

        if args.verbose || functions.len() <= 50 {
            println!("MATCHING FUNCTIONS:");
            for (name, _) in &matching {
                println!("  + {}", name);
            }
            if !non_matching.is_empty() {
                println!();
                println!("NOT IN TARGET:");
                for (name, _) in &non_matching {
                    println!("  - {}", name);
                }
            }
        } else {
            println!("Use --verbose to see full function list");
        }

        return Ok(());
    }

    // Show what would happen in dry run
    if args.dry_run {
        println!("DRY RUN - No changes will be made");
        println!();

        let functions = list_matching_functions(&target, &source, &patterns);
        let to_replace: Vec<_> = functions.iter().filter(|(_, exists)| *exists).collect();

        println!("Would replace {} functions:", to_replace.len());
        for (name, _) in &to_replace {
            println!("  {}", name);
        }

        return Ok(());
    }

    // Perform substitution (both function and native injection enabled by default)
    let inject_deps = !args.no_inject_deps;
    let inject_natives = !args.no_inject_natives;
    let result = if args.inject_new_types {
        hl_substitute::substitute_functions_by_pattern_with_type_injection(
            &mut target, &source, &patterns, inject_deps, inject_natives
        )
    } else {
        substitute_functions_by_pattern_with_options(
            &mut target, &source, &patterns, inject_deps, inject_natives
        )
    };

    // Report results
    println!("Substitution Results:");
    println!("  Replaced: {}", result.replaced.len());
    for name in &result.replaced {
        println!("    + {}", name);
    }

    if !result.injected_functions.is_empty() {
        println!();
        println!("  Injected dependencies: {}", result.injected_functions.len());
        for name in &result.injected_functions {
            println!("    ^ {}", name);
        }
    }

    if !result.injected_natives.is_empty() {
        println!();
        println!("  Injected natives: {}", result.injected_natives.len());
        for name in &result.injected_natives {
            println!("    @ {}", name);
        }
    }

    if !result.injected_types.is_empty() {
        println!();
        println!("  Injected types: {}", result.injected_types.len());
        for name in &result.injected_types {
            println!("    # {}", name);
        }
    }

    if result.injected_init_count > 0 {
        println!();
        println!("  Entry point init: {} opcode(s) for static field initialization", result.injected_init_count);
    }

    if !result.not_found.is_empty() {
        println!();
        println!("  Not found in target: {}", result.not_found.len());
        for name in &result.not_found {
            println!("    - {}", name);
        }
    }

    if !result.unresolvable_natives.is_empty() {
        println!();
        println!("  Unresolvable natives: {}", result.unresolvable_natives.len());
        for name in &result.unresolvable_natives {
            println!("    ! {}", name);
        }
        println!("    (native injection was disabled via --no-inject-natives)");
    }

    if !result.type_mismatches.is_empty() {
        // Filter out virtual field mismatches that are handled by force-mapping
        // A virtual mismatch is safe when source's fields are a subset of target's
        let filter_safe_virtual_mismatches = |ftm: &hl_substitute::FieldTypeMismatchInfo| -> bool {
            // Check if both are virtuals
            let src_inner = ftm.source_type.strip_prefix("virtual(").and_then(|s| s.strip_suffix(")"));
            let tgt_inner = ftm.target_type.strip_prefix("virtual(").and_then(|s| s.strip_suffix(")"));

            if let (Some(src_fields_str), Some(tgt_fields_str)) = (src_inner, tgt_inner) {
                // Parse field names - empty string means no fields
                let src_fields: std::collections::HashSet<&str> = if src_fields_str.is_empty() {
                    std::collections::HashSet::new()
                } else {
                    src_fields_str.split(',').collect()
                };
                let tgt_fields: std::collections::HashSet<&str> = if tgt_fields_str.is_empty() {
                    std::collections::HashSet::new()
                } else {
                    tgt_fields_str.split(',').collect()
                };

                // Safe if source is a subset of target (force-mapping handles this)
                if src_fields.is_subset(&tgt_fields) {
                    return false; // Filter out - it's safe
                }
            }
            true // Keep - it's a real mismatch
        };

        // Count significant mismatches (those not filtered out)
        let significant_mismatches: Vec<_> = result.type_mismatches.iter()
            .filter(|tm| {
                // Keep if has field count mismatch, field order mismatch,
                // or non-virtual field type mismatches
                tm.source_fields != tm.target_fields
                    || !tm.field_order_mismatches.is_empty()
                    || tm.field_type_mismatches.iter().any(filter_safe_virtual_mismatches)
            })
            .collect();

        if !significant_mismatches.is_empty() {
            println!();
            println!("  Type layout mismatches: {}", significant_mismatches.len());
            for tm in &significant_mismatches {
                println!("    ~ {} (source: {} fields, target: {} fields)",
                         tm.type_name, tm.source_fields, tm.target_fields);
                if args.verbose && !tm.missing_fields.is_empty() {
                    println!("      missing: {}", tm.missing_fields.join(", "));
                }
                // Show non-virtual field type mismatches (filter out safe virtual ones)
                for ftm in tm.field_type_mismatches.iter().filter(|f| filter_safe_virtual_mismatches(f)) {
                    println!("      ! {}: source={}, target={}",
                             ftm.field_name, ftm.source_type, ftm.target_type);
                }
                // Show field order mismatches - field indices won't match
                for fom in &tm.field_order_mismatches {
                    println!("      ! field {}: source={}, target={}",
                             fom.position, fom.source_field, fom.target_field);
                }
            }
            println!("    NOTE: Type mismatches may cause runtime errors.");
            println!("    Use --compare-types to see details.");
        } else if args.verbose {
            println!();
            println!("  Type layout mismatches: {} (all handled by force-mapping)", result.type_mismatches.len());
        }
    }

    if !result.skipped_type_mismatch.is_empty() {
        println!();
        println!("  Skipped (type mismatch): {}", result.skipped_type_mismatch.len());
        for (name, reason) in &result.skipped_type_mismatch {
            println!("    ~ {}: {}", name, reason);
        }
    }

    if !result.stdlib_mismatches.is_empty() {
        println!();
        println!("  STDLIB VERSION MISMATCH: {}", result.stdlib_mismatches.len());
        println!("    Source was compiled with a different Haxe version than target.");
        println!("    The following stdlib functions cannot be safely injected:");
        println!();
        for (name, reason) in &result.stdlib_mismatches {
            println!("    ✗ {}", name);
            println!("      {}", reason);
        }
        println!();
        println!("    To fix: Recompile your source with the same Haxe version as the target.");
        println!("    Dead Cells appears to use Haxe ~4.0-4.1 (has Std.is instead of Std.isOfType).");
    }

    if !result.errors.is_empty() {
        println!();
        println!("  Errors: {}", result.errors.len());
        for err in &result.errors {
            println!("    ! {}", err);
        }
    }

    if args.verbose && !result.warnings.is_empty() {
        println!();
        println!("  Warnings: {}", result.warnings.len());
        for warn in &result.warnings {
            println!("    ? {}", warn);
        }
    }

    // Save if there were no errors (stdlib mismatches are fatal)
    let has_fatal_errors = !result.errors.is_empty() || !result.stdlib_mismatches.is_empty();
    if !has_fatal_errors && !result.replaced.is_empty() {
        let output = args.output.unwrap_or_else(|| {
            let stem = args.target.file_stem().unwrap().to_str().unwrap();
            args.target.with_file_name(format!("{}.patched.hl", stem))
        });

        let out_file = fs::File::create(&output)
            .with_context(|| format!("Failed to create output file: {}", output.display()))?;
        let mut writer = BufWriter::new(out_file);

        target
            .serialize(&mut writer)
            .with_context(|| "Failed to serialize patched bytecode")?;

        println!();
        println!("Written to: {}", output.display());
    } else if result.replaced.is_empty() {
        println!();
        println!("No functions were replaced - no output written");
    } else {
        println!();
        if !result.stdlib_mismatches.is_empty() {
            println!("FATAL: Stdlib version mismatch prevents safe substitution - no output written");
        } else {
            println!("Errors occurred - no output written");
        }
        std::process::exit(1);
    }

    Ok(())
}

use hlbc::types::Type;
use std::collections::BTreeMap;

/// Check if a type name matches the prefix filter
fn matches_prefix(name: &str, prefix: Option<&str>) -> bool {
    match prefix {
        Some(p) => name.starts_with(p),
        None => true,
    }
}

/// Get type name from a Type
fn get_type_name(code: &Bytecode, ty: &Type) -> Option<String> {
    match ty {
        Type::Obj(obj) | Type::Struct(obj) => Some(code.get(obj.name).to_string()),
        Type::Enum { name, .. } => Some(code.get(*name).to_string()),
        _ => None,
    }
}

/// Get field count for a type
fn get_field_count(ty: &Type) -> usize {
    match ty {
        Type::Obj(obj) | Type::Struct(obj) => obj.fields.len(),
        Type::Enum { constructs, .. } => constructs.len(),
        _ => 0,
    }
}

/// Get field names for a type
fn get_field_names(code: &Bytecode, ty: &Type) -> Vec<String> {
    match ty {
        Type::Obj(obj) | Type::Struct(obj) => {
            obj.fields.iter().map(|f| code.get(f.name).to_string()).collect()
        }
        Type::Enum { constructs, .. } => {
            constructs.iter().map(|c| code.get(c.name).to_string()).collect()
        }
        _ => vec![],
    }
}

/// Get field names with their types for a type (for type comparison)
fn get_field_types(code: &Bytecode, ty: &Type) -> Vec<(String, String)> {
    match ty {
        Type::Obj(obj) | Type::Struct(obj) => {
            obj.fields.iter().map(|f| {
                (code.get(f.name).to_string(), format_type(code, f.t))
            }).collect()
        }
        _ => vec![],
    }
}

/// Check if two type strings are equivalent virtuals (same fields, different order)
/// Virtual types are looked up by field name, so order doesn't matter
fn are_equivalent_virtual_strings(type_a: &str, type_b: &str) -> bool {
    // Both must be virtual types
    let (Some(inner_a), Some(inner_b)) = (
        type_a.strip_prefix("virtual(").and_then(|s| s.strip_suffix(")")),
        type_b.strip_prefix("virtual(").and_then(|s| s.strip_suffix(")")),
    ) else {
        return false;
    };
    // Compare field names as sets
    let fields_a: std::collections::HashSet<&str> = inner_a.split(',').collect();
    let fields_b: std::collections::HashSet<&str> = inner_b.split(',').collect();
    fields_a == fields_b
}

/// Dump types from bytecode with optional prefix filter
fn dump_types(code: &Bytecode, prefix: Option<&str>) {
    let mut types_info: BTreeMap<String, (usize, Vec<String>, &'static str)> = BTreeMap::new();

    for ty in code.types.iter() {
        let name = match get_type_name(code, ty) {
            Some(n) => n,
            None => continue,
        };

        if !matches_prefix(&name, prefix) {
            continue;
        }

        let field_count = get_field_count(ty);
        let fields = get_field_names(code, ty);
        let kind = match ty {
            Type::Obj(_) => "class",
            Type::Struct(_) => "struct",
            Type::Enum { .. } => "enum",
            _ => "other",
        };

        types_info.insert(name, (field_count, fields, kind));
    }

    let filter_desc = prefix.unwrap_or("all");
    println!("# Type dump (filter: {})", filter_desc);
    println!("# Total types: {}", types_info.len());
    println!();

    for (name, (count, fields, kind)) in &types_info {
        if fields.len() <= 5 {
            println!("{} ({}) = {} ({})", name, kind, count, fields.join(", "));
        } else {
            println!("{} ({}) = {}", name, kind, count);
        }
    }
}

/// Compare types between target and source bytecode
fn compare_types(target: &Bytecode, source: &Bytecode, prefix: Option<&str>) {
    // Build type info maps for both
    // (field_count, field_names, field_types_map)
    let mut target_types: BTreeMap<String, (usize, Vec<String>, std::collections::HashMap<String, String>)> = BTreeMap::new();
    let mut source_types: BTreeMap<String, (usize, Vec<String>, std::collections::HashMap<String, String>)> = BTreeMap::new();

    for ty in target.types.iter() {
        if let Some(name) = get_type_name(target, ty) {
            if matches_prefix(&name, prefix) {
                let field_types: std::collections::HashMap<_, _> = get_field_types(target, ty).into_iter().collect();
                target_types.insert(name, (get_field_count(ty), get_field_names(target, ty), field_types));
            }
        }
    }

    for ty in source.types.iter() {
        if let Some(name) = get_type_name(source, ty) {
            if matches_prefix(&name, prefix) {
                let field_types: std::collections::HashMap<_, _> = get_field_types(source, ty).into_iter().collect();
                source_types.insert(name, (get_field_count(ty), get_field_names(source, ty), field_types));
            }
        }
    }

    let filter_desc = prefix.unwrap_or("all");
    println!("# Type comparison (filter: {})", filter_desc);
    println!("# Target types: {}, Source types: {}", target_types.len(), source_types.len());
    println!();

    let mut mismatches = 0;
    let mut field_type_mismatches = 0;
    let mut field_order_mismatches = 0;
    let mut missing_in_source = 0;
    let mut extra_in_source = 0;

    // Check for mismatches and missing types
    for (name, (target_count, target_fields, target_field_types)) in &target_types {
        match source_types.get(name) {
            Some((source_count, source_fields, source_field_types)) => {
                let mut has_field_count_mismatch = false;
                let mut field_type_diffs: Vec<(String, String, String)> = Vec::new();
                let mut field_order_diffs: Vec<(usize, String, String, String, String)> = Vec::new();

                if target_count != source_count {
                    has_field_count_mismatch = true;
                }

                // Check for field type mismatches on matching field names
                // Skip equivalent virtuals (same fields, different order - order doesn't matter for virtuals)
                for field_name in target_fields.iter() {
                    if let (Some(target_type), Some(source_type)) =
                        (target_field_types.get(field_name), source_field_types.get(field_name))
                    {
                        if target_type != source_type
                            && !are_equivalent_virtual_strings(target_type, source_type)
                        {
                            field_type_diffs.push((field_name.clone(), target_type.clone(), source_type.clone()));
                        }
                    }
                }

                // Check for field ORDER mismatches when counts match
                if target_count == source_count {
                    for (pos, (target_name, source_name)) in
                        target_fields.iter().zip(source_fields.iter()).enumerate()
                    {
                        if target_name != source_name {
                            let target_type = target_field_types.get(target_name)
                                .map(|s| s.as_str()).unwrap_or("?");
                            let source_type = source_field_types.get(source_name)
                                .map(|s| s.as_str()).unwrap_or("?");
                            field_order_diffs.push((
                                pos,
                                target_name.clone(),
                                target_type.to_string(),
                                source_name.clone(),
                                source_type.to_string(),
                            ));
                        }
                    }
                }

                if has_field_count_mismatch || !field_type_diffs.is_empty() || !field_order_diffs.is_empty() {
                    if has_field_count_mismatch {
                        println!("MISMATCH: {} - target has {} fields, source has {}",
                                 name, target_count, source_count);
                        mismatches += 1;
                    } else if !field_order_diffs.is_empty() {
                        println!("FIELD ORDER MISMATCH: {} ({} fields)", name, target_count);
                    } else {
                        println!("FIELD TYPE MISMATCH: {} ({} fields)", name, target_count);
                    }

                    if has_field_count_mismatch {
                        // Show field name differences
                        let target_set: std::collections::HashSet<_> = target_fields.iter().collect();
                        let source_set: std::collections::HashSet<_> = source_fields.iter().collect();

                        for f in source_set.difference(&target_set) {
                            println!("  + source has: {}", f);
                        }
                        for f in target_set.difference(&source_set) {
                            println!("  - target has: {}", f);
                        }
                    }

                    // Show field type differences
                    for (field_name, target_type, source_type) in &field_type_diffs {
                        println!("  ! {}: target={}, source={}", field_name, target_type, source_type);
                        field_type_mismatches += 1;
                    }

                    // Show field order differences
                    for (pos, target_name, target_type, source_name, source_type) in &field_order_diffs {
                        println!("  ! field {}: target={}:{}, source={}:{}",
                                 pos, target_name, target_type, source_name, source_type);
                        field_order_mismatches += 1;
                    }
                    println!();
                }
            }
            None => {
                println!("MISSING IN SOURCE: {} ({} fields)", name, target_count);
                missing_in_source += 1;
            }
        }
    }

    // Check for types only in source
    for (name, (source_count, _, _)) in &source_types {
        if !target_types.contains_key(name) {
            println!("EXTRA IN SOURCE: {} ({} fields)", name, source_count);
            extra_in_source += 1;
        }
    }

    println!();
    println!("Summary:");
    println!("  Field count mismatches: {}", mismatches);
    println!("  Field type mismatches: {}", field_type_mismatches);
    println!("  Field order mismatches: {}", field_order_mismatches);
    println!("  Missing in source: {}", missing_in_source);
    println!("  Extra in source: {}", extra_in_source);

    if mismatches > 0 || field_type_mismatches > 0 || field_order_mismatches > 0 {
        println!();
        println!("WARNING: Type mismatches will cause substitution failures!");
        println!("Recompile your source library with matching type layouts.");
    }
}
