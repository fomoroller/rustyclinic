//! Expression compiler: JSON expression tree -> flat instruction array.
//!
//! The compiled form uses stack-based evaluation with no dynamic memory
//! allocation during evaluation (pre-allocated stack).

use serde::{Deserialize, Serialize};

use crate::expression::{BinaryOperator, Expression, FieldProperty, UnaryOperator};

/// A single instruction in the compiled expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Instruction {
    /// Push a literal value onto the stack.
    PushLiteral(serde_json::Value),
    /// Push a field's value (or property) onto the stack.
    PushField(String, FieldProperty),
    /// Pop two values, apply binary op, push result.
    BinaryOp(BinaryOperator),
    /// Pop one value, apply unary op, push result.
    UnaryOp(UnaryOperator),
    /// Pop `arg_count` values, call function, push result.
    Call(String, u8),
    /// Pop top of stack; if falsy, jump to instruction index.
    JumpIfFalse(usize),
    /// Unconditional jump to instruction index.
    Jump(usize),
}

/// A compiled expression ready for stack-based evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompiledExpression {
    pub instructions: Vec<Instruction>,
}

/// Compile an expression tree into a flat instruction array.
pub fn compile(expr: &Expression) -> CompiledExpression {
    let mut instructions = Vec::new();
    emit(expr, &mut instructions);
    CompiledExpression { instructions }
}

fn emit(expr: &Expression, out: &mut Vec<Instruction>) {
    match expr {
        Expression::Literal { value, .. } => {
            out.push(Instruction::PushLiteral(value.clone()));
        }
        Expression::Field { link_id, property } => {
            out.push(Instruction::PushField(link_id.clone(), *property));
        }
        Expression::Op { op, left, right } => {
            emit(left, out);
            emit(right, out);
            out.push(Instruction::BinaryOp(*op));
        }
        Expression::Not { operand } => {
            emit(operand, out);
            out.push(Instruction::UnaryOp(UnaryOperator::Not));
        }
        Expression::Exists { operand } => {
            emit(operand, out);
            out.push(Instruction::UnaryOp(UnaryOperator::Exists));
        }
        Expression::Empty { operand } => {
            emit(operand, out);
            out.push(Instruction::UnaryOp(UnaryOperator::Empty));
        }
        Expression::FunctionCall { name, args } => {
            for arg in args {
                emit(arg, out);
            }
            let arg_count = if args.len() > 255 {
                255
            } else {
                args.len() as u8
            };
            out.push(Instruction::Call(name.clone(), arg_count));
        }
        Expression::Conditional {
            condition,
            then,
            else_branch,
        } => {
            // Emit: condition, JumpIfFalse(else), then, Jump(end), else, ...
            emit(condition, out);
            let jump_if_false_idx = out.len();
            out.push(Instruction::JumpIfFalse(0)); // placeholder

            emit(then, out);
            let jump_end_idx = out.len();
            out.push(Instruction::Jump(0)); // placeholder

            let else_start = out.len();
            emit(else_branch, out);
            let end = out.len();

            // Patch jump targets
            out[jump_if_false_idx] = Instruction::JumpIfFalse(else_start);
            out[jump_end_idx] = Instruction::Jump(end);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expression::{DataType, FieldProperty};

    #[test]
    fn compile_literal() {
        let expr = Expression::Literal {
            value: serde_json::json!(42),
            data_type: DataType::Integer,
        };
        let compiled = compile(&expr);
        assert_eq!(compiled.instructions.len(), 1);
        assert_eq!(
            compiled.instructions[0],
            Instruction::PushLiteral(serde_json::json!(42))
        );
    }

    #[test]
    fn compile_binary_op() {
        let expr = Expression::Op {
            op: BinaryOperator::Add,
            left: Box::new(Expression::Literal {
                value: serde_json::json!(1),
                data_type: DataType::Integer,
            }),
            right: Box::new(Expression::Literal {
                value: serde_json::json!(2),
                data_type: DataType::Integer,
            }),
        };
        let compiled = compile(&expr);
        assert_eq!(compiled.instructions.len(), 3);
        assert_eq!(
            compiled.instructions[2],
            Instruction::BinaryOp(BinaryOperator::Add)
        );
    }

    #[test]
    fn compile_conditional() {
        let expr = Expression::Conditional {
            condition: Box::new(Expression::Literal {
                value: serde_json::json!(true),
                data_type: DataType::Boolean,
            }),
            then: Box::new(Expression::Literal {
                value: serde_json::json!(1),
                data_type: DataType::Integer,
            }),
            else_branch: Box::new(Expression::Literal {
                value: serde_json::json!(2),
                data_type: DataType::Integer,
            }),
        };
        let compiled = compile(&expr);
        // condition(1) + JumpIfFalse(1) + then(1) + Jump(1) + else(1) = 5
        assert_eq!(compiled.instructions.len(), 5);
        assert_eq!(compiled.instructions[1], Instruction::JumpIfFalse(4));
        assert_eq!(compiled.instructions[3], Instruction::Jump(5));
    }

    #[test]
    fn compile_field_ref() {
        let expr = Expression::Field {
            link_id: "weight_kg".to_string(),
            property: FieldProperty::Value,
        };
        let compiled = compile(&expr);
        assert_eq!(compiled.instructions.len(), 1);
        assert_eq!(
            compiled.instructions[0],
            Instruction::PushField("weight_kg".to_string(), FieldProperty::Value)
        );
    }

    #[test]
    fn compile_function_call() {
        let expr = Expression::FunctionCall {
            name: "bmi".to_string(),
            args: vec![
                Expression::Field {
                    link_id: "weight".to_string(),
                    property: FieldProperty::Value,
                },
                Expression::Field {
                    link_id: "height".to_string(),
                    property: FieldProperty::Value,
                },
            ],
        };
        let compiled = compile(&expr);
        assert_eq!(compiled.instructions.len(), 3);
        assert_eq!(
            compiled.instructions[2],
            Instruction::Call("bmi".to_string(), 2)
        );
    }
}
