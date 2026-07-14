//! Form engine — the main orchestrator for form evaluation.
//!
//! Compiles all expressions at form load time, builds the dependency DAG,
//! and provides evaluation and incremental re-evaluation on field changes.

use std::collections::HashMap;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::compiler::{self, CompiledExpression};
use crate::dag::{DagError, DependencyDag};
use crate::definition::{FormDefinition, FormItem, Severity};
use crate::evaluator::{self, EvalContext, EvalError};

/// Errors that can occur when building or using the form engine.
#[derive(Debug, Error)]
pub enum FormError {
    #[error("dependency graph error: {0}")]
    Dag(#[from] DagError),

    #[error("evaluation error in field '{field}': {source}")]
    Evaluation { field: String, source: EvalError },
}

/// A validation result for a specific field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// The field this validation applies to.
    pub link_id: String,
    /// Severity level.
    pub severity: Severity,
    /// Human-readable message.
    pub message: String,
}

/// The renderer-neutral output of form evaluation.
///
/// Consumed by web/mobile/TUI renderers to display the form state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormEvaluationState {
    /// Current field values.
    pub field_values: HashMap<String, Value>,
    /// Visibility state per field (true = visible).
    pub visibility: HashMap<String, bool>,
    /// Computed field values.
    pub computed_values: HashMap<String, Value>,
    /// All validation results (errors, warnings, info).
    pub validation_results: Vec<ValidationResult>,
    /// Whether the form can be submitted (no error-severity validation failures on visible required fields).
    pub is_submittable: bool,
    /// Whether the form has been modified since last save.
    pub dirty: bool,
}

/// Compiled validation entry.
struct CompiledValidation {
    compiled: CompiledExpression,
    severity: Severity,
    message: String,
}

/// The form engine: compiles expressions and evaluates form state.
pub struct FormEngine {
    /// The form definition.
    pub definition: FormDefinition,
    /// The dependency DAG.
    dag: DependencyDag,
    /// Compiled enable_when expressions, keyed by link_id.
    compiled_enable_when: HashMap<String, CompiledExpression>,
    /// Compiled computed_value expressions, keyed by link_id.
    compiled_computed: HashMap<String, CompiledExpression>,
    /// Compiled validation rules, keyed by link_id.
    compiled_validation: HashMap<String, Vec<CompiledValidation>>,
    /// All link_ids that have computed values, in topological order.
    computed_order: Vec<String>,
    /// Required fields (link_id -> required flag).
    required_fields: HashMap<String, bool>,
}

impl FormEngine {
    /// Create a new form engine from a form definition.
    ///
    /// Compiles all expressions and builds the dependency DAG.
    /// Returns an error if a circular dependency is detected.
    pub fn new(definition: FormDefinition) -> Result<Self, FormError> {
        let dag = DependencyDag::build(&definition.items)?;

        let mut compiled_enable_when = HashMap::new();
        let mut compiled_computed = HashMap::new();
        let mut compiled_validation: HashMap<String, Vec<CompiledValidation>> = HashMap::new();
        let mut required_fields = HashMap::new();
        let mut computed_field_ids = Vec::new();

        Self::compile_items(
            &definition.items,
            &mut compiled_enable_when,
            &mut compiled_computed,
            &mut compiled_validation,
            &mut required_fields,
            &mut computed_field_ids,
        );

        let computed_order = dag.evaluation_order(&computed_field_ids);

        Ok(Self {
            definition,
            dag,
            compiled_enable_when,
            compiled_computed,
            compiled_validation,
            computed_order,
            required_fields,
        })
    }

