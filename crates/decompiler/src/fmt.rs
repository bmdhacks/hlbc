use std::fmt;
use std::fmt::{Display, Formatter};

use hlbc::fmt::{BytecodeFmt, EnhancedFmt};
use hlbc::types::{Function, RefField, RefType, Type, TypeFun, TypeObj};
use hlbc::Str;
use hlbc::{Bytecode, Resolve};

use crate::ast::{Class, ClassField, Confidence, Constant, ConstructorCall, Expr, Method, Operation, Statement};
use crate::type_mappings::expand_module_path;

/// Top-level standard library classes (from haxe/std/*.hx).
/// These should always be prefixed with "std." to avoid ambiguity with user types.
const STDLIB_CLASSES: &[&str] = &[
    "Any",
    "Array",
    "Class",
    "Date",
    "DateTools",
    "Enum",
    "EnumValue",
    "EReg",
    "IntIterator",
    "Lambda",
    "List",
    "Map",
    "Math",
    "Reflect",
    "Std",
    "StdTypes",
    // "String" - excluded, too common to shadow
    "StringBuf",
    "StringTools",
    "Sys",
    "Type",
    "UInt",
    "UnicodeString",
    "Xml",
];

/// Check if a class name is a standard library class that needs "std." prefix.
fn needs_std_prefix(class_name: &str) -> bool {
    STDLIB_CLASSES.contains(&class_name)
}

/// Escape a string for output as a Haxe string literal.
/// Handles quotes, backslashes, and control characters.
fn escape_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\0' => result.push_str("\\x00"),
            // Other control characters (0x01-0x1F, 0x7F)
            c if c.is_control() => {
                // Use \xNN for ASCII control chars, \uNNNN for others
                if (c as u32) < 0x100 {
                    result.push_str(&format!("\\x{:02X}", c as u32));
                } else {
                    result.push_str(&format!("\\u{:04X}", c as u32));
                }
            }
            c => result.push(c),
        }
    }
    result
}

/// Helper to panic during Display - returns a string that will never be used
fn panic_invalid_anon_type(ty: &Type) -> &'static str {
    panic!("Anonymous expr with non-Virtual type: {:?}", ty)
}

/// Helper to panic for Expr::Unknown during Display
fn panic_unknown_expr(msg: &str) -> &'static str {
    panic!("Expr::Unknown encountered during display: {}", msg)
}

/// Helper to panic for internal function calls that should have been suppressed
fn panic_internal_call(fun: &Expr) -> &'static str {
    panic!("Internal function call should have been suppressed by structurer: {:?}", fun)
}

/// Check if a type name is an internal HashLink type that should not appear in decompiled output.
/// These are implementation details that the Haxe compiler generates but aren't valid Haxe types.
fn is_internal_hl_type(name: &str) -> bool {
    // Strip common prefixes that get added
    let name = name.strip_prefix("haxe.std.").unwrap_or(name);

    // Array implementation types
    if name.starts_with("hl.types.Array") {
        return true;
    }
    // Iterator implementation types
    if name.contains("Iterator") && name.starts_with("hl.") {
        return true;
    }
    // Other internal hl.types
    if name.starts_with("hl.types.") {
        return true;
    }

    false
}


/// A formatter that produces clean Haxe-like output without index annotations.
/// Unlike EnhancedFmt, this doesn't add @index suffixes to type names.
#[derive(Copy, Clone, Default)]
pub struct HaxeFmt;

impl BytecodeFmt for HaxeFmt {
    fn fmt_reftype(&self, f: &mut Formatter, ctx: &Bytecode, v: RefType) -> fmt::Result {
        let ty = &ctx[v];
        self.fmt_type(f, ctx, ty)
        // Note: No @index suffix added
    }

    fn fmt_type(&self, f: &mut Formatter, ctx: &Bytecode, v: &Type) -> fmt::Result {
        match v {
            Type::Fun(fun) => self.fmt_typefun(f, ctx, fun),
            Type::Obj(TypeObj { name, .. }) => {
                let name_str = ctx.get(*name);
                // Map internal HL types to their Haxe equivalents
                match name_str.as_ref() {
                    "hl.types.ArrayBytes_Int" => write!(f, "Array<Int>"),
                    "hl.types.ArrayBytes_Float" | "hl.types.ArrayBytes_Single" => write!(f, "Array<Float>"),
                    "hl.types.ArrayObj" => write!(f, "Array<Dynamic>"),
                    "hl.types.ArrayDyn" => write!(f, "Array<Dynamic>"),
                    _ => {
                        // Fix nested type names with underscore-prefixed module containers
                        if let Some(fixed) = fix_nested_type_name(name_str.as_ref()) {
                            write!(f, "{}", fixed)
                        } else {
                            write!(f, "{}", name_str)
                        }
                    }
                }
            }
            Type::Ref(reftype) => {
                write!(f, "ref<")?;
                self.fmt_type(f, ctx, &ctx[*reftype])?;
                write!(f, ">")
            }
            Type::Virtual { .. } => write!(f, "Dynamic"),
            Type::Abstract { name } => {
                // Map internal HL abstracts (lowercase names) to Dynamic
                let name_str = ctx.get(*name);
                let first_char = name_str.chars().next();
                if first_char.map(|c| c.is_lowercase() || c == '_').unwrap_or(false) {
                    write!(f, "Dynamic")
                } else {
                    write!(f, "{}", name_str)
                }
            }
            Type::Enum { name, .. } => {
                let name_str = ctx.get(*name);
                if let Some(fixed) = fix_nested_type_name(name_str.as_ref()) {
                    write!(f, "{}", fixed)
                } else {
                    write!(f, "{}", name_str)
                }
            }
            Type::Null(reftype) => {
                write!(f, "Null<")?;
                self.fmt_reftype(f, ctx, *reftype)?;
                write!(f, ">")
            }
            Type::Method(fun) => self.fmt_typefun(f, ctx, fun),
            Type::Struct(TypeObj { name, .. }) => {
                let name_str = ctx.get(*name);
                if let Some(fixed) = fix_nested_type_name(name_str.as_ref()) {
                    write!(f, "{}", fixed)
                } else {
                    write!(f, "{}", name_str)
                }
            }
            Type::Packed(reftype) => self.fmt_reftype(f, ctx, *reftype),
            // Simple types
            Type::Void => write!(f, "Void"),
            Type::UI8 => write!(f, "Int"),
            Type::UI16 => write!(f, "Int"),
            Type::I32 => write!(f, "Int"),
            Type::I64 => write!(f, "haxe.Int64"),
            Type::F32 => write!(f, "Single"),
            Type::F64 => write!(f, "Float"),
            Type::Bool => write!(f, "Bool"),
            Type::Bytes => write!(f, "haxe.io.Bytes"),
            Type::Dyn | Type::DynObj => write!(f, "Dynamic"),
            Type::Array => write!(f, "Array<Dynamic>"),
            Type::Type => write!(f, "Class<Dynamic>"),
            Type::Guid => write!(f, "hl.Guid"),
        }
    }

    fn fmt_typefun(&self, f: &mut Formatter, ctx: &Bytecode, v: &TypeFun) -> fmt::Result {
        write!(f, "(")?;
        for (i, arg) in v.args.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            self.fmt_type(f, ctx, &ctx[*arg])?;
        }
        write!(f, ") -> ")?;
        self.fmt_type(f, ctx, &ctx[v.ret])
    }
}

const INDENT: &str = "                                                                                                                                                                                                                                                                ";

#[derive(Clone)]
pub struct FormatOptions {
    indent: &'static str,
    inc_indent: usize,
    /// Show type indices as comments (e.g., type@309)
    pub show_type_indices: bool,
    /// Show function indices as comments (e.g., fun@1409)
    pub show_fun_indices: bool,
    /// Show field indices as comments (e.g., F0, F1)
    pub show_field_indices: bool,
    /// Show string literal indices as comments (e.g., str@1234)
    pub show_string_indices: bool,
}

impl FormatOptions {
    pub fn new(inc_indent: usize) -> Self {
        Self {
            indent: "",
            inc_indent,
            show_type_indices: false,
            show_fun_indices: false,
            show_field_indices: false,
            show_string_indices: false,
        }
    }

    /// Create format options with all index annotations enabled
    pub fn with_indices(inc_indent: usize) -> Self {
        Self {
            indent: "",
            inc_indent,
            show_type_indices: true,
            show_fun_indices: true,
            show_field_indices: true,
            show_string_indices: true,
        }
    }

    /// Create format options with only function indices (useful for cross-referencing)
    pub fn with_fun_indices(inc_indent: usize) -> Self {
        Self {
            indent: "",
            inc_indent,
            show_type_indices: false,
            show_fun_indices: true,
            show_field_indices: false,
            show_string_indices: false,
        }
    }

