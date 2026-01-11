use hlbc::Bytecode;

use crate::ast::{add, Constant, ConstructorCall, Expr, Operation, Statement};
use crate::call_fun;

pub(crate) trait AstVisitor {
    fn visit_stmt(&mut self, _code: &Bytecode, _stmt: &mut Statement) {}
    fn visit_expr(&mut self, _code: &Bytecode, _expr: &mut Expr) {}
}

/// Visit everything depth-first
pub(crate) fn visit(
    code: &Bytecode,
    stmts: &mut [Statement],
    visitors: &mut [Box<dyn AstVisitor>],
) {
    // Recurse
    macro_rules! rec {
        ($stmts:expr) => {
            visit(code, $stmts, visitors)
        };
    }
    // Visit an expression
    macro_rules! v {
        ($e:expr) => {
            visit_expr(code, $e, visitors)
        };
    }
    for stmt in stmts {
        // No _ pattern, wouldn't want this match to de-sync when adding new items
        match stmt {
            Statement::Assign {
                assign, variable, ..
            } => {
                v!(assign);
                v!(variable);
            }
            Statement::ExprStatement(e) => {
                v!(e);
            }
            Statement::Return(opt) => {
                if let Some(e) = opt {
                    v!(e);
                }
            }
            Statement::IfElse { cond, if_, else_ } => {
                v!(cond);
                rec!(if_);
                rec!(else_);
            }
            Statement::Switch {
                arg,
                default,
                cases,
                ..
            } => {
                v!(arg);
                rec!(default);
                cases.iter_mut().for_each(|(_, case)| rec!(case));
            }
            Statement::While { cond, stmts } => {
                v!(cond);
                rec!(stmts);
            }
            Statement::Break => {}
            Statement::Continue => {}
            Statement::Throw(e) => {
                v!(e);
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                rec!(try_stmts);
                rec!(catch_stmts);
            }
            Statement::Comment(_) => {}
            Statement::Block { stmts } => {
                rec!(stmts);
            }
            Statement::Sequence { stmts } => {
                rec!(stmts);
            }
            Statement::VarDecl { .. } => {}
        }
        for visitor in visitors.iter_mut() {
            visitor.visit_stmt(code, stmt);
        }
    }
}