    fn compile_items(
        items: &[FormItem],
        enable_when: &mut HashMap<String, CompiledExpression>,
        computed: &mut HashMap<String, CompiledExpression>,
        validation: &mut HashMap<String, Vec<CompiledValidation>>,
        required: &mut HashMap<String, bool>,
        computed_ids: &mut Vec<String>,
    ) {
        for item in items {
            required.insert(item.link_id.clone(), item.required);

            if let Some(expr) = &item.enable_when {
                enable_when.insert(item.link_id.clone(), compiler::compile(expr));
            }

            if let Some(expr) = &item.computed_value {
                computed.insert(item.link_id.clone(), compiler::compile(expr));
                computed_ids.push(item.link_id.clone());
            }

            if !item.validation.is_empty() {
                let compiled_rules: Vec<CompiledValidation> = item
                    .validation
                    .iter()
                    .map(|rule| CompiledValidation {
                        compiled: compiler::compile(&rule.expression),
                        severity: rule.severity,
                        message: rule.message.clone(),
                    })
                    .collect();
                validation.insert(item.link_id.clone(), compiled_rules);
            }

            // Recurse into nested items
            if !item.items.is_empty() {
                Self::compile_items(
                    &item.items,
                    enable_when,
                    computed,
                    validation,
                    required,
                    computed_ids,
                );
            }
        }
    }

    /// Evaluate the entire form given current field values.
    ///
    /// Returns the complete evaluation state including visibility, computed
    /// values, and validation results.
    pub fn evaluate(&self, field_values: &HashMap<String, Value>) -> FormEvaluationState {
        let mut state = FormEvaluationState {
            field_values: field_values.clone(),
            visibility: HashMap::new(),
            computed_values: HashMap::new(),
            validation_results: Vec::new(),
            is_submittable: true,
            dirty: false,
        };

        self.evaluate_all(&mut state);
        state
    }

    /// Handle a field value change with minimal re-evaluation.
    ///
    /// Updates the field value and only re-evaluates expressions affected
    /// by the changed field.
    pub fn on_field_change(
        &self,
        field_id: &str,
        new_value: Value,
        state: &mut FormEvaluationState,
    ) {
        state.field_values.insert(field_id.to_string(), new_value);
        state.dirty = true;

        let affected = self.dag.affected_by(field_id);

        let ctx = self.make_context(&state.field_values);

        // Re-evaluate computed values for affected fields (in topological order)
        for field in &self.computed_order {
            if (affected.contains(field) || field == field_id)
                && let Some(compiled) = self.compiled_computed.get(field)
            {
                match evaluator::evaluate(compiled, &ctx) {
                    Ok(val) => {
                        state.computed_values.insert(field.clone(), val.clone());
                        state.field_values.insert(field.clone(), val);
                    }
                    Err(_) => {
                        state.computed_values.insert(field.clone(), Value::Null);
                    }
                }
            }
        }

        // Rebuild context with updated computed values
        let ctx = self.make_context(&state.field_values);

        // Re-evaluate visibility for the changed field and affected fields
        let mut fields_to_check: Vec<&str> = vec![field_id];
        for f in &affected {
            fields_to_check.push(f.as_str());
        }

        for field in &fields_to_check {
            if let Some(compiled) = self.compiled_enable_when.get(*field) {
                let visible = evaluator::evaluate(compiled, &ctx)
                    .map(|v| is_truthy(&v))
                    .unwrap_or(true);
                state.visibility.insert((*field).to_string(), visible);
            }
        }

        // Re-run all validation (simpler than tracking which validations are affected)
        self.evaluate_validation(&ctx, &state.visibility, &mut state.validation_results);
        state.is_submittable = self.check_submittable(&state.validation_results, &state.visibility);
    }

    fn evaluate_all(&self, state: &mut FormEvaluationState) {
        // 1. Evaluate computed values in topological order
        let ctx = self.make_context(&state.field_values);
        for field in &self.computed_order {
            if let Some(compiled) = self.compiled_computed.get(field) {
                match evaluator::evaluate(compiled, &ctx) {
                    Ok(val) => {
                        state.computed_values.insert(field.clone(), val.clone());
                        state.field_values.insert(field.clone(), val);
                    }
                    Err(_) => {
                        state.computed_values.insert(field.clone(), Value::Null);
                    }
                }
            }
        }

        // Rebuild context with computed values
        let ctx = self.make_context(&state.field_values);

        // 2. Evaluate visibility
        for (field, compiled) in &self.compiled_enable_when {
            let visible = evaluator::evaluate(compiled, &ctx)
                .map(|v| is_truthy(&v))
                .unwrap_or(true);
            state.visibility.insert(field.clone(), visible);
        }

        // Default: fields without enable_when are visible
        self.set_default_visibility(&self.definition.items, &mut state.visibility);

        // 3. Evaluate validation
        self.evaluate_validation(&ctx, &state.visibility, &mut state.validation_results);

        // 4. Check submittability
        state.is_submittable = self.check_submittable(&state.validation_results, &state.visibility);
    }