    pub fn inc_nesting(&self) -> Self {
        let new_len = self.indent.len() + self.inc_indent;
        // Clamp to INDENT length to avoid overflow on deeply nested code
        let clamped_len = new_len.min(INDENT.len());
        FormatOptions {
            indent: &INDENT[..clamped_len],
            ..*self
        }
    }
}

impl Display for FormatOptions {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.indent)
    }
}

/// Known generic types that require a single type parameter.
/// These will be annotated with `<Dynamic>` if no type param is present.
pub const KNOWN_SINGLE_PARAM_GENERICS: &[&str] = &[
    "haxe.ds.IntMap",
    "haxe.ds.StringMap",
    "haxe.ds.List",
    "haxe.ds.Vector",
    "haxe.ds.BalancedTree",
    "haxe.ds.GenericStack",
    "haxe.iterators.ArrayIterator",
    "haxe.iterators.MapIterator",
];

/// Known generic types that require two type parameters.
pub const KNOWN_TWO_PARAM_GENERICS: &[&str] = &[
    "haxe.ds.Map",
    "haxe.ds.HashMap",
    "haxe.ds.WeakMap",
    "haxe.ds.EnumValueMap",
    "haxe.ds.ObjectMap",
];

/// Known base names for monomorphized generics (without package prefix).
/// When @:generic is used, Haxe creates types like `List_Int`, `Vector_Float`.
const MONOMORPHIZED_SINGLE_PARAM: &[&str] = &[
    "List",
    "Vector",
    "GenericStack",
];

const MONOMORPHIZED_TWO_PARAM: &[&str] = &[
    "Map",
    "ObjectMap",
];

/// Packages that are known to contain monomorphized generic types.
/// Only types in these packages (or no package) will be demangled.
const KNOWN_STDLIB_PACKAGES: &[&str] = &[
    "haxe.ds",
    "hl.types",
];

/// Try to demangle a monomorphized generic type name.
/// E.g., "haxe.ds.List_Int" → "haxe.ds.List<Int>"
/// Only demaangles types from known stdlib packages to avoid false positives
/// with external libraries that might have types like "mylib.Vector_Float".
fn demangle_generic_name(name: &str) -> Option<String> {
    // Extract simple name (after last dot)
    let simple_name = name.rsplit('.').next().unwrap_or(name);
    let package = if name.contains('.') {
        Some(&name[..name.len() - simple_name.len() - 1])
    } else {
        None
    };

    // Only demangle types from known stdlib packages (or no package for top-level)
    let is_known_package = package.map_or(true, |pkg| {
        KNOWN_STDLIB_PACKAGES.iter().any(|known| pkg == *known)
    });
    if !is_known_package {
        return None;
    }

    // Try single-param generics first
    for base in MONOMORPHIZED_SINGLE_PARAM {
        let prefix = format!("{}_", base);
        if simple_name.starts_with(&prefix) {
            let type_param = &simple_name[prefix.len()..];
            let type_param = demangle_type_param(type_param);
            return Some(if let Some(pkg) = package {
                format!("{}.{}<{}>", pkg, base, type_param)
            } else {
                format!("{}<{}>", base, type_param)
            });
        }
    }

    // Try two-param generics
    for base in MONOMORPHIZED_TWO_PARAM {
        let prefix = format!("{}_", base);
        if simple_name.starts_with(&prefix) {
            let params_part = &simple_name[prefix.len()..];
            // Split on underscore - first part is key type, rest is value type
            if let Some(underscore_pos) = params_part.find('_') {
                let key_type = demangle_type_param(&params_part[..underscore_pos]);
                let val_type = demangle_type_param(&params_part[underscore_pos + 1..]);
                return Some(if let Some(pkg) = package {
                    format!("{}.{}<{}, {}>", pkg, base, key_type, val_type)
                } else {
                    format!("{}<{}, {}>", base, key_type, val_type)
                });
            }
        }
    }

    None
}

/// Convert mangled type parameter names to proper Haxe types.
/// E.g., "Int" → "Int", "String" → "String", "hl_I64" → "hl.I64"
fn demangle_type_param(param: &str) -> String {
    // Handle hl_ prefix types
    if let Some(rest) = param.strip_prefix("hl_") {
        return format!("hl.{}", rest);
    }
    // Common primitive mappings
    match param {
        "Int" => "Int".to_string(),
        "Float" => "Float".to_string(),
        "Single" => "Single".to_string(),
        "Bool" => "Bool".to_string(),
        "String" => "String".to_string(),
        "Dynamic" => "Dynamic".to_string(),
        other => other.to_string(),
    }
}

/// Fix nested type names with underscore-prefixed module containers.
/// E.g., "hxsl._Splitter.VarProps" → "hxsl.Splitter.VarProps"
/// In Haxe bytecode, nested types are stored under a module type named `_ClassName`,
/// but in Haxe source you reference them as `ClassName.NestedType`.
fn fix_nested_type_name(name: &str) -> Option<String> {
    // Look for "._" pattern indicating a nested type container
    if let Some(pos) = name.find("._") {
        // Find the end of the underscore-prefixed segment
        let after_underscore = pos + 2; // skip "._"
        if let Some(dot_pos) = name[after_underscore..].find('.') {
            // We have "pkg._Container.NestedType"
            // Transform to "pkg.Container.NestedType"
            let prefix = &name[..pos + 1]; // "pkg."
            let container = &name[after_underscore..after_underscore + dot_pos]; // "Container"
            let rest = &name[after_underscore + dot_pos..]; // ".NestedType"
            return Some(format!("{}{}{}", prefix, container, rest));
        }
    }
    // Also handle top-level underscore prefix: "_Splitter.VarProps" → "Splitter.VarProps"
    if name.starts_with('_') && name.contains('.') {
        let dot_pos = name.find('.').unwrap();
        let container = &name[1..dot_pos]; // Skip leading underscore
        let rest = &name[dot_pos..];
        return Some(format!("{}{}", container, rest));
    }
    None
}

/// Check if a HashLink type is a private nested type that shouldn't be used
/// in explicit type annotations (rely on type inference instead).
///
/// In HashLink bytecode, private nested types are indicated by underscore prefix:
/// - `hxsl._ShaderList.ShaderIterator` - ShaderIterator is private in ShaderList.hx
/// - `hxsl._Linker.AllocatedVar` - AllocatedVar is private in Linker.hx
///
/// This checks the RAW type name from bytecode (before fix_nested_type_name processing).
pub fn is_private_nested_type(ty: &Type, ctx: &Bytecode) -> bool {
    match ty {
        Type::Obj(obj) | Type::Struct(obj) => {
            let name = ctx.get(obj.name);
            // Check for "._" pattern indicating private nested type
            name.contains("._")
        }
        Type::Enum { name, .. } => {
            let name = ctx.get(*name);
            name.contains("._")
        }
        _ => false,
    }
}

/// Simplify a type name when used within a specific class context.
/// E.g., within "hxsl.Splitter", "hxsl.Splitter.VarProps" becomes "VarProps".
/// Also handles module-private classes: within "hxsl._Linker.AllocatedVar",
/// any type like "hxsl.Linker.OtherPrivateClass" becomes "OtherPrivateClass".
pub fn simplify_type_in_context(type_name: &str, current_class: Option<&str>) -> String {
    if let Some(class_name) = current_class {
        // Normalize current_class the same way we normalize type names
        // (fix underscore-prefixed module containers like hxsl._Linker → hxsl.Linker)
        let normalized_class = fix_nested_type_name(class_name)
            .unwrap_or_else(|| class_name.to_string());

        // If type starts with "CurrentClass.", strip it (nested type case)
        let prefix = format!("{}.", normalized_class);
        if type_name.starts_with(&prefix) {
            return type_name[prefix.len()..].to_string();
        }

        // For module-private classes: extract the module prefix from current class
        // E.g., "hxsl.Linker.AllocatedVar" → module prefix is "hxsl.Linker."
        // Then simplify any type from the same module to just its simple name.
        if let Some(last_dot) = normalized_class.rfind('.') {
            let module_prefix = format!("{}.", &normalized_class[..last_dot]);
            if type_name.starts_with(&module_prefix) {
                // Extract just the simple type name (after the module prefix)
                let remainder = &type_name[module_prefix.len()..];
                // Only simplify if remainder is a simple name (no more dots)
                // This ensures we don't over-simplify nested types from other modules
                if !remainder.contains('.') {
                    return remainder.to_string();
                }
            }
        }
    }
    type_name.to_string()
}

