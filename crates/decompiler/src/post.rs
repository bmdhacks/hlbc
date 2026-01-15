use hlbc::{Bytecode, Str};

use crate::ast::{add, not, Constant, ConstructorCall, Expr, Operation, Statement, Call};

pub(crate) trait AstVisitor {
    fn visit_stmt(&mut self, _code: &Bytecode, _stmt: &mut Statement) {}
    fn visit_expr(&mut self, _code: &Bytecode, _expr: &mut Expr) {}
}

/// Visit everything depth-first
#[allow(dead_code)]
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
            Statement::IfElseChain { branches, else_ } => {
                for (cond, body) in branches {
                    v!(cond);
                    rec!(body);
                }
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
#[allow(dead_code)]
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
        Expr::Cast(inner, _) | Expr::TypeAnnotated(inner, _) => {
            rec!(inner);
        }
    }
    for visitor in visitors.iter_mut() {
        visitor.visit_expr(code, expr);
    }
}

// NOTE: Unused visitor-based transforms (IfExpressions, BoundsCheckSimplify,
// SwitchExpressions, StringConcat visitor, Itos) were removed. The functionality
// is either handled in structurer.rs or fmt.rs, or was not needed.

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
        // Track the most recent constant assignment to each variable
        let mut last_constant: std::collections::HashMap<hlbc::types::Reg, Expr> = std::collections::HashMap::new();

        while j < stmts.len() {
            let stmt = &stmts[j];

            // Track constant assignments for value inlining
            if let Statement::Assign { variable: Expr::Variable(reg, _), assign, .. } = stmt {
                if is_constant_expr(assign) {
                    last_constant.insert(*reg, assign.clone());
                    stmts_to_remove.push(j); // Remove value setup statements
                    j += 1;
                    continue;
                }
            }

            // Check for array assignment: bytes[offset] = value
            if let Statement::Assign { variable: Expr::Array(arr, _), assign, .. } = stmt {
                if is_var_reg(arr, bytes_reg) {
                    // Try to inline constant if this is a variable reference
                    let value = inline_constant(assign, &last_constant);
                    values.push(value);
                    stmts_to_remove.push(j);
                    j += 1;
                    continue;
                }
            }

            // Check for method call: bytes.set(index, value)
            if let Statement::ExprStatement(Expr::Call(call)) = stmt {
                if let Expr::Field(receiver, method) = &call.fun {
                    if method.as_ref() == "set" && is_var_reg(receiver, bytes_reg) {
                        // The value is the second argument (index is first, value is second)
                        if call.args.len() >= 2 {
                            // Try to inline constant if this is a variable reference
                            let value = inline_constant(&call.args[1], &last_constant);
                            values.push(value);
                            stmts_to_remove.push(j);
                            j += 1;
                            continue;
                        }
                    }
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
            // Also detect increment-as-assignment: var r2_3 = r2_2 + 1
            // The structurer converts Incr opcodes to Assign(var, Add(var, 1))
            if let Statement::Assign { variable: Expr::Variable(dst_reg, _), assign: Expr::Op(Operation::Add(left, right)), .. } = stmt {
                // Check if it's adding 1 to a variable (increment pattern)
                let is_increment = match (left.as_ref(), right.as_ref()) {
                    (Expr::Variable(src_reg, _), Expr::Constant(Constant::InlineInt(1))) => {
                        dst_reg.0 == src_reg.0  // Same base register
                    }
                    (Expr::Constant(Constant::InlineInt(1)), Expr::Variable(src_reg, _)) => {
                        dst_reg.0 == src_reg.0  // Same base register
                    }
                    _ => false,
                };
                if is_increment {
                    stmts_to_remove.push(j);
                    j += 1;
                    continue;
                }
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

fn is_constant_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Constant(_))
}

fn inline_constant(expr: &Expr, constants: &std::collections::HashMap<hlbc::types::Reg, Expr>) -> Expr {
    if let Expr::Variable(reg, _) = expr {
        if let Some(constant) = constants.get(reg) {
            return constant.clone();
        }
    }
    expr.clone()
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

// NOTE: The old Trace visitor struct was removed. Trace collapsing is now done
// by collapse_trace_calls() at the end of the file.

// =============================================================================
// SingleUseInline: Inline single-use variables into their use sites
// =============================================================================

use std::collections::HashMap as StdHashMap;

/// Information about a variable definition
#[derive(Debug, Clone)]
struct VarDefInfo {
    /// Index of the defining statement
    def_idx: usize,
    /// The expression assigned to this variable
    expr: Expr,
    /// Is the expression pure (safe to inline)?
    is_pure: bool,
}

/// Check if an expression is pure (no side effects, safe to inline)
fn is_pure_expr(expr: &Expr) -> bool {
    match expr {
        // Constants are always pure
        Expr::Constant(_) => true,
        // Variable references are pure
        Expr::Variable(_, _) => true,
        Expr::Ident(_) => true,
        // Field access is NOT pure - can change type inference on Dynamic objects
        // e.g., `var r2 = p2.x; var dx = r2 - r3;` works differently than `var dx = p2.x - r3;`
        Expr::Field(_, _) => false,
        // Pure operations
        Expr::Op(op) => match op {
            Operation::Add(a, b) | Operation::Sub(a, b) | Operation::Mul(a, b) |
            Operation::Div(a, b) | Operation::Mod(a, b) | Operation::Shl(a, b) |
            Operation::Shr(a, b) | Operation::And(a, b) | Operation::Or(a, b) |
            Operation::Xor(a, b) | Operation::Eq(a, b) | Operation::NotEq(a, b) |
            Operation::Gt(a, b) | Operation::Gte(a, b) | Operation::Lt(a, b) |
            Operation::Lte(a, b) => is_pure_expr(a) && is_pure_expr(b),
            Operation::Neg(a) | Operation::Not(a) => is_pure_expr(a),
            // Incr/Decr have side effects
            Operation::Incr(_) | Operation::Decr(_) => false,
        },
        // Function calls are NOT pure (might have side effects)
        Expr::Call(_) => false,
        // Constructors are NOT pure
        Expr::Constructor(_) => false,
        // Array access might trigger bounds check
        Expr::Array(_, _) => false,
        // Other expressions - be conservative
        _ => false,
    }
}

/// Count uses of a variable name in an expression
fn count_uses_in_expr(expr: &Expr, var_name: &str) -> usize {
    match expr {
        Expr::Variable(_, Some(name)) if name.as_ref() == var_name => 1,
        Expr::Ident(name) if name.as_ref() == var_name => 1,
        Expr::Field(obj, _) => count_uses_in_expr(obj, var_name),
        Expr::Array(arr, idx) => count_uses_in_expr(arr, var_name) + count_uses_in_expr(idx, var_name),
        Expr::Call(call) => {
            let mut count = count_uses_in_expr(&call.fun, var_name);
            for arg in &call.args {
                count += count_uses_in_expr(arg, var_name);
            }
            count
        }
        Expr::Constructor(ctor) => {
            ctor.args.iter().map(|a| count_uses_in_expr(a, var_name)).sum()
        }
        Expr::Op(op) => match op {
            Operation::Add(a, b) | Operation::Sub(a, b) | Operation::Mul(a, b) |
            Operation::Div(a, b) | Operation::Mod(a, b) | Operation::Shl(a, b) |
            Operation::Shr(a, b) | Operation::And(a, b) | Operation::Or(a, b) |
            Operation::Xor(a, b) | Operation::Eq(a, b) | Operation::NotEq(a, b) |
            Operation::Gt(a, b) | Operation::Gte(a, b) | Operation::Lt(a, b) |
            Operation::Lte(a, b) => count_uses_in_expr(a, var_name) + count_uses_in_expr(b, var_name),
            Operation::Neg(a) | Operation::Not(a) | Operation::Incr(a) | Operation::Decr(a) => {
                count_uses_in_expr(a, var_name)
            }
        },
        Expr::Cast(inner, _) => count_uses_in_expr(inner, var_name),
        Expr::IfElse { cond, if_, else_ } => {
            count_uses_in_expr(cond, var_name) +
            count_uses_in_stmts(if_, var_name) +
            count_uses_in_stmts(else_, var_name)
        }
        Expr::EnumConstr(_, _, args) => {
            args.iter().map(|a| count_uses_in_expr(a, var_name)).sum()
        }
        Expr::ArrayLiteral(elems) => {
            elems.iter().map(|e| count_uses_in_expr(e, var_name)).sum()
        }
        Expr::Anonymous(_, fields) => {
            fields.values().map(|e| count_uses_in_expr(e, var_name)).sum()
        }
        _ => 0,
    }
}

/// Count uses of a variable name in statements
fn count_uses_in_stmts(stmts: &[Statement], var_name: &str) -> usize {
    let mut count = 0;
    for stmt in stmts {
        count += count_uses_in_stmt(stmt, var_name);
    }
    count
}

/// Count uses of a variable name in a statement
fn count_uses_in_stmt(stmt: &Statement, var_name: &str) -> usize {
    match stmt {
        Statement::Assign { variable, assign, .. } => {
            // Don't count the LHS variable as a use
            let lhs_uses = match variable {
                Expr::Field(obj, _) => count_uses_in_expr(obj, var_name),
                Expr::Array(arr, idx) => count_uses_in_expr(arr, var_name) + count_uses_in_expr(idx, var_name),
                _ => 0,
            };
            lhs_uses + count_uses_in_expr(assign, var_name)
        }
        Statement::ExprStatement(e) => count_uses_in_expr(e, var_name),
        Statement::Return(Some(e)) => count_uses_in_expr(e, var_name),
        Statement::Return(None) => 0,
        Statement::IfElse { cond, if_, else_ } => {
            count_uses_in_expr(cond, var_name) +
            count_uses_in_stmts(if_, var_name) +
            count_uses_in_stmts(else_, var_name)
        }
        Statement::While { cond, stmts } => {
            count_uses_in_expr(cond, var_name) + count_uses_in_stmts(stmts, var_name)
        }
        Statement::Switch { arg, default, cases, .. } => {
            let mut count = count_uses_in_expr(arg, var_name);
            count += count_uses_in_stmts(default, var_name);
            for (_, case_stmts) in cases {
                count += count_uses_in_stmts(case_stmts, var_name);
            }
            count
        }
        Statement::Throw(e) => count_uses_in_expr(e, var_name),
        Statement::TryCatch { try_stmts, catch_stmts, .. } => {
            count_uses_in_stmts(try_stmts, var_name) + count_uses_in_stmts(catch_stmts, var_name)
        }
        Statement::Block { stmts } | Statement::Sequence { stmts } => {
            count_uses_in_stmts(stmts, var_name)
        }
        _ => 0,
    }
}

/// Replace a variable with an expression in an expression (returns new expr)
fn replace_var_in_expr(expr: &Expr, var_name: &str, replacement: &Expr) -> Expr {
    match expr {
        Expr::Variable(_, Some(name)) if name.as_ref() == var_name => replacement.clone(),
        Expr::Ident(name) if name.as_ref() == var_name => replacement.clone(),
        Expr::Field(obj, field) => {
            Expr::Field(Box::new(replace_var_in_expr(obj, var_name, replacement)), field.clone())
        }
        Expr::Array(arr, idx) => {
            Expr::Array(
                Box::new(replace_var_in_expr(arr, var_name, replacement)),
                Box::new(replace_var_in_expr(idx, var_name, replacement)),
            )
        }
        Expr::Call(call) => {
            let new_fun = replace_var_in_expr(&call.fun, var_name, replacement);
            let new_args: Vec<_> = call.args.iter()
                .map(|a| replace_var_in_expr(a, var_name, replacement))
                .collect();
            Expr::Call(Box::new(Call { fun: new_fun, args: new_args }))
        }
        Expr::Constructor(ctor) => {
            let new_args: Vec<_> = ctor.args.iter()
                .map(|a| replace_var_in_expr(a, var_name, replacement))
                .collect();
            Expr::Constructor(ConstructorCall { ty: ctor.ty, args: new_args })
        }
        Expr::Op(op) => {
            let new_op = match op {
                Operation::Add(a, b) => Operation::Add(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Sub(a, b) => Operation::Sub(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Mul(a, b) => Operation::Mul(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Div(a, b) => Operation::Div(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Mod(a, b) => Operation::Mod(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Shl(a, b) => Operation::Shl(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Shr(a, b) => Operation::Shr(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::And(a, b) => Operation::And(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Or(a, b) => Operation::Or(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Xor(a, b) => Operation::Xor(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Eq(a, b) => Operation::Eq(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::NotEq(a, b) => Operation::NotEq(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Gt(a, b) => Operation::Gt(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Gte(a, b) => Operation::Gte(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Lt(a, b) => Operation::Lt(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Lte(a, b) => Operation::Lte(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::Neg(a) => Operation::Neg(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                ),
                Operation::Not(a) => Operation::Not(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                ),
                Operation::Incr(a) => Operation::Incr(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                ),
                Operation::Decr(a) => Operation::Decr(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                ),
            };
            Expr::Op(new_op)
        }
        Expr::Cast(inner, ty) => {
            Expr::Cast(Box::new(replace_var_in_expr(inner, var_name, replacement)), ty.clone())
        }
        Expr::EnumConstr(ty, idx, args) => {
            let new_args: Vec<_> = args.iter()
                .map(|a| replace_var_in_expr(a, var_name, replacement))
                .collect();
            Expr::EnumConstr(*ty, *idx, new_args)
        }
        Expr::ArrayLiteral(elems) => {
            let new_elems: Vec<_> = elems.iter()
                .map(|e| replace_var_in_expr(e, var_name, replacement))
                .collect();
            Expr::ArrayLiteral(new_elems)
        }
        Expr::Anonymous(ty, fields) => {
            let new_fields = fields.iter()
                .map(|(k, v)| (k.clone(), replace_var_in_expr(v, var_name, replacement)))
                .collect();
            Expr::Anonymous(*ty, new_fields)
        }
        Expr::IfElse { cond, if_, else_ } => {
            let new_if: Vec<_> = if_.iter()
                .map(|s| {
                    let mut s = s.clone();
                    replace_var_in_stmt(&mut s, var_name, replacement);
                    s
                })
                .collect();
            let new_else: Vec<_> = else_.iter()
                .map(|s| {
                    let mut s = s.clone();
                    replace_var_in_stmt(&mut s, var_name, replacement);
                    s
                })
                .collect();
            Expr::IfElse {
                cond: Box::new(replace_var_in_expr(cond, var_name, replacement)),
                if_: new_if,
                else_: new_else,
            }
        }
        // For other expressions, return as-is
        _ => expr.clone(),
    }
}

/// Replace a variable with an expression in a statement (modifies in place)
fn replace_var_in_stmt(stmt: &mut Statement, var_name: &str, replacement: &Expr) {
    match stmt {
        Statement::Assign { variable, assign, .. } => {
            // Replace in LHS if it's a field/array access
            match variable {
                Expr::Field(obj, field) => {
                    *variable = Expr::Field(
                        Box::new(replace_var_in_expr(obj, var_name, replacement)),
                        field.clone(),
                    );
                }
                Expr::Array(arr, idx) => {
                    *variable = Expr::Array(
                        Box::new(replace_var_in_expr(arr, var_name, replacement)),
                        Box::new(replace_var_in_expr(idx, var_name, replacement)),
                    );
                }
                _ => {}
            }
            *assign = replace_var_in_expr(assign, var_name, replacement);
        }
        Statement::ExprStatement(e) => {
            *e = replace_var_in_expr(e, var_name, replacement);
        }
        Statement::Return(Some(e)) => {
            *e = replace_var_in_expr(e, var_name, replacement);
        }
        Statement::IfElse { cond, if_, else_ } => {
            *cond = replace_var_in_expr(cond, var_name, replacement);
            replace_var_in_stmts(if_, var_name, replacement);
            replace_var_in_stmts(else_, var_name, replacement);
        }
        Statement::While { cond, stmts } => {
            *cond = replace_var_in_expr(cond, var_name, replacement);
            replace_var_in_stmts(stmts, var_name, replacement);
        }
        Statement::Switch { arg, default, cases, .. } => {
            *arg = replace_var_in_expr(arg, var_name, replacement);
            replace_var_in_stmts(default, var_name, replacement);
            for (_, case_stmts) in cases {
                replace_var_in_stmts(case_stmts, var_name, replacement);
            }
        }
        Statement::Throw(e) => {
            *e = replace_var_in_expr(e, var_name, replacement);
        }
        Statement::TryCatch { try_stmts, catch_stmts, .. } => {
            replace_var_in_stmts(try_stmts, var_name, replacement);
            replace_var_in_stmts(catch_stmts, var_name, replacement);
        }
        Statement::Block { stmts } | Statement::Sequence { stmts } => {
            replace_var_in_stmts(stmts, var_name, replacement);
        }
        _ => {}
    }
}

/// Replace a variable in a slice of statements
fn replace_var_in_stmts(stmts: &mut [Statement], var_name: &str, replacement: &Expr) {
    for stmt in stmts {
        replace_var_in_stmt(stmt, var_name, replacement);
    }
}

/// Get the variable name from a variable expression
fn get_var_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Variable(_, Some(name)) => Some(name.to_string()),
        Expr::Ident(name) => Some(name.to_string()),
        _ => None,
    }
}

/// Check if a variable is assigned to (excluding the initial declaration) anywhere in statements
fn is_reassigned_in_stmts(stmts: &[Statement], var_name: &str, skip_idx: usize) -> bool {
    for (idx, stmt) in stmts.iter().enumerate() {
        if idx == skip_idx {
            continue;
        }
        if is_reassigned_in_stmt(stmt, var_name) {
            return true;
        }
    }
    false
}

/// Check if a variable is assigned to in a range of statements (from start_idx+1 to end)
#[allow(dead_code)]
fn is_reassigned_in_range(stmts: &[Statement], var_name: &str, start_idx: usize) -> bool {
    for stmt in stmts.iter().skip(start_idx + 1) {
        if is_reassigned_in_stmt(stmt, var_name) {
            return true;
        }
    }
    false
}

/// Get all variable names referenced in an expression
fn get_var_refs_in_expr(expr: &Expr, vars: &mut Vec<String>) {
    match expr {
        Expr::Variable(_, Some(name)) => vars.push(name.to_string()),
        Expr::Ident(name) => vars.push(name.to_string()),
        Expr::Field(obj, _) => get_var_refs_in_expr(obj, vars),
        Expr::Array(arr, idx) => {
            get_var_refs_in_expr(arr, vars);
            get_var_refs_in_expr(idx, vars);
        }
        Expr::Call(call) => {
            get_var_refs_in_expr(&call.fun, vars);
            for arg in &call.args {
                get_var_refs_in_expr(arg, vars);
            }
        }
        Expr::Constructor(ctor) => {
            for arg in &ctor.args {
                get_var_refs_in_expr(arg, vars);
            }
        }
        Expr::Op(op) => match op {
            Operation::Add(a, b) | Operation::Sub(a, b) | Operation::Mul(a, b) |
            Operation::Div(a, b) | Operation::Mod(a, b) | Operation::Shl(a, b) |
            Operation::Shr(a, b) | Operation::And(a, b) | Operation::Or(a, b) |
            Operation::Xor(a, b) | Operation::Eq(a, b) | Operation::NotEq(a, b) |
            Operation::Gt(a, b) | Operation::Gte(a, b) | Operation::Lt(a, b) |
            Operation::Lte(a, b) => {
                get_var_refs_in_expr(a, vars);
                get_var_refs_in_expr(b, vars);
            }
            Operation::Neg(a) | Operation::Not(a) | Operation::Incr(a) | Operation::Decr(a) => {
                get_var_refs_in_expr(a, vars);
            }
        },
        Expr::Cast(inner, _) => get_var_refs_in_expr(inner, vars),
        Expr::EnumConstr(_, _, args) => {
            for arg in args {
                get_var_refs_in_expr(arg, vars);
            }
        }
        Expr::ArrayLiteral(elems) => {
            for elem in elems {
                get_var_refs_in_expr(elem, vars);
            }
        }
        Expr::Anonymous(_, fields) => {
            for expr in fields.values() {
                get_var_refs_in_expr(expr, vars);
            }
        }
        _ => {}
    }
}

/// Check if any variable in an expression is reassigned in the range of statements after def_idx
#[allow(dead_code)]
fn expr_vars_reassigned_after(stmts: &[Statement], expr: &Expr, def_idx: usize) -> bool {
    let mut vars = Vec::new();
    get_var_refs_in_expr(expr, &mut vars);

    for var_name in vars {
        if is_reassigned_in_range(stmts, &var_name, def_idx) {
            return true;
        }
    }
    false
}

/// Check if a variable is assigned to in a statement
fn is_reassigned_in_stmt(stmt: &Statement, var_name: &str) -> bool {
    match stmt {
        // Check for assignment to this variable (any assignment, including declarations)
        Statement::Assign { variable, .. } => {
            if let Some(name) = get_var_name(variable) {
                if name == var_name {
                    return true;
                }
            }
            // Also check nested structures in the assign expression
            false
        }
        Statement::IfElse { if_, else_, .. } => {
            is_reassigned_in_stmts(if_, var_name, usize::MAX) ||
            is_reassigned_in_stmts(else_, var_name, usize::MAX)
        }
        Statement::While { stmts, .. } => {
            is_reassigned_in_stmts(stmts, var_name, usize::MAX)
        }
        Statement::Switch { default, cases, .. } => {
            if is_reassigned_in_stmts(default, var_name, usize::MAX) {
                return true;
            }
            for (_, case_stmts) in cases {
                if is_reassigned_in_stmts(case_stmts, var_name, usize::MAX) {
                    return true;
                }
            }
            false
        }
        Statement::TryCatch { try_stmts, catch_stmts, .. } => {
            is_reassigned_in_stmts(try_stmts, var_name, usize::MAX) ||
            is_reassigned_in_stmts(catch_stmts, var_name, usize::MAX)
        }
        Statement::Block { stmts } | Statement::Sequence { stmts } => {
            is_reassigned_in_stmts(stmts, var_name, usize::MAX)
        }
        _ => false,
    }
}

/// Inline single-use variables into their use sites.
///
/// This pass finds patterns like:
/// ```haxe
/// var r4 = someExpr;
/// var r3 = r4.field;
/// ```
/// and transforms them to:
/// ```haxe
/// var r3 = someExpr.field;
/// ```
pub fn inline_single_use_vars(stmts: &mut Vec<Statement>) {
    // We need to iterate until no more changes, since inlining can enable more inlining
    let mut changed = true;
    while changed {
        changed = inline_single_use_vars_pass(stmts);
    }
}

/// Single pass of variable inlining. Returns true if any changes were made.
fn inline_single_use_vars_pass(stmts: &mut Vec<Statement>) -> bool {
    // Build map of variable definitions at the top level
    let mut var_defs: StdHashMap<String, VarDefInfo> = StdHashMap::new();

    // First pass: collect definitions
    for (idx, stmt) in stmts.iter().enumerate() {
        if let Statement::Assign { declaration: true, variable, assign, .. } = stmt {
            if let Some(name) = get_var_name(variable) {
                let is_pure = is_pure_expr(assign);
                var_defs.insert(name, VarDefInfo {
                    def_idx: idx,
                    expr: assign.clone(),
                    is_pure,
                });
            }
        }
    }

    // Second pass: find single uses and check if safe to inline
    let mut to_inline: Vec<(String, usize, Expr)> = Vec::new(); // (var_name, def_idx, expr)

    for (var_name, def_info) in &var_defs {
        if !def_info.is_pure {
            continue;
        }

        // Count total uses of this variable in the rest of the function
        let mut use_count = 0;
        let mut use_idx = None;
        for (idx, stmt) in stmts.iter().enumerate().skip(def_info.def_idx + 1) {
            let uses_in_stmt = count_uses_in_stmt(stmt, var_name);
            if uses_in_stmt > 0 {
                use_count += uses_in_stmt;
                if use_idx.is_none() {
                    use_idx = Some(idx);
                }
            }
        }

        // Only inline if exactly one use
        if use_count != 1 {
            continue;
        }

        // Don't inline if the variable is reassigned anywhere
        // (removing the declaration would leave later reassignments without a var declaration)
        if is_reassigned_in_stmts(stmts, var_name, def_info.def_idx) {
            continue;
        }

        let use_idx = match use_idx {
            Some(idx) => idx,
            None => continue,
        };

        // Check if any variable in the expression is reassigned between def and use
        // (if so, inlining would change semantics)
        let mut expr_vars_modified = false;
        let mut expr_vars = Vec::new();
        get_var_refs_in_expr(&def_info.expr, &mut expr_vars);
        for expr_var in &expr_vars {
            for (idx, stmt) in stmts.iter().enumerate().skip(def_info.def_idx + 1) {
                if idx >= use_idx {
                    break;
                }
                if is_reassigned_in_stmt(stmt, expr_var) {
                    expr_vars_modified = true;
                    break;
                }
            }
            if expr_vars_modified {
                break;
            }
        }
        if expr_vars_modified {
            continue;
        }

        to_inline.push((var_name.clone(), def_info.def_idx, def_info.expr.clone()));
    }

    if to_inline.is_empty() {
        return false;
    }

    // Sort by def_idx descending so we can remove from back to front
    to_inline.sort_by(|a, b| b.1.cmp(&a.1));

    // Third pass: perform inlining and remove definitions
    for (var_name, def_idx, replacement) in to_inline {
        // Replace uses in subsequent statements
        for stmt in stmts.iter_mut().skip(def_idx + 1) {
            replace_var_in_stmt(stmt, &var_name, &replacement);
        }

        // Remove the definition statement
        // (we already checked the variable isn't reassigned, so safe to remove)
        stmts.remove(def_idx);
    }

    // Recurse into nested structures
    for stmt in stmts.iter_mut() {
        match stmt {
            Statement::IfElse { if_, else_, .. } => {
                inline_single_use_vars(if_);
                inline_single_use_vars(else_);
            }
            Statement::While { stmts, .. } => {
                inline_single_use_vars(stmts);
            }
            Statement::Switch { default, cases, .. } => {
                inline_single_use_vars(default);
                for (_, case_stmts) in cases {
                    inline_single_use_vars(case_stmts);
                }
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                inline_single_use_vars(try_stmts);
                inline_single_use_vars(catch_stmts);
            }
            Statement::Block { stmts } | Statement::Sequence { stmts } => {
                inline_single_use_vars(stmts);
            }
            _ => {}
        }
    }

    true
}

/// Merge forward declarations with their first assignment.
///
/// This pass finds patterns like:
/// ```haxe
/// var r5;
/// var r6;
/// // ... code that doesn't use r5 or r6 ...
/// r5 = someValue;
/// ```
/// and transforms them to:
/// ```haxe
/// var r6;
/// // ... code ...
/// var r5 = someValue;
/// ```
///
/// IMPORTANT: Only merges if the first assignment is at the TOP LEVEL (same scope as the VarDecl).
/// Variables declared at function level but first assigned inside a nested scope (if/while/switch)
/// must keep their forward declaration - that's exactly why they were hoisted.
pub fn merge_declarations(stmts: &mut Vec<Statement>) {
    // Collect all VarDecl names and their indices
    let mut var_decls: Vec<(usize, String, Option<Str>)> = Vec::new(); // (idx, name, type_hint)

    for (idx, stmt) in stmts.iter().enumerate() {
        if let Statement::VarDecl { name, type_hint } = stmt {
            var_decls.push((idx, name.to_string(), type_hint.clone()));
        }
    }

    // For each VarDecl, try to find first assignment at top level
    let mut to_merge: Vec<(usize, usize)> = Vec::new(); // (var_decl_idx, assign_idx)

    for (decl_idx, var_name, _type_hint) in &var_decls {
        // Look for first assignment to this variable AFTER the declaration
        // Only consider top-level statements (not inside nested scopes)
        let mut found_use_before_assign = false;

        for (stmt_idx, stmt) in stmts.iter().enumerate().skip(*decl_idx + 1) {
            match stmt {
                // Found an assignment to this variable at top level
                Statement::Assign { declaration: false, variable, .. } => {
                    if let Some(name) = get_var_name(variable) {
                        if &name == var_name {
                            // Check if there were any uses before this assignment
                            if !found_use_before_assign {
                                to_merge.push((*decl_idx, stmt_idx));
                            }
                            break;
                        }
                    }
                    // Check if this statement uses the variable
                    if count_uses_in_stmt(stmt, var_name) > 0 {
                        found_use_before_assign = true;
                    }
                }
                // Any other statement - check for uses
                _ => {
                    if count_uses_in_stmt(stmt, var_name) > 0 {
                        found_use_before_assign = true;
                    }
                }
            }
        }
    }

    // First pass: convert all assignments to declarations (doesn't change indices)
    for (_decl_idx, assign_idx) in &to_merge {
        if let Statement::Assign { variable, assign, .. } = &stmts[*assign_idx] {
            // Create new statement with declaration: true
            let new_stmt = Statement::Assign {
                declaration: true,
                variable: variable.clone(),
                assign: assign.clone(),
            };
            stmts[*assign_idx] = new_stmt;
        }
    }

    // Second pass: collect VarDecl indices to remove, sort descending, remove from back to front
    let mut decl_indices: Vec<usize> = to_merge.iter().map(|(decl_idx, _)| *decl_idx).collect();
    decl_indices.sort_by(|a, b| b.cmp(a));
    decl_indices.dedup(); // In case same decl appears multiple times (shouldn't happen, but safe)

    for decl_idx in decl_indices {
        stmts.remove(decl_idx);
    }

    // Recurse into nested structures
    for stmt in stmts.iter_mut() {
        match stmt {
            Statement::IfElse { if_, else_, .. } => {
                merge_declarations(if_);
                merge_declarations(else_);
            }
            Statement::While { stmts, .. } => {
                merge_declarations(stmts);
            }
            Statement::Switch { default, cases, .. } => {
                merge_declarations(default);
                for (_, case_stmts) in cases {
                    merge_declarations(case_stmts);
                }
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                merge_declarations(try_stmts);
                merge_declarations(catch_stmts);
            }
            Statement::Block { stmts } | Statement::Sequence { stmts } => {
                merge_declarations(stmts);
            }
            _ => {}
        }
    }
}

/// Remove unused forward declarations (VarDecl statements for variables never used).
///
/// After inlining passes, some VarDecl statements may become orphaned if
/// their variable is never actually used in the function.
pub fn remove_unused_var_decls(stmts: &mut Vec<Statement>) {
    // Collect all variable names that are actually used
    let mut used_vars: std::collections::HashSet<String> = std::collections::HashSet::new();
    collect_used_vars(stmts, &mut used_vars);

    // Remove VarDecl statements for variables that aren't used
    stmts.retain(|stmt| {
        if let Statement::VarDecl { name, .. } = stmt {
            used_vars.contains(name.as_ref())
        } else {
            true
        }
    });

    // Recurse into nested structures
    for stmt in stmts.iter_mut() {
        match stmt {
            Statement::IfElse { if_, else_, .. } => {
                remove_unused_var_decls(if_);
                remove_unused_var_decls(else_);
            }
            Statement::While { stmts, .. } => {
                remove_unused_var_decls(stmts);
            }
            Statement::Switch { default, cases, .. } => {
                remove_unused_var_decls(default);
                for (_, case_stmts) in cases {
                    remove_unused_var_decls(case_stmts);
                }
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                remove_unused_var_decls(try_stmts);
                remove_unused_var_decls(catch_stmts);
            }
            Statement::Block { stmts } | Statement::Sequence { stmts } => {
                remove_unused_var_decls(stmts);
            }
            _ => {}
        }
    }
}

/// Collect all variable names that are used in expressions
fn collect_used_vars(stmts: &[Statement], used: &mut std::collections::HashSet<String>) {
    for stmt in stmts {
        match stmt {
            Statement::Assign { variable, assign, .. } => {
                collect_used_vars_in_expr(variable, used);
                collect_used_vars_in_expr(assign, used);
            }
            Statement::ExprStatement(e) => {
                collect_used_vars_in_expr(e, used);
            }
            Statement::Return(Some(e)) => {
                collect_used_vars_in_expr(e, used);
            }
            Statement::IfElse { cond, if_, else_ } => {
                collect_used_vars_in_expr(cond, used);
                collect_used_vars(if_, used);
                collect_used_vars(else_, used);
            }
            Statement::While { cond, stmts } => {
                collect_used_vars_in_expr(cond, used);
                collect_used_vars(stmts, used);
            }
            Statement::Switch { arg, default, cases, .. } => {
                collect_used_vars_in_expr(arg, used);
                collect_used_vars(default, used);
                for (_, case_stmts) in cases {
                    collect_used_vars(case_stmts, used);
                }
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                collect_used_vars(try_stmts, used);
                collect_used_vars(catch_stmts, used);
            }
            Statement::Block { stmts } | Statement::Sequence { stmts } => {
                collect_used_vars(stmts, used);
            }
            _ => {}
        }
    }
}

fn collect_used_vars_in_expr(expr: &Expr, used: &mut std::collections::HashSet<String>) {
    match expr {
        Expr::Variable(_, Some(name)) => {
            used.insert(name.to_string());
        }
        Expr::Field(inner, _) => collect_used_vars_in_expr(inner, used),
        Expr::Array(arr, idx) => {
            collect_used_vars_in_expr(arr, used);
            collect_used_vars_in_expr(idx, used);
        }
        Expr::Call(call) => {
            collect_used_vars_in_expr(&call.fun, used);
            for arg in &call.args {
                collect_used_vars_in_expr(arg, used);
            }
        }
        Expr::Op(op) => {
            match op {
                Operation::Add(l, r) | Operation::Sub(l, r) | Operation::Mul(l, r) |
                Operation::Div(l, r) | Operation::Mod(l, r) | Operation::Shl(l, r) |
                Operation::Shr(l, r) | Operation::And(l, r) | Operation::Or(l, r) |
                Operation::Xor(l, r) | Operation::Eq(l, r) | Operation::NotEq(l, r) |
                Operation::Gt(l, r) | Operation::Gte(l, r) | Operation::Lt(l, r) |
                Operation::Lte(l, r) => {
                    collect_used_vars_in_expr(l, used);
                    collect_used_vars_in_expr(r, used);
                }
                Operation::Neg(e) | Operation::Not(e) | Operation::Incr(e) | Operation::Decr(e) => {
                    collect_used_vars_in_expr(e, used);
                }
            }
        }
        Expr::IfElse { cond, if_, else_ } => {
            collect_used_vars_in_expr(cond, used);
            collect_used_vars(if_, used);
            collect_used_vars(else_, used);
        }
        Expr::Constructor(ctor) => {
            for arg in &ctor.args {
                collect_used_vars_in_expr(arg, used);
            }
        }
        Expr::ArrayLiteral(elems) => {
            for e in elems {
                collect_used_vars_in_expr(e, used);
            }
        }
        Expr::Anonymous(_, fields) => {
            for e in fields.values() {
                collect_used_vars_in_expr(e, used);
            }
        }
        Expr::EnumConstr(_, _, args) => {
            for arg in args {
                collect_used_vars_in_expr(arg, used);
            }
        }
        Expr::Closure(_, stmts) => {
            collect_used_vars(stmts, used);
        }
        Expr::Cast(inner, _) => {
            collect_used_vars_in_expr(inner, used);
        }
        _ => {}
    }
}

/// Inline constants that are immediately used in the next statement.
///
/// Transforms:
/// ```haxe
/// r6_4 = true;
/// return r6_4;
/// ```
/// into:
/// ```haxe
/// return true;
/// ```
///
/// Also handles:
/// ```haxe
/// r2_2 = 0;
/// return ((n >= r2_2) ? 0 : -1);
/// ```
/// into:
/// ```haxe
/// return ((n >= 0) ? 0 : -1);
/// ```
///
/// This handles forward-declared variables that are assigned a constant
/// and then immediately used, which the main inlining pass misses
/// because it only handles declaration assignments.
pub fn inline_constant_returns(stmts: &mut Vec<Statement>) {
    // Run until no more changes (inlining can enable more inlining)
    let mut changed = true;
    while changed {
        // Pass an empty set - at top level, no outer variables to protect
        changed = inline_constant_returns_pass(stmts, &std::collections::HashSet::new());
    }
}

fn inline_constant_returns_pass(
    stmts: &mut Vec<Statement>,
    outer_used_vars: &std::collections::HashSet<String>,
) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < stmts.len() {
        // First, recurse into nested structures, passing along info about
        // variables used later in this scope (so nested scopes don't inline them)
        let vars_used_after = collect_vars_used_in_remaining(stmts, i + 1);
        let mut combined_outer: std::collections::HashSet<String> = outer_used_vars.clone();
        combined_outer.extend(vars_used_after);

        match &mut stmts[i] {
            Statement::IfElse { if_, else_, .. } => {
                if inline_constant_returns_pass(if_, &combined_outer) { changed = true; }
                if inline_constant_returns_pass(else_, &combined_outer) { changed = true; }
            }
            Statement::IfElseChain { branches, else_ } => {
                for (_, body) in branches {
                    if inline_constant_returns_pass(body, &combined_outer) { changed = true; }
                }
                if inline_constant_returns_pass(else_, &combined_outer) { changed = true; }
            }
            Statement::While { stmts: inner, .. } => {
                if inline_constant_returns_pass(inner, &combined_outer) { changed = true; }
            }
            Statement::Switch { default, cases, .. } => {
                if inline_constant_returns_pass(default, &combined_outer) { changed = true; }
                for (_, case_stmts) in cases {
                    if inline_constant_returns_pass(case_stmts, &combined_outer) { changed = true; }
                }
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                if inline_constant_returns_pass(try_stmts, &combined_outer) { changed = true; }
                if inline_constant_returns_pass(catch_stmts, &combined_outer) { changed = true; }
            }
            Statement::Block { stmts: inner } | Statement::Sequence { stmts: inner } => {
                if inline_constant_returns_pass(inner, &combined_outer) { changed = true; }
            }
            _ => {}
        }

        // Look for pattern: assign pure expr to var, then use that var in next stmt
        if i + 1 < stmts.len() {
            if let Statement::Assign { variable, assign, declaration, .. } = &stmts[i] {
                // Only inline non-declaration assignments (forward-declared vars)
                // Declaration assignments are handled by inline_single_use_vars
                if !declaration && is_pure_expr(assign) {
                    if let Some(var_name) = get_var_name(variable) {
                        // Don't inline if variable is used in outer scope
                        if outer_used_vars.contains(&var_name) {
                            i += 1;
                            continue;
                        }

                        // CRITICAL: Don't inline self-modifying assignments like `i = i + 1`
                        // Removing such statements would lose the side effect of updating the variable.
                        // Check if the expression references the same variable being assigned.
                        if count_uses_in_expr(assign, &var_name) > 0 {
                            i += 1;
                            continue;
                        }

                        // Count uses in the next statement (including nested structures)
                        let uses_in_next = count_uses_in_stmt(&stmts[i + 1], &var_name);

                        // Also count uses in ALL remaining statements (i+2 onwards)
                        let mut uses_later = 0;
                        for stmt in stmts.iter().skip(i + 2) {
                            uses_later += count_uses_in_stmt(stmt, &var_name);
                        }

                        // Only inline if used exactly once in next stmt and never later
                        if uses_in_next == 1 && uses_later == 0 {
                            // Inline the constant into the next statement
                            let replacement = assign.clone();
                            replace_var_in_stmt(&mut stmts[i + 1], &var_name, &replacement);
                            stmts.remove(i);
                            changed = true;
                            // Don't increment i - we removed current statement
                            continue;
                        }
                    }
                }
            }
        }

        i += 1;
    }
    changed
}

/// Collect all variable names used in statements from index `start` onwards
fn collect_vars_used_in_remaining(stmts: &[Statement], start: usize) -> std::collections::HashSet<String> {
    let mut used = std::collections::HashSet::new();
    for stmt in stmts.iter().skip(start) {
        collect_var_names_in_stmt(stmt, &mut used);
    }
    used
}

fn collect_var_names_in_stmt(stmt: &Statement, used: &mut std::collections::HashSet<String>) {
    match stmt {
        Statement::Assign { variable, assign, .. } => {
            collect_var_names_in_expr(variable, used);
            collect_var_names_in_expr(assign, used);
        }
        Statement::ExprStatement(e) => collect_var_names_in_expr(e, used),
        Statement::Return(Some(e)) => collect_var_names_in_expr(e, used),
        Statement::IfElse { cond, if_, else_ } => {
            collect_var_names_in_expr(cond, used);
            for s in if_ { collect_var_names_in_stmt(s, used); }
            for s in else_ { collect_var_names_in_stmt(s, used); }
        }
        Statement::IfElseChain { branches, else_ } => {
            for (cond, body) in branches {
                collect_var_names_in_expr(cond, used);
                for s in body { collect_var_names_in_stmt(s, used); }
            }
            for s in else_ { collect_var_names_in_stmt(s, used); }
        }
        Statement::While { cond, stmts } => {
            collect_var_names_in_expr(cond, used);
            for s in stmts { collect_var_names_in_stmt(s, used); }
        }
        Statement::Switch { arg, default, cases, .. } => {
            collect_var_names_in_expr(arg, used);
            for s in default { collect_var_names_in_stmt(s, used); }
            for (_, case_stmts) in cases {
                for s in case_stmts { collect_var_names_in_stmt(s, used); }
            }
        }
        Statement::TryCatch { try_stmts, catch_stmts, .. } => {
            for s in try_stmts { collect_var_names_in_stmt(s, used); }
            for s in catch_stmts { collect_var_names_in_stmt(s, used); }
        }
        Statement::Block { stmts } | Statement::Sequence { stmts } => {
            for s in stmts { collect_var_names_in_stmt(s, used); }
        }
        _ => {}
    }
}

fn collect_var_names_in_expr(expr: &Expr, used: &mut std::collections::HashSet<String>) {
    match expr {
        Expr::Variable(_, Some(name)) => { used.insert(name.to_string()); }
        Expr::Field(inner, _) => collect_var_names_in_expr(inner, used),
        Expr::Array(arr, idx) => {
            collect_var_names_in_expr(arr, used);
            collect_var_names_in_expr(idx, used);
        }
        Expr::Call(call) => {
            collect_var_names_in_expr(&call.fun, used);
            for arg in &call.args { collect_var_names_in_expr(arg, used); }
        }
        Expr::Op(op) => {
            match op {
                Operation::Add(l, r) | Operation::Sub(l, r) | Operation::Mul(l, r) |
                Operation::Div(l, r) | Operation::Mod(l, r) | Operation::Shl(l, r) |
                Operation::Shr(l, r) | Operation::And(l, r) | Operation::Or(l, r) |
                Operation::Xor(l, r) | Operation::Eq(l, r) | Operation::NotEq(l, r) |
                Operation::Gt(l, r) | Operation::Gte(l, r) | Operation::Lt(l, r) |
                Operation::Lte(l, r) => {
                    collect_var_names_in_expr(l, used);
                    collect_var_names_in_expr(r, used);
                }
                Operation::Neg(e) | Operation::Not(e) | Operation::Incr(e) | Operation::Decr(e) => {
                    collect_var_names_in_expr(e, used);
                }
            }
        }
        Expr::IfElse { cond, if_, else_ } => {
            collect_var_names_in_expr(cond, used);
            for s in if_ { collect_var_names_in_stmt(s, used); }
            for s in else_ { collect_var_names_in_stmt(s, used); }
        }
        Expr::Constructor(ctor) => {
            for arg in &ctor.args { collect_var_names_in_expr(arg, used); }
        }
        Expr::ArrayLiteral(elems) => {
            for e in elems { collect_var_names_in_expr(e, used); }
        }
        Expr::Anonymous(_, fields) => {
            for e in fields.values() { collect_var_names_in_expr(e, used); }
        }
        Expr::EnumConstr(_, _, args) => {
            for arg in args { collect_var_names_in_expr(arg, used); }
        }
        Expr::Closure(_, stmts) => {
            for s in stmts { collect_var_names_in_stmt(s, used); }
        }
        Expr::Cast(inner, _) => collect_var_names_in_expr(inner, used),
        _ => {}
    }
}

/// Check if a statement is a terminating statement (return, throw, break, continue)
fn is_terminating(stmt: &Statement) -> bool {
    matches!(stmt, Statement::Return(_) | Statement::Throw(_) | Statement::Break | Statement::Continue)
}

/// Check if a block of statements ends with a terminating statement
fn ends_with_terminator(stmts: &[Statement]) -> bool {
    stmts.last().map(is_terminating).unwrap_or(false)
}

/// Flatten early returns: `if (x) { return; } else { body }` → `if (x) { return; } body`
/// This removes unnecessary else blocks after terminating statements.
/// Returns true if any changes were made.
pub fn flatten_early_returns(stmts: &mut Vec<Statement>) -> bool {
    let mut changed = false;
    let mut i = 0;

    while i < stmts.len() {
        // First, recursively process nested statements
        match &mut stmts[i] {
            Statement::IfElse { if_, else_, .. } => {
                if flatten_early_returns(if_) {
                    changed = true;
                }
                if flatten_early_returns(else_) {
                    changed = true;
                }
            }
            Statement::While { stmts: inner, .. } => {
                if flatten_early_returns(inner) {
                    changed = true;
                }
            }
            Statement::Switch { default, cases, .. } => {
                if flatten_early_returns(default) {
                    changed = true;
                }
                for (_, case_stmts) in cases.iter_mut() {
                    if flatten_early_returns(case_stmts) {
                        changed = true;
                    }
                }
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                if flatten_early_returns(try_stmts) {
                    changed = true;
                }
                if flatten_early_returns(catch_stmts) {
                    changed = true;
                }
            }
            Statement::Block { stmts: inner } | Statement::Sequence { stmts: inner } => {
                if flatten_early_returns(inner) {
                    changed = true;
                }
            }
            _ => {}
        }

        // Now check if this is an if-else where if ends with terminator
        if let Statement::IfElse { if_, else_, .. } = &mut stmts[i] {
            if ends_with_terminator(if_) && !else_.is_empty() {
                // Extract else statements and insert them after the if
                let else_stmts = std::mem::take(else_);
                let insert_pos = i + 1;
                for (j, stmt) in else_stmts.into_iter().enumerate() {
                    stmts.insert(insert_pos + j, stmt);
                }
                changed = true;
                // Don't increment i, re-process the newly inserted statements
                continue;
            }
        }

        i += 1;
    }

    changed
}

/// Invert empty if bodies: `if (c) {} else { body }` → `if (!c) { body }`
/// This produces cleaner output for guard-style conditionals.
/// Returns true if any changes were made.
pub fn invert_empty_ifs(stmts: &mut Vec<Statement>) -> bool {
    let mut changed = false;

    for stmt in stmts.iter_mut() {
        match stmt {
            Statement::IfElse { cond, if_, else_ } => {
                // Recursively process nested statements first
                if invert_empty_ifs(if_) {
                    changed = true;
                }
                if invert_empty_ifs(else_) {
                    changed = true;
                }

                // If the if-body is empty and else-body is not, invert
                if if_.is_empty() && !else_.is_empty() {
                    // Replace cond with its negation
                    let old_cond = std::mem::replace(cond, Expr::Constant(Constant::Null));
                    *cond = not(old_cond);
                    std::mem::swap(if_, else_);
                    changed = true;
                }
            }
            Statement::While { stmts, .. } => {
                if invert_empty_ifs(stmts) {
                    changed = true;
                }
            }
            Statement::Switch { default, cases, .. } => {
                if invert_empty_ifs(default) {
                    changed = true;
                }
                for (_, case_stmts) in cases.iter_mut() {
                    if invert_empty_ifs(case_stmts) {
                        changed = true;
                    }
                }
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                if invert_empty_ifs(try_stmts) {
                    changed = true;
                }
                if invert_empty_ifs(catch_stmts) {
                    changed = true;
                }
            }
            Statement::Block { stmts } | Statement::Sequence { stmts } => {
                if invert_empty_ifs(stmts) {
                    changed = true;
                }
            }
            _ => {}
        }
    }

    changed
}

// =============================================================================
// StringConcat: Restore `+` operator from `__add__` calls
// =============================================================================

/// Transform `__add__(a, b)` calls into `a + b` expressions.
/// Haxe compiles string concatenation to `__add__` at bytecode level.
pub fn apply_string_concat(code: &Bytecode, stmts: &mut Vec<Statement>) {
    for stmt in stmts.iter_mut() {
        apply_string_concat_stmt(code, stmt);
    }
}

fn apply_string_concat_stmt(code: &Bytecode, stmt: &mut Statement) {
    match stmt {
        Statement::Assign { assign, variable, .. } => {
            apply_string_concat_expr(code, assign);
            apply_string_concat_expr(code, variable);
        }
        Statement::ExprStatement(e) => {
            apply_string_concat_expr(code, e);
        }
        Statement::Return(Some(e)) => {
            apply_string_concat_expr(code, e);
        }
        Statement::IfElse { cond, if_, else_ } => {
            apply_string_concat_expr(code, cond);
            apply_string_concat(code, if_);
            apply_string_concat(code, else_);
        }
        Statement::IfElseChain { branches, else_ } => {
            for (cond, body) in branches.iter_mut() {
                apply_string_concat_expr(code, cond);
                apply_string_concat(code, body);
            }
            apply_string_concat(code, else_);
        }
        Statement::While { cond, stmts } => {
            apply_string_concat_expr(code, cond);
            apply_string_concat(code, stmts);
        }
        Statement::Switch { arg, default, cases, .. } => {
            apply_string_concat_expr(code, arg);
            apply_string_concat(code, default);
            for (_, case_stmts) in cases.iter_mut() {
                apply_string_concat(code, case_stmts);
            }
        }
        Statement::TryCatch { try_stmts, catch_stmts, .. } => {
            apply_string_concat(code, try_stmts);
            apply_string_concat(code, catch_stmts);
        }
        Statement::Throw(e) => {
            apply_string_concat_expr(code, e);
        }
        Statement::Block { stmts } | Statement::Sequence { stmts } => {
            apply_string_concat(code, stmts);
        }
        Statement::Return(None) | Statement::Break | Statement::Continue
        | Statement::Comment(_) | Statement::VarDecl { .. } => {}
    }
}

fn apply_string_concat_expr(code: &Bytecode, expr: &mut Expr) {
    // First, recurse into sub-expressions
    match expr {
        Expr::Call(call) => {
            apply_string_concat_expr(code, &mut call.fun);
            for arg in &mut call.args {
                apply_string_concat_expr(code, arg);
            }
        }
        Expr::Field(obj, _) => {
            apply_string_concat_expr(code, obj);
        }
        Expr::Array(arr, idx) => {
            apply_string_concat_expr(code, arr);
            apply_string_concat_expr(code, idx);
        }
        Expr::Constructor(ConstructorCall { args, .. }) => {
            for arg in args {
                apply_string_concat_expr(code, arg);
            }
        }
        Expr::Anonymous(_, fields) => {
            for (_, field_expr) in fields {
                apply_string_concat_expr(code, field_expr);
            }
        }
        Expr::Closure(_, stmts) => {
            apply_string_concat(code, stmts);
        }
        Expr::Op(op) => {
            match op {
                Operation::Add(a, b) | Operation::Sub(a, b) | Operation::Mul(a, b)
                | Operation::Div(a, b) | Operation::Mod(a, b) | Operation::And(a, b)
                | Operation::Or(a, b) | Operation::Xor(a, b) | Operation::Shl(a, b)
                | Operation::Shr(a, b) | Operation::Eq(a, b)
                | Operation::NotEq(a, b) | Operation::Gt(a, b) | Operation::Gte(a, b)
                | Operation::Lt(a, b) | Operation::Lte(a, b) => {
                    apply_string_concat_expr(code, a.as_mut());
                    apply_string_concat_expr(code, b.as_mut());
                }
                Operation::Neg(e) | Operation::Not(e) | Operation::Incr(e) | Operation::Decr(e) => {
                    apply_string_concat_expr(code, e.as_mut());
                }
            }
        }
        Expr::Cast(inner, _) | Expr::TypeAnnotated(inner, _) => {
            apply_string_concat_expr(code, inner);
        }
        Expr::ArrayLiteral(elems) => {
            for elem in elems {
                apply_string_concat_expr(code, elem);
            }
        }
        Expr::EnumConstr(_, _, args) => {
            for arg in args {
                apply_string_concat_expr(code, arg);
            }
        }
        Expr::IfElse { cond, if_, else_ } => {
            apply_string_concat_expr(code, cond.as_mut());
            apply_string_concat(code, if_);
            apply_string_concat(code, else_);
        }
        Expr::Constant(_) | Expr::Variable(_, _) | Expr::Ident(_)
        | Expr::FunRef(_) | Expr::Unknown(_) => {}
    }

    // Now check if this is an __add__ call
    if let Expr::Call(call) = expr {
        if let Expr::FunRef(fun) = &call.fun {
            if fun.name(code) == "__add__" && call.args.len() == 2 {
                let arg0 = call.args[0].clone();
                let arg1 = call.args[1].clone();
                *expr = add(arg0, arg1);
            }
        }
    }
}

// =============================================================================
// Trace: Collapse trace() call patterns into simple trace(message) calls
// =============================================================================

/// Collapse verbose trace patterns into simple `trace(message)` calls.
///
/// The decompiler outputs trace calls as:
/// ```haxe
/// var r1 = haxe.Log.trace;
/// // nullcheck r1
/// var r4:Dynamic = {};
/// r4.fileName = "File.hx";
/// r4.lineNumber = 10;
/// r4.className = "MyClass";
/// r4.methodName = "main";
/// r1("message", r4);
/// ```
///
/// This collapses to: `trace("message");`
pub fn collapse_trace_calls(stmts: &mut Vec<Statement>) {
    // Process nested statements first
    for stmt in stmts.iter_mut() {
        collapse_trace_in_stmt(stmt);
    }

    // Now scan for trace patterns at this level
    let mut i = 0;
    while i < stmts.len() {
        if let Some((trace_stmt, consumed)) = try_collapse_trace_pattern(&stmts[i..]) {
            // Remove the consumed statements and insert the collapsed one
            for _ in 0..consumed {
                stmts.remove(i);
            }
            stmts.insert(i, trace_stmt);
        }
        i += 1;
    }
}

fn collapse_trace_in_stmt(stmt: &mut Statement) {
    match stmt {
        Statement::IfElse { if_, else_, .. } => {
            collapse_trace_calls(if_);
            collapse_trace_calls(else_);
        }
        Statement::IfElseChain { branches, else_ } => {
            for (_, body) in branches.iter_mut() {
                collapse_trace_calls(body);
            }
            collapse_trace_calls(else_);
        }
        Statement::While { stmts, .. } => {
            collapse_trace_calls(stmts);
        }
        Statement::Switch { default, cases, .. } => {
            collapse_trace_calls(default);
            for (_, case_stmts) in cases.iter_mut() {
                collapse_trace_calls(case_stmts);
            }
        }
        Statement::TryCatch { try_stmts, catch_stmts, .. } => {
            collapse_trace_calls(try_stmts);
            collapse_trace_calls(catch_stmts);
        }
        Statement::Block { stmts } | Statement::Sequence { stmts } => {
            collapse_trace_calls(stmts);
        }
        _ => {}
    }
}

/// Try to match and collapse a trace pattern starting at the given slice.
/// Returns Some((collapsed_statement, num_statements_consumed)) on success.
fn try_collapse_trace_pattern(stmts: &[Statement]) -> Option<(Statement, usize)> {
    // Need at least 7 statements for the minimal pattern:
    // 1. var trace = haxe.Log.trace
    // 2. nullcheck comment (optional, but let's require it for safety)
    // 3. var posInfo:Dynamic = {}
    // 4. posInfo.fileName = ...
    // 5. posInfo.lineNumber = ...
    // 6. posInfo.className = ...
    // 7. posInfo.methodName = ...
    // 8. trace(message, posInfo)
    if stmts.len() < 7 {
        return None;
    }

    // Step 1: Check for var trace = haxe.Log.trace (or haxe.$Log.trace)
    let trace_var = match &stmts[0] {
        Statement::Assign { variable: Expr::Variable(_, Some(name)), assign, .. } => {
            if is_haxe_log_trace(assign) {
                name.clone()
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Step 2: Skip nullcheck comment if present
    let mut idx = 1;
    if let Statement::Comment(c) = &stmts[idx] {
        if c.contains("nullcheck") {
            idx += 1;
        }
    }

    if idx >= stmts.len() {
        return None;
    }

    // Step 3: Check for var posInfo:Dynamic = {}
    let pos_var = match &stmts[idx] {
        Statement::Assign {
            variable: Expr::Variable(_, Some(name)),
            assign: Expr::Anonymous(_, fields),
            ..
        } if fields.is_empty() => {
            idx += 1;
            name.clone()
        }
        _ => return None,
    };

    // Step 4-7: Check for the four field assignments (fileName, lineNumber, className, methodName)
    // They might be in any order
    let mut found_fields = std::collections::HashSet::new();
    let required_fields = ["fileName", "lineNumber", "className", "methodName"];

    while idx < stmts.len() && found_fields.len() < 4 {
        if let Some(field_name) = is_pos_info_field_assign(&stmts[idx], &pos_var) {
            if required_fields.contains(&field_name.as_str()) {
                found_fields.insert(field_name);
                idx += 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    // Must have all 4 fields
    if found_fields.len() != 4 {
        return None;
    }

    if idx >= stmts.len() {
        return None;
    }

    // Step 8: Check for trace(message, posInfo) call
    let message = match &stmts[idx] {
        Statement::ExprStatement(Expr::Call(call)) => {
            if is_trace_call(call, &trace_var, &pos_var) {
                call.args.get(0).cloned()
            } else {
                return None;
            }
        }
        // Also handle case where trace call result is assigned to void
        Statement::Assign { assign: Expr::Call(call), .. } => {
            if is_trace_call(call, &trace_var, &pos_var) {
                call.args.get(0).cloned()
            } else {
                return None;
            }
        }
        _ => return None,
    };

    let message = message?;
    idx += 1;

    // Create the collapsed trace call
    let trace_call = Statement::ExprStatement(Expr::Call(Box::new(Call {
        fun: Expr::Ident("trace".into()),
        args: vec![message],
    })));

    Some((trace_call, idx))
}

/// Check if an expression is haxe.Log.trace or haxe.$Log.trace
fn is_haxe_log_trace(expr: &Expr) -> bool {
    match expr {
        Expr::Field(obj, field) if field == "trace" => {
            match obj.as_ref() {
                Expr::Ident(name) => name == "haxe.Log" || name == "haxe.$Log",
                Expr::Field(inner, inner_field) => {
                    if inner_field == "Log" || inner_field == "$Log" {
                        matches!(inner.as_ref(), Expr::Ident(n) if n == "haxe")
                    } else {
                        false
                    }
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// Check if a statement is assigning to a posInfo field (fileName, lineNumber, etc.)
/// Returns the field name if it matches.
fn is_pos_info_field_assign(stmt: &Statement, pos_var: &Str) -> Option<String> {
    match stmt {
        Statement::Assign {
            variable: Expr::Field(obj, field_name),
            ..
        } => {
            if let Expr::Variable(_, Some(var_name)) = obj.as_ref() {
                if var_name == pos_var {
                    return Some(field_name.to_string());
                }
            }
            None
        }
        _ => None,
    }
}

/// Check if a call is trace_var(message, pos_var)
fn is_trace_call(call: &Call, trace_var: &Str, pos_var: &Str) -> bool {
    // Check that the function is the trace variable
    let is_trace_var = match &call.fun {
        Expr::Variable(_, Some(name)) => name == trace_var,
        _ => false,
    };

    if !is_trace_var || call.args.len() != 2 {
        return false;
    }

    // Check that the second argument is the posInfo variable
    match &call.args[1] {
        Expr::Variable(_, Some(name)) => name == pos_var,
        _ => false,
    }
}
