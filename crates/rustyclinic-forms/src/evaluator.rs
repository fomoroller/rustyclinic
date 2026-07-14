//! Stack-based expression evaluator.
//!
//! Evaluates compiled expressions using a pre-allocated stack.
//! No dynamic allocation during evaluation — the stack is reused.

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate, Utc};
use serde_json::Value;
use thiserror::Error;

use crate::compiler::{CompiledExpression, Instruction};
use crate::expression::{BinaryOperator, FieldProperty, UnaryOperator};

/// Errors that can occur during expression evaluation.
#[derive(Debug, Error)]
pub enum EvalError {
    #[error("stack underflow at instruction {0}")]
    StackUnderflow(usize),

    #[error("division by zero")]
    DivisionByZero,

    #[error("type mismatch: expected {expected}, got {got}")]
    TypeMismatch { expected: &'static str, got: String },

    #[error("unknown function: {0}")]
    UnknownFunction(String),

    #[error("wrong argument count for {name}: expected {expected}, got {got}")]
    WrongArgCount { name: String, expected: u8, got: u8 },

    #[error("invalid date value: {0}")]
    InvalidDate(String),

    #[error("instruction pointer out of bounds: {0}")]
    IpOutOfBounds(usize),
}

/// Context for expression evaluation.
pub struct EvalContext {
    /// Current field values (link_id -> JSON value).
    pub field_values: HashMap<String, Value>,
    /// Current date for `today()`.
    pub today: NaiveDate,
    /// Current datetime for `now()`.
    pub now: chrono::DateTime<Utc>,
}

impl EvalContext {
    /// Create a new evaluation context with the current date/time.
    pub fn new(field_values: HashMap<String, Value>) -> Self {
        let now = Utc::now();
        Self {
            field_values,
            today: now.date_naive(),
            now,
        }
    }
}

/// Evaluate a compiled expression against the given context.
///
/// Returns the final value on the stack, or `Value::Null` if the stack is empty.
pub fn evaluate(compiled: &CompiledExpression, ctx: &EvalContext) -> Result<Value, EvalError> {
    let mut stack: Vec<Value> = Vec::with_capacity(32);
    let mut ip = 0;
    let instructions = &compiled.instructions;

    while ip < instructions.len() {
        match &instructions[ip] {
            Instruction::PushLiteral(val) => {
                stack.push(val.clone());
            }
            Instruction::PushField(link_id, property) => {
                let val = resolve_field(ctx, link_id, *property);
                stack.push(val);
            }
            Instruction::BinaryOp(op) => {
                if stack.len() < 2 {
                    return Err(EvalError::StackUnderflow(ip));
                }
                let right = stack.pop().unwrap_or(Value::Null);
                let left = stack.pop().unwrap_or(Value::Null);
                let result = eval_binary_op(*op, &left, &right)?;
                stack.push(result);
            }
            Instruction::UnaryOp(op) => {
                if stack.is_empty() {
                    return Err(EvalError::StackUnderflow(ip));
                }
                let operand = stack.pop().unwrap_or(Value::Null);
                let result = eval_unary_op(*op, &operand);
                stack.push(result);
            }
            Instruction::Call(name, arg_count) => {
                let count = *arg_count as usize;
                if stack.len() < count {
                    return Err(EvalError::StackUnderflow(ip));
                }
                let start = stack.len() - count;
                let args: Vec<Value> = stack.drain(start..).collect();
                let result = eval_function(name, &args, ctx)?;
                stack.push(result);
            }
            Instruction::JumpIfFalse(target) => {
                if stack.is_empty() {
                    return Err(EvalError::StackUnderflow(ip));
                }
                let cond = stack.pop().unwrap_or(Value::Null);
                if !is_truthy(&cond) {
                    if *target > instructions.len() {
                        return Err(EvalError::IpOutOfBounds(*target));
                    }
                    ip = *target;
                    continue;
                }
            }
            Instruction::Jump(target) => {
                if *target > instructions.len() {
                    return Err(EvalError::IpOutOfBounds(*target));
                }
                ip = *target;
                continue;
            }
        }
        ip += 1;
    }

    Ok(stack.pop().unwrap_or(Value::Null))
}

fn resolve_field(ctx: &EvalContext, link_id: &str, property: FieldProperty) -> Value {
    match property {
        FieldProperty::Value => ctx
            .field_values
            .get(link_id)
            .cloned()
            .unwrap_or(Value::Null),
        FieldProperty::Exists => {
            Value::Bool(ctx.field_values.get(link_id).is_some_and(|v| !v.is_null()))
        }
        FieldProperty::Count => {
            let val = ctx.field_values.get(link_id);
            match val {
                Some(Value::Array(arr)) => Value::Number(serde_json::Number::from(arr.len())),
                Some(_) => Value::Number(serde_json::Number::from(1)),
                None => Value::Number(serde_json::Number::from(0)),
            }
        }
        FieldProperty::Length => {
            let val = ctx.field_values.get(link_id);
            match val {
                Some(Value::String(s)) => Value::Number(serde_json::Number::from(s.len())),
                Some(Value::Array(arr)) => Value::Number(serde_json::Number::from(arr.len())),
                _ => Value::Number(serde_json::Number::from(0)),
            }
        }
    }
}

fn to_f64(val: &Value) -> Option<f64> {
    match val {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

fn is_truthy(val: &Value) -> bool {
    match val {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(_) => true,
    }
}

fn values_equal(left: &Value, right: &Value) -> bool {
    // Handle numeric comparison across integer/float
    if let (Some(l), Some(r)) = (to_f64(left), to_f64(right)) {
        return (l - r).abs() < f64::EPSILON;
    }
    left == right
}

fn eval_binary_op(op: BinaryOperator, left: &Value, right: &Value) -> Result<Value, EvalError> {
    match op {
        BinaryOperator::Eq => Ok(Value::Bool(values_equal(left, right))),
        BinaryOperator::Ne => Ok(Value::Bool(!values_equal(left, right))),
        BinaryOperator::And => Ok(Value::Bool(is_truthy(left) && is_truthy(right))),
        BinaryOperator::Or => Ok(Value::Bool(is_truthy(left) || is_truthy(right))),
        BinaryOperator::Gt | BinaryOperator::Lt | BinaryOperator::Ge | BinaryOperator::Le => {
            let l = to_f64(left);
            let r = to_f64(right);
            match (l, r) {
                (Some(lv), Some(rv)) => {
                    let result = match op {
                        BinaryOperator::Gt => lv > rv,
                        BinaryOperator::Lt => lv < rv,
                        BinaryOperator::Ge => lv >= rv,
                        BinaryOperator::Le => lv <= rv,
                        _ => false,
                    };
                    Ok(Value::Bool(result))
                }
                // String comparison fallback
                _ => {
                    if let (Value::String(ls), Value::String(rs)) = (left, right) {
                        let result = match op {
                            BinaryOperator::Gt => ls > rs,
                            BinaryOperator::Lt => ls < rs,
                            BinaryOperator::Ge => ls >= rs,
                            BinaryOperator::Le => ls <= rs,
                            _ => false,
                        };
                        Ok(Value::Bool(result))
                    } else {
                        // Null or incomparable => false
                        Ok(Value::Bool(false))
                    }
                }
            }
        }
        BinaryOperator::Add | BinaryOperator::Sub | BinaryOperator::Mul | BinaryOperator::Div => {
            let l = to_f64(left).unwrap_or(0.0);
            let r = to_f64(right).unwrap_or(0.0);
            let result = match op {
                BinaryOperator::Add => l + r,
                BinaryOperator::Sub => l - r,
                BinaryOperator::Mul => l * r,
                BinaryOperator::Div => {
                    if r == 0.0 {
                        return Err(EvalError::DivisionByZero);
                    }
                    l / r
                }
                _ => 0.0,
            };
            Ok(serde_json::json!(result))
        }
    }
}

fn eval_unary_op(op: UnaryOperator, operand: &Value) -> Value {
    match op {
        UnaryOperator::Not => Value::Bool(!is_truthy(operand)),
        UnaryOperator::Exists => Value::Bool(!operand.is_null()),
        UnaryOperator::Empty => {
            let empty = match operand {
                Value::Null => true,
                Value::String(s) => s.is_empty(),
                Value::Array(a) => a.is_empty(),
                _ => false,
            };
            Value::Bool(empty)
        }
    }
}

fn parse_date(val: &Value) -> Result<NaiveDate, EvalError> {
    match val {
        Value::String(s) => {
            NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| EvalError::InvalidDate(s.clone()))
        }
        _ => Err(EvalError::InvalidDate(format!("{val}"))),
    }
}

fn eval_function(name: &str, args: &[Value], ctx: &EvalContext) -> Result<Value, EvalError> {
    match name {
        "today" => {
            check_arg_count(name, args, 0)?;
            Ok(Value::String(ctx.today.format("%Y-%m-%d").to_string()))
        }
        "now" => {
            check_arg_count(name, args, 0)?;
            Ok(Value::String(ctx.now.to_rfc3339()))
        }
        "age" => {
            check_arg_count(name, args, 1)?;
            let dob = parse_date(&args[0])?;
            let today = ctx.today;
            let mut age = today.year() - dob.year();
            if (today.month(), today.day()) < (dob.month(), dob.day()) {
                age -= 1;
            }
            Ok(serde_json::json!(age))
        }
        "bmi" => {
            check_arg_count(name, args, 2)?;
            let weight = to_f64(&args[0]).unwrap_or(0.0);
            let height_cm = to_f64(&args[1]).unwrap_or(0.0);
            if height_cm <= 0.0 {
                return Ok(Value::Null);
            }
            let height_m = height_cm / 100.0;
            let bmi = weight / (height_m * height_m);
            // Round to 1 decimal place
            let rounded = (bmi * 10.0).round() / 10.0;
            Ok(serde_json::json!(rounded))
        }
        "gestational_age" => {
            check_arg_count(name, args, 1)?;
            let lmp = parse_date(&args[0])?;
            let days = (ctx.today - lmp).num_days();
            if days < 0 {
                return Ok(Value::Null);
            }
            let weeks = days / 7;
            Ok(serde_json::json!(weeks))
        }
        "days_between" => {
            check_arg_count(name, args, 2)?;
            let d1 = parse_date(&args[0])?;
            let d2 = parse_date(&args[1])?;
            let days = (d2 - d1).num_days();
            Ok(serde_json::json!(days))
        }
        "sum" => {
            // Sum all numeric arguments (or array elements if a single array is passed)
            let mut total = 0.0;
            for arg in args {
                match arg {
                    Value::Array(arr) => {
                        for item in arr {
                            total += to_f64(item).unwrap_or(0.0);
                        }
                    }
                    other => {
                        total += to_f64(other).unwrap_or(0.0);
                    }
                }
            }
            Ok(serde_json::json!(total))
        }
        "count" => {
            // Count elements: if array, return length; otherwise count non-null args
            if args.len() == 1 {
                match &args[0] {
                    Value::Array(arr) => return Ok(serde_json::json!(arr.len())),
                    Value::Null => return Ok(serde_json::json!(0)),
                    _ => return Ok(serde_json::json!(1)),
                }
            }
            let count = args.iter().filter(|a| !a.is_null()).count();
            Ok(serde_json::json!(count))
        }
        "contains" => {
            // contains(coding_field, system, code)
            // Simplified: check if field value equals code, or if array contains code
            if args.len() < 2 {
                return Err(EvalError::WrongArgCount {
                    name: name.to_string(),
                    expected: 2,
                    got: args.len() as u8,
                });
            }
            let field_val = &args[0];
            let search = if args.len() == 3 { &args[2] } else { &args[1] };
            let found = match field_val {
                Value::Array(arr) => arr.iter().any(|item| {
                    // Check if item is an object with a "code" field
                    if let Value::Object(obj) = item {
                        obj.get("code").is_some_and(|c| c == search)
                    } else {
                        item == search
                    }
                }),
                Value::String(s) => {
                    if let Value::String(needle) = search {
                        s.contains(needle.as_str())
                    } else {
                        false
                    }
                }
                _ => field_val == search,
            };
            Ok(Value::Bool(found))
        }
        "lookup" => {
            // Placeholder: lookup(table, key) — returns null for now
            // Full implementation requires package lookup tables
            Ok(Value::Null)
        }
        _ => Err(EvalError::UnknownFunction(name.to_string())),
    }
}

fn check_arg_count(name: &str, args: &[Value], expected: u8) -> Result<(), EvalError> {
    if args.len() != expected as usize {
        return Err(EvalError::WrongArgCount {
            name: name.to_string(),
            expected,
            got: args.len() as u8,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile;
    use crate::expression::{DataType, Expression};

    fn make_ctx(fields: Vec<(&str, Value)>) -> EvalContext {
        let map = fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        EvalContext {
            field_values: map,
            today: NaiveDate::from_ymd_opt(2026, 3, 21).expect("valid date"),
            now: chrono::DateTime::parse_from_rfc3339("2026-03-21T12:00:00Z")
                .expect("valid datetime")
                .with_timezone(&Utc),
        }
    }

    #[test]
    fn eval_literal() {
        let expr = Expression::Literal {
            value: serde_json::json!(42),
            data_type: DataType::Integer,
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, serde_json::json!(42));
    }

    #[test]
    fn eval_field_ref() {
        let expr = Expression::Field {
            link_id: "weight".to_string(),
            property: FieldProperty::Value,
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![("weight", serde_json::json!(70))]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, serde_json::json!(70));
    }

    #[test]
    fn eval_missing_field_returns_null() {
        let expr = Expression::Field {
            link_id: "nonexistent".to_string(),
            property: FieldProperty::Value,
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn eval_comparison() {
        let expr = Expression::Op {
            op: BinaryOperator::Gt,
            left: Box::new(Expression::Field {
                link_id: "weight".to_string(),
                property: FieldProperty::Value,
            }),
            right: Box::new(Expression::Literal {
                value: serde_json::json!(0),
                data_type: DataType::Decimal,
            }),
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![("weight", serde_json::json!(70))]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn eval_and_or() {
        let expr = Expression::Op {
            op: BinaryOperator::And,
            left: Box::new(Expression::Literal {
                value: serde_json::json!(true),
                data_type: DataType::Boolean,
            }),
            right: Box::new(Expression::Literal {
                value: serde_json::json!(false),
                data_type: DataType::Boolean,
            }),
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn eval_division_by_zero() {
        let expr = Expression::Op {
            op: BinaryOperator::Div,
            left: Box::new(Expression::Literal {
                value: serde_json::json!(10),
                data_type: DataType::Integer,
            }),
            right: Box::new(Expression::Literal {
                value: serde_json::json!(0),
                data_type: DataType::Integer,
            }),
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn eval_age_function() {
        let expr = Expression::FunctionCall {
            name: "age".to_string(),
            args: vec![Expression::Literal {
                value: serde_json::json!("2000-06-15"),
                data_type: DataType::Date,
            }],
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        // Born 2000-06-15, today is 2026-03-21 => 25 years old
        assert_eq!(result, serde_json::json!(25));
    }

    #[test]
    fn eval_bmi_function() {
        let expr = Expression::FunctionCall {
            name: "bmi".to_string(),
            args: vec![
                Expression::Literal {
                    value: serde_json::json!(70),
                    data_type: DataType::Decimal,
                },
                Expression::Literal {
                    value: serde_json::json!(175),
                    data_type: DataType::Decimal,
                },
            ],
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        // BMI = 70 / (1.75 * 1.75) = 70 / 3.0625 = 22.857... => 22.9
        assert_eq!(result, serde_json::json!(22.9));
    }

    #[test]
    fn eval_gestational_age() {
        let expr = Expression::FunctionCall {
            name: "gestational_age".to_string(),
            args: vec![Expression::Literal {
                value: serde_json::json!("2025-12-01"),
                data_type: DataType::Date,
            }],
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        // 2025-12-01 to 2026-03-21 = 110 days = 15 weeks
        assert_eq!(result, serde_json::json!(15));
    }

    #[test]
    fn eval_days_between() {
        let expr = Expression::FunctionCall {
            name: "days_between".to_string(),
            args: vec![
                Expression::Literal {
                    value: serde_json::json!("2026-03-01"),
                    data_type: DataType::Date,
                },
                Expression::Literal {
                    value: serde_json::json!("2026-03-21"),
                    data_type: DataType::Date,
                },
            ],
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, serde_json::json!(20));
    }

    #[test]
    fn eval_today() {
        let expr = Expression::FunctionCall {
            name: "today".to_string(),
            args: vec![],
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, Value::String("2026-03-21".to_string()));
    }

    #[test]
    fn eval_sum_function() {
        let expr = Expression::FunctionCall {
            name: "sum".to_string(),
            args: vec![Expression::Field {
                link_id: "quantities".to_string(),
                property: FieldProperty::Value,
            }],
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![("quantities", serde_json::json!([10, 20, 30]))]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, serde_json::json!(60.0));
    }

    #[test]
    fn eval_count_function() {
        let expr = Expression::FunctionCall {
            name: "count".to_string(),
            args: vec![Expression::Field {
                link_id: "items".to_string(),
                property: FieldProperty::Value,
            }],
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![("items", serde_json::json!([1, 2, 3]))]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, serde_json::json!(3));
    }

    #[test]
    fn eval_conditional() {
        let expr = Expression::Conditional {
            condition: Box::new(Expression::Literal {
                value: serde_json::json!(true),
                data_type: DataType::Boolean,
            }),
            then: Box::new(Expression::Literal {
                value: serde_json::json!("yes"),
                data_type: DataType::String,
            }),
            else_branch: Box::new(Expression::Literal {
                value: serde_json::json!("no"),
                data_type: DataType::String,
            }),
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, Value::String("yes".to_string()));
    }

    #[test]
    fn eval_conditional_false() {
        let expr = Expression::Conditional {
            condition: Box::new(Expression::Literal {
                value: serde_json::json!(false),
                data_type: DataType::Boolean,
            }),
            then: Box::new(Expression::Literal {
                value: serde_json::json!("yes"),
                data_type: DataType::String,
            }),
            else_branch: Box::new(Expression::Literal {
                value: serde_json::json!("no"),
                data_type: DataType::String,
            }),
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, Value::String("no".to_string()));
    }

    #[test]
    fn eval_not_operator() {
        let expr = Expression::Not {
            operand: Box::new(Expression::Literal {
                value: serde_json::json!(true),
                data_type: DataType::Boolean,
            }),
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx).expect("should evaluate");
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn eval_exists_operator() {
        let expr = Expression::Exists {
            operand: Box::new(Expression::Field {
                link_id: "weight".to_string(),
                property: FieldProperty::Value,
            }),
        };
        let compiled = compile(&expr);

        let ctx_with = make_ctx(vec![("weight", serde_json::json!(70))]);
        let result = evaluate(&compiled, &ctx_with).expect("should evaluate");
        assert_eq!(result, Value::Bool(true));

        let ctx_without = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx_without).expect("should evaluate");
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn eval_unknown_function() {
        let expr = Expression::FunctionCall {
            name: "foobar".to_string(),
            args: vec![],
        };
        let compiled = compile(&expr);
        let ctx = make_ctx(vec![]);
        let result = evaluate(&compiled, &ctx);
        assert!(result.is_err());
    }
}
