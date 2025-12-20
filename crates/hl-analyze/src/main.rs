use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use glob::glob;
use goblin::Object;
use hlbc::opcodes::Opcode;
use hlbc::types::{FunPtr, RefFun};
use hlbc::{Bytecode, Resolve};

#[derive(Parser, Debug)]
#[command(name = "hl-analyze")]
#[command(about = "Analyze HashLink bytecode for native function compatibility")]
struct Args {
    /// Path to the .hl or hlboot.dat file to analyze
    #[arg(value_name = "FILE")]
    hl_file: PathBuf,

    /// Path to HashLink build directory containing .hdll files
    #[arg(long, value_name = "DIR")]
    hl_path: Option<PathBuf>,

    /// Show all natives, not just missing ones
    #[arg(long, short = 'a')]
    all: bool,

    /// Include unreachable (dead) code in analysis
    #[arg(long)]
    include_dead: bool,
}

/// Information about a native function reference in bytecode
#[derive(Debug)]
struct NativeRef {
    lib: String,
    name: String,
    signature: String,
    is_optional: bool,
}

/// Scan a shared library for exported hlp_* symbols
fn get_hdll_exports(path: &Path) -> Result<Vec<String>> {
    let data = fs::read(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let mut exports = Vec::new();

    match Object::parse(&data)? {
        Object::Elf(elf) => {
            for sym in elf.dynsyms.iter() {
                if sym.st_bind() == goblin::elf::sym::STB_GLOBAL
                    || sym.st_bind() == goblin::elf::sym::STB_WEAK
                {
                    if let Some(name) = elf.dynstrtab.get_at(sym.st_name) {
                        if let Some(func_name) = name.strip_prefix("hlp_") {
                            exports.push(func_name.to_string());
                        }
                    }
                }
            }
        }
        Object::Mach(mach) => {
            match mach {
                goblin::mach::Mach::Binary(macho) => {
                    for sym in macho.exports()? {
                        if let Some(func_name) = sym.name.strip_prefix("_hlp_") {
                            exports.push(func_name.to_string());
                        }
                    }
                }
                goblin::mach::Mach::Fat(_) => {
                    // Skip fat binaries for now
                }
            }
        }
        Object::PE(pe) => {
            for export in pe.exports {
                if let Some(name) = export.name {
                    if let Some(func_name) = name.strip_prefix("hlp_") {
                        exports.push(func_name.to_string());
                    }
                }
            }
        }
        _ => {}
    }

    Ok(exports)
}

/// Scan a directory for .hdll files and build a map of available natives
fn scan_hdll_directory(dir: &Path) -> Result<HashMap<String, HashSet<String>>> {
    let mut available: HashMap<String, HashSet<String>> = HashMap::new();

    // Scan for .hdll files
    let pattern = dir.join("*.hdll").to_string_lossy().to_string();
    for entry in glob(&pattern)? {
        if let Ok(path) = entry {
            let lib_name = path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            // Strip "64" suffix if present (e.g., "fmt64" -> "fmt")
            let lib_name = lib_name.strip_suffix("64").unwrap_or(&lib_name).to_string();

            match get_hdll_exports(&path) {
                Ok(exports) => {
                    let entry = available.entry(lib_name).or_default();
                    for export in exports {
                        entry.insert(export);
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to scan {}: {}", path.display(), e);
                }
            }
        }
    }

    // Scan for libhl.so / libhl.dll (contains "std" primitives)
    for name in &["libhl.so", "libhl64.so", "libhl.dll", "libhl64.dll"] {
        let path = dir.join(name);
        if path.exists() {
            match get_hdll_exports(&path) {
                Ok(exports) => {
                    let entry = available.entry("std".to_string()).or_default();
                    for export in exports {
                        entry.insert(export);
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to scan {}: {}", path.display(), e);
                }
            }
        }
    }

    Ok(available)
}

/// Format a type for display
fn format_type(t: &hlbc::types::Type, code: &Bytecode) -> String {
    use hlbc::types::Type;

    match t {
        Type::Void => "void".to_string(),
        Type::UI8 => "u8".to_string(),
        Type::UI16 => "u16".to_string(),
        Type::I32 => "i32".to_string(),
        Type::I64 => "i64".to_string(),
        Type::F32 => "f32".to_string(),
        Type::F64 => "f64".to_string(),
        Type::Bool => "bool".to_string(),
        Type::Bytes => "bytes".to_string(),
        Type::Dyn => "dyn".to_string(),
        Type::Fun(fun) | Type::Method(fun) => {
            let args: Vec<String> = fun.args.iter()
                .map(|a| format_type(code.get(*a), code))
                .collect();
            let ret = format_type(code.get(fun.ret), code);
            format!("({}) -> {}", args.join(", "), ret)
        }
        Type::Obj(obj) => {
            code.get(obj.name).to_string()
        }
        Type::Array => "array".to_string(),
        Type::Type => "type".to_string(),
        Type::Ref(inner) => format!("ref<{}>", format_type(code.get(*inner), code)),
        Type::Virtual { .. } => "virtual".to_string(),
        Type::DynObj => "dynobj".to_string(),
        Type::Abstract { name } => format!("abstract<{}>", code.get(*name)),
        Type::Enum { name, .. } => format!("enum<{}>", code.get(*name)),
        Type::Null(inner) => format!("null<{}>", format_type(code.get(*inner), code)),
        Type::Struct(obj) => {
            format!("struct<{}>", code.get(obj.name))
        }
        Type::Packed(inner) => format!("packed<{}>", format_type(code.get(*inner), code)),
    }
}

/// Find all functions reachable from the entrypoint via call graph analysis.
/// Returns a set of RefFun indices that are reachable.
fn find_reachable_functions(code: &Bytecode) -> HashSet<usize> {
    let mut visited: HashSet<usize> = HashSet::new();
    let mut worklist: Vec<RefFun> = vec![code.entrypoint];

    while let Some(fref) = worklist.pop() {
        let idx = fref.0;
        if visited.contains(&idx) {
            continue;
        }
        visited.insert(idx);

        // Get the function and find all calls within it
        match code.get(fref) {
            FunPtr::Fun(func) => {
                // Scan all opcodes for function calls
                for op in &func.ops {
                    let called_funs = extract_called_functions(op);
                    for called in called_funs {
                        if !visited.contains(&called.0) {
                            worklist.push(called);
                        }
                    }
                }
            }
            FunPtr::Native(_) => {
                // Natives don't call other functions in bytecode
            }
        }
    }

    visited
}

/// Extract function references from an opcode
fn extract_called_functions(op: &Opcode) -> Vec<RefFun> {
    match op {
        Opcode::Call0 { fun, .. } => vec![*fun],
        Opcode::Call1 { fun, .. } => vec![*fun],
        Opcode::Call2 { fun, .. } => vec![*fun],
        Opcode::Call3 { fun, .. } => vec![*fun],
        Opcode::Call4 { fun, .. } => vec![*fun],
        Opcode::CallN { fun, .. } => vec![*fun],
        Opcode::StaticClosure { fun, .. } => vec![*fun],
        Opcode::InstanceClosure { fun, .. } => vec![*fun],
        // VirtualClosure, CallMethod, CallThis use field indices resolved at runtime
        // We'd need type analysis to resolve these statically
        _ => vec![],
    }
}

/// Check if a RefFun is a native function
fn is_native(code: &Bytecode, fref: RefFun) -> bool {
    matches!(code.get(fref), FunPtr::Native(_))
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Read and parse bytecode
    let data = fs::read(&args.hl_file)
        .with_context(|| format!("Failed to read {}", args.hl_file.display()))?;

    let code = Bytecode::deserialize(&mut data.as_slice())
        .with_context(|| "Failed to parse bytecode")?;

    // Determine HL path
    let hl_path = args.hl_path.unwrap_or_else(|| {
        // Default to common locations
        if Path::new("./bin").exists() {
            PathBuf::from("./bin")
        } else if Path::new("/run/media/bmd/dev/hashlink/build/bin").exists() {
            PathBuf::from("/run/media/bmd/dev/hashlink/build/bin")
        } else {
            PathBuf::from(".")
        }
    });

    // Scan available natives
    let available = scan_hdll_directory(&hl_path)?;

    // Find reachable functions (unless --include-dead is set)
    let reachable = if args.include_dead {
        None
    } else {
        Some(find_reachable_functions(&code))
    };

    // Extract natives from bytecode (filtered by reachability)
    let mut natives: Vec<NativeRef> = Vec::new();
    let mut total_natives = 0;
    let mut unreachable_natives = 0;

    for native in &code.natives {
        total_natives += 1;

        // Check if this native is reachable
        let is_reachable = reachable
            .as_ref()
            .map(|r| r.contains(&native.findex.0))
            .unwrap_or(true);

        if !is_reachable {
            unreachable_natives += 1;
            continue;
        }

        let lib_raw: String = code.get(native.lib).to_string();
        let is_optional = lib_raw.starts_with('?');
        let lib = lib_raw.trim_start_matches('?').to_string();
        let name: String = code.get(native.name).to_string();
        let signature = format_type(code.get(native.t), &code);

        natives.push(NativeRef {
            lib,
            name,
            signature,
            is_optional,
        });
    }

    // Categorize natives
    let mut missing: Vec<&NativeRef> = Vec::new();
    let mut optional_missing: Vec<&NativeRef> = Vec::new();
    let mut found: Vec<&NativeRef> = Vec::new();

    for native in &natives {
        let lib_available = available.get(&native.lib);
        let is_found = lib_available
            .map(|funcs| funcs.contains(&native.name))
            .unwrap_or(false);

        if is_found {
            found.push(native);
        } else if native.is_optional {
            optional_missing.push(native);
        } else {
            missing.push(native);
        }
    }

    // Print report
    println!("HashLink Native Compatibility Report");
    println!("====================================");
    println!("Game: {} (version {})", args.hl_file.display(), code.version);
    println!("HashLink: {}", hl_path.display());
    println!();

    if reachable.is_some() {
        let reachable_set = reachable.as_ref().unwrap();
        let reachable_funcs = reachable_set.iter()
            .filter(|&idx| matches!(code.get(RefFun(*idx)), FunPtr::Fun(_)))
            .count();
        let total_funcs = code.functions.len();
        println!("Reachability Analysis:");
        println!("  Functions: {} / {} reachable ({:.1}% dead code)",
            reachable_funcs, total_funcs,
            (1.0 - reachable_funcs as f64 / total_funcs as f64) * 100.0);
        println!("  Natives: {} / {} reachable ({} unreachable)",
            natives.len(), total_natives, unreachable_natives);
        println!();
    }

    println!("Reachable natives: {}", natives.len());
    println!("Available in HL: {}", found.len());
    println!("Missing: {}", missing.len());
    println!("Optional missing: {}", optional_missing.len());
    println!();

    // List available modules
    println!("AVAILABLE MODULES:");
    let mut modules: Vec<_> = available.keys().collect();
    modules.sort();
    for module in &modules {
        let count = available.get(*module).map(|s| s.len()).unwrap_or(0);
        println!("  {} ({} functions)", module, count);
    }
    println!();

    // Show missing natives
    if !missing.is_empty() {
        println!("MISSING NATIVES ({}): [WILL FAIL]", missing.len());
        // Group by library
        let mut by_lib: HashMap<&str, Vec<&&NativeRef>> = HashMap::new();
        for native in &missing {
            by_lib.entry(&native.lib).or_default().push(native);
        }
        let mut libs: Vec<_> = by_lib.keys().collect();
        libs.sort();
        for lib in libs {
            println!("  [{}]", lib);
            for native in by_lib.get(lib).unwrap() {
                println!("    {}  {}", native.name, native.signature);
            }
        }
        println!();
    }

    // Show optional missing
    if !optional_missing.is_empty() {
        println!("OPTIONAL MISSING ({}): [may be OK]", optional_missing.len());
        let mut by_lib: HashMap<&str, Vec<&&NativeRef>> = HashMap::new();
        for native in &optional_missing {
            by_lib.entry(&native.lib).or_default().push(native);
        }
        let mut libs: Vec<_> = by_lib.keys().collect();
        libs.sort();
        for lib in libs {
            println!("  [?{}]", lib);
            for native in by_lib.get(lib).unwrap() {
                println!("    {}  {}", native.name, native.signature);
            }
        }
        println!();
    }

    // Show all found if requested
    if args.all && !found.is_empty() {
        println!("FOUND NATIVES ({}):", found.len());
        let mut by_lib: HashMap<&str, Vec<&&NativeRef>> = HashMap::new();
        for native in &found {
            by_lib.entry(&native.lib).or_default().push(native);
        }
        let mut libs: Vec<_> = by_lib.keys().collect();
        libs.sort();
        for lib in libs {
            println!("  [{}]", lib);
            for native in by_lib.get(lib).unwrap() {
                println!("    {}  {}", native.name, native.signature);
            }
        }
    }

    // Exit with error if there are missing required natives
    if !missing.is_empty() {
        std::process::exit(1);
    }

    Ok(())
}
