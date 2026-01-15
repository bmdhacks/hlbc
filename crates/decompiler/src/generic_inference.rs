//! Generic Type Parameter Inference
//!
//! This module analyzes method bodies to infer type parameters for generic container fields.
//! For example, if a field is `var map: IntMap<Dynamic>` and we see `map.set(key, "hello")`,
//! we can infer the type parameter is String.

use std::collections::HashMap;

use hlbc::types::Type;
use hlbc::Bytecode;

use crate::ast::{ClassField, Constant, Expr, InferredGenericParams, Statement};
use crate::fmt::{to_haxe_type, KNOWN_SINGLE_PARAM_GENERICS, KNOWN_TWO_PARAM_GENERICS};

/// Analyze a class's methods and infer generic type parameters for fields.
pub fn infer_generic_params_for_class(
    fields: &mut [ClassField],
    methods: &[Vec<Statement>],
    code: &Bytecode,
) {
    // Build a map from field name to field index for quick lookup
    // Use String keys to avoid borrowing issues
    let field_map: HashMap<String, usize> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.to_string(), i))
        .collect();

    // Initialize inference for fields with generic types
    for field in fields.iter_mut() {
        if let Some(param_count) = get_generic_param_count(&code[field.ty], code) {
            field.inferred_generics = Some(InferredGenericParams::new(param_count));
        }
    }

    // Analyze each method's statements
    for statements in methods {
        analyze_statements(statements, fields, &field_map, code);
    }

    // Finalize all inferences
    for field in fields.iter_mut() {
        if let Some(ref mut inference) = field.inferred_generics {
            inference.finalize();
        }
    }
}

/// Check if a type is a known generic container and return its parameter count.
fn get_generic_param_count(ty: &Type, code: &Bytecode) -> Option<usize> {
    let type_name = ty.get_type_obj()?.name(code);
    let type_name = type_name.as_ref();

    if KNOWN_SINGLE_PARAM_GENERICS.iter().any(|g| type_name == *g) {
        Some(1)
    } else if KNOWN_TWO_PARAM_GENERICS.iter().any(|g| type_name == *g) {
        Some(2)
    } else {
        None
    }
}

/// Analyze statements for generic type usage.
fn analyze_statements(
    statements: &[Statement],
    fields: &mut [ClassField],
    field_map: &HashMap<String, usize>,
    code: &Bytecode,
) {
    for stmt in statements {
        analyze_statement(stmt, fields, field_map, code);
    }
}

/// Analyze a single statement for generic type usage.
fn analyze_statement(
    stmt: &Statement,
    fields: &mut [ClassField],
    field_map: &HashMap<String, usize>,
    code: &Bytecode,
) {
    match stmt {
        Statement::Assign { assign, .. } => {
            analyze_expr(assign, fields, field_map, code);
        }
        Statement::ExprStatement(expr) => {
            analyze_expr(expr, fields, field_map, code);
        }
        Statement::Return(Some(expr)) => {
            analyze_expr(expr, fields, field_map, code);
        }
        Statement::IfElse { cond, if_, else_ } => {
            analyze_expr(cond, fields, field_map, code);
            analyze_statements(if_, fields, field_map, code);
            analyze_statements(else_, fields, field_map, code);
        }
        Statement::IfElseChain { branches, else_ } => {
            for (cond, body) in branches {
                analyze_expr(cond, fields, field_map, code);
                analyze_statements(body, fields, field_map, code);
            }
            analyze_statements(else_, fields, field_map, code);
        }
        Statement::While { cond, stmts } => {
            analyze_expr(cond, fields, field_map, code);
            analyze_statements(stmts, fields, field_map, code);
        }
        Statement::Switch { arg, cases, default, .. } => {
            analyze_expr(arg, fields, field_map, code);
            for (_, case_stmts) in cases {
                analyze_statements(case_stmts, fields, field_map, code);
            }
            analyze_statements(default, fields, field_map, code);
        }
        Statement::TryCatch { try_stmts, catch_stmts, .. } => {
            analyze_statements(try_stmts, fields, field_map, code);
            analyze_statements(catch_stmts, fields, field_map, code);
        }
        _ => {}
    }
}

