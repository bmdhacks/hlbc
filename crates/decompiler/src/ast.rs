use std::collections::HashMap;

use hlbc::fmt::EnhancedFmt;
use hlbc::types::{RefEnumConstruct, RefField, RefFloat, RefFun, RefInt, RefString, RefType, Reg};
use hlbc::{Bytecode, Str};

#[derive(Debug)]
pub struct SourceFile {
    pub class: Class,
}

#[derive(Debug)]
pub struct Class {
    pub name: Str,
    pub parent: Option<Str>,
    pub fields: Vec<ClassField>,
    pub methods: Vec<Method>,
}

#[derive(Debug)]
pub struct ClassField {
    pub name: Str,
    pub ty: RefType,
    pub static_: bool,
    pub initializer: Option<Expr>,
}

#[derive(Debug)]
pub struct Method {
    pub fun: RefFun,
    pub static_: bool,
    pub dynamic: bool,
    pub override_: bool,
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, Copy)]
pub enum Constant {
    InlineInt(usize),
    Int(RefInt),
    Float(RefFloat),
    String(RefString),
    Bool(bool),
    Null,
    /// 'this' instance
    This,
    /// Type reference (for alloc_array, typeof, etc.)
    TypeRef(RefType),
}

#[derive(Debug, Clone)]
pub enum Operation {
    /// `+`
    Add(Box<Expr>, Box<Expr>),
    /// `-`
    Sub(Box<Expr>, Box<Expr>),
    /// `*`
    Mul(Box<Expr>, Box<Expr>),
    /// `/`
    Div(Box<Expr>, Box<Expr>),
    /// `%`
    Mod(Box<Expr>, Box<Expr>),
    /// `<<`
    Shl(Box<Expr>, Box<Expr>),
    /// `>>`
    Shr(Box<Expr>, Box<Expr>),
    /// && &
    And(Box<Expr>, Box<Expr>),
    /// || |
    Or(Box<Expr>, Box<Expr>),
    /// ^
    Xor(Box<Expr>, Box<Expr>),
    /// \-
    Neg(Box<Expr>),
    /// !
    Not(Box<Expr>),
    /// ++
    Incr(Box<Expr>),
    /// --
    Decr(Box<Expr>),
    /// ==
    Eq(Box<Expr>, Box<Expr>),
    /// !=
    NotEq(Box<Expr>, Box<Expr>),
    /// \>
    Gt(Box<Expr>, Box<Expr>),
    /// \>=
    Gte(Box<Expr>, Box<Expr>),
    /// \<
    Lt(Box<Expr>, Box<Expr>),
    /// \<=
    Lte(Box<Expr>, Box<Expr>),
}

impl Operation {
    /// Returns the precedence of this operation (higher = binds tighter).
    /// Used for determining when parentheses are needed.
    pub fn precedence(&self) -> u8 {
        use Operation::*;
        match self {
            // Unary operators - highest precedence
            Neg(_) | Not(_) | Incr(_) | Decr(_) => 10,
            // Multiplicative
            Mul(_, _) | Div(_, _) | Mod(_, _) => 7,
            // Additive
            Add(_, _) | Sub(_, _) => 6,
            // Shift
            Shl(_, _) | Shr(_, _) => 5,
            // Comparison
            Lt(_, _) | Lte(_, _) | Gt(_, _) | Gte(_, _) => 4,
            // Equality
            Eq(_, _) | NotEq(_, _) => 3,
            // Bitwise (already wrapped in parens)
            And(_, _) | Xor(_, _) | Or(_, _) => 2,
        }
    }
}

/// Constructor call
#[derive(Debug, Clone)]
pub struct ConstructorCall {
    pub ty: RefType,
    pub args: Vec<Expr>,
}

impl ConstructorCall {
    pub fn new(ty: RefType, args: Vec<Expr>) -> Self {
        Self { ty, args }
    }
}

/// Function or method call
#[derive(Debug, Clone)]
pub struct Call {
    pub fun: Expr,
    pub args: Vec<Expr>,
}

impl Call {
    pub fn new(fun: Expr, args: Vec<Expr>) -> Self {
        Self { fun, args }
    }

    pub fn new_fun(fun: RefFun, args: Vec<Expr>) -> Self {
        Self {
            fun: Expr::FunRef(fun),
            args,
        }
    }

    /// Create a super constructor call: super(args)
    pub fn new_super(args: Vec<Expr>) -> Self {
        Self {
            fun: Expr::Ident("super".into()),
            args,
        }
    }

