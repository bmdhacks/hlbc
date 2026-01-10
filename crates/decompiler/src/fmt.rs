use std::fmt;
use std::fmt::{Display, Formatter};

use hlbc::fmt::{BytecodeFmt, EnhancedFmt};
use hlbc::types::{Function, RefField, RefType, Type, TypeFun, TypeObj};
use hlbc::Str;
use hlbc::{Bytecode, Resolve};

use crate::ast::{Class, Constant, ConstructorCall, Expr, Method, Operation, Statement};

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
        FormatOptions {
            indent: &INDENT[..self.indent.len() + self.inc_indent],
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
        Obj(obj) | Struct(obj) => ctx.get(obj.name),
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
        Ref(_) => Str::from_static("hl.Ref"),
        Null(_) => Str::from_static("Null"),
        Packed(_) => Str::from_static("Dynamic"),
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
        // Also extract simple parent name
        let simple_parent = self.parent.as_ref().map(|p| {
            if let Some(pos) = p.rfind('.') {
                &p[pos + 1..]
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
            if let Some(parent) = simple_parent {
                " extends "{parent}
            }
            " {\n"
            // Fields with indices
            for (i, f) in self.fields.iter().enumerate() {
                {new_opts} if f.static_ { "static " } "var "{f.name}": "{to_haxe_type(&ctx[f.ty], ctx)}";"
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
            {opts} if self.static_ && !is_constructor { "static " } if self.dynamic { "dynamic " }
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
    #[allow(dead_code)]
    fn fmt(&self, f: &mut Formatter, code: &Bytecode) -> fmt::Result {
        self.fmt_with_opts(f, code, false)
    }

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
            Not(expr) => write!(fmt, "!{}", disp(expr)),
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

impl Expr {
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
                    _ => "[invalid anonymous type]",
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
                            // itos/ftos/dtos convert numbers to strings - use Std.string(first_arg)
                            "itos" | "ftos" | "dtos" => {
                                call.args.first().map(|arg| {
                                    CallHandling::SpecialFormat(format!("Std.string({})", arg.display(indent, code, f)))
                                }).unwrap_or(CallHandling::Normal)
                            }
                            // __alloc__ creates a String from bytes - use the first arg
                            "__alloc__" => call.args.first().map(CallHandling::Elide).unwrap_or(CallHandling::Normal),
                            // thrown wraps an exception - use the argument
                            "thrown" => call.args.first().map(CallHandling::Elide).unwrap_or(CallHandling::Normal),
                            // caught extracts exception from HL wrapper - use the argument
                            "caught" => call.args.first().map(CallHandling::Elide).unwrap_or(CallHandling::Normal),
                            // string converts to string - use the argument
                            "string" => call.args.first().map(CallHandling::Elide).unwrap_or(CallHandling::Normal),
                            // Internal array methods that shouldn't be visible
                            "__expand" | "__construct" => CallHandling::Skip,
                            _ => CallHandling::Normal,
                        }
                    } else if let Expr::Field(receiver, method) = &call.fun {
                        // Check for method calls that should be elided or skipped
                        match method.as_ref() {
                            // __exceptionMessage extracts actual exception value - return receiver
                            "__exceptionMessage" => CallHandling::Elide(receiver.as_ref()),
                            // Internal array methods that shouldn't be visible
                            "__expand" | "__construct" => CallHandling::Skip,
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
                    let fun = f.as_fn(code).unwrap();
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
                }
                Expr::EnumConstr(ty, constr, args) => {
                    {constr.display::<EnhancedFmt>(code, &code[*ty])}"("{fmtools::join(", ", args.iter().map(|e| disp!(e)))}")"
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
                        _ => {{fun.name(code)}}
                    }
                },
                Expr::IfElse { cond, if_, else_ } => {
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
                Expr::Op(op) => {{disp!(op)}},
                Expr::Unknown(msg) => {
                     "["{msg}"]"
                }
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
            }
        }
    }
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
                    if *declaration { "var " } else { "" }{disp!(variable)}" = "{disp!(assign)}";"
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
                    if !default.is_empty() {
                        {indent2}"default:\n"
                        for stmt in default {
                            {indent3}{stmt.display(&indent3, code, f)}"\n"
                        }
                    }
                    for (patterns, stmts) in cases {
                        // Format combined cases: case 0, 1, 2: or case None, Some(_):
                        {indent2}"case "
                        for (i, pattern) in patterns.iter().enumerate() {
                            if i > 0 { ", " }
                            // If this is an enum switch, use constructor name instead of index
                            if let Some(constructs) = enum_constructs {
                                if let Some(construct) = constructs.get(*pattern) {
                                    {code.get(construct.name)}
                                    // Add wildcard parameters for constructors with params
                                    if !construct.params.is_empty() {
                                        "("
                                        for (pi, _) in construct.params.iter().enumerate() {
                                            if pi > 0 { ", " }
                                            "_"
                                        }
                                        ")"
                                    }
                                } else {
                                    {pattern}
                                }
                            } else {
                                {pattern}
                            }
                        }
                        ":\n"
                        for stmt in stmts {
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
                    {indent}"} catch ("{catch_var}") {\n"
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
                Statement::VarDecl { name } => {
                    "var "{name}";"
                }
            }
        }
    }
}
