use std::fs;
use std::io::BufWriter;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use hlbc::{Bytecode, Resolve};

use hl_substitute::{list_matching_functions, substitute_functions_by_pattern};

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

    /// Generate an hxml file that compiles source library to match target's type layouts.
    /// Analyzes both files and outputs hxml with --macro keep() for matching types.
    #[arg(long)]
    gen_hxml: bool,

    /// Library name for generated hxml (default: heaps)
    #[arg(long, default_value = "heaps")]
    hxml_lib: String,

    /// Output .hl filename for generated hxml (default: library.hl)
    #[arg(long)]
    hxml_output: Option<String>,

    /// Output path for generated Dummy.hx file (used with --gen-hxml)
    #[arg(long)]
    dummy_output: Option<PathBuf>,

    /// Disable automatic injection of missing function dependencies.
    /// By default, functions called by substituted code are automatically injected
    /// from source if they don't exist in target.
    #[arg(long)]
    no_inject_deps: bool,

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

    // Handle --gen-hxml (requires source)
    if args.gen_hxml {
        let source_path = args.source.as_ref()
            .ok_or_else(|| anyhow::anyhow!("--gen-hxml requires a source file"))?;
        let source_data = fs::read(source_path)
            .with_context(|| format!("Failed to read source file: {}", source_path.display()))?;
        let source = Bytecode::deserialize(&mut source_data.as_slice())
            .with_context(|| "Failed to parse source bytecode")?;

        let hl_output = args.hxml_output.unwrap_or_else(|| format!("{}.hl", args.hxml_lib));
        generate_hxml(&target, &source, &args.hxml_lib, &hl_output, args.dummy_output.as_ref())?;
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

    // Perform substitution (injection is enabled by default)
    let inject_deps = !args.no_inject_deps;
    let result = substitute_functions_by_pattern(&mut target, &source, &patterns, inject_deps);

    // Report results
    println!("Substitution Results:");
    println!("  Replaced: {}", result.replaced.len());
    if args.verbose || result.replaced.len() <= 20 {
        for name in &result.replaced {
            println!("    + {}", name);
        }
    } else {
        for name in result.replaced.iter().take(10) {
            println!("    + {}", name);
        }
        println!("    ... and {} more", result.replaced.len() - 10);
    }

    if !result.injected_functions.is_empty() {
        println!();
        println!("  Injected dependencies: {}", result.injected_functions.len());
        if args.verbose || result.injected_functions.len() <= 10 {
            for name in &result.injected_functions {
                println!("    ^ {}", name);
            }
        } else {
            for name in result.injected_functions.iter().take(10) {
                println!("    ^ {}", name);
            }
            println!("    ... and {} more", result.injected_functions.len() - 10);
        }
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
        println!("    (natives are external bindings and cannot be injected)");
    }

    if !result.type_mismatches.is_empty() {
        println!();
        println!("  Type layout mismatches: {}", result.type_mismatches.len());
        for tm in &result.type_mismatches {
            println!("    ~ {} (source: {} fields, target: {} fields)",
                     tm.type_name, tm.source_fields, tm.target_fields);
            if args.verbose && !tm.missing_fields.is_empty() {
                println!("      missing: {}", tm.missing_fields.join(", "));
            }
        }
        println!("    NOTE: Type mismatches may cause runtime errors.");
        println!("    Use --compare-types to see details.");
    }

    if !result.skipped_type_mismatch.is_empty() {
        println!();
        println!("  Skipped (type mismatch): {}", result.skipped_type_mismatch.len());
        for (name, reason) in &result.skipped_type_mismatch {
            println!("    ~ {}: {}", name, reason);
        }
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

    // Save if there were no errors
    if result.errors.is_empty() && !result.replaced.is_empty() {
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
        println!("Errors occurred - no output written");
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

/// Check if a type name is likely to be publicly accessible
/// Used to filter Dummy.hx generation to avoid compilation errors
fn is_public_type(name: &str) -> bool {
    // Skip inner classes like h3d.impl._GlDriver.CompiledProgram
    if name.contains("._") {
        return false;
    }

    // Skip abstract implementations like h2d.col._Point.Point_Impl_
    if name.contains("_Impl_") {
        return false;
    }

    // Get the class name (last segment)
    let class_name = name.rsplit('.').next().unwrap_or(name);

    // Skip classes starting with underscore
    if class_name.starts_with('_') {
        return false;
    }

    // Known internal/problematic types in heaps that aren't publicly accessible
    // These are typically structs, typedefs, or private inner types
    const KNOWN_INTERNAL: &[&str] = &[
        "h2d.FontChar",
        "h2d.Kerning",
        "h3d.anim.LinearFrame",
        "h3d.prim.UV",
        "hxd.clipper.Rect",
    ];

    if KNOWN_INTERNAL.contains(&name) {
        return false;
    }

    true
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

/// Generate an hxml file that compiles source library to match target's type layouts
fn generate_hxml(target: &Bytecode, source: &Bytecode, lib_name: &str, hl_output: &str, dummy_output: Option<&PathBuf>) -> Result<()> {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Write;

    // Collect types from target (what we need to match)
    let mut target_types: BTreeMap<String, usize> = BTreeMap::new();
    for ty in target.types.iter() {
        if let Some(name) = get_type_name(target, ty) {
            // Skip internal/compiler-generated types
            if name.starts_with("$") || name.contains("$") || name.starts_with("_") {
                continue;
            }
            target_types.insert(name, get_field_count(ty));
        }
    }

    // Collect types from source
    let mut source_types: BTreeMap<String, usize> = BTreeMap::new();
    for ty in source.types.iter() {
        if let Some(name) = get_type_name(source, ty) {
            if name.starts_with("$") || name.contains("$") || name.starts_with("_") {
                continue;
            }
            source_types.insert(name, get_field_count(ty));
        }
    }

    // Find types that exist in both (these are what we want to keep)
    let mut matching_types: BTreeSet<String> = BTreeSet::new();
    let mut mismatched_types: Vec<(String, usize, usize)> = Vec::new();

    for (name, target_fields) in &target_types {
        if let Some(&source_fields) = source_types.get(name) {
            matching_types.insert(name.clone());
            if *target_fields != source_fields {
                mismatched_types.push((name.clone(), *target_fields, source_fields));
            }
        }
    }

    // Filter to only library types (h3d, h2d, hxsl, hxd for heaps)
    let lib_prefixes: Vec<&str> = if lib_name == "heaps" {
        vec!["h3d.", "h2d.", "hxsl.", "hxd."]
    } else {
        vec![] // For other libs, include all
    };

    let lib_types: BTreeSet<String> = if lib_prefixes.is_empty() {
        matching_types.clone()
    } else {
        matching_types
            .iter()
            .filter(|name| lib_prefixes.iter().any(|p| name.starts_with(p)))
            .cloned()
            .collect()
    };

    // Generate hxml header
    println!("# Generated hxml for matching target type layouts");
    println!("# Target: {} types, Source: {} types", target_types.len(), source_types.len());
    println!("# Library types to reference: {}", lib_types.len());
    if !mismatched_types.is_empty() {
        let lib_mismatches: Vec<_> = mismatched_types
            .iter()
            .filter(|(name, _, _)| lib_prefixes.is_empty() || lib_prefixes.iter().any(|p| name.starts_with(p)))
            .collect();
        if !lib_mismatches.is_empty() {
            println!("# WARNING: {} library types have field count mismatches", lib_mismatches.len());
            println!("# These indicate the source was compiled differently than target:");
            for (name, target_fields, source_fields) in lib_mismatches {
                println!("#   {} (target: {}, source: {})", name, target_fields, source_fields);
            }
        }
    }
    println!();

    // Library dependencies
    println!("-lib {}", lib_name);
    if lib_name == "heaps" {
        println!("-lib hlsdl");
        println!("-lib hlopenal");
    }
    println!();
    println!("-D hl-ver=1.15.0");
    println!();

    // Main class that references all the types
    println!("# Compile the dummy file that references all needed types");
    println!("Dummy");
    println!();
    println!("-hl {}", hl_output);

    // Print summary to stderr
    eprintln!();
    eprintln!("Generated hxml referencing {} library types", lib_types.len());

    // Build Dummy.hx content
    let mut dummy_content = String::new();
    dummy_content.push_str("// Auto-generated file to reference types from target bytecode\n");
    dummy_content.push_str("// This ensures the compiled library has matching type layouts\n");
    dummy_content.push_str("// Using Class<T> references forces compile-time type inclusion\n");
    dummy_content.push('\n');
    dummy_content.push_str("class Dummy {\n");

    // Reference each type using Class<T> to force compile-time inclusion
    let mut idx = 0;
    let mut skipped = Vec::new();
    for name in &lib_types {
        if !is_public_type(name) {
            skipped.push(name.clone());
            continue;
        }
        // Use Class<T> which forces compile-time type resolution
        dummy_content.push_str(&format!("    static var _{}: Class<{}>;\n", idx, name));
        idx += 1;
    }

    if !skipped.is_empty() {
        dummy_content.push('\n');
        dummy_content.push_str(&format!("    // Skipped {} types (internal/private):\n", skipped.len()));
        for name in skipped.iter().take(10) {
            dummy_content.push_str(&format!("    // - {}\n", name));
        }
        if skipped.len() > 10 {
            dummy_content.push_str(&format!("    // ... and {} more\n", skipped.len() - 10));
        }
    }
    dummy_content.push('\n');
    dummy_content.push_str("    public static function main() {}\n");
    dummy_content.push_str("}\n");

    // Write Dummy.hx to file or print to stderr
    if let Some(path) = dummy_output {
        let mut file = fs::File::create(path)
            .with_context(|| format!("Failed to create Dummy.hx file: {}", path.display()))?;
        file.write_all(dummy_content.as_bytes())
            .with_context(|| format!("Failed to write Dummy.hx file: {}", path.display()))?;
        eprintln!("Written Dummy.hx to: {}", path.display());
    } else {
        eprintln!();
        eprintln!("=== Dummy.hx ===");
        eprint!("{}", dummy_content);
        eprintln!("=== End Dummy.hx ===");
        eprintln!();
        eprintln!("Save the Dummy.hx content above to your library directory, then run:");
        eprintln!("  haxe <generated.hxml>");
    }

    Ok(())
}

/// Compare types between target and source bytecode
fn compare_types(target: &Bytecode, source: &Bytecode, prefix: Option<&str>) {
    // Build type info maps for both
    let mut target_types: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    let mut source_types: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();

    for ty in target.types.iter() {
        if let Some(name) = get_type_name(target, ty) {
            if matches_prefix(&name, prefix) {
                target_types.insert(name, (get_field_count(ty), get_field_names(target, ty)));
            }
        }
    }

    for ty in source.types.iter() {
        if let Some(name) = get_type_name(source, ty) {
            if matches_prefix(&name, prefix) {
                source_types.insert(name, (get_field_count(ty), get_field_names(source, ty)));
            }
        }
    }

    let filter_desc = prefix.unwrap_or("all");
    println!("# Type comparison (filter: {})", filter_desc);
    println!("# Target types: {}, Source types: {}", target_types.len(), source_types.len());
    println!();

    let mut mismatches = 0;
    let mut missing_in_source = 0;
    let mut extra_in_source = 0;

    // Check for mismatches and missing types
    for (name, (target_count, target_fields)) in &target_types {
        match source_types.get(name) {
            Some((source_count, source_fields)) => {
                if target_count != source_count {
                    println!("MISMATCH: {} - target has {} fields, source has {}",
                             name, target_count, source_count);

                    // Show field differences
                    let target_set: std::collections::HashSet<_> = target_fields.iter().collect();
                    let source_set: std::collections::HashSet<_> = source_fields.iter().collect();

                    for f in source_set.difference(&target_set) {
                        println!("  + source has: {}", f);
                    }
                    for f in target_set.difference(&source_set) {
                        println!("  - target has: {}", f);
                    }
                    println!();
                    mismatches += 1;
                }
            }
            None => {
                println!("MISSING IN SOURCE: {} ({} fields)", name, target_count);
                missing_in_source += 1;
            }
        }
    }

    // Check for types only in source
    for (name, (source_count, _)) in &source_types {
        if !target_types.contains_key(name) {
            println!("EXTRA IN SOURCE: {} ({} fields)", name, source_count);
            extra_in_source += 1;
        }
    }

    println!();
    println!("Summary:");
    println!("  Mismatches: {}", mismatches);
    println!("  Missing in source: {}", missing_in_source);
    println!("  Extra in source: {}", extra_in_source);

    if mismatches > 0 {
        println!();
        println!("WARNING: Type mismatches will cause substitution failures!");
        println!("Recompile your source library with matching type layouts.");
    }
}