/// Convert a HashLink type to Haxe string, simplifying nested types when within a class context.
pub fn to_haxe_type_in_context<'a>(ty: &Type, ctx: &'a Bytecode, current_class: Option<&str>) -> Str {
    let base = to_haxe_type(ty, ctx);
    if current_class.is_some() {
        Str::from(simplify_type_in_context(&base, current_class))
    } else {
        base
    }
}

/// Convert a HashLink type to its Haxe equivalent string representation.
/// Maps internal HL types to Haxe types (e.g., hl.types.ArrayDyn → Array<Dynamic>)
pub fn to_haxe_type<'a>(ty: &Type, ctx: &'a Bytecode) -> Str {
    use crate::Type::*;
    match ty {
        Void => Str::from_static("Void"),
        UI8 => Str::from_static("hl.UI8"),
        UI16 => Str::from_static("hl.UI16"),
        I32 => Str::from_static("Int"),
        I64 => Str::from_static("hl.I64"),
        F32 => Str::from_static("Single"),
        F64 => Str::from_static("Float"),
        Bool => Str::from_static("Bool"),
        Bytes => Str::from_static("hl.Bytes"),
        Dyn | DynObj | Virtual { .. } => Str::from_static("Dynamic"),
        Fun(fun) | Method(fun) => {
            // Format function types: Arg -> RetType or (Arg1, Arg2) -> RetType
            // Single arg doesn't need parentheses in Haxe: Int -> Void
            // Multiple args need parentheses: (Int, String) -> Void
            // Return type needs parentheses if it's a function: Int -> (Int -> Int)
            let args: Vec<_> = fun.args.iter().map(|a| to_haxe_type(&ctx[*a], ctx)).collect();
            let ret = to_haxe_type(&ctx[fun.ret], ctx);
            // Wrap return type in parentheses if it's a function type
            let ret_str = if ret.contains("->") {
                format!("({})", ret)
            } else {
                ret.to_string()
            };
            if args.is_empty() {
                Str::from(format!("Void -> {}", ret_str))
            } else if args.len() == 1 {
                // Single arg: no parentheses needed
                // But if the arg is itself a function type, wrap it
                let arg = &args[0];
                if arg.contains("->") {
                    Str::from(format!("({}) -> {}", arg, ret_str))
                } else {
                    Str::from(format!("{} -> {}", arg, ret_str))
                }
            } else {
                Str::from(format!("({}) -> {}", args.join(", "), ret_str))
            }
        }
        Obj(obj) | Struct(obj) => {
            let name_str = ctx.get(obj.name);
            // Map internal HL types to their Haxe equivalents
            match name_str.as_ref() {
                "hl.types.ArrayBytes_Int" | "hl.types.ArrayBytes_hl_UI16" => Str::from_static("Array<Int>"),
                "hl.types.ArrayBytes_Float" | "hl.types.ArrayBytes_Single" | "hl.types.ArrayBytes_hl_F64" | "hl.types.ArrayBytes_hl_F32" => Str::from_static("Array<Float>"),
                "hl.types.ArrayObj" => Str::from_static("Array<Dynamic>"),
                "hl.types.ArrayDyn" => Str::from_static("Array<Dynamic>"),
                _ => {
                    // Fix nested type names with underscore-prefixed module containers
                    // E.g., "hxsl._Splitter.VarProps" → "hxsl.Splitter.VarProps"
                    if let Some(fixed) = fix_nested_type_name(name_str.as_ref()) {
                        return Str::from(fixed);
                    }
                    // Expand shortened module paths (e.g., haxe.macro.Binop → haxe.macro.Expr.Binop)
                    if let Some(expanded) = expand_module_path(name_str.as_ref()) {
                        return Str::from(expanded);
                    }
                    // Try to demangle monomorphized generic types (e.g., List_Int → List<Int>)
                    if let Some(demangled) = demangle_generic_name(name_str.as_ref()) {
                        return Str::from(demangled);
                    }
                    // Check if this is a known generic type that needs type parameters
                    if KNOWN_SINGLE_PARAM_GENERICS.iter().any(|g| name_str.as_ref() == *g) {
                        return Str::from(format!("{}<Dynamic>", name_str));
                    }
                    if KNOWN_TWO_PARAM_GENERICS.iter().any(|g| name_str.as_ref() == *g) {
                        return Str::from(format!("{}<Dynamic, Dynamic>", name_str));
                    }
                    // Handle $ prefix for class type holders (e.g., "haxe.$Log" -> "Class<haxe.Log>")
                    // The $ indicates this is the class object itself, not an instance
                    if name_str.contains(".$") {
                        let class_name = name_str.replace(".$", ".");
                        return Str::from(format!("Class<{}>", class_name));
                    }
                    // Add std. prefix for stdlib classes to avoid shadowing by user types
                    if !name_str.contains('.') && needs_std_prefix(name_str.as_ref()) {
                        return Str::from(format!("std.{}", name_str));
                    }
                    name_str
                }
            }
        }
        Array => Str::from_static("Array<Dynamic>"),
        Type => Str::from_static("Class<Dynamic>"),
        Abstract { name } => {
            // Map internal HL abstracts (lowercase names) to Dynamic
            let name_str = ctx.get(*name);
            let first_char = name_str.chars().next();
            if first_char.map(|c| c.is_lowercase() || c == '_').unwrap_or(false) {
                Str::from_static("Dynamic")
            } else {
                // Add std. prefix for stdlib classes to avoid shadowing by user types
                if !name_str.contains('.') && needs_std_prefix(name_str.as_ref()) {
                    Str::from(format!("std.{}", name_str))
                } else {
                    name_str
                }
            }
        }
        Enum { name, .. } => {
            let name_str = ctx.get(*name);
            // Fix nested type names with underscore-prefixed module containers
            if let Some(fixed) = fix_nested_type_name(name_str.as_ref()) {
                return Str::from(fixed);
            }
            // Expand shortened module paths (e.g., haxe.macro.Binop → haxe.macro.Expr.Binop)
            if let Some(expanded) = expand_module_path(name_str.as_ref()) {
                return Str::from(expanded);
            }
            // Add std. prefix for stdlib classes to avoid shadowing by user types
            if !name_str.contains('.') && needs_std_prefix(name_str.as_ref()) {
                return Str::from(format!("std.{}", name_str));
            }
            name_str
        }
        Ref(inner) => {
            // hl.Ref<T> is used internally for nullable parameters
            // At Haxe source level, this is Null<T>
            let inner_name = to_haxe_type(&ctx[*inner], ctx);
            Str::from(format!("Null<{}>", inner_name))
        }
        Null(inner) => {
            let inner_name = to_haxe_type(&ctx[*inner], ctx);
            Str::from(format!("Null<{}>", inner_name))
        }
        Packed(_) => Str::from_static("Dynamic"),
        Guid => Str::from_static("hl.Guid"),
    }
}

/// Result of formatting a field type with inferred generics.
pub struct FieldTypeFormat {
    /// The formatted type string.
    pub type_str: String,
    /// Optional comment about inference (e.g., "// likely String").
    pub comment: Option<String>,
}

/// Format a field's type, using inferred generic parameters if available.
/// Returns the type string and an optional comment about the inference.
/// If `current_class` is provided, type names starting with that class will be simplified.
pub fn format_field_type(field: &ClassField, ctx: &Bytecode, current_class: Option<&str>) -> FieldTypeFormat {
    let base_type = to_haxe_type(&ctx[field.ty], ctx);
    let base_type = simplify_type_in_context(&base_type, current_class);

    // If we have inferred generics and this is a known generic type, use them
    if let Some(ref inference) = field.inferred_generics {
        // Get type name from TypeObj if available
        let type_name = ctx[field.ty].get_type_obj()
            .map(|obj| obj.name(ctx).to_string());
        let type_name = type_name.as_deref().unwrap_or("");

        // Check if this is a generic type that we might have inference for
        let is_single_param = KNOWN_SINGLE_PARAM_GENERICS.iter().any(|g| type_name == *g);
        let is_two_param = KNOWN_TWO_PARAM_GENERICS.iter().any(|g| type_name == *g);

        if is_single_param || is_two_param {
            let param_count = if is_single_param { 1 } else { 2 };

            // Build the type parameters from inference
            let params: Vec<String> = (0..param_count)
                .map(|i| {
                    inference.params.get(i)
                        .and_then(|p| p.clone())
                        .unwrap_or_else(|| "Dynamic".to_string())
                })
                .collect();

            let type_str = format!("{}<{}>", type_name, params.join(", "));

            // Generate comment based on confidence and observations
            let has_observations = inference.observed_types.iter()
                .any(|obs| !obs.is_empty());

            let comment = if !has_observations {
                // No observations made - indicate inference found nothing
                Some("// no type usage observed".to_string())
            } else {
                match inference.confidence {
                    Confidence::High => None, // High confidence - no comment needed
                    Confidence::Medium => {
                        // Medium confidence - add "// likely X" comment
                        let observed = &inference.observed_types;
                        if !observed.is_empty() && !observed[0].is_empty() {
                            Some(format!("// inferred: {}", observed[0].join(" | ")))
                        } else {
                            None
                        }
                    }
                    Confidence::Low => {
                        // Low confidence - add "// could be X or Y" comment
                        let observed = &inference.observed_types;
                        if !observed.is_empty() && observed[0].len() > 1 {
                            Some(format!("// could be: {}", observed[0].join(" | ")))
                        } else {
                            None
                        }
                    }
                }
            };

            return FieldTypeFormat { type_str, comment };
        }
    }

    // No inference or not a generic type - use base type
    FieldTypeFormat {
        type_str: base_type.to_string(),
        comment: None,
    }
}

