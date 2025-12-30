//! Generate Haxe extern definitions from HashLink bytecode.
//!
//! This module provides functionality to generate compilable Haxe extern files
//! from bytecode, enabling modding of games without source code access.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::Path;

use crate::types::{EnumConstruct, FunPtr, ObjField, ObjProto, RefType, Type, TypeFun, TypeObj};
use crate::{Bytecode, Resolve};

/// Options for extern generation
#[derive(Debug, Clone)]
pub struct ExternGenOptions {
    /// Filter to only include types matching these patterns
    pub type_filter: Option<Vec<String>>,
    /// Include private/internal types (starting with _ or containing $)
    pub include_internal: bool,
    /// Generate @:native metadata for renamed types
    pub generate_native_meta: bool,
}

impl Default for ExternGenOptions {
    fn default() -> Self {
        Self {
            type_filter: None,
            include_internal: false,
            generate_native_meta: true,
        }
    }
}

/// Result of extern generation
#[derive(Debug, Default)]
pub struct ExternGenResult {
    /// Map of package path -> file content
    /// e.g., "h3d/impl/GlDriver.hx" -> "package h3d.impl;\n\nextern class GlDriver { ... }"
    pub files: BTreeMap<String, String>,
    /// Types that were skipped
    pub skipped: Vec<(String, String)>, // (type_name, reason)
    /// Statistics
    pub class_count: usize,
    pub enum_count: usize,
    pub abstract_count: usize,
}

/// Maximum recursion depth for type formatting
const MAX_TYPE_DEPTH: usize = 10;

/// Convert a HashLink type to its Haxe representation
pub fn hl_type_to_haxe(ty: &Type, code: &Bytecode) -> String {
    hl_type_to_haxe_depth(ty, code, 0)
}

/// Convert a HashLink type to its Haxe representation with depth tracking
fn hl_type_to_haxe_depth(ty: &Type, code: &Bytecode, depth: usize) -> String {
    if depth > MAX_TYPE_DEPTH {
        return "Dynamic".to_string(); // Bail out on deeply nested types
    }

    match ty {
        Type::Void => "Void".to_string(),
        Type::UI8 => "hl.UI8".to_string(),
        Type::UI16 => "hl.UI16".to_string(),
        Type::I32 => "Int".to_string(),
        Type::I64 => "haxe.Int64".to_string(),
        Type::F32 => "Single".to_string(),
        Type::F64 => "Float".to_string(),
        Type::Bool => "Bool".to_string(),
        Type::Bytes => "hl.Bytes".to_string(),
        Type::Dyn => "Dynamic".to_string(),
        Type::Array => "hl.NativeArray<Dynamic>".to_string(),
        Type::Type => "hl.Type".to_string(),
        Type::DynObj => "Dynamic".to_string(),

        Type::Fun(fun) | Type::Method(fun) => format_function_type_depth(fun, code, depth + 1),

        Type::Obj(obj) => format_type_name(&code.get(obj.name)),
        Type::Struct(obj) => format_type_name(&code.get(obj.name)),

        Type::Ref(inner) => format!("hl.Ref<{}>", hl_type_to_haxe_depth(&code[*inner], code, depth + 1)),
        Type::Null(inner) => format!("Null<{}>", hl_type_to_haxe_depth(&code[*inner], code, depth + 1)),
        Type::Packed(inner) => hl_type_to_haxe_depth(&code[*inner], code, depth + 1),

        Type::Virtual { fields } => format_virtual_type_depth(fields, code, depth + 1),

        Type::Abstract { name } => format_type_name(&code.get(*name)),

        Type::Enum { name, .. } => format_type_name(&code.get(*name)),
    }
}

/// Convert a RefType to Haxe type string
pub fn ref_type_to_haxe(ref_type: RefType, code: &Bytecode) -> String {
    hl_type_to_haxe_depth(code.get(ref_type), code, 0)
}

/// Convert a RefType to Haxe type string with depth tracking
fn ref_type_to_haxe_depth(ref_type: RefType, code: &Bytecode, depth: usize) -> String {
    hl_type_to_haxe_depth(code.get(ref_type), code, depth)
}

/// Format a function type as Haxe
fn format_function_type_depth(fun: &TypeFun, code: &Bytecode, depth: usize) -> String {
    if depth > MAX_TYPE_DEPTH {
        return "Dynamic".to_string();
    }

    let args: Vec<String> = fun.args.iter()
        .map(|a| ref_type_to_haxe_depth(*a, code, depth + 1))
        .collect();

    let ret = ref_type_to_haxe_depth(fun.ret, code, depth + 1);

    if args.is_empty() {
        format!("Void -> {}", ret)
    } else {
        format!("{} -> {}", args.join(" -> "), ret)
    }
}

