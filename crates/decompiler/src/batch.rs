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
use std::sync::atomic::{AtomicUsize, Ordering};

use hlbc::types::Type;
use hlbc::{Bytecode, Resolve};

#[cfg(feature = "batch")]
use rayon::prelude::*;

use crate::fmt::FormatOptions;
use crate::closure_analysis::ClosureAnalysis;
use crate::{decompile_class_with_closures, decompile_function_with_closures, extract_static_initializers, StaticInitMap};

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
    /// Include metadata files (_natives.hx, _globals.hx, _standalone.hx)
    pub include_metadata: bool,
    /// Number of parallel jobs (None = use all CPUs)
    pub jobs: Option<usize>,
}

impl Default for BatchOptions {
    fn default() -> Self {
        Self {
            include: vec![],
            exclude: vec![],
            verbose: false,
            include_metadata: false,
            jobs: None,
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
    if pattern == "_*.**" {
        // Special pattern to match types with underscore-prefixed segments
        // "_*.**" should match "_Xml.Foo", "h3d.scene._Graphics.GPoint", etc.
        name.starts_with('_') || name.contains("._")
    } else if pattern.ends_with(".**") {
        // Match package and all subpackages
        // pattern "h2d.**" should match "h2d.Foo" and "h2d.sub.Bar"
        let prefix = &pattern[..pattern.len() - 2]; // Keep the dot: "h2d."
        name.starts_with(prefix)
    } else if pattern.ends_with(".*") {
        // Match only direct children (no nested packages)
        // pattern "h2d.*" should match "h2d.Foo" but not "h2d.sub.Bar"
        let prefix = &pattern[..pattern.len() - 1]; // Keep the dot: "h2d."
        name.starts_with(prefix) && !name[prefix.len()..].contains('.')
    } else if pattern == "$*" {
        // Special pattern to match all types containing $ (internal static holders)
        // This catches both top-level $String and nested h3d.scene.$Skin
        name.contains('$')
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
    static_inits: StaticInitMap,
    closure_analysis: ClosureAnalysis,
}

impl<'a> BatchDecompiler<'a> {
    /// Create a new batch decompiler with default options.
    pub fn new(code: &'a Bytecode) -> Self {
        Self {
            code,
            opts: FormatOptions::with_fun_indices(2),
            batch_opts: BatchOptions::default(),
            static_inits: extract_static_initializers(code),
            closure_analysis: ClosureAnalysis::analyze(code),
        }
    }

    /// Create a new batch decompiler with custom options.
    pub fn with_options(code: &'a Bytecode, batch_opts: BatchOptions) -> Self {
        Self {
            code,
            opts: FormatOptions::with_fun_indices(2),
            batch_opts,
            static_inits: extract_static_initializers(code),
            closure_analysis: ClosureAnalysis::analyze(code),
        }
    }

    /// Decompile all types to the output directory.
    #[cfg(feature = "batch")]
    pub fn decompile_all(&self, output_dir: &Path) -> io::Result<IndexFile> {
        // Configure rayon thread pool with larger stack size for deeply nested AST
        // (some functions like level.LevelStruct.get have 100+ nested if statements)
        let pool_builder = rayon::ThreadPoolBuilder::new()
            .stack_size(16 * 1024 * 1024); // 16MB stack per thread

        let pool_builder = if let Some(jobs) = self.batch_opts.jobs {
            pool_builder.num_threads(jobs)
        } else {
            pool_builder
        };

        pool_builder.build_global().ok(); // Ignore error if pool already initialized

        let mut index = IndexFile::new();

        // Create output directory
        fs::create_dir_all(output_dir)?;

        // Build map of nested types (parent_name -> vec of nested types)
        let nested_type_map = build_nested_type_map(self.code);

        // Collect types to decompile, excluding nested types (they'll be included in their parent)
        let types_to_decompile: Vec<_> = self.code.types.iter().enumerate()
            .filter_map(|(type_idx, ty)| {
                ty.get_type_obj().and_then(|obj| {
                    let name = obj.name(self.code).to_string();
                    // Skip nested types - they will be included in their parent class
                    if parse_nested_type(&name).is_some() {
                        return None;
                    }
                    if self.batch_opts.should_include(&name) {
                        Some((type_idx, obj, name))
                    } else {
                        None
                    }
                })
            })
            .collect();

        let total = types_to_decompile.len();
        let count = AtomicUsize::new(0);

        // Create all parent directories upfront (sequential)
        for (_, _, name) in &types_to_decompile {
            let path = type_name_to_path(name);
            let full_path = output_dir.join(&path);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent)?;
            }
        }

        // Decompile in parallel
        let results: Vec<_> = types_to_decompile.par_iter()
            .map(|(type_idx, obj, name)| {
                let path = type_name_to_path(name);
                let full_path = output_dir.join(&path);

                // Decompile with panic recovery
                let static_inits = &self.static_inits;
                let closure_analysis = &self.closure_analysis;
                let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                    let class = decompile_class_with_closures(self.code, *obj, static_inits, Some(closure_analysis));
                    let display = class.display_with_index(self.code, &self.opts, Some(*type_idx));
                    let mut content = display.to_string();

                    // Insert any nested types BEFORE the main class (Haxe module-private style)
                    if let Some(nested_types) = nested_type_map.get(name) {
                        // Collect all nested class content
                        let mut nested_content_all = String::new();

                        for (nested_name, nested_idx, nested_obj) in nested_types {
                            // Skip $-prefixed types (static type holders)
                            if nested_name.starts_with('$') {
                                continue;
                            }

                            // Decompile nested type
                            let nested_class = decompile_class_with_closures(
                                self.code, *nested_obj, static_inits, Some(closure_analysis)
                            );
                            let nested_display = nested_class.display_with_index(
                                self.code, &self.opts, Some(*nested_idx)
                            );
                            let nested_content = nested_display.to_string();

                            // Strip package declaration from nested class
                            // (it will have "package _ClassName;" which is invalid)
                            let nested_content = if let Some(class_start) = nested_content.find("class ") {
                                &nested_content[class_start..]
                            } else {
                                &nested_content
                            };

                            // Add "private" modifier for module-private class
                            let nested_content = format!("private {}", nested_content);

                            nested_content_all.push_str(&nested_content);
                            nested_content_all.push_str("\n");
                        }

                        // Insert nested classes BEFORE the main class declaration
                        // Find where the main class starts (after package/imports)
                        if !nested_content_all.is_empty() {
                            if let Some(class_pos) = content.find("\nclass ").or_else(|| content.find("class ")) {
                                // Adjust position to be at start of line
                                let insert_pos = if content[..class_pos].ends_with('\n') {
                                    class_pos
                                } else if let Some(newline_pos) = content[..class_pos].rfind('\n') {
                                    newline_pos + 1
                                } else {
                                    class_pos
                                };
                                content.insert_str(insert_pos, &nested_content_all);
                            }
                        }
                    }

                    content
                }));

                let content = match result {
                    Ok(content) => content,
                    Err(_) => {
                        format!(
                            "// type@{}\n// Decompilation failed for {}\n// Reason: internal decompiler error\n\nclass {} {{\n    // Could not decompile\n}}\n",
                            type_idx, name, name.split('.').last().unwrap_or(name)
                        )
                    }
                };

                // Write file
                let _ = fs::write(&full_path, &content);

                // Progress
                if self.batch_opts.verbose {
                    let c = count.fetch_add(1, Ordering::Relaxed) + 1;
                    eprintln!("[{}/{}] {}", c, total, name);
                }

                // Return index data
                let mut funcs = Vec::new();
                for proto in &obj.protos {
                    let fun_name = format!("{}::{}", name, proto.name(self.code));
                    funcs.push((fun_name, proto.findex.0));
                }
                for (_, findex) in &obj.bindings {
                    if let Some(fun) = findex.as_fn(self.code) {
                        let fun_name = format!("{}::{}", name, fun.name(self.code));
                        funcs.push((fun_name, findex.0));
                    }
                }

                (name.clone(), *type_idx, funcs)
            })
            .collect();

        // Merge results into index (sequential)
        for (name, type_idx, funcs) in results {
            index.types.insert(name, type_idx);
            for (fun_name, fun_idx) in funcs {
                index.functions.insert(fun_name, fun_idx);
            }
        }

        // Write enum types
        self.write_enums(output_dir, &mut index)?;

        // Extract embedded bytes/resources
        self.write_resources(output_dir)?;

        // Write metadata files only if requested (they can break recompilation)
        if self.batch_opts.include_metadata {
            // Write standalone functions (not part of a class)
            self.write_standalone_functions(output_dir, &mut index)?;

            // Write natives
            self.write_natives(output_dir)?;

            // Write globals
            self.write_globals(output_dir, &mut index)?;
        }

        // Write index file
        let index_path = output_dir.join("_index.json");
        let index_json = serde_json::to_string_pretty(&index)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(index_path, index_json)?;

        if self.batch_opts.verbose {
            eprintln!("Decompiled {} types", count.load(Ordering::Relaxed));
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

            let closure_analysis = &self.closure_analysis;
            let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                let method = decompile_function_with_closures(self.code, fun, Some(closure_analysis));
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

    /// Write enum type definitions.
    #[cfg(feature = "batch")]
    fn write_enums(&self, output_dir: &Path, index: &mut IndexFile) -> io::Result<()> {
        for (type_idx, ty) in self.code.types.iter().enumerate() {
            if let Type::Enum { name, constructs, .. } = ty {
                let enum_name = self.code.get(*name).to_string();

                // Check if we should process this type
                if !self.batch_opts.should_include(&enum_name) {
                    continue;
                }

                // Skip anonymous closure enums (names starting with $)
                if enum_name.starts_with('$') {
                    continue;
                }

                let path = type_name_to_path(&enum_name);
                let full_path = output_dir.join(&path);

                // Create parent directories
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent)?;
                }

                // Generate enum content
                let mut content = String::new();

                // Package declaration
                if let Some(pos) = enum_name.rfind('.') {
                    content.push_str(&format!("package {};\n\n", &enum_name[..pos]));
                }

                // Type index comment
                content.push_str(&format!("// type@{}\n", type_idx));

                // Enum declaration
                let simple_name = enum_name.rsplit('.').next().unwrap_or(&enum_name);
                content.push_str(&format!("enum {} {{\n", simple_name));

                // Enum constructors
                for construct in constructs {
                    let construct_name = self.code.get(construct.name);
                    if construct.params.is_empty() {
                        content.push_str(&format!("    {};\n", construct_name));
                    } else {
                        // Constructor with parameters
                        content.push_str(&format!("    {}(", construct_name));
                        for (i, param) in construct.params.iter().enumerate() {
                            if i > 0 {
                                content.push_str(", ");
                            }
                            // Parameter name: param0, param1, etc. (bytecode doesn't have names)
                            content.push_str(&format!("param{}: Dynamic", i));
                            let _ = param; // Acknowledge param type (could be used for better typing)
                        }
                        content.push_str(");\n");
                    }
                }

                content.push_str("}\n");

                fs::write(&full_path, &content)?;

                // Record in index
                index.types.insert(enum_name.clone(), type_idx);
            }
        }
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

    /// Extract embedded bytes constants to files.
    ///
    /// Creates a `resources/` subdirectory and saves each bytes constant
    /// as a binary file. Also generates a `_resources.txt` manifest that
    /// can be used for recompilation with `-resource` flags.
    fn write_resources(&self, output_dir: &Path) -> io::Result<()> {
        // Check if there are any bytes constants
        let (data, offsets) = match &self.code.bytes {
            Some((data, offsets)) if !offsets.is_empty() => (data, offsets),
            _ => return Ok(()), // No bytes to extract
        };

        // Create resources directory
        let resources_dir = output_dir.join("resources");
        fs::create_dir_all(&resources_dir)?;

        let mut manifest = String::new();
        manifest.push_str("# Embedded bytes resources extracted from bytecode\n");
        manifest.push_str("# Format: index filename size\n");
        manifest.push_str("# To recompile, add -resource flags for each resource\n\n");

        for (idx, &start) in offsets.iter().enumerate() {
            // Calculate end position (next offset or data length)
            let end = offsets.get(idx + 1).copied().unwrap_or(data.len());
            let mut bytes_slice = &data[start..end];

            // Strip trailing null terminator if present
            // HashLink adds a null terminator when storing resources, but the
            // original file size (without null) is stored in ResourceContent.dataLen.
            // To round-trip correctly, we need to extract the original content.
            if bytes_slice.last() == Some(&0) {
                bytes_slice = &bytes_slice[..bytes_slice.len() - 1];
            }

            // Save as binary file
            let filename = format!("bytes_{}.bin", idx);
            let file_path = resources_dir.join(&filename);
            fs::write(&file_path, bytes_slice)?;

            // Add to manifest
            manifest.push_str(&format!("{} {} {}\n", idx, filename, bytes_slice.len()));

            if self.batch_opts.verbose {
                eprintln!("Extracted bytes@{}: {} bytes", idx, bytes_slice.len());
            }
        }

        // Write manifest
        let manifest_path = output_dir.join("_resources.txt");
        fs::write(manifest_path, manifest)?;

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

/// Check if a type name represents a nested type (e.g., "_Parent.Nested" or "pkg._Parent.Nested").
/// Returns Some((parent_name, nested_name)) if it's a nested type, None otherwise.
fn parse_nested_type(name: &str) -> Option<(String, String)> {
    // Pattern: something._Parent.Nested or _Parent.Nested
    // The underscore-prefixed segment is the module container for nested types

    // Find "._" pattern indicating a nested type container
    if let Some(pos) = name.find("._") {
        // Find the end of the underscore-prefixed segment
        let after_underscore = pos + 2; // skip "._"
        if let Some(dot_pos) = name[after_underscore..].find('.') {
            // We have "pkg._Container.NestedType"
            let prefix = &name[..pos]; // "pkg"
            let container = &name[after_underscore..after_underscore + dot_pos]; // "Container"
            let nested = &name[after_underscore + dot_pos + 1..]; // "NestedType"

            // Parent is "pkg.Container", nested is "NestedType"
            let parent = if prefix.is_empty() {
                container.to_string()
            } else {
                format!("{}.{}", prefix, container)
            };
            return Some((parent, nested.to_string()));
        }
    }

    // Also handle top-level underscore prefix: "_Parent.Nested"
    if name.starts_with('_') {
        if let Some(dot_pos) = name.find('.') {
            let container = &name[1..dot_pos]; // Skip leading underscore
            let nested = &name[dot_pos + 1..];
            return Some((container.to_string(), nested.to_string()));
        }
    }

    None
}

/// Build a map from parent type names to their nested types.
/// Returns a HashMap where keys are parent type names and values are lists of (nested_name, type_index, TypeObj).
fn build_nested_type_map<'a>(
    code: &'a Bytecode,
) -> HashMap<String, Vec<(String, usize, &'a hlbc::types::TypeObj)>> {
    let mut map: HashMap<String, Vec<_>> = HashMap::new();

    for (type_idx, ty) in code.types.iter().enumerate() {
        if let Some(obj) = ty.get_type_obj() {
            let name = obj.name(code).to_string();
            if let Some((parent_name, nested_name)) = parse_nested_type(&name) {
                map.entry(parent_name)
                    .or_default()
                    .push((nested_name, type_idx, obj));
            }
        }
    }

    map
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