impl Class {
    /// Display without type index (for backward compatibility)
    pub fn display<'a>(&'a self, ctx: &'a Bytecode, opts: &'a FormatOptions) -> impl Display + 'a {
        self.display_with_index(ctx, opts, None)
    }

    /// Display with optional type index annotation
    pub fn display_with_index<'a>(
        &'a self,
        ctx: &'a Bytecode,
        opts: &'a FormatOptions,
        type_idx: Option<usize>,
    ) -> impl Display + 'a {
        let new_opts = opts.inc_nesting();
        // Split package and class name
        let (package, simple_name) = if let Some(pos) = self.name.rfind('.') {
            (Some(&self.name[..pos]), &self.name[pos + 1..])
        } else {
            (None, self.name.as_str())
        };
        // Extract parent name - use simple name only if in same package
        let parent_display = self.parent.as_ref().map(|p| {
            let (parent_pkg, parent_simple) = if let Some(pos) = p.rfind('.') {
                (Some(&p[..pos]), &p[pos + 1..])
            } else {
                (None, p.as_str())
            };
            // Use simple name if same package, otherwise use full qualified name
            if parent_pkg == package {
                parent_simple
            } else {
                p.as_str()
            }
        });

        // Precompute field type info (for inference comments)
        // Pass the current class name so nested types can be simplified
        let class_name = &self.name;
        let field_types: Vec<FieldTypeFormat> = self.fields.iter()
            .map(|f| format_field_type(f, ctx, Some(class_name.as_str())))
            .collect();

        fmtools::fmt! { move
            // Package declaration
            if let Some(pkg) = package {
                "package "{pkg}";\n\n"
            }
            // Type header with index
            if opts.show_type_indices {
                if let Some(idx) = type_idx {
                    "// Type: "{self.name}" (type@"{idx}")\n"
                }
            }
            {opts}"class "{simple_name}
            if let Some(parent) = parent_display {
                " extends "{parent}
            }
            " {\n"
            // Fields - bytecode doesn't preserve visibility, so make all public (private by default in Haxe)
            for (i, (f, ft)) in self.fields.iter().zip(field_types.iter()).enumerate() {
                {new_opts}if f.static_ { "public static " } else { "public " } "var "{f.name}": "{&ft.type_str}
                if let Some(init) = &f.initializer {
                    " = "{init.display_simple(ctx, &new_opts)}
                }
                ";"
                if opts.show_field_indices {
                    "  // F"{i}", type@"{f.ty.0}
                }
                if let Some(comment) = &ft.comment {
                    "  "{comment}
                }
                "\n"
            }
            for m in &self.methods {
                "\n"
                {m.display_in_class(ctx, &new_opts, Some(class_name.as_str()))}
            }
            {opts}"}"
        }
    }
}

impl Method {
    pub fn display<'a>(&'a self, ctx: &'a Bytecode, opts: &'a FormatOptions) -> impl Display + 'a {
        self.display_in_class(ctx, opts, None)
    }

    pub fn display_in_class<'a>(&'a self, ctx: &'a Bytecode, opts: &'a FormatOptions, current_class: Option<&'a str>) -> impl Display + 'a {
        let new_opts = opts.inc_nesting();
        let fun = self.fun.as_fn(ctx).unwrap();
        let fun_idx = self.fun.0;
        let nops = fun.ops.len();
        let name = fun.name(ctx);
        let is_constructor = name == "__constructor__";
        // For constructors and instance methods, skip the first param (this)
        let skip_params = if self.static_ && !is_constructor { 0 } else { 1 };

        // Compute return type, but skip annotation for private nested types
        // (they can't be referenced explicitly - rely on type inference)
        let ret_type_str = if !fun.ty(ctx).ret.is_void() && !is_constructor {
            let ret_ty = fun.ret(ctx);
            // Check raw type for private nested indicator (._) before formatting
            if is_private_nested_type(ret_ty, ctx) {
                None // Skip annotation for private nested types
            } else {
                Some(to_haxe_type_in_context(ret_ty, ctx, current_class))
            }
        } else {
            None
        };

        fmtools::fmt! { move
            // Function header comment with index
            if opts.show_fun_indices {
                {opts}"// fun@"{fun_idx}" ("{nops}" ops)\n"
            }
            // Don't output 'static' for constructors
            // Add 'override' for methods that override parent methods
            // Add 'public' for all methods (Haxe defaults to private)
            {opts} if self.override_ { "override " } "public " if self.static_ && !is_constructor { "static " } if self.dynamic { "dynamic " }
            // Output 'new' instead of '__constructor__'
            "function " if is_constructor { "new" } else { {name} } "("
            {fmtools::join(", ", fun.args(ctx).iter().enumerate().skip(skip_params)
                .map(move |(i, arg)| {
                    // arg_name expects index relative to user params (excluding this)
                    let name_idx = i - skip_params;
                    let arg_name = fun.arg_name(ctx, name_idx).unwrap_or(Str::from("_"));
                    let type_str = to_haxe_type_in_context(&ctx[*arg], ctx, current_class);
                    // Omit ": Dynamic" since that's the default in Haxe
                    if type_str == "Dynamic" {
                        arg_name.to_string()
                    } else {
                        format!("{}: {}", arg_name, type_str)
                    }
                }))}
            ")" if let Some(ref ret_type) = ret_type_str { ": "{ret_type} } " {"

            if self.statements.is_empty() {
                "}"
            } else {
                "\n"
                for stmt in &self.statements {
                    {new_opts}{stmt.display(&new_opts, ctx, fun)}"\n"
                }
                {opts}"}"
            }
            "\n"
        }
    }
}

impl Constant {
    fn fmt_with_opts(&self, f: &mut Formatter, code: &Bytecode, show_indices: bool) -> fmt::Result {
        use Constant::*;
        match *self {
            InlineInt(c) => Display::fmt(&c, f),
            Int(c) => {
                EnhancedFmt.fmt_refint(f, code, c)?;
                if show_indices {
                    write!(f, " /* int@{} */", c.0)?;
                }
                Ok(())
            }
            Float(c) => {
                EnhancedFmt.fmt_reffloat(f, code, c)?;
                if show_indices {
                    write!(f, " /* float@{} */", c.0)?;
                }
                Ok(())
            }
            String(c) => {
                write!(f, "\"{}\"", escape_string(&code[c]))?;
                if show_indices {
                    write!(f, " /* str@{} */", c.0)?;
                }
                Ok(())
            }
            Bytes(c) => {
                // Format bytes constant as hex string
                if let Some((data, offsets)) = &code.bytes {
                    let start = offsets.get(c.0).copied().unwrap_or(0);
                    let end = offsets.get(c.0 + 1).copied().unwrap_or(data.len());
                    let bytes = &data[start..end];
                    let hex: std::string::String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                    write!(f, "haxe.io.Bytes.ofHex(\"{}\")", hex)?;
                } else {
                    write!(f, "haxe.io.Bytes.ofHex(\"\")")?;
                }
                if show_indices {
                    write!(f, " /* bytes@{} */", c.0)?;
                }
                Ok(())
            }
            Bool(c) => Display::fmt(&c, f),
            Null => f.write_str("null"),
            This => f.write_str("this"),
            TypeRef(ty) => {
                write!(f, "{}", ty.display::<HaxeFmt>(code))?;
                if show_indices {
                    write!(f, " /* type@{} */", ty.0)?;
                }
                Ok(())
            }
        }
    }
}

impl Operation {
    pub fn display<'a>(
        &'a self,
        indent: &'a FormatOptions,
        code: &'a Bytecode,
        f: &'a Function,
    ) -> impl Display + 'a {
        OperationDisplay {
            op: self,
            indent,
            code,
            f,
        }
    }
}