    fn set_default_visibility(&self, items: &[FormItem], visibility: &mut HashMap<String, bool>) {
        for item in items {
            visibility.entry(item.link_id.clone()).or_insert(true);
            if !item.items.is_empty() {
                self.set_default_visibility(&item.items, visibility);
            }
        }
    }

    fn evaluate_validation(
        &self,
        ctx: &EvalContext,
        visibility: &HashMap<String, bool>,
        results: &mut Vec<ValidationResult>,
    ) {
        results.clear();

        for (field, rules) in &self.compiled_validation {
            // Skip validation for hidden fields
            let visible = visibility.get(field).copied().unwrap_or(true);
            if !visible {
                continue;
            }

            for rule in rules {
                let passes = evaluator::evaluate(&rule.compiled, ctx)
                    .map(|v| is_truthy(&v))
                    .unwrap_or(false);

                if !passes {
                    results.push(ValidationResult {
                        link_id: field.clone(),
                        severity: rule.severity,
                        message: rule.message.clone(),
                    });
                }
            }
        }

        // Check required fields
        for (field, required) in &self.required_fields {
            if !required {
                continue;
            }
            let visible = visibility.get(field).copied().unwrap_or(true);
            if !visible {
                continue;
            }
            let has_value = ctx
                .field_values
                .get(field)
                .is_some_and(|v| !v.is_null() && *v != Value::String(String::new()));
            if !has_value {
                results.push(ValidationResult {
                    link_id: field.clone(),
                    severity: Severity::Error,
                    message: format!("Field '{}' is required", field),
                });
            }
        }
    }

    fn check_submittable(
        &self,
        results: &[ValidationResult],
        visibility: &HashMap<String, bool>,
    ) -> bool {
        !results.iter().any(|r| {
            r.severity == Severity::Error && visibility.get(&r.link_id).copied().unwrap_or(true)
        })
    }

