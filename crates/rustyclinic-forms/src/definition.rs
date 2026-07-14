//! Form definition types — the schema for clinical forms.

use serde::{Deserialize, Serialize};

use crate::expression::Expression;

/// A complete form definition, loaded from a JSON package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormDefinition {
    /// Unique identifier for this form family (e.g. "anc-visit").
    pub id: String,
    /// Semantic version string (e.g. "1.2.0").
    pub version: String,
    /// Human-readable title.
    pub title: String,
    /// Top-level items (fields and groups).
    pub items: Vec<FormItem>,
    /// FHIR mapping rules (placeholder for future implementation).
    #[serde(default)]
    pub mappings: Vec<MappingRule>,
}

/// A single item in a form (field or group).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormItem {
    /// Unique identifier within the form.
    #[serde(rename = "linkId")]
    pub link_id: String,
    /// The data type / widget type of this item.
    #[serde(rename = "type")]
    pub item_type: ItemType,
    /// Display text / label.
    pub text: String,
    /// Hint text displayed below the label (e.g. units, instructions).
    #[serde(default)]
    pub hint: Option<String>,
    /// Whether this field must be filled for submission.
    #[serde(default)]
    pub required: bool,
    /// Whether this field is computed / read-only.
    #[serde(default, rename = "readOnly")]
    pub read_only: bool,
    /// Conditional visibility expression.
    #[serde(default, rename = "enableWhen")]
    pub enable_when: Option<Expression>,
    /// Expression that computes this field's value.
    #[serde(default, rename = "computedValue")]
    pub computed_value: Option<Expression>,
    /// Validation rules for this field.
    #[serde(default)]
    pub validation: Vec<ValidationRule>,
    /// Nested items (for groups).
    #[serde(default)]
    pub items: Vec<FormItem>,
}

/// The type of a form item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemType {
    String,
    Integer,
    Decimal,
    Boolean,
    Date,
    DateTime,
    Choice { options: Vec<ChoiceOption> },
    Group,
}

/// A single option in a choice field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
}

/// A validation rule attached to a form item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRule {
    /// Expression that must evaluate to true for the field to be valid.
    pub expression: Expression,
    /// Severity of the validation failure.
    pub severity: Severity,
    /// Human-readable error message.
    pub message: String,
}

/// Severity levels for validation results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Blocks submission.
    Error,
    /// Shows warning but allows submission.
    Warning,
    /// Advisory only.
    Info,
}

/// Placeholder for FHIR mapping rules (future implementation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingRule {
    #[serde(rename = "type")]
    pub rule_type: String,
    #[serde(flatten)]
    pub fields: serde_json::Value,
}
