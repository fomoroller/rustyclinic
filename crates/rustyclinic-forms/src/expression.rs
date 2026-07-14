//! Expression tree types for form skip logic, computed fields, and validation.

use serde::{Deserialize, Serialize};

/// An expression node in the JSON rule tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Expression {
    /// A literal value.
    Literal {
        value: serde_json::Value,
        #[serde(rename = "dataType")]
        data_type: DataType,
    },
    /// A reference to another field's value or property.
    Field {
        #[serde(rename = "linkId")]
        link_id: String,
        #[serde(default)]
        property: FieldProperty,
    },
    /// A binary operation (e.g., eq, add, and).
    Op {
        op: BinaryOperator,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    /// A unary operation (not, exists, empty).
    #[serde(rename = "not")]
    Not { operand: Box<Expression> },
    /// Unary exists check.
    #[serde(rename = "exists")]
    Exists { operand: Box<Expression> },
    /// Unary empty check.
    #[serde(rename = "empty")]
    Empty { operand: Box<Expression> },
    /// A function call with arguments.
    #[serde(rename = "fn")]
    FunctionCall { name: String, args: Vec<Expression> },
    /// A conditional (if/then/else) expression.
    #[serde(rename = "if")]
    Conditional {
        condition: Box<Expression>,
        then: Box<Expression>,
        #[serde(rename = "else")]
        else_branch: Box<Expression>,
    },
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryOperator {
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
    And,
    Or,
    Add,
    Sub,
    Mul,
    Div,
}

/// Unary operators (used in compiler/evaluator, not directly in serde).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnaryOperator {
    Not,
    Exists,
    Empty,
}

/// Which property of a field to reference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldProperty {
    #[default]
    Value,
    Exists,
    Count,
    Length,
}

/// Data types for literal values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataType {
    String,
    Integer,
    Decimal,
    Boolean,
    Date,
    Coding,
}

impl Expression {
    /// Collect all field link_ids referenced by this expression.
    pub fn field_references(&self) -> Vec<String> {
        let mut refs = Vec::new();
        self.collect_field_refs(&mut refs);
        refs
    }

    fn collect_field_refs(&self, refs: &mut Vec<String>) {
        match self {
            Expression::Literal { .. } => {}
            Expression::Field { link_id, .. } => {
                if !refs.contains(link_id) {
                    refs.push(link_id.clone());
                }
            }
            Expression::Op { left, right, .. } => {
                left.collect_field_refs(refs);
                right.collect_field_refs(refs);
            }
            Expression::Not { operand }
            | Expression::Exists { operand }
            | Expression::Empty { operand } => {
                operand.collect_field_refs(refs);
            }
            Expression::FunctionCall { args, .. } => {
                for arg in args {
                    arg.collect_field_refs(refs);
                }
            }
            Expression::Conditional {
                condition,
                then,
                else_branch,
            } => {
                condition.collect_field_refs(refs);
                then.collect_field_refs(refs);
                else_branch.collect_field_refs(refs);
            }
        }
    }
}