/// Format a virtual type (anonymous structure)
fn format_virtual_type_depth(fields: &[ObjField], code: &Bytecode, depth: usize) -> String {
    if fields.is_empty() {
        return "{}".to_string();
    }

    if depth > MAX_TYPE_DEPTH {
        return "Dynamic".to_string();
    }

    let field_strs: Vec<String> = fields.iter()
        .map(|f| {
            let name = code.get(f.name);
            let ty = ref_type_to_haxe_depth(f.t, code, depth + 1);
            format!("{}: {}", name, ty)
        })
        .collect();

    format!("{{ {} }}", field_strs.join(", "))
}

/// Format a type name, handling packages and special characters
fn format_type_name(name: &str) -> String {
    // Handle empty or special names
    if name.is_empty() || name == "<none>" {
        return "Dynamic".to_string();
    }

    // Already fully qualified, use as-is
    name.to_string()
}

/// Split a fully qualified name into (package, class_name)
fn split_package(full_name: &str) -> (String, String) {
    if let Some(last_dot) = full_name.rfind('.') {
        (full_name[..last_dot].to_string(), full_name[last_dot + 1..].to_string())
    } else {
        (String::new(), full_name.to_string())
    }
}

/// Convert a package string to a file path
fn package_to_path(package: &str, class_name: &str) -> String {
    if package.is_empty() {
        format!("{}.hx", class_name)
    } else {
        format!("{}/{}.hx", package.replace('.', "/"), class_name)
    }
}

/// Check if a type name should be included based on filters
fn should_include_type(name: &str, options: &ExternGenOptions) -> bool {
    // Skip internal types unless explicitly included
    if !options.include_internal {
        if name.starts_with('_') || name.contains("._") || name.contains('$') {
            return false;
        }
        // Skip abstract implementations
        if name.contains("_Impl_") {
            return false;
        }
    }

    // Apply type filter if present
    if let Some(ref filters) = options.type_filter {
        for filter in filters {
            if matches_type_pattern(name, filter) {
                return true;
            }
        }
        return false;
    }

    true
}

/// Check if a type name matches a pattern (supports * and ** wildcards)
fn matches_type_pattern(name: &str, pattern: &str) -> bool {
    // Handle ** (matches any path segments)
    if pattern.contains("**") {
        let parts: Vec<&str> = pattern.split("**").collect();
        if parts.len() == 2 {
            let prefix = parts[0].trim_end_matches('.');
            let suffix = parts[1].trim_start_matches('.');

            if !prefix.is_empty() && !name.starts_with(prefix) {
                return false;
            }
            if !suffix.is_empty() && !name.ends_with(suffix) {
                return false;
            }
            return true;
        }
    }

    // Handle single * (matches single segment)
    if pattern.contains('*') {
        let pattern_parts: Vec<&str> = pattern.split('.').collect();
        let name_parts: Vec<&str> = name.split('.').collect();

        if pattern_parts.len() != name_parts.len() {
            return false;
        }

        for (p, n) in pattern_parts.iter().zip(name_parts.iter()) {
            if *p != "*" && *p != *n {
                return false;
            }
        }
        return true;
    }

    // Exact match or prefix match
    name == pattern || name.starts_with(&format!("{}.", pattern))
}

/// Check if a field/method name is a valid Haxe identifier
fn is_valid_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    // Haxe reserved keywords
    const KEYWORDS: &[&str] = &[
        "abstract", "break", "case", "cast", "catch", "class", "continue",
        "default", "do", "dynamic", "else", "enum", "extends", "extern",
        "false", "final", "for", "function", "if", "implements", "import",
        "in", "inline", "interface", "macro", "new", "null", "operator",
        "overload", "override", "package", "private", "public", "return",
        "static", "switch", "this", "throw", "true", "try", "typedef",
        "untyped", "using", "var", "while",
    ];

    if KEYWORDS.contains(&name) {
        return false;
    }

    // Must start with letter or underscore
    let first = name.chars().next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }

    // Rest must be alphanumeric or underscore
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Escape a name if it's a keyword or invalid identifier
fn escape_identifier(name: &str) -> String {
    if is_valid_identifier(name) {
        name.to_string()
    } else {
        // Use Haxe's escape syntax for invalid identifiers
        format!("@'{}'", name)
    }
}

