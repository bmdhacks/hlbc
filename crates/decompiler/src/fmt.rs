use std::fmt;
use std::fmt::{Display, Formatter};

use hlbc::fmt::{BytecodeFmt, EnhancedFmt};
use hlbc::types::{Function, RefField, RefType, Type, TypeFun, TypeObj};
use hlbc::Str;
use hlbc::{Bytecode, Resolve};

use crate::ast::{Class, Constant, ConstructorCall, Expr, Method, Operation, Statement};

/// Helper to panic during Display - returns a string that will never be used
fn panic_invalid_anon_type(ty: &Type) -> &'static str {
    panic!("Anonymous expr with non-Virtual type: {:?}", ty)
}

/// Helper to panic for Expr::Unknown during Display
fn panic_unknown_expr(msg: &str) -> &'static str {
    panic!("Expr::Unknown encountered during display: {}", msg)
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
                    _ => write!(f, "{}", name_str),
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
            Type::Enum { name, .. } => write!(f, "{}", ctx.get(*name)),
            Type::Null(reftype) => {
                write!(f, "Null<")?;
                self.fmt_reftype(f, ctx, *reftype)?;
                write!(f, ">")
            }
            Type::Method(fun) => self.fmt_typefun(f, ctx, fun),
            Type::Struct(TypeObj { name, .. }) => write!(f, "{}", ctx.get(*name)),
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

fn to_haxe_type<'a>(ty: &Type, ctx: &'a Bytecode) -> Str {
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
            // Format function types as (Arg1, Arg2) -> RetType or simplified for single arg
            let args: Vec<_> = fun.args.iter().map(|a| to_haxe_type(&ctx[*a], ctx)).collect();
            let ret = to_haxe_type(&ctx[fun.ret], ctx);
            if args.is_empty() {
                Str::from(format!("Void -> {}", ret))
            } else if args.len() == 1 {
                Str::from(format!("{} -> {}", args[0], ret))
            } else {
                Str::from(format!("({}) -> {}", args.join(", "), ret))
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
                _ => name_str,
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
                name_str
            }
        }
        Enum { name, .. } => ctx.get(*name),
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
            // Fields with indices - add 'public' for static fields (private by default in Haxe)
            for (i, f) in self.fields.iter().enumerate() {
                {new_opts} if f.static_ { "public static " } "var "{f.name}": "{to_haxe_type(&ctx[f.ty], ctx)}
                if let Some(init) = &f.initializer {
                    " = "{init.display_simple(ctx, &new_opts)}
                }
                ";"
                if opts.show_field_indices {
                    "  // F"{i}", type@"{f.ty.0}
                }
                "\n"
            }
            for m in &self.methods {
                "\n"
                {m.display(ctx, &new_opts)}
            }
            {opts}"}"
        }
    }
}

impl Method {
    pub fn display<'a>(&'a self, ctx: &'a Bytecode, opts: &'a FormatOptions) -> impl Display + 'a {
        let new_opts = opts.inc_nesting();
        let fun = self.fun.as_fn(ctx).unwrap();
        let fun_idx = self.fun.0;
        let nops = fun.ops.len();
        let name = fun.name(ctx);
        let is_constructor = name == "__constructor__";
        // For constructors and instance methods, skip the first param (this)
        let skip_params = if self.static_ && !is_constructor { 0 } else { 1 };
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
                    fmtools::fmt! {move
                        {fun.arg_name(ctx, name_idx).unwrap_or(Str::from("_"))}": "{to_haxe_type(&ctx[*arg], ctx)}
                    }
                }))}
            ")" if !fun.ty(ctx).ret.is_void() && !is_constructor { ": "{to_haxe_type(fun.ret(ctx), ctx)} } " {"

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
                write!(f, "\"{}\"", code[c])?;
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
            Expr::Constant(c) => match c {
                Constant::InlineInt(i) => write!(f, "{}", i),
                Constant::Int(r) => write!(f, "{}", self.code[*r]),
                Constant::Float(r) => write!(f, "{}", self.code[*r]),
                Constant::String(r) => write!(f, "\"{}\"", self.code[*r]),
                Constant::Bytes(r) => {
                    // Format bytes constant as hex string
                    if let Some((data, offsets)) = &self.code.bytes {
                        let start = offsets.get(r.0).copied().unwrap_or(0);
                        let end = offsets.get(r.0 + 1).copied().unwrap_or(data.len());
                        let bytes = &data[start..end];
                        // Format as haxe.io.Bytes.ofHex("...")
                        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                        write!(f, "haxe.io.Bytes.ofHex(\"{}\")", hex)
                    } else {
                        write!(f, "haxe.io.Bytes.ofHex(\"\")")
                    }
                }
                Constant::Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
                Constant::Null => write!(f, "null"),
                Constant::This => write!(f, "this"),
                Constant::TypeRef(t) => write!(f, "{}", to_haxe_type(&self.code[*t], self.code)),
            },
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
                        CallHandling::Skip => {"0 /* internal */"}
                        CallHandling::Normal => {
                            {disp!(call.fun)}"("{fmtools::join(", ", call.args.iter().map(|e| disp!(e)))}")"
                            // Add function index comment if the callee is a FunRef
                            if indent.show_fun_indices {
                                if let Expr::FunRef(fun_ref) = &call.fun {
                                    " /* fun@"{fun_ref.0}" */"
                                }
                            }
                        }
                    }
                }
                Expr::Constant(c) => {|f| c.fmt_with_opts(f, code, indent.show_string_indices)?;},
                Expr::Constructor(ConstructorCall { ty, args }) => {
                    "new "{ty.display::<HaxeFmt>(code)}"("{fmtools::join(", ", args.iter().map(|e| disp!(e)))}")"
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
                        let enum_name = code.strings.get(name.0)
                            .map(|s| s.as_ref())
                            .unwrap_or("Enum");
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
                            if let Some(haxe_name) = crate::natives::lookup_native(&lib, &name) {
                                {haxe_name}
                            } else {
                                // Unknown native - show raw name for debugging
                                "@native("{lib}"/"{name}")"
                            }
                        }
                        hlbc::types::FunPtr::Fun(func) => {
                            let name = func.name(code);
                            // For static methods with a parent class, include the class qualifier
                            if let Some(parent_ref) = func.parent {
                                if let Some(parent_obj) = parent_ref.as_obj(code) {
                                    let parent_name = parent_obj.name(code);
                                    // Strip leading $ from static class type names
                                    let clean_name = parent_name.strip_prefix('$').unwrap_or(&parent_name);
                                    // Check if this is a static method (parent is a static class type)
                                    if parent_name.starts_with('$') {
                                        {clean_name}"."{name}
                                    } else {
                                        {name}
                                    }
                                } else {
                                    {name}
                                }
                            } else {
                                {name}
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