/// Analyze an expression for generic type usage.
fn analyze_expr(
    expr: &Expr,
    fields: &mut [ClassField],
    field_map: &HashMap<String, usize>,
    code: &Bytecode,
) {
    match expr {
        // Method call on a field: field.method(args)
        Expr::Call(call) => {
            // Check if this is a method call on a field: this.field.method(args)
            if let Expr::Field(object, method_name) = &call.fun {
                if let Some(field_name) = get_field_name(object.as_ref()) {
                    if let Some(&field_idx) = field_map.get(field_name.as_str()) {
                        let field = &mut fields[field_idx];
                        if let Some(ref mut inference) = field.inferred_generics {
                            // Analyze based on method name
                            analyze_method_call(method_name.as_ref(), &call.args, inference, code);
                        }
                    }
                }
            }

            // Also analyze the arguments
            for arg in &call.args {
                analyze_expr(arg, fields, field_map, code);
            }
        }
        Expr::Op(op) => {
            // Handle binary and unary operations
            use crate::ast::Operation;
            match op {
                Operation::Add(l, r) | Operation::Sub(l, r) | Operation::Mul(l, r)
                | Operation::Div(l, r) | Operation::Mod(l, r) | Operation::And(l, r)
                | Operation::Or(l, r) | Operation::Xor(l, r) | Operation::Shl(l, r)
                | Operation::Shr(l, r) | Operation::Eq(l, r) | Operation::NotEq(l, r)
                | Operation::Lt(l, r) | Operation::Lte(l, r) | Operation::Gt(l, r)
                | Operation::Gte(l, r) => {
                    analyze_expr(l, fields, field_map, code);
                    analyze_expr(r, fields, field_map, code);
                }
                Operation::Neg(e) | Operation::Not(e)
                | Operation::Incr(e) | Operation::Decr(e) => {
                    analyze_expr(e, fields, field_map, code);
                }
            }
        }
        Expr::Field(object, _) => {
            analyze_expr(object, fields, field_map, code);
        }
        Expr::Array(array, index) => {
            analyze_expr(array, fields, field_map, code);
            analyze_expr(index, fields, field_map, code);
        }
        Expr::Cast(value, _) => {
            analyze_expr(value, fields, field_map, code);
        }
        Expr::IfElse { cond, if_, else_ } => {
            analyze_expr(cond, fields, field_map, code);
            analyze_statements(if_, fields, field_map, code);
            analyze_statements(else_, fields, field_map, code);
        }
        Expr::Constructor(ctor) => {
            for arg in &ctor.args {
                analyze_expr(arg, fields, field_map, code);
            }
        }
        Expr::ArrayLiteral(items) => {
            for item in items {
                analyze_expr(item, fields, field_map, code);
            }
        }
        Expr::Anonymous(_, field_values) => {
            for (_, value) in field_values {
                analyze_expr(value, fields, field_map, code);
            }
        }
        Expr::EnumConstr(_, _, args) => {
            for arg in args {
                analyze_expr(arg, fields, field_map, code);
            }
        }
        Expr::Closure(_, stmts) => {
            analyze_statements(stmts, fields, field_map, code);
        }
        _ => {}
    }
}

/// Try to extract a field name from an expression like `this.fieldName`.
fn get_field_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Field(object, field) => {
            // Check if object is 'this'
            if let Expr::Constant(Constant::This) = object.as_ref() {
                Some(field.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Analyze a method call on a generic container to infer type parameters.
fn analyze_method_call(
    method_name: &str,
    args: &[Expr],
    inference: &mut InferredGenericParams,
    code: &Bytecode,
) {
    match method_name {
        // IntMap/StringMap methods that reveal value type
        "set" => {
            // set(key, value) - value is at index 1 (for IntMap) or could vary
            // For IntMap<V>: set(key: Int, value: V)
            if args.len() >= 2 {
                if let Some(type_name) = infer_type_from_expr(&args[1], code) {
                    inference.observe_type(0, type_name);
                }
            }
        }
        "get" => {
            // get() returns the value type - we'd need return type analysis
            // This is harder to track without more context
        }
        "push" | "add" => {
            // For List<V>: push(value: V)
            if !args.is_empty() {
                if let Some(type_name) = infer_type_from_expr(&args[0], code) {
                    inference.observe_type(0, type_name);
                }
            }
        }
        "iterator" | "keys" | "values" => {
            // These return iterators that reveal the type - would need return analysis
        }
        _ => {}
    }
}

/// Try to infer a type name from an expression.
fn infer_type_from_expr(expr: &Expr, code: &Bytecode) -> Option<String> {
    match expr {
        Expr::Constant(c) => {
            match c {
                Constant::InlineInt(_) | Constant::Int(_) => Some("Int".to_string()),
                Constant::Float(_) => Some("Float".to_string()),
                Constant::String(_) => Some("String".to_string()),
                Constant::Bool(_) => Some("Bool".to_string()),
                Constant::Null => None, // Null doesn't tell us the type
                Constant::This => None,
                Constant::TypeRef(ty) => Some(to_haxe_type(&code[*ty], code).to_string()),
                _ => None,
            }
        }
        Expr::Cast(_, type_str) => {
            Some(type_str.to_string())
        }
        Expr::Constructor(ctor) => {
            Some(to_haxe_type(&code[ctor.ty], code).to_string())
        }
        _ => None,
    }
}