/// Helper to determine if an expression needs parentheses when used as operand
fn needs_parens(expr: &Expr, parent_prec: u8, is_right: bool) -> bool {
    if let Expr::Op(child_op) = expr {
        let child_prec = child_op.precedence();
        // RHS needs parens if equal or lower precedence (right-to-left would need different handling)
        // LHS needs parens if strictly lower precedence
        if is_right {
            child_prec <= parent_prec
        } else {
            child_prec < parent_prec
        }
    } else {
        false
    }
}

struct OperationDisplay<'a> {
    op: &'a Operation,
    indent: &'a FormatOptions,
    code: &'a Bytecode,
    f: &'a Function,
}

impl<'a> Display for OperationDisplay<'a> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Operation::*;

        let disp = |e: &'a Expr| e.display(self.indent, self.code, self.f);
        let prec = self.op.precedence();

        match self.op {
            Add(e1, e2) | Sub(e1, e2) | Mul(e1, e2) | Div(e1, e2) | Mod(e1, e2) => {
                let op_str = match self.op {
                    Add(_, _) => "+",
                    Sub(_, _) => "-",
                    Mul(_, _) => "*",
                    Div(_, _) => "/",
                    Mod(_, _) => "%",
                    _ => unreachable!(),
                };
                if needs_parens(e1, prec, false) {
                    write!(fmt, "({})", disp(e1))?;
                } else {
                    write!(fmt, "{}", disp(e1))?;
                }
                write!(fmt, " {} ", op_str)?;
                if needs_parens(e2, prec, true) {
                    write!(fmt, "({})", disp(e2))?;
                } else {
                    write!(fmt, "{}", disp(e2))?;
                }
                Ok(())
            }
            // Shift and bitwise always wrapped in parens (low precedence)
            Shl(e1, e2) => write!(fmt, "({} << {})", disp(e1), disp(e2)),
            Shr(e1, e2) => write!(fmt, "({} >> {})", disp(e1), disp(e2)),
            And(e1, e2) => write!(fmt, "({} & {})", disp(e1), disp(e2)),
            Or(e1, e2) => write!(fmt, "({} | {})", disp(e1), disp(e2)),
            Xor(e1, e2) => write!(fmt, "({} ^ {})", disp(e1), disp(e2)),
            // Unary
            Neg(expr) => write!(fmt, "-{}", disp(expr)),
            Not(inner) => {
                // Add parentheses for complex expressions (comparisons, binary ops)
                // The inner is a Box<Expr>, so we need to check if it's an Op variant
                let needs_parens = if let Expr::Op(op) = inner.as_ref() {
                    matches!(op,
                        Operation::Eq(..) | Operation::NotEq(..) |
                        Operation::Lt(..) | Operation::Lte(..) |
                        Operation::Gt(..) | Operation::Gte(..) |
                        Operation::Add(..) | Operation::Sub(..) |
                        Operation::Mul(..) | Operation::Div(..) |
                        Operation::Mod(..) | Operation::Shl(..) |
                        Operation::Shr(..) | Operation::And(..) |
                        Operation::Or(..) | Operation::Xor(..)
                    )
                } else {
                    false
                };
                if needs_parens {
                    write!(fmt, "!({})", disp(inner))
                } else {
                    write!(fmt, "!{}", disp(inner))
                }
            }
            Incr(expr) => write!(fmt, "{}++", disp(expr)),
            Decr(expr) => write!(fmt, "{}--", disp(expr)),
            // Comparison
            Eq(e1, e2) => write!(fmt, "{} == {}", disp(e1), disp(e2)),
            NotEq(e1, e2) => write!(fmt, "{} != {}", disp(e1), disp(e2)),
            Gt(e1, e2) => write!(fmt, "{} > {}", disp(e1), disp(e2)),
            Gte(e1, e2) => write!(fmt, "{} >= {}", disp(e1), disp(e2)),
            Lt(e1, e2) => write!(fmt, "{} < {}", disp(e1), disp(e2)),
            Lte(e1, e2) => write!(fmt, "{} <= {}", disp(e1), disp(e2)),
        }
    }
}

/// Helper enum for special call handling
enum CallHandling<'a> {
    SpecialFormat(String),
    Elide(&'a Expr),
    /// Skip internal methods that shouldn't be visible (e.g., __expand)
    Skip,
    Normal,
}

/// Helper struct for displaying simple expressions
pub struct SimpleExprDisplay<'a> {
    expr: &'a Expr,
    code: &'a Bytecode,
}

impl<'a> Display for SimpleExprDisplay<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.expr {
            Expr::Constant(c) => c.fmt_with_opts(f, self.code, false),
            Expr::Ident(s) => write!(f, "{}", s),
            Expr::Variable(_, Some(name)) => write!(f, "{}", name),
            other => panic!("display_simple called with complex expression: {:?}", other),
        }
    }
}