    /// Create a super method call: super.method(args)
    pub fn new_super_method(method: Str, args: Vec<Expr>) -> Self {
        Self {
            fun: Expr::Field(Box::new(Expr::Ident("super".into())), method),
            args,
        }
    }
}

/// An expression with a value
#[derive(Debug, Clone)]
pub enum Expr {
    /// An anonymous structure : { field: value }
    Anonymous(RefType, HashMap<RefField, Expr>),
    /// Array access : array\[index]
    Array(Box<Expr>, Box<Expr>),
    /// Array literal : [a, b, c]
    ArrayLiteral(Vec<Expr>),
    /// Function call
    Call(Box<Call>),
    /// Constant value
    Constant(Constant),
    /// Constructor call
    Constructor(ConstructorCall),
    /// Arrow function (...) -> {...}
    Closure(RefFun, Vec<Statement>),
    EnumConstr(RefType, RefEnumConstruct, Vec<Expr>),
    /// Field access : obj.field
    Field(Box<Expr>, Str),
    /// Function reference
    FunRef(RefFun),
    /// If/Else expression, both branches expressions types must unify (https://haxe.org/manual/expression-if.html)
    IfElse {
        cond: Box<Expr>,
        /// Not empty
        if_: Vec<Statement>,
        /// Not empty
        else_: Vec<Statement>,
    },
    /// Operator
    Op(Operation),
    // For when there should be something, but we don't known what
    Unknown(String),
    /// Variable identifier
    Variable(Reg, Option<Str>),
    /// Simple identifier (super, this, etc.)
    Ident(Str),
    /// Type cast: cast(expr, Type)
    Cast(Box<Expr>, Str),
}

pub const fn cst_int(cst: RefInt) -> Expr {
    Expr::Constant(Constant::Int(cst))
}

pub const fn cst_float(cst: RefFloat) -> Expr {
    Expr::Constant(Constant::Float(cst))
}

pub const fn cst_bool(cst: bool) -> Expr {
    Expr::Constant(Constant::Bool(cst))
}

pub const fn cst_string(cst: RefString) -> Expr {
    Expr::Constant(Constant::String(cst))
}

pub const fn cst_null() -> Expr {
    Expr::Constant(Constant::Null)
}

pub const fn cst_this() -> Expr {
    Expr::Constant(Constant::This)
}

pub const fn cst_type(ty: RefType) -> Expr {
    Expr::Constant(Constant::TypeRef(ty))
}

/// Create a shorthand function to create an expression from an operator
macro_rules! make_op_shorthand {
    ($name:ident, $op:ident, $( $e:ident ),+) => {
        #[allow(dead_code)]
        pub(crate) fn $name($( $e: Expr ),+) -> Expr {
            Expr::Op(Operation::$op($( Box::new($e) ),+))
        }
    }
}

make_op_shorthand!(add, Add, e1, e2);
make_op_shorthand!(sub, Sub, e1, e2);
make_op_shorthand!(mul, Mul, e1, e2);
make_op_shorthand!(div, Div, e1, e2);
make_op_shorthand!(modulo, Mod, e1, e2);
make_op_shorthand!(shl, Shl, e1, e2);
make_op_shorthand!(shr, Shr, e1, e2);
make_op_shorthand!(and, And, e1, e2);
make_op_shorthand!(or, Or, e1, e2);
make_op_shorthand!(xor, Xor, e1, e2);
make_op_shorthand!(neg, Neg, e1);
make_op_shorthand!(incr, Incr, e1);
make_op_shorthand!(decr, Decr, e1);
make_op_shorthand!(eq, Eq, e1, e2);
make_op_shorthand!(noteq, NotEq, e1, e2);
make_op_shorthand!(gt, Gt, e1, e2);
make_op_shorthand!(gte, Gte, e1, e2);
make_op_shorthand!(lt, Lt, e1, e2);
make_op_shorthand!(lte, Lte, e1, e2);

/// Invert an expression, will also optimize the expression.
pub fn not(e: Expr) -> Expr {
    use Expr::Op;
    use Operation::*;
    match e {
        Op(Not(a)) => *a,
        Op(Eq(a, b)) => Op(NotEq(a, b)),
        Op(NotEq(a, b)) => Op(Eq(a, b)),
        Op(Gt(a, b)) => Op(Lte(a, b)),
        Op(Gte(a, b)) => Op(Lt(a, b)),
        Op(Lt(a, b)) => Op(Gte(a, b)),
        Op(Lte(a, b)) => Op(Gt(a, b)),
        _ => Op(Not(Box::new(e))),
    }
}

