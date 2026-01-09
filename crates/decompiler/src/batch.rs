//! Batch decompilation to files with index preservation.
//!
//! This module provides functionality to decompile an entire bytecode file
//! to a directory of Haxe source files, with numerical indices preserved
//! as comments for debugging and bytecode manipulation.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::panic;
use std::path::Path;

use hlbc::types::Type;
use hlbc::Bytecode;

use crate::fmt::FormatOptions;
use crate::{decompile_class, decompile_function};

/// Index file containing mappings from names to bytecode indices.
#[cfg(feature = "batch")]
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct IndexFile {
    /// Type name -> type index
    pub types: HashMap<String, usize>,
    /// Qualified function name (Type::method) -> function index
    pub functions: HashMap<String, usize>,
    /// String literal -> string index (only commonly used strings)
    pub strings: HashMap<String, usize>,
    /// Global name -> global index
    pub globals: HashMap<String, usize>,
}

#[cfg(feature = "batch")]
impl IndexFile {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Options for batch decompilation.
pub struct BatchOptions {
    /// Include patterns (e.g., "h2d.**", "libs.**")
    pub include: Vec<String>,
    /// Exclude patterns (e.g., "haxe.**", "sys.**")
    pub exclude: Vec<String>,
    /// Show progress during decompilation
    pub verbose: bool,
}

impl Default for BatchOptions {
    fn default() -> Self {
        Self {
            include: vec![],
            exclude: vec![],
            verbose: false,
        }
    }
}

impl BatchOptions {
    /// Check if a type name should be included based on patterns.
    pub fn should_include(&self, name: &str) -> bool {
        // If no include patterns, include everything
        let included = if self.include.is_empty() {
            true
        } else {
            self.include.iter().any(|p| matches_pattern(name, p))
        };

        // Check exclusions
        let excluded = self.exclude.iter().any(|p| matches_pattern(name, p));

        included && !excluded
    }
}

/// Simple glob-style pattern matching.
fn matches_pattern(name: &str, pattern: &str) -> bool {
    if pattern.ends_with(".**") {
        // Match package and all subpackages
        let prefix = &pattern[..pattern.len() - 3];
        name.starts_with(prefix)
    } else if pattern.ends_with(".*") {
        // Match only direct children
        let prefix = &pattern[..pattern.len() - 2];
        name.starts_with(prefix) && !name[prefix.len()..].contains('.')
    } else {
        // Exact match
        name == pattern
    }
}

/// Batch decompiler that outputs all types to files.
pub struct BatchDecompiler<'a> {
    code: &'a Bytecode,
    opts: FormatOptions,
    batch_opts: BatchOptions,
}

impl<'a> BatchDecompiler<'a> {
    /// Create a new batch decompiler with default options.
    pub fn new(code: &'a Bytecode) -> Self {
        Self {
            code,
            opts: FormatOptions::with_fun_indices(2),
            batch_opts: BatchOptions::default(),
        }
    }

    /// Create a new batch decompiler with custom options.
    pub fn with_options(code: &'a Bytecode, batch_opts: BatchOptions) -> Self {
        Self {
            code,
            opts: FormatOptions::with_fun_indices(2),
            batch_opts,
        }
    }