/// Visit expressions by depth-first recursion into [Expr].
pub(crate) fn visit_expr(code: &Bytecode, expr: &mut Expr, visitors: &mut [Box<dyn AstVisitor>]) {
    // Recurse
    macro_rules! rec {
        ($e:expr) => {
            visit_expr(code, $e, visitors)
        };
    }
    // Visit statements
    macro_rules! v {
        ($stmts:expr) => {
            visit(code, $stmts, visitors)
        };
    }
    // No _ pattern, wouldn't want this match to de-sync when adding new items
    match expr {
        Expr::Anonymous(_, fields) => {
            for e in fields.values_mut() {
                rec!(e);
            }
        }
        Expr::Array(arr, index) => {
            rec!(arr);
            rec!(index);
        }
        Expr::ArrayLiteral(elems) => {
            for e in elems.iter_mut() {
                rec!(e);
            }
        }
        Expr::Call(call) => {
            rec!(&mut call.fun);
            for arg in call.args.iter_mut() {
                rec!(arg);
            }
        }
        Expr::Constant(_) => {}
        Expr::Constructor(ConstructorCall { args, .. }) => {
            for arg in args {
                rec!(arg);
            }
        }
        // /!\ No recurse in closure, as closure decompilation is already recursive.
        Expr::Closure(_, _) => {}
        Expr::EnumConstr(_, _, args) => {
            for arg in args {
                rec!(arg);
            }
        }
        Expr::Field(obj, _) => {
            rec!(obj);
        }
        Expr::FunRef(_) => {}
        Expr::IfElse { cond, if_, else_ } => {
            rec!(cond);
            v!(if_);
            v!(else_);
        }
        Expr::Op(op) => match op {
            Operation::Add(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Sub(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Mul(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Div(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Mod(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Shl(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Shr(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::And(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Or(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Xor(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Neg(e1) => {
                rec!(e1);
            }
            Operation::Not(e1) => {
                rec!(e1);
            }
            Operation::Incr(e1) => {
                rec!(e1);
            }
            Operation::Decr(e1) => {
                rec!(e1);
            }
            Operation::Eq(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::NotEq(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Gt(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Gte(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Lt(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
            Operation::Lte(e1, e2) => {
                rec!(e1);
                rec!(e2);
            }
        },
        Expr::Unknown(_) => {}
        Expr::Variable(_, _) => {}
        Expr::Ident(_) => {}
    }
    for visitor in visitors.iter_mut() {
        visitor.visit_expr(code, expr);
    }
}

/// Transforms an if/else statement where both branches assign a value to the same variable to an if/else expression.
/// ```haxe
/// if (cond) {
///     var a = 1;
/// } else {
///     a = 2;
/// }
/// ```
/// becomes this :
/// ```haxe
/// var a = if (cond) {
///     1
/// } else {
///     2
/// };
/// ```
pub(crate) struct IfExpressions;

/// Simplify bounds-check patterns for array access.
///
/// When Haxe accesses `arr[index]`, HashLink generates a bounds check:
/// ```
/// if (arr.length <= index) {
///     var x = default;  // Out of bounds - use default
/// } else {
///     var x = arr[...]; // In bounds - read value
/// }
/// ```
///
/// The decompiler incorrectly reconstructs this with self-referential variables:
/// `var first = if (...) { 0 } else { arr[first] };`
///
/// This visitor detects the pattern and simplifies to just the array access,
/// extracting the correct index from the condition.
pub(crate) struct BoundsCheckSimplify;

impl AstVisitor for BoundsCheckSimplify {
    fn visit_stmt(&mut self, _code: &Bytecode, stmt: &mut Statement) {
        // Match the bounds-check if/else pattern
        let replacement = match stmt {
            Statement::IfElse { cond, if_, else_ } => {
                // Check if both branches assign to the same variable
                match (if_.last(), else_.last()) {
                    (
                        Some(Statement::Assign {
                            declaration: if_decl,
                            variable: if_var,
                            assign: if_assign,
                        }),
                        Some(Statement::Assign {
                            variable: else_var,
                            assign: else_assign,
                            ..
                        }),
                    ) => {
                        // Both must assign to the same register
                        match (if_var, else_var) {
                            (Expr::Variable(r1, name1), Expr::Variable(r2, _)) if r1 == r2 => {
                                // Try to extract arr and index from the bounds check
                                if let Some((arr_expr, index_expr)) = extract_bounds_check_parts(cond) {
                                    if is_default_value(if_assign) && is_array_access(else_assign) {
                                        // Rebuild array access with correct index
                                        let fixed_array_access = Expr::Array(
                                            Box::new(arr_expr),
                                            Box::new(index_expr),
                                        );
                                        Some(Statement::Assign {
                                            declaration: *if_decl,
                                            variable: Expr::Variable(*r1, name1.clone()),
                                            assign: fixed_array_access,
                                        })
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        };

        if let Some(new_stmt) = replacement {
            *stmt = new_stmt;
        }
    }
}

/// Extract array and index from a bounds check condition.
/// Returns (arr, index) if this looks like `arr.length <= index` or similar.
fn extract_bounds_check_parts(cond: &Expr) -> Option<(Expr, Expr)> {
    match cond {
        Expr::Op(Operation::Lte(left, right)) => {
            // arr.length <= INDEX
            if let Expr::Field(arr, name) = left.as_ref() {
                if name == "length" {
                    return Some((*arr.clone(), *right.clone()));
                }
            }
            None
        }
        Expr::Op(Operation::Lt(left, right)) => {
            // INDEX < arr.length (inverted check)
            if let Expr::Field(arr, name) = right.as_ref() {
                if name == "length" {
                    return Some((*arr.clone(), *left.clone()));
                }
            }
            None
        }
        _ => None,
    }
}

/// Check if expression is a default value (constant 0, null, or empty)
fn is_default_value(expr: &Expr) -> bool {
    match expr {
        Expr::Constant(Constant::InlineInt(0)) => true,
        Expr::Constant(Constant::Null) => true,
        // Match any constant as a default
        Expr::Constant(_) => true,
        _ => false,
    }
}

/// Check if expression is an array access
fn is_array_access(expr: &Expr) -> bool {
    matches!(expr, Expr::Array(_, _))
}

impl AstVisitor for IfExpressions {
    fn visit_stmt(&mut self, _code: &Bytecode, stmt: &mut Statement) {
        let opt = match stmt {
            Statement::IfElse { cond, if_, else_ } => {
                // We only have to check the last statement in each branches.
                // We assume their types to be the same (checked by the haxe compiler)
                match if_.last() {
                    Some(Statement::Assign {
                        declaration,
                        variable: if_var,
                        assign: if_assign,
                    }) => match else_.last() {
                        Some(Statement::Assign {
                            variable: else_var,
                            assign: else_assign,
                            ..
                        }) => match if_var {
                            Expr::Variable(r1, _) => match else_var {
                                Expr::Variable(r2, _) if r1 == r2 => Some((
                                    *declaration,
                                    if_var.clone(),
                                    cond.clone(),
                                    if_assign.clone(),
                                    else_assign.clone(),
                                    if_.clone(),
                                    else_.clone(),
                                )),
                                _ => None,
                            },
                            _ => None,
                        },
                        _ => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        };

        if let Some((decl, var, cond, if_assign, else_assign, mut if_stmts, mut else_stmts)) = opt {
            *if_stmts.last_mut().unwrap() = Statement::ExprStatement(if_assign);
            *else_stmts.last_mut().unwrap() = Statement::ExprStatement(else_assign);
            *stmt = Statement::Assign {
                declaration: decl,
                variable: var,
                assign: Expr::IfElse {
                    cond: Box::new(cond),
                    if_: if_stmts,
                    else_: else_stmts,
                },
            }
        }
    }
}

/// Hoists variable declarations from switch cases to before the switch.
/// In Haxe, each switch case has its own scope, so a variable declared in one case
/// isn't visible in others. When a switch is used as an expression (assigning to a variable
/// in each case), the decompiler may put the declaration in one case (e.g., default).
///
/// This transforms:
/// ```haxe
/// switch (x) {
///     default: var result = "other";
///     case 0: result = "zero";  // ERROR: result not in scope
/// }
/// ```
/// into:
/// ```haxe
/// var result;
/// switch (x) {
///     default: result = "other";
///     case 0: result = "zero";
/// }
/// ```
pub(crate) struct SwitchExpressions;

impl SwitchExpressions {
    /// Collect all variables declared in statements (returns (name, register))
    fn find_declarations(stmts: &[Statement]) -> Vec<(String, hlbc::types::Reg)> {
        let mut decls = Vec::new();
        for stmt in stmts {
            if let Statement::Assign {
                declaration: true,
                variable: Expr::Variable(reg, Some(name)),
                ..
            } = stmt
            {
                decls.push((name.to_string(), *reg));
            }
        }
        decls
    }

    /// Check if a variable name is used (not declared) in statements
    fn is_used_in(stmts: &[Statement], name: &str) -> bool {
        for stmt in stmts {
            match stmt {
                Statement::Assign {
                    declaration: false,
                    variable: Expr::Variable(_, Some(var_name)),
                    ..
                } if var_name.as_ref() == name => return true,
                Statement::Assign { assign, .. } => {
                    if Self::expr_uses_var(assign, name) {
                        return true;
                    }
                }
                Statement::ExprStatement(e) => {
                    if Self::expr_uses_var(e, name) {
                        return true;
                    }
                }
                Statement::Return(Some(e)) => {
                    if Self::expr_uses_var(e, name) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Check if an expression uses a variable by name
    fn expr_uses_var(expr: &Expr, name: &str) -> bool {
        match expr {
            Expr::Variable(_, Some(var_name)) if var_name.as_ref() == name => true,
            Expr::Op(op) => match op {
                Operation::Add(a, b)
                | Operation::Sub(a, b)
                | Operation::Mul(a, b)
                | Operation::Div(a, b)
                | Operation::Mod(a, b)
                | Operation::Eq(a, b)
                | Operation::NotEq(a, b)
                | Operation::Gt(a, b)
                | Operation::Gte(a, b)
                | Operation::Lt(a, b)
                | Operation::Lte(a, b) => Self::expr_uses_var(a, name) || Self::expr_uses_var(b, name),
                Operation::Neg(a) | Operation::Not(a) | Operation::Incr(a) | Operation::Decr(a) => {
                    Self::expr_uses_var(a, name)
                }
                _ => false,
            },
            Expr::Call(call) => {
                call.args.iter().any(|a| Self::expr_uses_var(a, name))
            }
            Expr::Field(obj, _) => Self::expr_uses_var(obj, name),
            _ => false,
        }
    }

    /// Remove the declaration flag from an assignment
    fn undeclare(stmts: &mut [Statement], name: &str) {
        for stmt in stmts {
            if let Statement::Assign {
                declaration,
                variable: Expr::Variable(_, Some(var_name)),
                ..
            } = stmt
            {
                if var_name.as_ref() == name && *declaration {
                    *declaration = false;
                }
            }
        }
    }
}

impl AstVisitor for SwitchExpressions {
    fn visit_stmt(&mut self, _code: &Bytecode, stmt: &mut Statement) {
        let hoisted = match stmt {
            Statement::Switch {
                default, cases, ..
            } => {
                // Collect all declarations from all cases
                let mut all_decls: Vec<(String, hlbc::types::Reg)> = Vec::new();
                all_decls.extend(Self::find_declarations(default));
                for (_, case_stmts) in cases.iter() {
                    all_decls.extend(Self::find_declarations(case_stmts));
                }

                // Find which declarations are used in other cases
                let mut to_hoist = Vec::new();
                for (name, reg) in &all_decls {
                    // Check if this variable is used in default or any case
                    let used_in_default = Self::is_used_in(default, name);
                    let used_in_cases = cases.iter().any(|(_, stmts)| Self::is_used_in(stmts, name));

                    if used_in_default || used_in_cases {
                        to_hoist.push((name.clone(), *reg));
                    }
                }

                // Remove duplicates
                to_hoist.sort_by(|a, b| a.0.cmp(&b.0));
                to_hoist.dedup_by(|a, b| a.0 == b.0);

                if !to_hoist.is_empty() {
                    // Undeclare in all cases
                    for (name, _) in &to_hoist {
                        Self::undeclare(default, name);
                        for (_, case_stmts) in cases.iter_mut() {
                            Self::undeclare(case_stmts, name);
                        }
                    }
                    Some(to_hoist)
                } else {
                    None
                }
            }
            _ => None,
        };

        // If we have variables to hoist, create a sequence with declarations first
        if let Some(vars) = hoisted {
            let switch_stmt = std::mem::replace(stmt, Statement::Break); // placeholder
            let mut stmts = Vec::new();

            // Add declarations for hoisted variables (no initializer)
            for (name, _reg) in vars {
                stmts.push(Statement::VarDecl { name: name.into() });
            }

            stmts.push(switch_stmt);
            *stmt = Statement::Sequence { stmts };
        }
    }
}

/// Restore string concatenation. They are translated to calls to \_\_add__ at compilation.
/// ```haxe
/// __add__("hello ", "world")
/// ```
/// becomes :
/// ```haxe
/// "hello " + "world"
/// ```
pub(crate) struct StringConcat;

impl AstVisitor for StringConcat {
    fn visit_expr(&mut self, code: &Bytecode, expr: &mut Expr) {
        let args = match expr {
            Expr::Call(call) => match call.fun {
                Expr::FunRef(fun) => {
                    if fun.name(code) == "__add__" && call.args.len() == 2 {
                        Some((call.args[0].clone(), call.args[1].clone()))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        };

        if let Some((arg0, arg1)) = args {
            *expr = add(arg0, arg1);
        }
    }
}

/// Simplify `__alloc__(itos/ftos/dtos(x, ref), len)` to just the conversion call.
/// The fmt.rs CallHandling will then transform itos/ftos/dtos to Std.string(x).
pub(crate) struct Itos;

impl AstVisitor for Itos {
    fn visit_expr(&mut self, code: &Bytecode, expr: &mut Expr) {
        let replacement = match expr {
            Expr::Call(call) => match &call.fun {
                Expr::FunRef(fun) if fun.name(code) == "__alloc__" => match &call.args.get(0) {
                    Some(Expr::Call(inner_call)) => match &inner_call.fun {
                        Expr::FunRef(inner_fun) => {
                            let name = inner_fun.name(code);
                            if name == "itos" || name == "ftos" || name == "dtos" {
                                // Keep the conversion call, strip __alloc__ wrapper
                                Some(Expr::Call(inner_call.clone()))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };

        if let Some(new_expr) = replacement {
            *expr = new_expr;
        }
    }
}

/// Reconstruct array literals from alloc_bytes + SetMem + allocI32 patterns.
///
/// The pattern:
/// ```
/// var bytes = alloc_bytes(N);
/// bytes[offset] = value1;
/// ...
/// var arr = allocI32(bytes, count);
/// ```
/// becomes:
/// ```
/// var arr = [value1, value2, ...];
/// ```
pub(crate) fn reconstruct_array_literals(code: &Bytecode, stmts: &mut Vec<Statement>) {
    // Scan for the pattern across statements
    let mut i = 0;
    while i < stmts.len() {
        // Look for alloc_bytes call
        let bytes_reg = if let Statement::Assign { variable: Expr::Variable(reg, _), assign, .. } = &stmts[i] {
            if is_alloc_bytes_call(assign, code) {
                Some(*reg)
            } else {
                None
            }
        } else {
            None
        };

        let Some(bytes_reg) = bytes_reg else {
            // Recurse into nested statements
            recurse_array_literals(code, &mut stmts[i]);
            i += 1;
            continue;
        };

        // Found alloc_bytes, now collect values and find allocI32
        let mut values: Vec<Expr> = Vec::new();
        let mut j = i + 1;
        let mut stmts_to_remove: Vec<usize> = vec![i]; // Start with alloc_bytes stmt

        while j < stmts.len() {
            let stmt = &stmts[j];

            // Check for array assignment: bytes[offset] = value
            if let Statement::Assign { variable: Expr::Array(arr, _), assign, .. } = stmt {
                if is_var_reg(arr, bytes_reg) {
                    values.push(assign.clone());
                    stmts_to_remove.push(j);
                    j += 1;
                    continue;
                }
            }

            // Check for allocI32 call: var arr = allocI32(bytes, count)
            if let Statement::Assign { declaration, variable, assign, .. } = stmt {
                if is_alloc_i32_call(assign, bytes_reg, code).is_some() {
                    // Found the final allocI32 - reconstruct as array literal
                    let array_literal = Expr::ArrayLiteral(values);

                    // Replace the allocI32 statement with array literal
                    stmts[j] = Statement::Assign {
                        declaration: *declaration,
                        variable: variable.clone(),
                        assign: array_literal,
                    };

                    // Remove all the intermediate statements (alloc_bytes, SetMem, index manipulations)
                    // Sort in reverse so removal doesn't invalidate indices
                    stmts_to_remove.sort_by(|a, b| b.cmp(a));
                    for idx in stmts_to_remove {
                        stmts.remove(idx);
                    }

                    // Don't increment i since we removed statements before current position
                    break;
                }
            }

            // Check for index variable manipulations (var v4 = 0; v4++;) - these should be removed
            if let Statement::Assign { variable: Expr::Variable(_, _), assign, .. } = stmt {
                let is_zero = match assign {
                    Expr::Constant(Constant::InlineInt(0)) => true,
                    Expr::Constant(Constant::Int(ref_int)) => code[*ref_int] == 0,
                    _ => false,
                };
                if is_zero {
                    stmts_to_remove.push(j);
                    j += 1;
                    continue;
                }
            }
            if let Statement::ExprStatement(Expr::Op(Operation::Incr(_))) = stmt {
                stmts_to_remove.push(j);
                j += 1;
                continue;
            }

            // Not part of the pattern
            j += 1;
        }

        // If we didn't find allocI32, don't modify anything, just continue
        if j >= stmts.len() {
            i += 1;
        }
        // Otherwise, loop continues from adjusted position
    }
}

fn is_alloc_bytes_call(expr: &Expr, code: &Bytecode) -> bool {
    if let Expr::Call(call) = expr {
        if let Expr::FunRef(fun_ref) = &call.fun {
            return fun_ref.name(code).as_ref() == "alloc_bytes";
        }
    }
    false
}

fn is_alloc_i32_call(expr: &Expr, bytes_reg: hlbc::types::Reg, code: &Bytecode) -> Option<Expr> {
    if let Expr::Call(call) = expr {
        if let Expr::FunRef(fun_ref) = &call.fun {
            if fun_ref.name(code).as_ref() == "allocI32" {
                // Check if first arg is our bytes variable
                if let Some(first_arg) = call.args.first() {
                    if is_var_reg(first_arg, bytes_reg) {
                        return Some(first_arg.clone());
                    }
                }
            }
        }
    }
    None
}

fn is_var_reg(expr: &Expr, reg: hlbc::types::Reg) -> bool {
    matches!(expr, Expr::Variable(r, _) if *r == reg)
}

fn recurse_array_literals(code: &Bytecode, stmt: &mut Statement) {
    match stmt {
        Statement::IfElse { if_, else_, .. } => {
            reconstruct_array_literals(code, if_);
            reconstruct_array_literals(code, else_);
        }
        Statement::Switch { default, cases, .. } => {
            reconstruct_array_literals(code, default);
            for (_, case_stmts) in cases {
                reconstruct_array_literals(code, case_stmts);
            }
        }
        Statement::While { stmts, .. } => {
            reconstruct_array_literals(code, stmts);
        }
        Statement::TryCatch { try_stmts, catch_stmts, .. } => {
            reconstruct_array_literals(code, try_stmts);
            reconstruct_array_literals(code, catch_stmts);
        }
        Statement::Block { stmts } | Statement::Sequence { stmts } => {
            reconstruct_array_literals(code, stmts);
        }
        _ => {}
    }
}

/// Restore inlined `trace` calls.
pub(crate) struct Trace;

impl AstVisitor for Trace {
    fn visit_expr(&mut self, code: &Bytecode, expr: &mut Expr) {
        let call = match expr {
            Expr::Call(call) => match &call.fun {
                Expr::Field(obj, field) => match obj.as_ref() {
                    Expr::Variable(_, _) => {
                        if field == "trace" {
                            let trace = code.function_by_name(field).unwrap();
                            Some(call_fun(trace.findex, vec![call.args[0].clone()]))
                        } else {
                            None
                        }
                    }
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };
        if let Some(call) = call {
            *expr = call;
        }
    }
}