/// Generate an extern class definition
fn generate_class_extern(code: &Bytecode, obj: &TypeObj, options: &ExternGenOptions) -> String {
    let mut out = String::new();

    let full_name = code.get(obj.name).to_string();
    let (package, class_name) = split_package(&full_name);

    // Package declaration
    if !package.is_empty() {
        writeln!(out, "package {};", package).unwrap();
        writeln!(out).unwrap();
    }

    // Imports (collect unique types used)
    let imports = collect_type_imports(code, obj);
    let mut has_imports = false;
    for imp in &imports {
        if !imp.starts_with(&package) || imp.matches('.').count() > package.matches('.').count() + 1 {
            writeln!(out, "import {};", imp).unwrap();
            has_imports = true;
        }
    }
    if has_imports {
        writeln!(out).unwrap();
    }

    // Class declaration with optional @:native metadata
    if options.generate_native_meta && class_name.contains('_') {
        writeln!(out, "@:native(\"{}\"))", full_name).unwrap();
    }

    // Class header with inheritance
    write!(out, "extern class {}", escape_identifier(&class_name)).unwrap();
    if let Some(super_ref) = obj.super_ {
        if let Type::Obj(super_obj) | Type::Struct(super_obj) = code.get(super_ref) {
            let super_name = code.get(super_obj.name);
            write!(out, " extends {}", format_type_name(&super_name)).unwrap();
        }
    }
    writeln!(out, " {{").unwrap();

    // Fields (only own_fields to avoid duplicates from parent)
    for field in &obj.own_fields {
        let field_name = code.get(field.name).to_string();

        // Skip fields with invalid names
        if field_name.is_empty() || field_name == "<none>" {
            continue;
        }

        let field_type = ref_type_to_haxe(field.t, code);
        writeln!(out, "    public var {}:{};", escape_identifier(&field_name), field_type).unwrap();
    }

    if !obj.own_fields.is_empty() && !obj.protos.is_empty() {
        writeln!(out).unwrap();
    }

    // Methods
    let mut seen_methods: BTreeSet<String> = BTreeSet::new();
    for proto in &obj.protos {
        let method_name = code.get(proto.name).to_string();

        // Skip duplicate method names (overloads not supported in externs)
        if seen_methods.contains(&method_name) {
            continue;
        }
        seen_methods.insert(method_name.clone());

        // Skip methods with invalid names
        if method_name.is_empty() || method_name == "<none>" {
            continue;
        }

        let sig = format_method_signature(code, proto);

        // Handle constructors
        if method_name == "__constructor__" || method_name == "new" {
            writeln!(out, "    public function new({});", sig.args).unwrap();
        } else {
            let escaped_name = escape_identifier(&method_name);
            writeln!(out, "    public function {}({}):{};", escaped_name, sig.args, sig.ret).unwrap();
        }
    }

    writeln!(out, "}}").unwrap();

    out
}

/// Method signature components
struct MethodSignature {
    args: String,
    ret: String,
}

/// Format a method signature from a proto
fn format_method_signature(code: &Bytecode, proto: &ObjProto) -> MethodSignature {
    let fun_ptr = code.get(proto.findex);

    match fun_ptr {
        FunPtr::Fun(func) => {
            let fun_type = func.ty(code);
            format_method_signature_from_type(code, fun_type, Some(func))
        }
        FunPtr::Native(native) => {
            let fun_type = native.ty(code);
            format_method_signature_from_type(code, fun_type, None)
        }
    }
}

/// Format method signature from TypeFun
fn format_method_signature_from_type(
    code: &Bytecode,
    fun_type: &TypeFun,
    func: Option<&crate::types::Function>
) -> MethodSignature {
    // Skip first arg if it's 'this' (method receiver)
    let args_start = if !fun_type.args.is_empty() {
        // Check if first arg is likely 'this'
        1
    } else {
        0
    };

    let args: Vec<String> = fun_type.args.iter()
        .skip(args_start)
        .enumerate()
        .map(|(i, arg_type)| {
            let arg_name = func
                .and_then(|f| f.arg_name(code, i))
                .unwrap_or_else(|| format!("arg{}", i).into());
            let type_str = ref_type_to_haxe(*arg_type, code);
            format!("{}:{}", escape_identifier(&arg_name), type_str)
        })
        .collect();

    let ret = ref_type_to_haxe(fun_type.ret, code);

    MethodSignature {
        args: args.join(", "),
        ret,
    }
}