    fn make_context(&self, field_values: &HashMap<String, Value>) -> EvalContext {
        let now = Utc::now();
        EvalContext {
            field_values: field_values.clone(),
            today: now.date_naive(),
            now,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::{ChoiceOption, FormItem, ItemType, Severity, ValidationRule};
    use crate::expression::{BinaryOperator, DataType, Expression, FieldProperty};

    fn simple_registration_form() -> FormDefinition {
        FormDefinition {
            id: "patient-registration".to_string(),
            version: "1.0.0".to_string(),
            title: "Patient Registration".to_string(),
            items: vec![
                FormItem {
                    link_id: "first_name".to_string(),
                    item_type: ItemType::String,
                    text: "First Name".to_string(),
                    hint: None,
                    required: true,
                    read_only: false,
                    enable_when: None,
                    computed_value: None,
                    validation: vec![],
                    items: vec![],
                },
                FormItem {
                    link_id: "last_name".to_string(),
                    item_type: ItemType::String,
                    text: "Last Name".to_string(),
                    hint: None,
                    required: true,
                    read_only: false,
                    enable_when: None,
                    computed_value: None,
                    validation: vec![],
                    items: vec![],
                },
                FormItem {
                    link_id: "sex".to_string(),
                    item_type: ItemType::Choice {
                        options: vec![
                            ChoiceOption {
                                value: "female".to_string(),
                                label: "Female".to_string(),
                            },
                            ChoiceOption {
                                value: "male".to_string(),
                                label: "Male".to_string(),
                            },
                        ],
                    },
                    text: "Sex".to_string(),
                    hint: None,
                    required: true,
                    read_only: false,
                    enable_when: None,
                    computed_value: None,
                    validation: vec![],
                    items: vec![],
                },
                FormItem {
                    link_id: "weight_kg".to_string(),
                    item_type: ItemType::Decimal,
                    text: "Weight (kg)".to_string(),
                    hint: None,
                    required: false,
                    read_only: false,
                    enable_when: None,
                    computed_value: None,
                    validation: vec![
                        ValidationRule {
                            expression: Expression::Op {
                                op: BinaryOperator::Gt,
                                left: Box::new(Expression::Field {
                                    link_id: "weight_kg".to_string(),
                                    property: FieldProperty::Value,
                                }),
                                right: Box::new(Expression::Literal {
                                    value: serde_json::json!(0),
                                    data_type: DataType::Decimal,
                                }),
                            },
                            severity: Severity::Error,
                            message: "Weight must be positive".to_string(),
                        },
                        ValidationRule {
                            expression: Expression::Op {
                                op: BinaryOperator::Lt,
                                left: Box::new(Expression::Field {
                                    link_id: "weight_kg".to_string(),
                                    property: FieldProperty::Value,
                                }),
                                right: Box::new(Expression::Literal {
                                    value: serde_json::json!(300),
                                    data_type: DataType::Decimal,
                                }),
                            },
                            severity: Severity::Warning,
                            message: "Weight seems unusually high".to_string(),
                        },
                    ],
                    items: vec![],
                },
                FormItem {
                    link_id: "height_cm".to_string(),
                    item_type: ItemType::Decimal,
                    text: "Height (cm)".to_string(),
                    hint: None,
                    required: false,
                    read_only: false,
                    enable_when: None,
                    computed_value: None,
                    validation: vec![],
                    items: vec![],
                },
                FormItem {
                    link_id: "bmi_display".to_string(),
                    item_type: ItemType::Decimal,
                    text: "BMI".to_string(),
                    hint: None,
                    required: false,
                    read_only: true,
                    enable_when: None,
                    computed_value: Some(Expression::FunctionCall {
                        name: "bmi".to_string(),
                        args: vec![
                            Expression::Field {
                                link_id: "weight_kg".to_string(),
                                property: FieldProperty::Value,
                            },
                            Expression::Field {
                                link_id: "height_cm".to_string(),
                                property: FieldProperty::Value,
                            },
                        ],
                    }),
                    validation: vec![],
                    items: vec![],
                },
            ],
            mappings: vec![],
        }
    }

    fn anc_visit_form() -> FormDefinition {
        FormDefinition {
            id: "anc-visit".to_string(),
            version: "1.0.0".to_string(),
            title: "ANC Visit".to_string(),
            items: vec![
                FormItem {
                    link_id: "hiv_status".to_string(),
                    item_type: ItemType::Choice {
                        options: vec![
                            ChoiceOption {
                                value: "positive".to_string(),
                                label: "Positive".to_string(),
                            },
                            ChoiceOption {
                                value: "negative".to_string(),
                                label: "Negative".to_string(),
                            },
                            ChoiceOption {
                                value: "unknown".to_string(),
                                label: "Unknown".to_string(),
                            },
                        ],
                    },
                    text: "HIV Status".to_string(),
                    hint: None,
                    required: true,
                    read_only: false,
                    enable_when: None,
                    computed_value: None,
                    validation: vec![],
                    items: vec![],
                },
                FormItem {
                    link_id: "arv_regimen".to_string(),
                    item_type: ItemType::String,
                    text: "Current ARV Regimen".to_string(),
                    hint: None,
                    required: false,
                    read_only: false,
                    enable_when: Some(Expression::Op {
                        op: BinaryOperator::Eq,
                        left: Box::new(Expression::Field {
                            link_id: "hiv_status".to_string(),
                            property: FieldProperty::Value,
                        }),
                        right: Box::new(Expression::Literal {
                            value: serde_json::json!("positive"),
                            data_type: DataType::String,
                        }),
                    }),
                    computed_value: None,
                    validation: vec![],
                    items: vec![],
                },
                FormItem {
                    link_id: "weight_kg".to_string(),
                    item_type: ItemType::Decimal,
                    text: "Weight (kg)".to_string(),
                    hint: None,
                    required: true,
                    read_only: false,
                    enable_when: None,
                    computed_value: None,
                    validation: vec![ValidationRule {
                        expression: Expression::Op {
                            op: BinaryOperator::Gt,
                            left: Box::new(Expression::Field {
                                link_id: "weight_kg".to_string(),
                                property: FieldProperty::Value,
                            }),
                            right: Box::new(Expression::Literal {
                                value: serde_json::json!(0),
                                data_type: DataType::Decimal,
                            }),
                        },
                        severity: Severity::Error,
                        message: "Weight must be positive".to_string(),
                    }],
                    items: vec![],
                },
                FormItem {
                    link_id: "height_cm".to_string(),
                    item_type: ItemType::Decimal,
                    text: "Height (cm)".to_string(),
                    hint: None,
                    required: true,
                    read_only: false,
                    enable_when: None,
                    computed_value: None,
                    validation: vec![],
                    items: vec![],
                },
                FormItem {
                    link_id: "bmi_display".to_string(),
                    item_type: ItemType::Decimal,
                    text: "BMI".to_string(),
                    hint: None,
                    required: false,
                    read_only: true,
                    enable_when: None,
                    computed_value: Some(Expression::FunctionCall {
                        name: "bmi".to_string(),
                        args: vec![
                            Expression::Field {
                                link_id: "weight_kg".to_string(),
                                property: FieldProperty::Value,
                            },
                            Expression::Field {
                                link_id: "height_cm".to_string(),
                                property: FieldProperty::Value,
                            },
                        ],
                    }),
                    validation: vec![],
                    items: vec![],
                },
            ],
            mappings: vec![],
        }
    }

    #[test]
    fn engine_creation() {
        let form = simple_registration_form();
        let engine = FormEngine::new(form);
        assert!(engine.is_ok());
    }

    #[test]
    fn full_evaluation_empty_form() {
        let form = simple_registration_form();
        let engine = FormEngine::new(form).expect("should create engine");
        let state = engine.evaluate(&HashMap::new());

        // Required fields missing => not submittable
        assert!(!state.is_submittable);
        // All fields should be visible (no enable_when on registration form)
        assert_eq!(state.visibility.get("first_name"), Some(&true));
    }

    #[test]
    fn full_evaluation_with_values() {
        let form = simple_registration_form();
        let engine = FormEngine::new(form).expect("should create engine");
        let mut values = HashMap::new();
        values.insert("first_name".to_string(), serde_json::json!("Alice"));
        values.insert("last_name".to_string(), serde_json::json!("Uwimana"));
        values.insert("sex".to_string(), serde_json::json!("female"));
        values.insert("weight_kg".to_string(), serde_json::json!(65));
        values.insert("height_cm".to_string(), serde_json::json!(165));

        let state = engine.evaluate(&values);

        // Should be submittable with all required fields filled
        assert!(state.is_submittable);

        // BMI should be computed
        let bmi = state.computed_values.get("bmi_display");
        assert!(bmi.is_some());
        // 65 / (1.65^2) = 65 / 2.7225 = 23.875... => 23.9
        let bmi_val = bmi.and_then(|v| v.as_f64()).unwrap_or(0.0);
        assert!((bmi_val - 23.9).abs() < 0.1);
    }

    #[test]
    fn skip_logic_hides_field() {
        let form = anc_visit_form();
        let engine = FormEngine::new(form).expect("should create engine");

        // HIV negative => arv_regimen should be hidden
        let mut values = HashMap::new();
        values.insert("hiv_status".to_string(), serde_json::json!("negative"));
        values.insert("weight_kg".to_string(), serde_json::json!(60));
        values.insert("height_cm".to_string(), serde_json::json!(160));

        let state = engine.evaluate(&values);
        assert_eq!(state.visibility.get("arv_regimen"), Some(&false));
    }

    #[test]
    fn skip_logic_shows_field() {
        let form = anc_visit_form();
        let engine = FormEngine::new(form).expect("should create engine");

        // HIV positive => arv_regimen should be visible
        let mut values = HashMap::new();
        values.insert("hiv_status".to_string(), serde_json::json!("positive"));
        values.insert("weight_kg".to_string(), serde_json::json!(60));
        values.insert("height_cm".to_string(), serde_json::json!(160));

        let state = engine.evaluate(&values);
        assert_eq!(state.visibility.get("arv_regimen"), Some(&true));
    }

    #[test]
    fn on_field_change_updates_computed() {
        let form = simple_registration_form();
        let engine = FormEngine::new(form).expect("should create engine");

        let mut values = HashMap::new();
        values.insert("first_name".to_string(), serde_json::json!("Alice"));
        values.insert("last_name".to_string(), serde_json::json!("Uwimana"));
        values.insert("sex".to_string(), serde_json::json!("female"));
        values.insert("weight_kg".to_string(), serde_json::json!(65));
        values.insert("height_cm".to_string(), serde_json::json!(165));

        let mut state = engine.evaluate(&values);

        // Change weight
        engine.on_field_change("weight_kg", serde_json::json!(70), &mut state);

        // BMI should be recomputed: 70 / (1.65^2) = 25.7
        let bmi_val = state
            .computed_values
            .get("bmi_display")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        assert!((bmi_val - 25.7).abs() < 0.1);
    }

    #[test]
    fn on_field_change_updates_visibility() {
        let form = anc_visit_form();
        let engine = FormEngine::new(form).expect("should create engine");

        let mut values = HashMap::new();
        values.insert("hiv_status".to_string(), serde_json::json!("negative"));
        values.insert("weight_kg".to_string(), serde_json::json!(60));
        values.insert("height_cm".to_string(), serde_json::json!(160));

        let mut state = engine.evaluate(&values);
        assert_eq!(state.visibility.get("arv_regimen"), Some(&false));

        // Change HIV status to positive
        engine.on_field_change("hiv_status", serde_json::json!("positive"), &mut state);
        assert_eq!(state.visibility.get("arv_regimen"), Some(&true));
    }

    #[test]
    fn validation_blocks_submission() {
        let form = simple_registration_form();
        let engine = FormEngine::new(form).expect("should create engine");

        let mut values = HashMap::new();
        values.insert("first_name".to_string(), serde_json::json!("Alice"));
        values.insert("last_name".to_string(), serde_json::json!("Uwimana"));
        values.insert("sex".to_string(), serde_json::json!("female"));
        values.insert("weight_kg".to_string(), serde_json::json!(-5));

        let state = engine.evaluate(&values);

        // Weight validation should fail
        let weight_errors: Vec<&ValidationResult> = state
            .validation_results
            .iter()
            .filter(|r| r.link_id == "weight_kg" && r.severity == Severity::Error)
            .collect();
        assert!(!weight_errors.is_empty());
        assert!(!state.is_submittable);
    }

    #[test]
    fn hidden_field_validation_skipped() {
        let form = anc_visit_form();
        let engine = FormEngine::new(form).expect("should create engine");

        // HIV negative => arv_regimen hidden, its required status shouldn't block
        let mut values = HashMap::new();
        values.insert("hiv_status".to_string(), serde_json::json!("negative"));
        values.insert("weight_kg".to_string(), serde_json::json!(60));
        values.insert("height_cm".to_string(), serde_json::json!(160));

        let state = engine.evaluate(&values);

        // arv_regimen is hidden, so no error for it being empty
        let arv_errors: Vec<&ValidationResult> = state
            .validation_results
            .iter()
            .filter(|r| r.link_id == "arv_regimen")
            .collect();
        assert!(arv_errors.is_empty());
    }

    #[test]
    fn form_definition_json_roundtrip() {
        let form = simple_registration_form();
        let json = serde_json::to_string_pretty(&form).expect("should serialize");
        let deserialized: FormDefinition = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deserialized.id, form.id);
        assert_eq!(deserialized.version, form.version);
        assert_eq!(deserialized.items.len(), form.items.len());
    }
}