impl Expr {
    /// Display a simple expression without needing a Function context.
    /// Works for constants and identifiers (used for field initializers).
    pub fn display_simple<'a>(
        &'a self,
        code: &'a Bytecode,
        _opts: &'a FormatOptions,
    ) -> impl Display + 'a {
        SimpleExprDisplay { expr: self, code }
    }

    pub fn display<'a>(
        &'a self,
        indent: &'a FormatOptions,
        code: &'a Bytecode,
        f: &'a Function,
    ) -> impl Display + 'a {
        macro_rules! disp {
            ($e:expr) => {
                $e.display(indent, code, f)
            };
        }
        fmtools::fmt! { move
            match self {
                Expr::Anonymous(ty, values) => match &code[*ty] {
                    Type::Virtual { fields } => {
                        "{"{ fmtools::join(", ", fields
                            .iter()
                            .enumerate()
                            .filter_map(|(i, f)| {
                                // Only include fields that have values
                                values.get(&RefField(i)).map(|v| {
                                    fmtools::fmt! { move
                                        {f.name(code)}": "{disp!(v)}
                                    }
                                })
                            })) }"}"
                    }
                    other => {{panic_invalid_anon_type(other)}},
                },
                Expr::Array(array, index) => {
                    // Check for array.bytes[index << 2] pattern (HL array access)
                    // and convert to array[index] for valid Haxe
                    let simplified = if let Expr::Field(obj, field) = array.as_ref() {
                        if field.as_ref() == "bytes" {
                            // Extract the unshifted index: if index is `x << 2`, use `x`
                            let real_index = if let Expr::Op(Operation::Shl(left, right)) = index.as_ref() {
                                if let Expr::Constant(Constant::InlineInt(2)) = right.as_ref() {
                                    Some((obj.as_ref(), left.as_ref()))
                                } else if let Expr::Constant(Constant::Int(ref_int)) = right.as_ref() {
                                    if code[*ref_int] == 2 {
                                        Some((obj.as_ref(), left.as_ref()))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            };
                            real_index
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some((arr, idx)) = simplified {
                        {disp!(arr)}"["{disp!(idx)}"]"
                    } else {
                        {disp!(array)}"["{disp!(index)}"]"
                    }
                }
                Expr::ArrayLiteral(elems) => {
                    "["{fmtools::join(", ", elems.iter().map(|e| disp!(e)))}"]"
                }
                Expr::Call(call) => {
                    // Check for builtin functions that need special handling or elision
                    let handling = if let Expr::FunRef(fun_ref) = &call.fun {
                        let name = fun_ref.name(code);
                        match name.as_ref() {
                            // itos/ftos/dtos are internal - they return intermediate bytes
                            // The actual string is created by __alloc__, so skip these
                            "itos" | "ftos" | "dtos" => CallHandling::Skip,
                            // __alloc__ creates a String from bytes
                            // If the first arg looks like intermediate bytes from itos/ftos/dtos, emit Std.string()
                            // Otherwise just elide to the first arg
                            "__alloc__" => {
                                // Check if first arg is a variable that might be from itos/ftos/dtos
                                // For now, emit Std.string() wrapping the second arg (the original value)
                                // since __alloc__(bytes, length) where length often comes from the original value
                                if call.args.len() >= 2 {
                                    // Second arg is typically the value that was converted
                                    CallHandling::SpecialFormat(format!("Std.string({})", call.args[1].display(indent, code, f)))
                                } else {
                                    call.args.first().map(CallHandling::Elide).unwrap_or(CallHandling::Normal)
                                }
                            }
                            // thrown wraps an exception - use the argument
                            "thrown" => call.args.first().map(CallHandling::Elide).unwrap_or(CallHandling::Normal),
                            // caught wraps a raw exception in haxe.Exception
                            // With :Dynamic catch type, the value is already raw, so elide
                            "caught" => call.args.first().map(CallHandling::Elide).unwrap_or(CallHandling::Normal),
                            // string converts dynamic to string - use Std.string()
                            "string" => {
                                call.args.first().map(|arg| {
                                    CallHandling::SpecialFormat(format!("Std.string({})", arg.display(indent, code, f)))
                                }).unwrap_or(CallHandling::Normal)
                            }
                            // Internal array allocation functions need fully qualified names
                            "allocI32" | "allocI64" | "allocF64" | "allocObj" | "allocDyn" => {
                                // These are hl.types.ArrayBase.allocXXX functions
                                CallHandling::SpecialFormat(format!(
                                    "hl.types.ArrayBase.{}({})",
                                    name,
                                    call.args.iter().map(|a| format!("{}", a.display(indent, code, f))).collect::<Vec<_>>().join(", ")
                                ))
                            }
                            // alloc_bytes is also internal - use haxe.io.Bytes.alloc
                            "alloc_bytes" => {
                                CallHandling::SpecialFormat(format!(
                                    "haxe.io.Bytes.alloc({})",
                                    call.args.iter().map(|a| format!("{}", a.display(indent, code, f))).collect::<Vec<_>>().join(", ")
                                ))
                            }
                            // Internal array methods that shouldn't be visible
                            "__expand" | "__construct" => CallHandling::Skip,
                            // __constructor__ is called after new Type() - skip since object is already created
                            "__constructor__" => CallHandling::Skip,
                            // __add__ is string concatenation - convert String.__add__(a, b) to (a + b)
                            "__add__" => {
                                if call.args.len() == 2 {
                                    let left = &call.args[0];
                                    let right = &call.args[1];
                                    CallHandling::SpecialFormat(format!(
                                        "({} + {})",
                                        left.display(indent, code, f),
                                        right.display(indent, code, f)
                                    ))
                                } else {
                                    CallHandling::Normal
                                }
                            }
                            _ => CallHandling::Normal,
                        }
                    } else if let Expr::Field(receiver, method) = &call.fun {
                        // Check for method calls that should be elided or skipped
                        match method.as_ref() {
                            // unwrap extracts the original thrown value from haxe.Exception
                            // With :Dynamic catch type and caught() elided, just use the receiver
                            "unwrap" => CallHandling::SpecialFormat(format!(
                                "{}",
                                receiver.display(indent, code, f)
                            )),
                            // __exceptionMessage is the old/internal name for the same thing
                            "__exceptionMessage" => CallHandling::SpecialFormat(format!(
                                "{}",
                                receiver.display(indent, code, f)
                            )),
                            // Internal array methods that shouldn't be visible
                            "__expand" | "__construct" => CallHandling::Skip,
                            // __constructor__ is called after new Type() - skip since object is already created
                            "__constructor__" => CallHandling::Skip,
                            // __add__ is string concatenation - convert String.__add__(a, b) to (a + b)
                            "__add__" => {
                                if call.args.len() == 2 {
                                    let left = &call.args[0];
                                    let right = &call.args[1];
                                    CallHandling::SpecialFormat(format!(
                                        "({} + {})",
                                        left.display(indent, code, f),
                                        right.display(indent, code, f)
                                    ))
                                } else {
                                    CallHandling::Normal
                                }
                            }
                            _ => CallHandling::Normal,
                        }
                    } else {
                        CallHandling::Normal
                    };

                    match handling {
                        CallHandling::SpecialFormat(s) => {{s}}
                        CallHandling::Elide(replacement) => {{disp!(replacement)}}
                        CallHandling::Skip => {|_f| { panic_internal_call(&call.fun); }},
                        CallHandling::Normal => {
                            {disp!(call.fun)}"("{fmtools::join(", ", call.args.iter().map(|e| disp!(e)))}")"
                            // Add function index comment if the callee is a FunRef
                            if indent.show_fun_indices {
                                if let Expr::FunRef(fun_ref) = &call.fun {
                                    // Distinguish natives from regular functions in comment
                                    if matches!(code.get(*fun_ref), hlbc::types::FunPtr::Native(_)) {
                                        " /* native@"{fun_ref.0}" */"
                                    } else {
                                        " /* fun@"{fun_ref.0}" */"
                                    }
                                }
                            }
                        }
                    }
                }
                Expr::Constant(c) => {|f| c.fmt_with_opts(f, code, indent.show_string_indices)?;},
                Expr::Constructor(ConstructorCall { ty, args }) => {
                    // Get the type name
                    let type_name = to_haxe_type(&code[*ty], code);

                    // Simplify if this is a nested type of the current class
                    // e.g., "NestedClass.Inner" -> "Inner" when inside NestedClass
                    let simplified_name = if let Some(parent_ref) = f.parent {
                        if let Some(parent_obj) = code[parent_ref].get_type_obj() {
                            let parent_name = parent_obj.name(code);
                            // Strip $ prefix from static type holders (e.g., "$NestedClass" -> "NestedClass")
                            let parent_name = parent_name.strip_prefix('$').unwrap_or(&parent_name);
                            let prefix = format!("{}.", parent_name);
                            if type_name.starts_with(&prefix) {
                                // Strip "ParentClass." prefix for nested types
                                type_name[prefix.len()..].to_string()
                            } else {
                                type_name.to_string()
                            }
                        } else {
                            type_name.to_string()
                        }
                    } else {
                        type_name.to_string()
                    };

                    "new "{simplified_name}"("{fmtools::join(", ", args.iter().map(|e| disp!(e)))}")"
                }
                Expr::Closure(f, stmts) => {
                    if let Some(fun) = f.as_fn(code) {
                        let args = &fun.ty(code).args;

                        // Check if first param is closure context (enum type)
                        let has_capture = args.first().map(|t| matches!(&code[*t], Type::Enum { .. })).unwrap_or(false);
                        let skip_first = if has_capture { 1 } else { 0 };

                        // Build parameter names using same logic as DecompilerState::new()
                        // Uses a counter for synthetic names to match body variable references
                        let mut param_counter = 0u32;
                        let param_names = args.iter().enumerate().map(|(i, _)| {
                            fun.arg_name(code, i).unwrap_or_else(|| {
                                let name = Str::from(format!("arg{}", param_counter));
                                param_counter += 1;
                                name
                            })
                        }).collect::<Vec<_>>();

                        // Format visible parameters (skip closure context if present)
                        let params_display = args.iter().enumerate().skip(skip_first).map(|(i, arg)| {
                            format!("{}: {}", param_names[i], to_haxe_type(&code[*arg], code))
                        }).collect::<Vec<_>>().join(", ");
                        "("{params_display}") -> {\n"
                        let indent2 = indent.inc_nesting();
                        for stmt in stmts {
                            {indent2}{stmt.display(&indent2, code, fun)}"\n"
                        }
                        {indent}"}"
                    } else {
                        // Native function reference - emit placeholder
                        "/* native closure fun@"{f.0}" */ () -> {\n"
                        let indent2 = indent.inc_nesting();
                        for stmt in stmts {
                            {indent2}{stmt.display(&indent2, code, &code.functions[0])}"\n"
                        }
                        {indent}"}"
                    }
                }
                Expr::EnumConstr(ty, constr, args) => {
                    // Emit EnumName.ConstructorName(args) syntax
                    if let Type::Enum { name, constructs, .. } = &code[*ty] {
                        let raw_enum_name = code.strings.get(name.0)
                            .map(|s| s.as_ref())
                            .unwrap_or("Enum");
                        // Expand shortened module paths (e.g., haxe.macro.Binop → haxe.macro.Expr.Binop)
                        let enum_name = expand_module_path(raw_enum_name).unwrap_or(raw_enum_name);
                        if let Some(c) = constructs.get(constr.0) {
                            let construct_name = c.name(code);
                            if args.is_empty() {
                                {enum_name}"."{construct_name}
                            } else {
                                {enum_name}"."{construct_name}"("{fmtools::join(", ", args.iter().map(|e| disp!(e)))}")"
                            }
                        } else {
                            {enum_name}".Construct"{constr.0}"("{fmtools::join(", ", args.iter().map(|e| disp!(e)))}")"
                        }
                    } else {
                        "EnumConstruct"{constr.0}"("{fmtools::join(", ", args.iter().map(|e| disp!(e)))}")"
                    }
                }
                Expr::Field(receiver, name) => {
                    {disp!(receiver)}"."{name}
                }
                Expr::FunRef(fun) => {
                    // Check if this is a native function and translate to Haxe name
                    match code.get(*fun) {
                        hlbc::types::FunPtr::Native(n) => {
                            let lib = n.lib(code);
                            let name = n.name(code);
                            // 1. Try dynamic binding lookup first (covers all libraries)
                            if let Some((class_name, method_name)) = crate::natives::lookup_native_binding(code, *fun) {
                                {class_name}"."{method_name}
                            }
                            // 2. Fall back to hardcoded lookup table (stdlib)
                            else if let Some(haxe_name) = crate::natives::lookup_native(&lib, &name) {
                                {haxe_name}
                            } else {
                                // Unknown native - emit as identifier (lib_name style)
                                // This is valid Haxe if an extern declaration exists
                                {lib}"_"{name}
                            }
                        }
                        hlbc::types::FunPtr::Fun(func) => {
                            let name = func.name(code);
                            // For static methods with a parent class, include the class qualifier
                            if let Some(parent_ref) = func.parent {
                                if let Some(parent_obj) = parent_ref.as_obj(code) {
                                    let parent_name = parent_obj.name(code);
                                    // Skip class qualifier for internal HL types
                                    if is_internal_hl_type(&parent_name) {
                                        {name}
                                    } else {
                                        // Strip leading $ from static class type names
                                        let clean_name = parent_name.strip_prefix('$').unwrap_or(&parent_name);
                                        // Check if this is a static method (parent is a static class type)
                                        if parent_name.starts_with('$') {
                                            // Add std. prefix for stdlib classes to avoid shadowing
                                            if needs_std_prefix(clean_name) {
                                                "std."{clean_name}"."{name}
                                            } else {
                                                {clean_name}"."{name}
                                            }
                                        } else {
                                            // Parent is not a static class type - try debug inference
                                            if let Some(class_name) = infer_class_from_debug(code, func) {
                                                // Add std. prefix for stdlib classes
                                                if needs_std_prefix(&class_name) {
                                                    "std."{class_name}"."{name}
                                                } else {
                                                    {class_name}"."{name}
                                                }
                                            } else {
                                                {name}
                                            }
                                        }
                                    }
                                } else {
                                    // Parent ref doesn't resolve to Obj - try debug inference
                                    if let Some(class_name) = infer_class_from_debug(code, func) {
                                        // Add std. prefix for stdlib classes
                                        if needs_std_prefix(&class_name) {
                                            "std."{class_name}"."{name}
                                        } else {
                                            {class_name}"."{name}
                                        }
                                    } else {
                                        {name}
                                    }
                                }
                            } else {
                                // No parent - try to infer class from debug source file path
                                // e.g., "haxe/Resource.hx" -> "haxe.Resource"
                                if let Some(class_name) = infer_class_from_debug(code, func) {
                                    // Add std. prefix for stdlib classes
                                    if needs_std_prefix(&class_name) {
                                        "std."{class_name}"."{name}
                                    } else {
                                        {class_name}"."{name}
                                    }
                                } else {
                                    {name}
                                }
                            }
                        }
                    }
                },
                Expr::IfElse { cond, if_, else_ } => {
                    // Use ternary syntax for simple single-expression branches
                    let if_simple = if_.len() == 1 && matches!(&if_[0], Statement::ExprStatement(_));
                    let else_simple = else_.len() == 1 && matches!(&else_[0], Statement::ExprStatement(_));

                    if if_simple && else_simple {
                        // Extract the expressions from ExprStatement
                        let if_expr = match &if_[0] {
                            Statement::ExprStatement(e) => e,
                            _ => unreachable!(),
                        };
                        let else_expr = match &else_[0] {
                            Statement::ExprStatement(e) => e,
                            _ => unreachable!(),
                        };
                        // Ternary syntax: cond ? a : b
                        "(("{disp!(cond)}") ? "{disp!(if_expr)}" : "{disp!(else_expr)}")"
                    } else {
                        // Block-style if-expression
                        "if ("{disp!(cond)}") {\n"
                        let indent2 = indent.inc_nesting();
                        for stmt in if_ {
                            {indent2}{stmt.display(&indent2, code, f)}"\n"
                        }
                        {indent}"} else {\n"
                        for stmt in else_ {
                            {indent2}{stmt.display(&indent2, code, f)}"\n"
                        }
                        {indent}"}"
                    }
                }
                Expr::Op(op) => {{disp!(op)}},
                Expr::Unknown(msg) => {{panic_unknown_expr(msg)}}
                Expr::Variable(x, name) => {{
                    if let Some(name) = name {
                        name.clone()
                    } else {
                        Str::from(x.to_string())
                    }
                }}
                Expr::Ident(name) => {{
                    name.clone()
                }}
                Expr::Cast(expr, type_name) => {
                    "cast("{disp!(expr)}", "{type_name}")"
                }
                Expr::TypeAnnotated(expr, type_name) => {
                    // For variable declarations: var x:Type
                    {disp!(expr)}":"{type_name}
                }
            }
        }
    }
}

/// Check if an expression is an empty anonymous object (needs :Dynamic type annotation)
fn is_empty_anonymous(expr: &Expr) -> bool {
    matches!(expr, Expr::Anonymous(_, fields) if fields.is_empty())
}

use hlbc::types::EnumConstruct;

/// Format a switch case pattern, handling enum constructor names when applicable
fn format_switch_pattern<'a>(
    pattern: &'a Expr,
    enum_constructs: Option<&'a [EnumConstruct]>,
    indent: &FormatOptions,
    code: &'a Bytecode,
    f: &'a Function,
) -> impl Display + 'a {
    struct PatternFormatter<'a> {
        pattern: &'a Expr,
        enum_constructs: Option<&'a [EnumConstruct]>,
        indent: FormatOptions,
        code: &'a Bytecode,
        f: &'a Function,
    }

    impl<'a> Display for PatternFormatter<'a> {
        fn fmt(&self, fmt: &mut Formatter<'_>) -> fmt::Result {
            // Try to use enum constructor name if this is an enum switch
            if let Some(constructs) = self.enum_constructs {
                // Extract integer index from pattern expression
                let idx = match self.pattern {
                    Expr::Constant(Constant::InlineInt(n)) => Some(*n),
                    Expr::Constant(Constant::Int(ptr)) => Some(self.code.ints[ptr.0] as usize),
                    _ => None,
                };
                if let Some(idx) = idx {
                    if let Some(construct) = constructs.get(idx) {
                        write!(fmt, "{}", self.code.get(construct.name))?;
                        // Add wildcard parameters for constructors with params
                        if !construct.params.is_empty() {
                            write!(fmt, "(")?;
                            for (pi, _) in construct.params.iter().enumerate() {
                                if pi > 0 {
                                    write!(fmt, ", ")?;
                                }
                                write!(fmt, "_")?;
                            }
                            write!(fmt, ")")?;
                        }
                        return Ok(());
                    }
                }
            }
            // Fallback: display pattern as expression
            write!(fmt, "{}", self.pattern.display(&self.indent, self.code, self.f))
        }
    }

    PatternFormatter {
        pattern,
        enum_constructs,
        indent: indent.clone(),
        code,
        f,
    }
}