/// Collect imports needed for a class
fn collect_type_imports(code: &Bytecode, obj: &TypeObj) -> BTreeSet<String> {
    let mut imports = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let own_package = {
        let (pkg, _) = split_package(&code.get(obj.name));
        pkg
    };

    // Collect from fields
    for field in &obj.own_fields {
        collect_type_ref_imports(code, field.t, &own_package, &mut imports, &mut visited);
    }

    // Collect from method signatures
    for proto in &obj.protos {
        if let FunPtr::Fun(func) = code.get(proto.findex) {
            let fun_type = func.ty(code);
            for arg in &fun_type.args {
                collect_type_ref_imports(code, *arg, &own_package, &mut imports, &mut visited);
            }
            collect_type_ref_imports(code, fun_type.ret, &own_package, &mut imports, &mut visited);
        }
    }

    // Collect from parent
    if let Some(super_ref) = obj.super_ {
        collect_type_ref_imports(code, super_ref, &own_package, &mut imports, &mut visited);
    }

    imports
}

/// Collect imports from a type reference
fn collect_type_ref_imports(
    code: &Bytecode,
    ref_type: RefType,
    own_package: &str,
    imports: &mut BTreeSet<String>,
    visited: &mut BTreeSet<usize>,
) {
    // Prevent infinite recursion
    if !visited.insert(ref_type.0) {
        return;
    }

    let ty = code.get(ref_type);

    match ty {
        Type::Obj(obj) | Type::Struct(obj) => {
            let name = code.get(obj.name).to_string();
            let (pkg, _) = split_package(&name);
            if !pkg.is_empty() && pkg != own_package && !name.starts_with("hl.") && !name.starts_with("haxe.") {
                imports.insert(name);
            }
        }
        Type::Enum { name, .. } => {
            let name_str = code.get(*name).to_string();
            let (pkg, _) = split_package(&name_str);
            if !pkg.is_empty() && pkg != own_package && !name_str.starts_with("hl.") {
                imports.insert(name_str);
            }
        }
        Type::Null(inner) | Type::Ref(inner) | Type::Packed(inner) => {
            collect_type_ref_imports(code, *inner, own_package, imports, visited);
        }
        Type::Fun(fun) | Type::Method(fun) => {
            for arg in &fun.args {
                collect_type_ref_imports(code, *arg, own_package, imports, visited);
            }
            collect_type_ref_imports(code, fun.ret, own_package, imports, visited);
        }
        Type::Virtual { fields } => {
            for field in fields {
                collect_type_ref_imports(code, field.t, own_package, imports, visited);
            }
        }
        _ => {}
    }
}

/// Generate an extern enum definition
fn generate_enum_extern(code: &Bytecode, name: &str, constructs: &[EnumConstruct], _options: &ExternGenOptions) -> String {
    let mut out = String::new();

    let (package, enum_name) = split_package(name);

    // Package declaration
    if !package.is_empty() {
        writeln!(out, "package {};", package).unwrap();
        writeln!(out).unwrap();
    }

    // Enum declaration
    writeln!(out, "enum {} {{", escape_identifier(&enum_name)).unwrap();

    // Constructors
    for construct in constructs {
        let cons_name = code.get(construct.name).to_string();

        // Skip anonymous/internal constructors
        if cons_name.is_empty() || cons_name == "<none>" || cons_name.starts_with('_') {
            continue;
        }

        if construct.params.is_empty() {
            writeln!(out, "    {};", escape_identifier(&cons_name)).unwrap();
        } else {
            let params: Vec<String> = construct.params.iter()
                .enumerate()
                .map(|(i, p)| {
                    let ty = ref_type_to_haxe(*p, code);
                    format!("v{}:{}", i, ty)
                })
                .collect();
            writeln!(out, "    {}({});", escape_identifier(&cons_name), params.join(", ")).unwrap();
        }
    }

    writeln!(out, "}}").unwrap();

    out
}

/// Generate an extern abstract definition
fn generate_abstract_extern(_code: &Bytecode, name: &str, _options: &ExternGenOptions) -> String {
    let mut out = String::new();

    let (package, abstract_name) = split_package(name);

    // Package declaration
    if !package.is_empty() {
        writeln!(out, "package {};", package).unwrap();
        writeln!(out).unwrap();
    }

    // Abstract declaration (basic - we don't have underlying type info)
    writeln!(out, "abstract {}(Dynamic) {{", escape_identifier(&abstract_name)).unwrap();
    writeln!(out, "}}").unwrap();

    out
}

