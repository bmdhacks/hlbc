use hlbc::{Bytecode, Str};

use crate::ast::{add, Constant, ConstructorCall, Expr, Operation, Statement, Call};

/// Reconstruct array literals from alloc_bytes + SetMem + allocI32 patterns.
///
/// The pattern:
/// ```text
/// var bytes = alloc_bytes(N);
/// bytes[offset] = value1;
/// ...
/// var arr = allocI32(bytes, count);
/// ```
/// becomes:
/// ```text
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

            // Check for return allocI32(bytes, count) - direct return of array
            if let Statement::Return(Some(ret_expr)) = stmt {
                if is_alloc_i32_call(ret_expr, bytes_reg, code).is_some() {
                    // Found return allocI32 - reconstruct as array literal
                    let array_literal = Expr::ArrayLiteral(values);

                    // Replace with return [...]
                    stmts[j] = Statement::Return(Some(array_literal));

                    // Remove all the intermediate statements
                    stmts_to_remove.sort_by(|a, b| b.cmp(a));
                    for idx in stmts_to_remove {
                        stmts.remove(idx);
                    }

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

/// Reconstruct empty array literals from inlined allocI32(alloc_bytes(0), 0) patterns.
///
/// When SSA inlining combines alloc_bytes into allocI32, we get:
/// ```text
/// var arr = allocI32(alloc_bytes(0), 0);
/// ```
/// This should become:
/// ```text
/// var arr = [];
/// ```
pub(crate) fn reconstruct_empty_arrays(code: &Bytecode, stmts: &mut Vec<Statement>) {
    for stmt in stmts.iter_mut() {
        // Check for assignment of allocI32 with inlined alloc_bytes
        if let Statement::Assign { assign, .. } = stmt {
            if let Some(array_literal) = try_convert_inlined_empty_array(assign, code) {
                *assign = array_literal;
            }
        }
        // Recurse into nested statements
        recurse_empty_arrays(code, stmt);
    }
}

fn try_convert_inlined_empty_array(expr: &Expr, code: &Bytecode) -> Option<Expr> {
    if let Expr::Call(call) = expr {
        if let Expr::FunRef(fun_ref) = &call.fun {
            let name = fun_ref.name(code);
            // Check for allocI32, allocI64, allocF64, allocObj, allocDyn
            if matches!(name.as_ref(), "allocI32" | "allocI64" | "allocF64" | "allocObj" | "allocDyn") {
                // Check if first arg is an inlined alloc_bytes call
                if let Some(first_arg) = call.args.first() {
                    if is_alloc_bytes_call(first_arg, code) {
                        // This is allocXXX(alloc_bytes(...), count) - convert to []
                        return Some(Expr::ArrayLiteral(vec![]));
                    }
                }
            }
        }
    }
    None
}

fn recurse_empty_arrays(code: &Bytecode, stmt: &mut Statement) {
    match stmt {
        Statement::IfElse { if_, else_, .. } => {
            reconstruct_empty_arrays(code, if_);
            reconstruct_empty_arrays(code, else_);
        }
        Statement::Switch { default, cases, .. } => {
            reconstruct_empty_arrays(code, default);
            for (_, case_stmts) in cases {
                reconstruct_empty_arrays(code, case_stmts);
            }
        }
        Statement::While { stmts, .. } => {
            reconstruct_empty_arrays(code, stmts);
        }
        Statement::TryCatch { try_stmts, catch_stmts, .. } => {
            reconstruct_empty_arrays(code, try_stmts);
            reconstruct_empty_arrays(code, catch_stmts);
        }
        Statement::Block { stmts } | Statement::Sequence { stmts } => {
            reconstruct_empty_arrays(code, stmts);
        }
        Statement::ForIn { stmts, .. } => {
            reconstruct_empty_arrays(code, stmts);
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
            Operation::LogicalAnd(a, b) | Operation::LogicalOr(a, b) |
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
            Operation::LogicalAnd(a, b) | Operation::LogicalOr(a, b) |
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
        Statement::IfElseChain { branches, else_ } => {
            let mut count = 0;
            for (cond, branch_stmts) in branches {
                count += count_uses_in_expr(cond, var_name);
                count += count_uses_in_stmts(branch_stmts, var_name);
            }
            count += count_uses_in_stmts(else_, var_name);
            count
        }
        Statement::ForIn { var_name: loop_var, iterable, stmts } => {
            // Count uses in the iterable expression
            let mut count = count_uses_in_expr(iterable, var_name);
            // Count uses in the loop body
            count += count_uses_in_stmts(stmts, var_name);
            // Note: The loop variable itself (loop_var) is declared by the for-in,
            // so we don't count it as a use unless it matches var_name
            if loop_var.as_ref() == var_name {
                // The loop variable is "defined" by the for-in, not "used"
                // so we don't add to count here
            }
            count
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
                Operation::LogicalAnd(a, b) => Operation::LogicalAnd(
                    Box::new(replace_var_in_expr(a, var_name, replacement)),
                    Box::new(replace_var_in_expr(b, var_name, replacement)),
                ),
                Operation::LogicalOr(a, b) => Operation::LogicalOr(
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
        Statement::IfElseChain { branches, else_ } => {
            for (cond, branch_stmts) in branches {
                *cond = replace_var_in_expr(cond, var_name, replacement);
                replace_var_in_stmts(branch_stmts, var_name, replacement);
            }
            replace_var_in_stmts(else_, var_name, replacement);
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
            Operation::LogicalAnd(a, b) | Operation::LogicalOr(a, b) |
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

        let use_idx = match use_idx {
            Some(idx) => idx,
            None => continue,
        };

        // Check if safe to inline:
        // - Pure expressions can always be inlined (no side effects)
        // - Impure expressions (function calls) can be inlined into adjacent return statements only
        //   (adjacency + return means no reordering of side effects possible)
        let is_adjacent = use_idx == def_info.def_idx + 1;
        let is_return_stmt = matches!(stmts.get(use_idx), Some(Statement::Return { .. }));
        if !def_info.is_pure && !(is_adjacent && is_return_stmt) {
            continue;
        }

        // Don't inline if the variable is reassigned anywhere
        // (removing the declaration would leave later reassignments without a var declaration)
        if is_reassigned_in_stmts(stmts, var_name, def_info.def_idx) {
            continue;
        }

        // Check if any variable in the expression is reassigned between def and use
        // (if so, inlining would change semantics)
        // Skip this check for adjacent statements - nothing can be reassigned in between
        if !is_adjacent {
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

// Note: remove_unused_var_decls and collect_used_vars functions have been removed.
// The structurer now uses SSA info to track which variables actually have assignments
// emitted, and only emits VarDecls for those. This avoids the fragile approach of
// post-processing to remove orphaned declarations, which was broken for ForIn loops
// and would need to be updated for every new Statement variant.

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

    // For large functions, use conservative approach (all vars) to avoid O(n²) complexity.
    // For normal functions, use precise per-position tracking for better inlining.
    let use_conservative = stmts.len() > LARGE_FUNCTION_THRESHOLD;
    let vars_used_from = if use_conservative {
        Vec::new() // Won't be used
    } else {
        precompute_vars_used_from(stmts)
    };
    let all_vars_used = if use_conservative {
        collect_all_var_names(stmts)
    } else {
        std::collections::HashSet::new() // Won't be used
    };

    while i < stmts.len() {
        // First, recurse into nested structures, passing along info about
        // variables used later in this scope (so nested scopes don't inline them)
        let mut combined_outer: std::collections::HashSet<String> = outer_used_vars.clone();
        if use_conservative {
            combined_outer.extend(all_vars_used.iter().cloned());
        } else if i + 1 < vars_used_from.len() {
            combined_outer.extend(vars_used_from[i + 1].iter().cloned());
        }

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

/// Threshold for switching to conservative variable tracking.
/// Functions with more statements than this use the fast conservative approach.
const LARGE_FUNCTION_THRESHOLD: usize = 1000;

/// Pre-compute variable usage sets for all suffix positions.
/// `result[i]` contains all variable names used in statements from index i to end.
/// This allows O(1) lookup instead of O(n) rescanning.
/// Note: This is O(n × m) due to cloning, where m = unique variable count.
fn precompute_vars_used_from(stmts: &[Statement]) -> Vec<std::collections::HashSet<String>> {
    let mut result = vec![std::collections::HashSet::new(); stmts.len() + 1];

    // Build from end to start: result[i] = result[i+1] ∪ vars_in(stmts[i])
    for i in (0..stmts.len()).rev() {
        result[i] = result[i + 1].clone();
        collect_var_names_in_stmt(&stmts[i], &mut result[i]);
    }

    result
}

/// Collect all variable names used anywhere in the statements.
/// This is a conservative over-approximation: treats all variables as potentially
/// used later, which may miss some inlining opportunities but is truly O(n).
fn collect_all_var_names(stmts: &[Statement]) -> std::collections::HashSet<String> {
    let mut result = std::collections::HashSet::new();
    for stmt in stmts {
        collect_var_names_in_stmt(stmt, &mut result);
    }
    result
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
                Operation::LogicalAnd(l, r) | Operation::LogicalOr(l, r) |
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
        Statement::ForIn { iterable, stmts, .. } => {
            apply_string_concat_expr(code, iterable);
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
                | Operation::Or(a, b) | Operation::LogicalAnd(a, b) | Operation::LogicalOr(a, b)
                | Operation::Xor(a, b) | Operation::Shl(a, b)
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
        Expr::Range(start, end) => {
            apply_string_concat_expr(code, start.as_mut());
            apply_string_concat_expr(code, end.as_mut());
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

    // Scan backwards from the end to find trace calls, then remove their components
    collapse_trace_patterns_reverse(stmts);
}

/// Collapse trace patterns by scanning for trace calls and removing setup code.
fn collapse_trace_patterns_reverse(stmts: &mut Vec<Statement>) {
    // Process from end to start so removals don't affect indices we haven't processed yet
    let mut i = stmts.len();
    while i > 0 {
        i -= 1;

        // Step 1: Check if this is a trace call, and if so replace it with trace(msg)
        if let Some((trace_var, pos_var, message)) = extract_trace_call_info(&stmts[i]) {
            // Try to find the required components backwards
            let mut found_trace_load = false;
            let mut found_pos_init = false;
            let mut found_fields = std::collections::HashSet::new();
            let required_fields = ["fileName", "lineNumber", "className", "methodName"];

            // First pass: check if all components exist
            for j in (0..i).rev() {
                if !found_trace_load {
                    if let Some(name) = is_trace_load_stmt(&stmts[j]) {
                        if name == trace_var {
                            found_trace_load = true;
                            continue;
                        }
                    }
                }
                if !found_pos_init {
                    if let Some(name) = is_pos_info_init_stmt(&stmts[j]) {
                        if name == pos_var {
                            found_pos_init = true;
                            continue;
                        }
                    }
                }
                if let Some(field_name) = is_pos_info_field_assign(&stmts[j], &pos_var) {
                    if required_fields.contains(&field_name.as_str()) {
                        found_fields.insert(field_name);
                    }
                }
                if found_trace_load && found_pos_init && found_fields.len() == 4 {
                    break;
                }
            }

            // Only proceed if we found all components
            if !found_trace_load || !found_pos_init || found_fields.len() != 4 {
                continue;
            }

            // Replace the trace call in-place with simple trace(msg)
            stmts[i] = Statement::ExprStatement(Expr::Call(Box::new(Call {
                fun: Expr::Ident("trace".into()),
                args: vec![message],
            })));

            // Step 2: Walk backwards and delete the setup components
            let mut j = i;
            let mut deleted_trace_load = false;
            let mut deleted_pos_init = false;
            let mut deleted_fields = std::collections::HashSet::new();

            while j > 0 {
                j -= 1;
                let mut should_remove = false;

                if !deleted_trace_load {
                    if let Some(name) = is_trace_load_stmt(&stmts[j]) {
                        if name == trace_var {
                            deleted_trace_load = true;
                            should_remove = true;
                        }
                    }
                }
                if !should_remove && !deleted_pos_init {
                    if let Some(name) = is_pos_info_init_stmt(&stmts[j]) {
                        if name == pos_var {
                            deleted_pos_init = true;
                            should_remove = true;
                        }
                    }
                }
                if !should_remove {
                    if let Some(field_name) = is_pos_info_field_assign(&stmts[j], &pos_var) {
                        if required_fields.contains(&field_name.as_str()) && !deleted_fields.contains(&field_name) {
                            deleted_fields.insert(field_name);
                            should_remove = true;
                        }
                    }
                }

                if should_remove {
                    stmts.remove(j);
                    i -= 1; // Adjust our position since we removed something before it
                }

                // Stop when we've deleted everything
                if deleted_trace_load && deleted_pos_init && deleted_fields.len() == 4 {
                    break;
                }
            }
        }
    }
}

/// Extract trace call info: returns (trace_var_name, posInfo_var_name, message_expr)
fn extract_trace_call_info(stmt: &Statement) -> Option<(Str, Str, Expr)> {
    let call = match stmt {
        Statement::ExprStatement(Expr::Call(call)) => call,
        Statement::Assign { assign: Expr::Call(call), .. } => call,
        _ => return None,
    };

    // Must be a 2-arg call
    if call.args.len() != 2 {
        return None;
    }

    // First arg is the message
    let message = call.args[0].clone();

    // Second arg must be a variable (the posInfo)
    let pos_var = match &call.args[1] {
        Expr::Variable(_, Some(name)) => name.clone(),
        _ => return None,
    };

    // Function must be a variable (the trace function)
    let trace_var = match &call.fun {
        Expr::Variable(_, Some(name)) => name.clone(),
        _ => return None,
    };

    Some((trace_var, pos_var, message))
}

/// Check if statement is a trace load: var X = haxe.Log.trace
fn is_trace_load_stmt(stmt: &Statement) -> Option<Str> {
    match stmt {
        Statement::Assign { variable: Expr::Variable(_, Some(name)), assign, .. } => {
            if is_haxe_log_trace(assign) {
                Some(name.clone())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Check if statement is posInfo init: var X:Dynamic = {}
fn is_pos_info_init_stmt(stmt: &Statement) -> Option<Str> {
    match stmt {
        Statement::Assign {
            variable: Expr::Variable(_, Some(name)),
            assign: Expr::Anonymous(_, fields),
            ..
        } if fields.is_empty() => Some(name.clone()),
        _ => None,
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

// =============================================================================
// Boolean Simplification: Fold nested if statements into && and || expressions
// =============================================================================

/// Simplify nested if statements into boolean expressions.
///
/// Transforms:
/// - `if (a) { if (b) { X } }` → `if (a && b) { X }`
/// - `if (a) { X } else if (b) { X }` → `if (a || b) { X }` (when X is identical)
///
/// This produces cleaner, more idiomatic code that matches the original source.
pub fn simplify_boolean_conditions(stmts: &mut Vec<Statement>) -> bool {
    let mut changed = false;

    for stmt in stmts.iter_mut() {
        // Recurse into nested structures first
        match stmt {
            Statement::IfElse { if_, else_, .. } => {
                if simplify_boolean_conditions(if_) {
                    changed = true;
                }
                if simplify_boolean_conditions(else_) {
                    changed = true;
                }
            }
            Statement::IfElseChain { branches, else_ } => {
                for (_, body) in branches.iter_mut() {
                    if simplify_boolean_conditions(body) {
                        changed = true;
                    }
                }
                if simplify_boolean_conditions(else_) {
                    changed = true;
                }
            }
            Statement::While { stmts, .. } => {
                if simplify_boolean_conditions(stmts) {
                    changed = true;
                }
            }
            Statement::Switch { default, cases, .. } => {
                if simplify_boolean_conditions(default) {
                    changed = true;
                }
                for (_, case_stmts) in cases.iter_mut() {
                    if simplify_boolean_conditions(case_stmts) {
                        changed = true;
                    }
                }
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                if simplify_boolean_conditions(try_stmts) {
                    changed = true;
                }
                if simplify_boolean_conditions(catch_stmts) {
                    changed = true;
                }
            }
            Statement::Block { stmts } | Statement::Sequence { stmts } => {
                if simplify_boolean_conditions(stmts) {
                    changed = true;
                }
            }
            _ => {}
        }

        // Now try to simplify this statement itself
        if let Statement::IfElse { cond, if_, else_ } = stmt {
            // Pattern 1: if (a) { if (b) { X } } with empty else
            // → if (a && b) { X }
            if else_.is_empty() && if_.len() == 1 {
                if let Statement::IfElse {
                    cond: inner_cond,
                    if_: inner_if,
                    else_: inner_else,
                } = &if_[0]
                {
                    if inner_else.is_empty() {
                        // Fold: if (a) { if (b) { X } } → if (a && b) { X }
                        let new_cond = Expr::Op(Operation::LogicalAnd(
                            Box::new(cond.clone()),
                            Box::new(inner_cond.clone()),
                        ));
                        *cond = new_cond;
                        *if_ = inner_if.clone();
                        changed = true;
                    }
                }
            }
        }
    }

    // Pattern 2: if (a) { X } else if (b) { X } → if (a || b) { X }
    // This needs to compare statement bodies for equality, which is more complex
    // Implement a simpler version: look for if-else chains with identical bodies
    changed |= fold_or_chains(stmts);

    changed
}

/// Fold if-else chains with identical bodies into || expressions.
///
/// Transforms:
/// ```haxe
/// if (a) {
///     return x;
/// } else if (b) {
///     return x;
/// }
/// ```
/// into:
/// ```haxe
/// if (a || b) {
///     return x;
/// }
/// ```
fn fold_or_chains(stmts: &mut Vec<Statement>) -> bool {
    let mut changed = false;

    for stmt in stmts.iter_mut() {
        if let Statement::IfElse { cond, if_, else_ } = stmt {
            // Check if else is a single if-else with identical body
            if else_.len() == 1 {
                if let Statement::IfElse {
                    cond: else_cond,
                    if_: else_if,
                    else_: else_else,
                } = &else_[0]
                {
                    // Check if bodies are identical
                    if statements_equal(if_, else_if) {
                        // Fold: if (a) { X } else if (b) { X } → if (a || b) { X }
                        let new_cond = Expr::Op(Operation::LogicalOr(
                            Box::new(cond.clone()),
                            Box::new(else_cond.clone()),
                        ));
                        *cond = new_cond;
                        // Keep the original if_ body, take the inner else as our else
                        *else_ = else_else.clone();
                        changed = true;
                    }
                }
            }
        }
    }

    changed
}

/// Check if two statement lists are structurally equal.
///
/// This is a conservative comparison - returns true only if statements
/// are obviously identical. Used for boolean folding.
fn statements_equal(a: &[Statement], b: &[Statement]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    for (s1, s2) in a.iter().zip(b.iter()) {
        if !statement_equal(s1, s2) {
            return false;
        }
    }

    true
}

/// Check if two statements are structurally equal.
fn statement_equal(a: &Statement, b: &Statement) -> bool {
    match (a, b) {
        (Statement::Return(Some(e1)), Statement::Return(Some(e2))) => expr_equal(e1, e2),
        (Statement::Return(None), Statement::Return(None)) => true,
        (Statement::Break, Statement::Break) => true,
        (Statement::Continue, Statement::Continue) => true,
        (Statement::Throw(e1), Statement::Throw(e2)) => expr_equal(e1, e2),
        (Statement::ExprStatement(e1), Statement::ExprStatement(e2)) => expr_equal(e1, e2),
        (
            Statement::Assign { declaration: d1, variable: v1, assign: a1 },
            Statement::Assign { declaration: d2, variable: v2, assign: a2 },
        ) => d1 == d2 && expr_equal(v1, v2) && expr_equal(a1, a2),
        (
            Statement::IfElse { cond: c1, if_: if1, else_: else1 },
            Statement::IfElse { cond: c2, if_: if2, else_: else2 },
        ) => expr_equal(c1, c2) && statements_equal(if1, if2) && statements_equal(else1, else2),
        (
            Statement::While { cond: c1, stmts: s1 },
            Statement::While { cond: c2, stmts: s2 },
        ) => expr_equal(c1, c2) && statements_equal(s1, s2),
        // For other complex statements, be conservative and return false
        _ => false,
    }
}

/// Check if two expressions are structurally equal.
fn expr_equal(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Constant(c1), Expr::Constant(c2)) => constant_equal(c1, c2),
        (Expr::Variable(r1, n1), Expr::Variable(r2, n2)) => r1 == r2 && n1 == n2,
        (Expr::Ident(i1), Expr::Ident(i2)) => i1 == i2,
        (Expr::Field(o1, f1), Expr::Field(o2, f2)) => f1 == f2 && expr_equal(o1, o2),
        (Expr::Array(a1, i1), Expr::Array(a2, i2)) => expr_equal(a1, a2) && expr_equal(i1, i2),
        (Expr::Call(c1), Expr::Call(c2)) => {
            expr_equal(&c1.fun, &c2.fun)
                && c1.args.len() == c2.args.len()
                && c1.args.iter().zip(c2.args.iter()).all(|(x, y)| expr_equal(x, y))
        }
        (Expr::Op(o1), Expr::Op(o2)) => op_equal(o1, o2),
        (Expr::FunRef(f1), Expr::FunRef(f2)) => f1 == f2,
        // For other expressions, be conservative
        _ => false,
    }
}

/// Check if two constants are equal.
fn constant_equal(a: &Constant, b: &Constant) -> bool {
    match (a, b) {
        (Constant::Null, Constant::Null) => true,
        (Constant::This, Constant::This) => true,
        (Constant::Bool(b1), Constant::Bool(b2)) => b1 == b2,
        (Constant::Int(i1), Constant::Int(i2)) => i1 == i2,
        (Constant::InlineInt(i1), Constant::InlineInt(i2)) => i1 == i2,
        (Constant::Float(f1), Constant::Float(f2)) => f1 == f2,
        (Constant::String(s1), Constant::String(s2)) => s1 == s2,
        (Constant::TypeRef(t1), Constant::TypeRef(t2)) => t1 == t2,
        _ => false,
    }
}

/// Check if two operations are equal.
fn op_equal(a: &Operation, b: &Operation) -> bool {
    match (a, b) {
        (Operation::Add(l1, r1), Operation::Add(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Sub(l1, r1), Operation::Sub(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Mul(l1, r1), Operation::Mul(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Div(l1, r1), Operation::Div(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Mod(l1, r1), Operation::Mod(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Shl(l1, r1), Operation::Shl(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Shr(l1, r1), Operation::Shr(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::And(l1, r1), Operation::And(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Or(l1, r1), Operation::Or(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::LogicalAnd(l1, r1), Operation::LogicalAnd(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::LogicalOr(l1, r1), Operation::LogicalOr(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Xor(l1, r1), Operation::Xor(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Eq(l1, r1), Operation::Eq(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::NotEq(l1, r1), Operation::NotEq(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Gt(l1, r1), Operation::Gt(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Gte(l1, r1), Operation::Gte(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Lt(l1, r1), Operation::Lt(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Lte(l1, r1), Operation::Lte(l2, r2)) => expr_equal(l1, l2) && expr_equal(r1, r2),
        (Operation::Neg(e1), Operation::Neg(e2)) => expr_equal(e1, e2),
        (Operation::Not(e1), Operation::Not(e2)) => expr_equal(e1, e2),
        (Operation::Incr(e1), Operation::Incr(e2)) => expr_equal(e1, e2),
        (Operation::Decr(e1), Operation::Decr(e2)) => expr_equal(e1, e2),
        _ => false,
    }
}

// =============================================================================
// For-In Detection: Convert while loops with counter patterns to for-in loops
// =============================================================================

/// Detect and convert while loops with counter patterns to for-in loops.
///
/// Transforms patterns like:
/// ```haxe
/// var i = 0;
/// while (i < n) {
///     // body
///     i++;
/// }
/// ```
/// into:
/// ```haxe
/// for (i in 0...n) {
///     // body
/// }
/// ```
///
/// This produces cleaner, more idiomatic Haxe code.
pub fn detect_for_in_loops(stmts: &mut Vec<Statement>) -> bool {
    let mut changed = false;
    let mut i = 0;

    while i < stmts.len() {
        // Recursively process nested structures first
        match &mut stmts[i] {
            Statement::IfElse { if_, else_, .. } => {
                if detect_for_in_loops(if_) { changed = true; }
                if detect_for_in_loops(else_) { changed = true; }
            }
            Statement::IfElseChain { branches, else_ } => {
                for (_, body) in branches.iter_mut() {
                    if detect_for_in_loops(body) { changed = true; }
                }
                if detect_for_in_loops(else_) { changed = true; }
            }
            Statement::While { stmts: body, .. } => {
                if detect_for_in_loops(body) { changed = true; }
            }
            Statement::ForIn { stmts: body, .. } => {
                if detect_for_in_loops(body) { changed = true; }
            }
            Statement::Switch { default, cases, .. } => {
                if detect_for_in_loops(default) { changed = true; }
                for (_, case_stmts) in cases.iter_mut() {
                    if detect_for_in_loops(case_stmts) { changed = true; }
                }
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                if detect_for_in_loops(try_stmts) { changed = true; }
                if detect_for_in_loops(catch_stmts) { changed = true; }
            }
            Statement::Block { stmts: inner } | Statement::Sequence { stmts: inner } => {
                if detect_for_in_loops(inner) { changed = true; }
            }
            _ => {}
        }

        // Look for pattern: var i = start; while (i < end) { body; i++; }
        // Also handles Haxe's pattern where increment is at START: while (i < end) { i++; body; }
        if i + 1 < stmts.len() {
            if let Some((counter_name, start_expr)) = extract_counter_init(&stmts[i]) {
                if let Statement::While { cond, stmts: body } = &stmts[i + 1] {
                    if let Some(end_expr) = extract_less_than_condition(cond, &counter_name) {
                        // Try trailing increment first (C-style: body; i++)
                        if let Some(new_body) = extract_body_with_increment(body, &counter_name) {
                            // Found the pattern! Convert to for-in
                            let for_in = Statement::ForIn {
                                var_name: counter_name.into(),
                                iterable: Expr::Range(
                                    Box::new(start_expr),
                                    Box::new(end_expr),
                                ),
                                stmts: new_body,
                            };

                            // Remove the init statement and replace while with for-in
                            stmts.remove(i);
                            stmts[i] = for_in;
                            changed = true;
                            continue;
                        }

                        // Try Haxe-style pattern: visible = counter; counter++; body
                        // This is for (visible in 0...end) { body }
                        if let Some((visible_var, new_body)) = extract_haxe_range_pattern(body, &counter_name) {
                            let for_in = Statement::ForIn {
                                var_name: visible_var.into(),
                                iterable: Expr::Range(
                                    Box::new(start_expr),
                                    Box::new(end_expr),
                                ),
                                stmts: new_body,
                            };

                            stmts.remove(i);
                            stmts[i] = for_in;
                            changed = true;
                            continue;
                        }

                        // Try simple leading increment (just i++; body)
                        if let Some(new_body) = extract_body_with_leading_increment(body, &counter_name) {
                            let for_in = Statement::ForIn {
                                var_name: counter_name.into(),
                                iterable: Expr::Range(
                                    Box::new(start_expr),
                                    Box::new(end_expr),
                                ),
                                stmts: new_body,
                            };

                            stmts.remove(i);
                            stmts[i] = for_in;
                            changed = true;
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

/// Extract counter initialization from a statement.
/// Returns (var_name, start_value) if this is `name = value` with an integer.
/// Handles both declarations (`var i = 0`) and assignments (`i = 0`).
fn extract_counter_init(stmt: &Statement) -> Option<(String, Expr)> {
    match stmt {
        // Declaration: var i = 0
        Statement::Assign {
            declaration: true,
            variable: Expr::Variable(_, Some(name)),
            assign,
        } => {
            if is_integer_expr(assign) {
                Some((name.to_string(), assign.clone()))
            } else {
                None
            }
        }
        // Non-declaration assignment: i = 0 (variable was declared separately)
        Statement::Assign {
            declaration: false,
            variable: Expr::Variable(_, Some(name)),
            assign,
        } => {
            if is_integer_expr(assign) {
                Some((name.to_string(), assign.clone()))
            } else {
                None
            }
        }
        // Also handle Expr::Ident for names without registers
        Statement::Assign {
            variable: Expr::Ident(name),
            assign,
            ..
        } => {
            if is_integer_expr(assign) {
                Some((name.to_string(), assign.clone()))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Check if expression is an integer constant.
fn is_integer_expr(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Constant(Constant::Int(_)) | Expr::Constant(Constant::InlineInt(_))
    )
}

/// Extract the upper bound from a less-than condition.
/// Returns Some(end_expr) if condition is `var_name < end_expr`.
fn extract_less_than_condition(cond: &Expr, var_name: &str) -> Option<Expr> {
    match cond {
        Expr::Op(Operation::Lt(left, right)) => {
            // Check if left side is our counter variable
            if is_var_named(left, var_name) {
                Some((**right).clone())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Check if an expression is a variable with the given name.
fn is_var_named(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Variable(_, Some(var_name)) => var_name.as_ref() == name,
        Expr::Ident(ident) => ident.as_ref() == name,
        _ => false,
    }
}

/// Extract the loop body if it ends with an increment of the counter variable.
/// Returns Some(body_without_increment) if the body ends with `var++` or `var = var + 1`.
fn extract_body_with_increment(body: &[Statement], var_name: &str) -> Option<Vec<Statement>> {
    if body.is_empty() {
        return None;
    }

    let last = &body[body.len() - 1];

    // Check for i++ as expression statement
    if let Statement::ExprStatement(Expr::Op(Operation::Incr(inner))) = last {
        if is_var_named(inner, var_name) {
            return Some(body[..body.len() - 1].to_vec());
        }
    }

    // Check for i = i + 1 pattern
    if let Statement::Assign {
        variable,
        assign: Expr::Op(Operation::Add(left, right)),
        ..
    } = last
    {
        if is_var_named(variable, var_name) {
            // Check if it's var = var + 1 or var = 1 + var
            let is_add_one = (is_var_named(left, var_name) && is_one(right))
                || (is_one(left) && is_var_named(right, var_name));
            if is_add_one {
                return Some(body[..body.len() - 1].to_vec());
            }
        }
    }

    None
}

/// Check if expression is the constant 1.
fn is_one(expr: &Expr) -> bool {
    matches!(expr, Expr::Constant(Constant::InlineInt(1)))
}

/// Extract the loop body if it STARTS with an increment of the counter variable.
/// This is the Haxe pattern where `for (i in 0...n)` compiles to:
/// ```
/// i = 0;
/// while (i < n) {
///     i++;  // increment at START
///     // body
/// }
/// ```
/// Returns Some(body_without_increment) if the body starts with `var++` or `var = var + 1`.
fn extract_body_with_leading_increment(body: &[Statement], var_name: &str) -> Option<Vec<Statement>> {
    if body.is_empty() {
        return None;
    }

    let first = &body[0];

    // Check for i++ as expression statement
    if let Statement::ExprStatement(Expr::Op(Operation::Incr(inner))) = first {
        if is_var_named(inner, var_name) {
            return Some(body[1..].to_vec());
        }
    }

    // Check for i = i + 1 pattern
    if let Statement::Assign {
        variable,
        assign: Expr::Op(Operation::Add(left, right)),
        ..
    } = first
    {
        if is_var_named(variable, var_name) {
            // Check if it's var = var + 1 or var = 1 + var
            let is_add_one = (is_var_named(left, var_name) && is_one(right))
                || (is_one(left) && is_var_named(right, var_name));
            if is_add_one {
                return Some(body[1..].to_vec());
            }
        }
    }

    None
}

/// Extract the Haxe range loop pattern where there's a separate loop variable.
/// Pattern:
/// ```
/// counter = 0;
/// while (counter < limit) {
///     loop_var = counter;    // copy counter to user variable
///     counter = counter + 1; // increment counter
///     ... body using loop_var ...
/// }
/// ```
/// Returns Some((loop_var_name, body)) if the pattern matches.
fn extract_haxe_range_pattern(body: &[Statement], counter_name: &str) -> Option<(String, Vec<Statement>)> {
    if body.len() < 2 {
        return None;
    }

    // First statement should be: loop_var = counter
    let loop_var_name = match &body[0] {
        Statement::Assign {
            variable: Expr::Variable(_, Some(name)),
            assign,
            ..
        } => {
            // Check if assign is the counter variable
            if is_var_named(assign, counter_name) {
                // Make sure loop_var is different from counter
                if name.as_ref() != counter_name {
                    Some(name.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        }
        Statement::Assign {
            variable: Expr::Ident(name),
            assign,
            ..
        } => {
            if is_var_named(assign, counter_name) && name.as_ref() != counter_name {
                Some(name.to_string())
            } else {
                None
            }
        }
        _ => None,
    }?;

    // Second statement should be: counter = counter + 1 (or counter++)
    let is_increment = match &body[1] {
        Statement::ExprStatement(Expr::Op(Operation::Incr(inner))) => {
            is_var_named(inner, counter_name)
        }
        Statement::Assign {
            variable,
            assign: Expr::Op(Operation::Add(left, right)),
            ..
        } => {
            is_var_named(variable, counter_name)
                && ((is_var_named(left, counter_name) && is_one(right))
                    || (is_one(left) && is_var_named(right, counter_name)))
        }
        _ => false,
    };

    if !is_increment {
        return None;
    }

    // The rest is the loop body
    Some((loop_var_name, body[2..].to_vec()))
}

// =============================================================================
// Remove Trailing Continues: Remove superfluous `continue` at end of loops
// =============================================================================

/// Remove superfluous `continue` statements at the end of loop bodies.
///
/// A `continue` at the very end of a loop body is redundant since the loop
/// would naturally continue anyway. This cleanup produces cleaner output.
///
/// Also handles `continue` at the end of if-else branches within loops.
pub fn remove_trailing_continues(stmts: &mut Vec<Statement>) -> bool {
    let mut changed = false;

    for stmt in stmts.iter_mut() {
        match stmt {
            Statement::While { stmts: body, .. } => {
                // Recursively process nested structures
                if remove_trailing_continues(body) {
                    changed = true;
                }
                // Remove trailing continue from this loop body
                if remove_trailing_continue_from_body(body) {
                    changed = true;
                }
            }
            Statement::ForIn { stmts: body, .. } => {
                if remove_trailing_continues(body) {
                    changed = true;
                }
                if remove_trailing_continue_from_body(body) {
                    changed = true;
                }
            }
            Statement::IfElse { if_, else_, .. } => {
                if remove_trailing_continues(if_) {
                    changed = true;
                }
                if remove_trailing_continues(else_) {
                    changed = true;
                }
            }
            Statement::IfElseChain { branches, else_ } => {
                for (_, body) in branches.iter_mut() {
                    if remove_trailing_continues(body) {
                        changed = true;
                    }
                }
                if remove_trailing_continues(else_) {
                    changed = true;
                }
            }
            Statement::Switch { cases, default, .. } => {
                for (_, body) in cases.iter_mut() {
                    if remove_trailing_continues(body) {
                        changed = true;
                    }
                }
                if remove_trailing_continues(default) {
                    changed = true;
                }
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                if remove_trailing_continues(try_stmts) {
                    changed = true;
                }
                if remove_trailing_continues(catch_stmts) {
                    changed = true;
                }
            }
            _ => {}
        }
    }

    changed
}

/// Remove a trailing `continue` from a loop body.
/// Also handles `continue` at the end of if-else branches.
fn remove_trailing_continue_from_body(body: &mut Vec<Statement>) -> bool {
    if body.is_empty() {
        return false;
    }

    // Check if the last statement is a bare continue
    if matches!(body.last(), Some(Statement::Continue)) {
        body.pop();
        return true;
    }

    // Check if the last statement is an if-else where both branches end in continue
    // In this case, we can remove the continues from both branches
    if let Some(Statement::IfElse { if_, else_, .. }) = body.last_mut() {
        let mut changed = false;

        // Remove trailing continue from if branch
        if matches!(if_.last(), Some(Statement::Continue)) {
            if_.pop();
            changed = true;
        }

        // Remove trailing continue from else branch
        if matches!(else_.last(), Some(Statement::Continue)) {
            else_.pop();
            changed = true;
        }

        return changed;
    }

    false
}

// =============================================================================
// Remove Empty If Statements: Clean up empty if/else after internal call suppression
// =============================================================================

/// Remove empty if statements that result from suppressing internal calls like `__expand`.
///
/// When internal functions like `__expand` are suppressed, the bounds-check if statements
/// that guard them become empty:
/// ```haxe
/// if (idx >= arr.length) {
///     // __expand was here but got suppressed
/// }
/// arr[idx] = value;
/// ```
/// This pass removes such empty if statements.
pub fn remove_empty_if_statements(stmts: &mut Vec<Statement>) -> bool {
    let mut changed = false;

    // First, recursively process nested structures
    for stmt in stmts.iter_mut() {
        match stmt {
            Statement::IfElse { if_, else_, .. } => {
                if remove_empty_if_statements(if_) { changed = true; }
                if remove_empty_if_statements(else_) { changed = true; }
            }
            Statement::IfElseChain { branches, else_ } => {
                for (_, body) in branches.iter_mut() {
                    if remove_empty_if_statements(body) { changed = true; }
                }
                if remove_empty_if_statements(else_) { changed = true; }
            }
            Statement::While { stmts: body, .. } => {
                if remove_empty_if_statements(body) { changed = true; }
            }
            Statement::ForIn { stmts: body, .. } => {
                if remove_empty_if_statements(body) { changed = true; }
            }
            Statement::Switch { default, cases, .. } => {
                if remove_empty_if_statements(default) { changed = true; }
                for (_, case_stmts) in cases.iter_mut() {
                    if remove_empty_if_statements(case_stmts) { changed = true; }
                }
            }
            Statement::TryCatch { try_stmts, catch_stmts, .. } => {
                if remove_empty_if_statements(try_stmts) { changed = true; }
                if remove_empty_if_statements(catch_stmts) { changed = true; }
            }
            Statement::Block { stmts: inner } | Statement::Sequence { stmts: inner } => {
                if remove_empty_if_statements(inner) { changed = true; }
            }
            _ => {}
        }
    }

    // Then, remove if statements where BOTH branches are empty
    stmts.retain(|stmt| {
        match stmt {
            Statement::IfElse { if_, else_, .. } => {
                if if_.is_empty() && else_.is_empty() {
                    changed = true;
                    false // remove
                } else {
                    true // keep
                }
            }
            _ => true,
        }
    });

    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Constant, Expr, Operation, Statement};

    #[test]
    fn test_simplify_nested_if_to_and() {
        // if (a) { if (b) { return true; } }
        // should become: if (a && b) { return true; }
        let mut stmts = vec![Statement::IfElse {
            cond: Expr::Variable(hlbc::types::Reg(0), Some("a".into())),
            if_: vec![Statement::IfElse {
                cond: Expr::Variable(hlbc::types::Reg(1), Some("b".into())),
                if_: vec![Statement::Return(Some(Expr::Constant(Constant::Bool(true))))],
                else_: vec![],
            }],
            else_: vec![],
        }];

        let changed = simplify_boolean_conditions(&mut stmts);
        assert!(changed, "Should have made changes");

        // Check the result is if (a && b) { return true; }
        match &stmts[0] {
            Statement::IfElse { cond, if_, else_ } => {
                // Condition should be a && b
                assert!(matches!(cond, Expr::Op(Operation::LogicalAnd(_, _))));
                // Body should be single return
                assert_eq!(if_.len(), 1);
                assert!(matches!(&if_[0], Statement::Return(Some(_))));
                // Else should be empty
                assert!(else_.is_empty());
            }
            _ => panic!("Expected IfElse"),
        }
    }

    #[test]
    fn test_simplify_or_chain() {
        // if (a) { return x; } else if (b) { return x; }
        // should become: if (a || b) { return x; }
        let return_x = Statement::Return(Some(Expr::Variable(hlbc::types::Reg(2), Some("x".into()))));

        let mut stmts = vec![Statement::IfElse {
            cond: Expr::Variable(hlbc::types::Reg(0), Some("a".into())),
            if_: vec![return_x.clone()],
            else_: vec![Statement::IfElse {
                cond: Expr::Variable(hlbc::types::Reg(1), Some("b".into())),
                if_: vec![return_x.clone()],
                else_: vec![],
            }],
        }];

        let changed = simplify_boolean_conditions(&mut stmts);
        assert!(changed, "Should have made changes");

        // Check the result is if (a || b) { return x; }
        match &stmts[0] {
            Statement::IfElse { cond, if_, else_ } => {
                // Condition should be a || b
                assert!(matches!(cond, Expr::Op(Operation::LogicalOr(_, _))));
                // Body should be single return
                assert_eq!(if_.len(), 1);
                // Else should be empty (from the inner else)
                assert!(else_.is_empty());
            }
            _ => panic!("Expected IfElse"),
        }
    }

    #[test]
    fn test_statements_equal() {
        let ret1 = Statement::Return(Some(Expr::Constant(Constant::Bool(true))));
        let ret2 = Statement::Return(Some(Expr::Constant(Constant::Bool(true))));
        let ret3 = Statement::Return(Some(Expr::Constant(Constant::Bool(false))));

        assert!(statement_equal(&ret1, &ret2));
        assert!(!statement_equal(&ret1, &ret3));
    }

    #[test]
    fn test_expr_equal() {
        let a = Expr::Variable(hlbc::types::Reg(0), Some("x".into()));
        let b = Expr::Variable(hlbc::types::Reg(0), Some("x".into()));
        let c = Expr::Variable(hlbc::types::Reg(1), Some("y".into()));

        assert!(expr_equal(&a, &b));
        assert!(!expr_equal(&a, &c));
    }

    #[test]
    fn test_detect_for_in_loop() {
        // var i = 0; while (i < 10) { doSomething(); i++; }
        // should become: for (i in 0...10) { doSomething(); }
        let mut stmts = vec![
            // var i = 0;
            Statement::Assign {
                declaration: true,
                variable: Expr::Variable(hlbc::types::Reg(0), Some("i".into())),
                assign: Expr::Constant(Constant::InlineInt(0)),
            },
            // while (i < 10) { doSomething(); i++; }
            Statement::While {
                cond: Expr::Op(Operation::Lt(
                    Box::new(Expr::Variable(hlbc::types::Reg(0), Some("i".into()))),
                    Box::new(Expr::Constant(Constant::InlineInt(10))),
                )),
                stmts: vec![
                    // doSomething() - placeholder
                    Statement::ExprStatement(Expr::Call(Box::new(Call {
                        fun: Expr::Ident("doSomething".into()),
                        args: vec![],
                    }))),
                    // i++
                    Statement::ExprStatement(Expr::Op(Operation::Incr(
                        Box::new(Expr::Variable(hlbc::types::Reg(0), Some("i".into()))),
                    ))),
                ],
            },
        ];

        let changed = detect_for_in_loops(&mut stmts);
        assert!(changed, "Should have detected for-in pattern");
        assert_eq!(stmts.len(), 1, "Should have combined into single for-in");

        // Check the result is for (i in 0...10) { doSomething(); }
        match &stmts[0] {
            Statement::ForIn { var_name, iterable, stmts } => {
                assert_eq!(var_name.as_ref(), "i");
                assert!(matches!(iterable, Expr::Range(_, _)));
                assert_eq!(stmts.len(), 1, "Body should only have doSomething()");
            }
            _ => panic!("Expected ForIn statement"),
        }
    }
}