/// Flip the operands of an expression
pub fn flip(e: Expr) -> Expr {
    use Expr::Op;
    use Operation::*;
    match e {
        Op(Add(a, b)) => Op(Add(b, a)),
        Op(Eq(a, b)) => Op(Eq(b, a)),
        Op(NotEq(a, b)) => Op(NotEq(b, a)),
        Op(Gt(a, b)) => Op(Lt(b, a)),
        Op(Gte(a, b)) => Op(Lte(b, a)),
        Op(Lt(a, b)) => Op(Gt(b, a)),
        Op(Lte(a, b)) => Op(Gte(b, a)),
        _ => e,
    }
}

pub fn array(array: Expr, index: Expr) -> Expr {
    Expr::Array(Box::new(array), Box::new(index))
}

pub fn call(fun: Expr, args: Vec<Expr>) -> Expr {
    Expr::Call(Box::new(Call::new(fun, args)))
}

pub fn call_fun(fun: RefFun, args: Vec<Expr>) -> Expr {
    Expr::Call(Box::new(Call::new_fun(fun, args)))
}

pub fn field(expr: Expr, obj: RefType, field: RefField, code: &Bytecode) -> Expr {
    let field_name = field.display::<EnhancedFmt>(code, &code[obj]).to_string();
    // Empty field names are internal virtual interface fields
    let field_name = if field_name.is_empty() {
        "__proto".to_string()
    } else {
        field_name
    };
    Expr::Field(Box::new(expr), Str::from(field_name))
}

/// Get a method expression for a CallMethod opcode.
/// Unlike `field`, this uses vtable/pindex lookup to resolve the method name.
pub fn method(expr: Expr, obj: RefType, pindex: RefField, code: &Bytecode) -> Expr {
    // Use the proper method lookup that searches protos by pindex
    let method_name = obj.method(pindex.0, code)
        .map(|proto| proto.name(code).to_string())
        .unwrap_or_else(|| panic!("Failed to find method pindex={} on type {:?}", pindex.0, obj));
    Expr::Field(Box::new(expr), Str::from(method_name))
}

#[derive(Debug, Clone)]
pub enum Statement {
    /// Variable assignment
    Assign {
        /// Should 'var' appear
        declaration: bool,
        variable: Expr,
        assign: Expr,
    },
    /// Expression statement
    ExprStatement(Expr),
    /// Return an expression or nothing (void)
    Return(Option<Expr>),
    /// If/Else statement
    IfElse {
        cond: Expr,
        if_: Vec<Statement>,
        /// Else clause if the vec isn't empty
        else_: Vec<Statement>,
    },
    /// Flat if-else-if chain (avoids deep recursion for long chains)
    IfElseChain {
        /// Conditions and their bodies: [(cond1, body1), (cond2, body2), ...]
        branches: Vec<(Expr, Vec<Statement>)>,
        /// Final else body (may be empty)
        else_: Vec<Statement>,
    },
    Switch {
        arg: Expr,
        default: Vec<Statement>,
        /// Cases with expression patterns (e.g., case 0, 1, 2: or case "foo", "bar":)
        cases: Vec<(Vec<Expr>, Vec<Statement>)>,
        /// If this switch is on an enum constructor index, the enum type for lookup
        enum_type: Option<RefType>,
    },
    /// While statement
    While {
        cond: Expr,
        stmts: Vec<Statement>,
    },
    Break,
    Continue,
    Throw(Expr),
    TryCatch {
        try_stmts: Vec<Statement>,
        catch_var: String,
        catch_stmts: Vec<Statement>,
    },
    Comment(String),
    /// A block of statements (used for orphan scopes)
    Block {
        stmts: Vec<Statement>,
    },
    /// A sequence of statements without creating a new scope (no braces)
    /// Used for hoisting declarations before switch statements
    Sequence {
        stmts: Vec<Statement>,
    },
    /// Variable declaration without initialization (e.g., `var x;` or `var x:Dynamic;`)
    VarDecl {
        name: Str,
        /// Optional type hint (e.g., "Dynamic" for vars that will hold anonymous objects)
        type_hint: Option<Str>,
    },
}

/// Create an expression statement
pub fn stmt(e: Expr) -> Statement {
    Statement::ExprStatement(e)
}

pub fn comment(comment: impl Into<String>) -> Statement {
    Statement::Comment(comment.into())
}