/// Helper struct to format else-if chains iteratively (avoids stack overflow from recursion)
struct ElseChainFormatter<'a> {
    else_stmts: &'a [Statement],
    indent: &'a FormatOptions,
    code: &'a Bytecode,
    f: &'a Function,
}

impl<'a> Display for ElseChainFormatter<'a> {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> fmt::Result {
        let mut current = self.else_stmts;
        let indent2 = self.indent.inc_nesting();

        while !current.is_empty() {
            if current.len() == 1 {
                if let Statement::IfElse { cond, if_, else_ } = &current[0] {
                    // Continue the else-if chain
                    write!(fmt, " else if ({}) {{\n", cond.display(self.indent, self.code, self.f))?;
                    for stmt in if_ {
                        write!(fmt, "{}{}\n", indent2, stmt.display(&indent2, self.code, self.f))?;
                    }
                    write!(fmt, "{}}}", self.indent)?;
                    // Move to the next else clause (iteration, not recursion)
                    current = else_;
                } else {
                    // Single non-if statement - emit else block and stop
                    write!(fmt, " else {{\n")?;
                    for stmt in current {
                        write!(fmt, "{}{}\n", indent2, stmt.display(&indent2, self.code, self.f))?;
                    }
                    write!(fmt, "{}}}", self.indent)?;
                    break;
                }
            } else {
                // Multiple statements - emit else block and stop
                write!(fmt, " else {{\n")?;
                for stmt in current {
                    write!(fmt, "{}{}\n", indent2, stmt.display(&indent2, self.code, self.f))?;
                }
                write!(fmt, "{}}}", self.indent)?;
                break;
            }
        }
        Ok(())
    }
}

fn format_else_chain<'a>(
    else_stmts: &'a [Statement],
    indent: &'a FormatOptions,
    code: &'a Bytecode,
    f: &'a Function,
) -> impl Display + 'a {
    ElseChainFormatter { else_stmts, indent, code, f }
}

