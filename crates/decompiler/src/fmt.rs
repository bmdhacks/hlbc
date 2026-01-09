use std::fmt;
use std::fmt::{Display, Formatter};

use hlbc::fmt::{BytecodeFmt, EnhancedFmt};
use hlbc::types::{Function, RefField, Type};
use hlbc::Str;
use hlbc::{Bytecode, Resolve};

use crate::ast::{Class, Constant, ConstructorCall, Expr, Method, Operation, Statement};

const INDENT: &str = "                                                                ";

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

fn to_haxe_type<'a>(ty: &Type, ctx: &'a Bytecode) -> impl Display + 'a {
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
        Fun(_) | Method(_) => Str::from_static("Function"),
        Obj(obj) | Struct(obj) => ctx.get(obj.name),
        Array => Str::from_static("Array<Dynamic>"),
        Type => Str::from_static("Class<Dynamic>"),
        Abstract { name } => ctx.get(*name),
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
        fmtools::fmt! { move
            // Type header with index
            if opts.show_type_indices {
                if let Some(idx) = type_idx {
                    "// Type: "{self.name}" (type@"{idx}")\n"
                }
            }
            {opts}"class "{self.name}
            if let Some(parent) = self.parent.as_ref() {
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
        fmtools::fmt! { move
            // Function header comment with index
            if opts.show_fun_indices {
                {opts}"// fun@"{fun_idx}" ("{nops}" ops)\n"
            }
            {opts} if self.static_ { "static " } if self.dynamic { "dynamic " }
            "function "{fun.name(ctx)}"("
            {fmtools::join(", ", fun.args(ctx).iter().enumerate().skip(if self.static_ { 0 } else { 1 })
                .map(move |(i, arg)| fmtools::fmt! {move
                    {fun.arg_name(ctx, i).unwrap_or(Str::from("_"))}": "{to_haxe_type(&ctx[*arg], ctx)}
                }))}
            ")" if !fun.ty(ctx).ret.is_void() { ": "{to_haxe_type(fun.ret(ctx), ctx)} } " {"

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
                write!(f, "{}", ty.display::<EnhancedFmt>(code))?;
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
        use Operation::*;
        macro_rules! disp {
            ($e:ident) => {
                $e.display(indent, code, f)
            };
        }
        fmtools::fmt! { move
            match self {
                Add(e1, e2) => {{disp!(e1)}" + "{disp!(e2)}}
                Sub(e1, e2) => {{disp!(e1)}" - "{disp!(e2)}}
                Mul(e1, e2) => {{disp!(e1)}" * "{disp!(e2)}}
                Div(e1, e2) => {{disp!(e1)}" / "{disp!(e2)}}
                Mod(e1, e2) => {{disp!(e1)}" % "{disp!(e2)}}
                Shl(e1, e2) => {{disp!(e1)}" << "{disp!(e2)}}
                Shr(e1, e2) => {{disp!(e1)}" >> "{disp!(e2)}}
                And(e1, e2) => {{disp!(e1)}" && "{disp!(e2)}}
                Or(e1, e2) => {{disp!(e1)}" || "{disp!(e2)}}
                Xor(e1, e2) => {{disp!(e1)}" ^ "{disp!(e2)}}
                Neg(expr) => {"-"{disp!(expr)}}
                Not(expr) => {"!"{disp!(expr)}}
                Incr(expr) => {{disp!(expr)}"++"}
                Decr(expr) => {{disp!(expr)}"--"}
                Eq(e1, e2) => {{disp!(e1)}" == "{disp!(e2)}}
                NotEq(e1, e2) => {{disp!(e1)}" != "{disp!(e2)}}
                Gt(e1, e2) => {{disp!(e1)}" > "{disp!(e2)}}
                Gte(e1, e2) => {{disp!(e1)}" >= "{disp!(e2)}}
                Lt(e1, e2) => {{disp!(e1)}" < "{disp!(e2)}}
                Lte(e1, e2) => {{disp!(e1)}" <= "{disp!(e2)}}
            }
        }
    }
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
                    {disp!(array)}"["{disp!(index)}"]"
                }
                Expr::Call(call) => {
                    {disp!(call.fun)}"("{fmtools::join(", ", call.args.iter().map(|e| disp!(e)))}")"
                    // Add function index comment if the callee is a FunRef
                    if indent.show_fun_indices {
                        if let Expr::FunRef(fun_ref) = &call.fun {
                            " /* fun@"{fun_ref.0}" */"
                        }
                    }
                }
                Expr::Constant(c) => {|f| c.fmt_with_opts(f, code, indent.show_string_indices)?;},
                Expr::Constructor(ConstructorCall { ty, args }) => {
                    "new "{ty.display::<EnhancedFmt>(code)}"("{fmtools::join(", ", args.iter().map(|e| disp!(e)))}")"
                }
                Expr::Closure(f, stmts) => {
                    let fun = f.as_fn(code).unwrap();
                    "("{fmtools::join(", ", fun.ty(code).args.iter().enumerate().map(move |(i, arg)|
                        fmtools::fmt! { move
                            {fun.arg_name(code, i).unwrap_or(Str::from("_"))}": "{to_haxe_type(&code[*arg], code)}
                        }
                    ))}") -> {\n"
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
                Expr::FunRef(fun) => {{fun.name(code)}},
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
                Statement::Switch {arg, default, cases} => {
                    "switch ("{disp!(arg)}") {\n"
                    let indent2 = indent.inc_nesting();
                    let indent3 = indent2.inc_nesting();
                    if !default.is_empty() {
                        {indent2}"default:\n"
                        for stmt in default {
                            {indent3}{stmt.display(&indent3, code, f)}"\n"
                        }
                    }
                    for (pattern, stmts) in cases {
                        {indent2}"case "{disp!(pattern)}":\n"
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
                    "throw "{disp!(exc)}
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
            }
        }
    }
}