/// Generate all extern definitions from bytecode
pub fn generate_all_externs(code: &Bytecode, options: &ExternGenOptions) -> ExternGenResult {
    let mut result = ExternGenResult::default();

    // Track which types we've already processed
    let mut processed: BTreeSet<String> = BTreeSet::new();

    for ty in &code.types {
        match ty {
            Type::Obj(obj) | Type::Struct(obj) => {
                let name = code.get(obj.name).to_string();

                if processed.contains(&name) {
                    continue;
                }
                processed.insert(name.clone());

                if !should_include_type(&name, options) {
                    result.skipped.push((name, "filtered out".to_string()));
                    continue;
                }

                let content = generate_class_extern(code, obj, options);
                let (package, class_name) = split_package(&name);
                let path = package_to_path(&package, &class_name);

                result.files.insert(path, content);
                result.class_count += 1;
            }

            Type::Enum { name, constructs, .. } => {
                let name_str = code.get(*name).to_string();

                if processed.contains(&name_str) {
                    continue;
                }
                processed.insert(name_str.clone());

                if !should_include_type(&name_str, options) {
                    result.skipped.push((name_str, "filtered out".to_string()));
                    continue;
                }

                let content = generate_enum_extern(code, &name_str, constructs, options);
                let (package, enum_name) = split_package(&name_str);
                let path = package_to_path(&package, &enum_name);

                result.files.insert(path, content);
                result.enum_count += 1;
            }

            Type::Abstract { name } => {
                let name_str = code.get(*name).to_string();

                if processed.contains(&name_str) {
                    continue;
                }
                processed.insert(name_str.clone());

                if !should_include_type(&name_str, options) {
                    result.skipped.push((name_str, "filtered out".to_string()));
                    continue;
                }

                let content = generate_abstract_extern(code, &name_str, options);
                let (package, abstract_name) = split_package(&name_str);
                let path = package_to_path(&package, &abstract_name);

                result.files.insert(path, content);
                result.abstract_count += 1;
            }

            _ => {}
        }
    }

    result
}

/// Write generated externs to a directory
pub fn write_externs_to_dir(result: &ExternGenResult, output_dir: &Path) -> std::io::Result<()> {
    use std::fs;

    for (path, content) in &result.files {
        let full_path = output_dir.join(path);

        // Create parent directories
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&full_path, content)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_package() {
        assert_eq!(split_package("h3d.impl.GlDriver"), ("h3d.impl".to_string(), "GlDriver".to_string()));
        assert_eq!(split_package("Player"), ("".to_string(), "Player".to_string()));
        assert_eq!(split_package("a.b.c.D"), ("a.b.c".to_string(), "D".to_string()));
    }

    #[test]
    fn test_package_to_path() {
        assert_eq!(package_to_path("h3d.impl", "GlDriver"), "h3d/impl/GlDriver.hx");
        assert_eq!(package_to_path("", "Player"), "Player.hx");
    }

    #[test]
    fn test_matches_type_pattern() {
        // Exact match
        assert!(matches_type_pattern("h3d.impl.GlDriver", "h3d.impl.GlDriver"));

        // Prefix match
        assert!(matches_type_pattern("h3d.impl.GlDriver", "h3d.impl"));
        assert!(matches_type_pattern("h3d.impl.GlDriver.Inner", "h3d.impl.GlDriver"));

        // Single wildcard
        assert!(matches_type_pattern("h3d.impl.GlDriver", "h3d.impl.*"));
        assert!(matches_type_pattern("h3d.impl.Other", "h3d.impl.*"));
        assert!(!matches_type_pattern("h3d.impl.sub.GlDriver", "h3d.impl.*"));

        // Double wildcard
        assert!(matches_type_pattern("h3d.impl.GlDriver", "h3d.**"));
        assert!(matches_type_pattern("h3d.impl.sub.deep.Class", "h3d.**"));
        assert!(!matches_type_pattern("h2d.impl.GlDriver", "h3d.**"));
    }

    #[test]
    fn test_is_valid_identifier() {
        assert!(is_valid_identifier("foo"));
        assert!(is_valid_identifier("_foo"));
        assert!(is_valid_identifier("foo123"));
        assert!(is_valid_identifier("foo_bar"));

        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("123foo"));
        assert!(!is_valid_identifier("foo-bar"));
        assert!(!is_valid_identifier("class")); // keyword
        assert!(!is_valid_identifier("function")); // keyword
    }

    #[test]
    fn test_escape_identifier() {
        assert_eq!(escape_identifier("foo"), "foo");
        assert_eq!(escape_identifier("class"), "@'class'");
        assert_eq!(escape_identifier("123foo"), "@'123foo'");
    }
}