impl Statement {
    pub fn display<'a>(
        &'a self,
        indent: &'a FormatOptions,
        code: &'a Bytecode,
        f: &'a Function,
    ) -> impl Display + 'a {
        macro_rules! disp {
            ($e:expr) => {
                $e.display(indent, code, f)
            };
        }
        fmtools::fmt! { move
            match self {
                Statement::Assign {
                    declaration,
                    variable,
                    assign,
                } => {
                    // Add :Dynamic type annotation for empty anonymous objects
                    // so that field assignments work afterward
                    let needs_dynamic = *declaration && is_empty_anonymous(assign);
                    if *declaration { "var " } else { "" }{disp!(variable)}if needs_dynamic { ":Dynamic" } else { "" }" = "{disp!(assign)}";"
                }
                Statement::ExprStatement(expr) => {
                    {disp!(expr)}";"
                }
                Statement::Return(expr) => {
                    "return" if let Some(e) = expr { " "{disp!(e)} } ";"
                }
                Statement::IfElse { cond, if_, else_ } => {
                    "if ("{disp!(cond)}") {\n"
                    let indent2 = indent.inc_nesting();
                    for stmt in if_ {
                        {indent2}{stmt.display(&indent2, code, f)}"\n"
                    }
                    {indent}"}"
                    if !else_.is_empty() {
                        // Check if else_ is a single IfElse - if so, flatten to "else if"
                        if else_.len() == 1 {
                            if let Statement::IfElse { cond: else_cond, if_: else_if, else_: else_else } = &else_[0] {
                                // Emit "else if" without extra nesting - use the SAME indent level
                                " else if ("{disp!(else_cond)}") {\n"
                                for stmt in else_if {
                                    {indent2}{stmt.display(&indent2, code, f)}"\n"
                                }
                                {indent}"}"
                                // Recursively handle the else-else chain
                                {format_else_chain(else_else, indent, code, f)}
                            } else {
                                // Single non-if statement in else
                                " else {\n"
                                for stmt in else_ {
                                    {indent2}{stmt.display(&indent2, code, f)}"\n"
                                }
                                {indent}"}"
                            }
                        } else {
                            // Multiple statements in else block
                            " else {\n"
                            for stmt in else_ {
                                {indent2}{stmt.display(&indent2, code, f)}"\n"
                            }
                            {indent}"}"
                        }
                    }
                }
                Statement::IfElseChain { branches, else_ } => {
                    let indent2 = indent.inc_nesting();
                    for (i, (cond, body)) in branches.iter().enumerate() {
                        if i == 0 {
                            "if ("{disp!(cond)}") {\n"
                        } else {
                            " else if ("{disp!(cond)}") {\n"
                        }
                        for stmt in body {
                            {indent2}{stmt.display(&indent2, code, f)}"\n"
                        }
                        {indent}"}"
                    }
                    if !else_.is_empty() {
                        " else {\n"
                        for stmt in else_ {
                            {indent2}{stmt.display(&indent2, code, f)}"\n"
                        }
                        {indent}"}"
                    }
                }
                Statement::Switch {arg, default, cases, enum_type} => {
                    // Get enum constructs if this is an enum switch
                    let enum_constructs = enum_type.and_then(|ty| {
                        if let Type::Enum { constructs, .. } = &code[ty] {
                            Some(constructs.as_slice())
                        } else {
                            None
                        }
                    });

                    // For enum switches, extract the inner enum value from Type.enumIndex(x) call
                    // and use just the enum value for proper pattern matching syntax
                    let enum_switch_inner = if enum_constructs.is_some() {
                        // Check if arg is Type.enumIndex(x) and extract x
                        if let Expr::Call(call) = arg {
                            if let Expr::Field(base, method) = &call.fun {
                                if method.as_ref() == "enumIndex" {
                                    if let Expr::Ident(name) = base.as_ref() {
                                        if name.as_ref() == "Type" {
                                            call.args.first()
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    "switch ("
                    if let Some(inner) = enum_switch_inner {
                        {disp!(inner)}
                    } else {
                        {disp!(arg)}
                    }
                    ") {\n"
                    let indent2 = indent.inc_nesting();
                    let indent3 = indent2.inc_nesting();
                    // Output numbered cases first, then default
                    for (patterns, stmts) in cases {
                        // Format combined cases: case 0, 1, 2: or case None, Some(_):
                        {indent2}"case "
                        for (i, pattern) in patterns.iter().enumerate() {
                            if i > 0 { ", " }
                            {format_switch_pattern(pattern, enum_constructs, indent, code, f)}
                        }
                        ":\n"
                        for stmt in stmts {
                            {indent3}{stmt.display(&indent3, code, f)}"\n"
                        }
                    }
                    // Default case comes last
                    if !default.is_empty() {
                        {indent2}"default:\n"
                        for stmt in default {
                            {indent3}{stmt.display(&indent3, code, f)}"\n"
                        }
                    }
                    {indent}"}"
                }
                Statement::While { cond, stmts } => {
                    "while ("{disp!(cond)}") {\n"
                    let indent2 = indent.inc_nesting();
                    for stmt in stmts {
                        {indent2}{stmt.display(&indent2, code, f)}"\n"
                    }
                    {indent}"}"
                }
                Statement::Break => {
                    "break;"
                }
                Statement::Continue => {
                    "continue;"
                }
                Statement::Throw(exc) => {
                    "throw "{disp!(exc)}";"
                }
                Statement::TryCatch { try_stmts, catch_var, catch_stmts } => {
                    "try {\n"
                    let indent2 = indent.inc_nesting();
                    for stmt in try_stmts {
                        {indent2}{stmt.display(&indent2, code, f)}"\n"
                    }
                    // Use :Dynamic to get raw exception value without haxe.Exception wrapping
                    {indent}"} catch ("{catch_var}":Dynamic) {\n"
                    for stmt in catch_stmts {
                        {indent2}{stmt.display(&indent2, code, f)}"\n"
                    }
                    {indent}"}"
                }
                Statement::Comment(comment) => {
                    "// "{comment}
                }
                Statement::Block { stmts } => {
                    "{\n"
                    let indent2 = indent.inc_nesting();
                    for stmt in stmts {
                        {indent2}{stmt.display(&indent2, code, f)}"\n"
                    }
                    {indent}"}"
                }
                Statement::Sequence { stmts } => {
                    // Sequence is like Block but without braces (no new scope)
                    for (i, stmt) in stmts.iter().enumerate() {
                        {stmt.display(indent, code, f)}
                        if i < stmts.len() - 1 { "\n"{indent} }
                    }
                }
                Statement::VarDecl { name, type_hint } => {
                    "var "{name}if let Some(t) = type_hint { ":"{t} }";"
                }
            }
        }
    }
}

/// Try to infer the class name from the function's debug source file path.
/// For standard library functions that don't have a parent type set,
/// we can extract the class from paths like "haxe/Resource.hx" -> "haxe.Resource"
fn infer_class_from_debug(code: &Bytecode, func: &hlbc::types::Function) -> Option<Str> {
    // Get the debug info for this function
    let debug = func.debug_info.as_ref()?;

    // Look for the source file - debug info is (file_idx, line)
    let (file_idx, _) = debug.iter().next()?;
    let file_path = code.debug_files.as_ref()?.get(*file_idx as usize)?;

    // Look for standard library paths like "haxe/Resource.hx" or
    // paths containing "_std/haxe/" which is the HashLink-specific std lib
    let path_str = file_path.as_ref();

    // Try to find the class path starting from known patterns
    // Pattern 1: "_std/haxe/Something.hx" -> "haxe.Something"
    // Pattern 2: "haxe/Something.hx" -> "haxe.Something"
    let class_part = if let Some(pos) = path_str.find("_std/") {
        &path_str[pos + 5..]  // Skip "_std/"
    } else if path_str.starts_with("haxe/") || path_str.contains("/haxe/") {
        // Find the haxe/ part
        if let Some(pos) = path_str.rfind("/haxe/") {
            &path_str[pos + 1..]  // Skip the leading /
        } else if path_str.starts_with("haxe/") {
            path_str
        } else {
            return None;
        }
    } else {
        return None;
    };

    // Convert "haxe/Resource.hx" to "haxe.Resource"
    let without_ext = class_part.strip_suffix(".hx")?;
    let class_name = without_ext.replace('/', ".");

    // Don't add class prefix for main class methods (would cause "BytesTest.main" for BytesTest)
    // Only add for clearly standard library classes
    if class_name.starts_with("haxe.") || class_name.starts_with("sys.") {
        Some(class_name.into())
    } else {
        None
    }
}