    /// Decompile all types to the output directory.
    #[cfg(feature = "batch")]
    pub fn decompile_all(&self, output_dir: &Path) -> io::Result<IndexFile> {
        let mut index = IndexFile::new();
        let mut count = 0;
        let total = self.code.types.iter().filter(|t| t.get_type_obj().is_some()).count();

        // Create output directory
        fs::create_dir_all(output_dir)?;

        // Process all object types (classes)
        for (type_idx, ty) in self.code.types.iter().enumerate() {
            if let Some(obj) = ty.get_type_obj() {
                let name = obj.name(self.code).to_string();

                // Check if we should process this type
                if !self.batch_opts.should_include(&name) {
                    continue;
                }

                let path = type_name_to_path(&name);
                let full_path = output_dir.join(&path);

                // Create parent directories
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent)?;
                }

                // Decompile and write (with panic recovery)
                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    let class = decompile_class(self.code, obj);
                    let display = class.display_with_index(self.code, &self.opts, Some(type_idx));
                    let s = display.to_string();
                    s
                }));

                let content = match result {
                    Ok(content) => content,
                    Err(_) => {
                        // Decompilation panicked, write a stub file
                        if self.batch_opts.verbose {
                            eprintln!("  WARNING: decompilation failed, writing stub");
                        }
                        format!(
                            "// type@{}\n// Decompilation failed for {}\n// Reason: internal decompiler error\n\nclass {} {{\n    // Could not decompile\n}}\n",
                            type_idx, name, name.split('.').last().unwrap_or(&name)
                        )
                    }
                };
                fs::write(&full_path, &content)?;

                // Record in index
                index.types.insert(name.clone(), type_idx);

                // Record functions
                for proto in &obj.protos {
                    let fun_name = format!("{}::{}", name, proto.name(self.code));
                    index.functions.insert(fun_name, proto.findex.0);
                }
                for (_, findex) in &obj.bindings {
                    if let Some(fun) = findex.as_fn(self.code) {
                        let fun_name = format!("{}::{}", name, fun.name(self.code));
                        index.functions.insert(fun_name, findex.0);
                    }
                }

                count += 1;
                if self.batch_opts.verbose {
                    eprintln!("[{}/{}] {}", count, total, name);
                }
            }
        }

        // Write standalone functions (not part of a class)
        self.write_standalone_functions(output_dir, &mut index)?;

        // Write natives
        self.write_natives(output_dir)?;

        // Write globals
        self.write_globals(output_dir, &mut index)?;

        // Write index file
        let index_path = output_dir.join("_index.json");
        let index_json = serde_json::to_string_pretty(&index)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(index_path, index_json)?;

        if self.batch_opts.verbose {
            eprintln!("Decompiled {} types", count);
        }

        Ok(index)
    }

    /// Write standalone functions that aren't part of any class.
    #[cfg(feature = "batch")]
    fn write_standalone_functions(&self, output_dir: &Path, index: &mut IndexFile) -> io::Result<()> {
        let mut standalone = String::new();
        standalone.push_str("// Standalone functions (not part of any class)\n\n");

        for (fun_idx, fun) in self.code.functions.iter().enumerate() {
            // Skip functions that are part of a type
            if fun.is_method() {
                continue;
            }

            let name = fun.name(self.code).to_string();
            if name.starts_with("$") || name == "<none>" {
                continue;
            }

            let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                let method = decompile_function(self.code, fun);
                let display = method.display(self.code, &self.opts);
                let s = display.to_string();
                s
            }));

            standalone.push_str(&format!("// fun@{}\n", fun_idx));
            match result {
                Ok(content) => standalone.push_str(&content),
                Err(_) => standalone.push_str(&format!("// Decompilation failed for {}\n", name)),
            }
            standalone.push_str("\n");

            index.functions.insert(name.clone(), fun_idx);
        }

        let path = output_dir.join("_standalone.hx");
        fs::write(path, standalone)?;
        Ok(())
    }

    /// Write native function declarations.
    fn write_natives(&self, output_dir: &Path) -> io::Result<()> {
        let mut natives = String::new();
        natives.push_str("// Native function declarations\n\n");

        for (idx, native) in self.code.natives.iter().enumerate() {
            natives.push_str(&format!(
                "// native@{}\n@:native(\"{}\") extern function {};\n\n",
                idx,
                native.lib(self.code),
                native.name(self.code)
            ));
        }

        let path = output_dir.join("_natives.hx");
        fs::write(path, natives)?;
        Ok(())
    }

    /// Write global variables.
    #[cfg(feature = "batch")]
    fn write_globals(&self, output_dir: &Path, index: &mut IndexFile) -> io::Result<()> {
        let mut globals = String::new();
        globals.push_str("// Global variables\n\n");

        for (idx, global) in self.code.globals.iter().enumerate() {
            let ty_name = match &self.code.types[global.0] {
                Type::Obj(obj) => obj.name(self.code).to_string(),
                Type::Struct(obj) => obj.name(self.code).to_string(),
                _ => format!("type@{}", global.0),
            };
            globals.push_str(&format!("// global@{}\nvar global_{}: {};\n\n", idx, idx, ty_name));
            index.globals.insert(format!("global_{}", idx), idx);
        }

        let path = output_dir.join("_globals.hx");
        fs::write(path, globals)?;
        Ok(())
    }
}

/// Convert a type name to a file path.
fn type_name_to_path(name: &str) -> String {
    // "h2d.SpriteBatch" -> "h2d/SpriteBatch.hx"
    // Handle nested types with $ separator
    let clean_name = name.replace('$', "_");
    clean_name.replace('.', "/") + ".hx"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_pattern() {
        assert!(matches_pattern("h2d.SpriteBatch", "h2d.**"));
        assert!(matches_pattern("h2d.sub.Thing", "h2d.**"));
        assert!(!matches_pattern("h3d.Buffer", "h2d.**"));

        assert!(matches_pattern("h2d.SpriteBatch", "h2d.*"));
        assert!(!matches_pattern("h2d.sub.Thing", "h2d.*"));

        assert!(matches_pattern("h2d.SpriteBatch", "h2d.SpriteBatch"));
        assert!(!matches_pattern("h2d.Drawable", "h2d.SpriteBatch"));
    }

    #[test]
    fn test_type_name_to_path() {
        assert_eq!(type_name_to_path("h2d.SpriteBatch"), "h2d/SpriteBatch.hx");
        assert_eq!(type_name_to_path("libs.heaps.slib.HSprite"), "libs/heaps/slib/HSprite.hx");
        assert_eq!(type_name_to_path("h2d.$SpriteBatch"), "h2d/_SpriteBatch.hx");
    }
}
