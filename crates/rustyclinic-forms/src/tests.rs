//! Integration tests for the form engine.

use std::collections::HashMap;

use crate::definition::{
    ChoiceOption, FormDefinition, FormItem, ItemType, Severity, ValidationRule,
};
use crate::engine::FormEngine;
use crate::expression::{BinaryOperator, DataType, Expression, FieldProperty};

/// Build a complete ANC visit form definition for integration testing.
fn anc_visit_form() -> FormDefinition {
    FormDefinition {
        id: "anc-visit".to_string(),
        version: "1.2.0".to_string(),
        title: "Antenatal Care Visit".to_string(),
        items: vec![
            // Patient vitals group
            FormItem {
                link_id: "vitals".to_string(),
                item_type: ItemType::Group,
                text: "Vital Signs".to_string(),
                hint: None,
                required: false,
                read_only: false,
                enable_when: None,
                computed_value: None,
                validation: vec![],
                items: vec![
                    FormItem {
                        link_id: "weight_kg".to_string(),
                        item_type: ItemType::Decimal,
                        text: "Weight (kg)".to_string(),
                        hint: None,
                        required: true,
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
                                message: "Weight exceeds 300 kg — please verify".to_string(),
                            },
                        ],
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
            },
            // HIV section
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
                link_id: "viral_load".to_string(),
                item_type: ItemType::Decimal,
                text: "Viral Load".to_string(),
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
            // LMP and gestational age
            FormItem {
                link_id: "lmp_date".to_string(),
                item_type: ItemType::Date,
                text: "Last Menstrual Period".to_string(),
                hint: None,
                required: true,
                read_only: false,
                enable_when: None,
                computed_value: None,
                validation: vec![],
                items: vec![],
            },
            FormItem {
                link_id: "gestational_weeks".to_string(),
                item_type: ItemType::Integer,
                text: "Gestational Age (weeks)".to_string(),
                hint: None,
                required: false,
                read_only: true,
                enable_when: None,
                computed_value: Some(Expression::FunctionCall {
                    name: "gestational_age".to_string(),
                    args: vec![Expression::Field {
                        link_id: "lmp_date".to_string(),
                        property: FieldProperty::Value,
                    }],
                }),
                validation: vec![],
                items: vec![],
            },
        ],
        mappings: vec![],
    }
}

#[test]
fn integration_anc_form_loads() {
    let form = anc_visit_form();
    let engine = FormEngine::new(form);
    assert!(engine.is_ok());
}

#[test]
fn integration_anc_form_full_evaluation() {
    let form = anc_visit_form();
    let engine = FormEngine::new(form).expect("should create engine");

    let mut values = HashMap::new();
    values.insert("weight_kg".to_string(), serde_json::json!(65));
    values.insert("height_cm".to_string(), serde_json::json!(160));
    values.insert("hiv_status".to_string(), serde_json::json!("negative"));
    values.insert("lmp_date".to_string(), serde_json::json!("2025-12-01"));

    let state = engine.evaluate(&values);

    // BMI computed
    assert!(state.computed_values.contains_key("bmi_display"));
    let bmi = state
        .computed_values
        .get("bmi_display")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    // 65 / (1.6^2) = 65 / 2.56 = 25.390625 => 25.4
    assert!((bmi - 25.4).abs() < 0.1);

    // HIV negative => arv_regimen and viral_load hidden
    assert_eq!(state.visibility.get("arv_regimen"), Some(&false));
    assert_eq!(state.visibility.get("viral_load"), Some(&false));

    // Gestational age computed
    assert!(state.computed_values.contains_key("gestational_weeks"));
}

#[test]
fn integration_anc_skip_logic_toggle() {
    let form = anc_visit_form();
    let engine = FormEngine::new(form).expect("should create engine");

    let mut values = HashMap::new();
    values.insert("weight_kg".to_string(), serde_json::json!(65));
    values.insert("height_cm".to_string(), serde_json::json!(160));
    values.insert("hiv_status".to_string(), serde_json::json!("negative"));
    values.insert("lmp_date".to_string(), serde_json::json!("2025-12-01"));

    let mut state = engine.evaluate(&values);
    assert_eq!(state.visibility.get("arv_regimen"), Some(&false));

    // Toggle HIV status to positive
    engine.on_field_change("hiv_status", serde_json::json!("positive"), &mut state);
    assert_eq!(state.visibility.get("arv_regimen"), Some(&true));
    assert_eq!(state.visibility.get("viral_load"), Some(&true));

    // Toggle back
    engine.on_field_change("hiv_status", serde_json::json!("negative"), &mut state);
    assert_eq!(state.visibility.get("arv_regimen"), Some(&false));
    assert_eq!(state.visibility.get("viral_load"), Some(&false));
}

#[test]
fn integration_validation_weight() {
    let form = anc_visit_form();
    let engine = FormEngine::new(form).expect("should create engine");

    let mut values = HashMap::new();
    values.insert("weight_kg".to_string(), serde_json::json!(-5));
    values.insert("height_cm".to_string(), serde_json::json!(160));
    values.insert("hiv_status".to_string(), serde_json::json!("negative"));
    values.insert("lmp_date".to_string(), serde_json::json!("2025-12-01"));

    let state = engine.evaluate(&values);

    let weight_errors: Vec<_> = state
        .validation_results
        .iter()
        .filter(|r| r.link_id == "weight_kg" && r.severity == Severity::Error)
        .collect();
    assert!(!weight_errors.is_empty());
    assert!(!state.is_submittable);
}

#[test]
fn integration_form_json_roundtrip() {
    let form = anc_visit_form();
    let json = serde_json::to_string_pretty(&form).expect("should serialize");
    let deserialized: FormDefinition = serde_json::from_str(&json).expect("should deserialize");

    assert_eq!(deserialized.id, "anc-visit");
    assert_eq!(deserialized.version, "1.2.0");
    assert_eq!(deserialized.items.len(), form.items.len());

    // Ensure the deserialized form can be loaded into the engine
    let engine = FormEngine::new(deserialized);
    assert!(engine.is_ok());
}

#[test]
fn integration_dirty_flag() {
    let form = anc_visit_form();
    let engine = FormEngine::new(form).expect("should create engine");

    let mut values = HashMap::new();
    values.insert("weight_kg".to_string(), serde_json::json!(65));
    values.insert("height_cm".to_string(), serde_json::json!(160));
    values.insert("hiv_status".to_string(), serde_json::json!("negative"));
    values.insert("lmp_date".to_string(), serde_json::json!("2025-12-01"));

    let mut state = engine.evaluate(&values);
    assert!(!state.dirty);

    engine.on_field_change("weight_kg", serde_json::json!(70), &mut state);
    assert!(state.dirty);
}

#[test]
fn integration_field_change_recomputes_bmi() {
    let form = anc_visit_form();
    let engine = FormEngine::new(form).expect("should create engine");

    let mut values = HashMap::new();
    values.insert("weight_kg".to_string(), serde_json::json!(65));
    values.insert("height_cm".to_string(), serde_json::json!(160));
    values.insert("hiv_status".to_string(), serde_json::json!("negative"));
    values.insert("lmp_date".to_string(), serde_json::json!("2025-12-01"));

    let mut state = engine.evaluate(&values);
    let bmi_before = state
        .computed_values
        .get("bmi_display")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    engine.on_field_change("weight_kg", serde_json::json!(80), &mut state);
    let bmi_after = state
        .computed_values
        .get("bmi_display")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    // BMI should have changed
    assert!((bmi_after - bmi_before).abs() > 1.0);
    // 80 / (1.6^2) = 80 / 2.56 = 31.25 => 31.2
    assert!((bmi_after - 31.2).abs() < 0.1);
}

#[test]
fn integration_expression_json_parse() {
    // Test that expressions can be parsed from JSON matching the design doc format
    let json = r#"{
        "type": "op",
        "op": "eq",
        "left": { "type": "field", "linkId": "hiv_status" },
        "right": { "type": "literal", "value": "positive", "dataType": "string" }
    }"#;

    let expr: Expression = serde_json::from_str(json).expect("should parse expression");

    match &expr {
        Expression::Op {
            op: BinaryOperator::Eq,
            left,
            right,
        } => {
            match left.as_ref() {
                Expression::Field { link_id, .. } => assert_eq!(link_id, "hiv_status"),
                other => panic!("expected Field, got {:?}", other),
            }
            match right.as_ref() {
                Expression::Literal { value, .. } => {
                    assert_eq!(value, &serde_json::json!("positive"));
                }
                other => panic!("expected Literal, got {:?}", other),
            }
        }
        other => panic!("expected Op(Eq), got {:?}", other),
    }
}

#[test]
fn integration_nested_expression_json_parse() {
    let json = r#"{
        "type": "op",
        "op": "and",
        "left": {
            "type": "op",
            "op": "eq",
            "left": { "type": "field", "linkId": "hiv_status" },
            "right": { "type": "literal", "value": "positive", "dataType": "string" }
        },
        "right": {
            "type": "op",
            "op": "eq",
            "left": { "type": "field", "linkId": "on_art" },
            "right": { "type": "literal", "value": true, "dataType": "boolean" }
        }
    }"#;

    let expr: Expression = serde_json::from_str(json).expect("should parse");
    let refs = expr.field_references();
    assert!(refs.contains(&"hiv_status".to_string()));
    assert!(refs.contains(&"on_art".to_string()));
}

#[test]
fn integration_function_expression_json_parse() {
    let json = r#"{
        "type": "fn",
        "name": "bmi",
        "args": [
            { "type": "field", "linkId": "weight_kg" },
            { "type": "field", "linkId": "height_cm" }
        ]
    }"#;

    let expr: Expression = serde_json::from_str(json).expect("should parse");
    match &expr {
        Expression::FunctionCall { name, args } => {
            assert_eq!(name, "bmi");
            assert_eq!(args.len(), 2);
        }
        other => panic!("expected FunctionCall, got {:?}", other),
    }
}

#[test]
fn integration_conditional_expression_json_parse() {
    let json = r#"{
        "type": "if",
        "condition": { "type": "field", "linkId": "hiv_status", "property": "exists" },
        "then": { "type": "literal", "value": "known", "dataType": "string" },
        "else": { "type": "literal", "value": "unknown", "dataType": "string" }
    }"#;

    let expr: Expression = serde_json::from_str(json).expect("should parse");
    match &expr {
        Expression::Conditional { condition, .. } => match condition.as_ref() {
            Expression::Field { link_id, property } => {
                assert_eq!(link_id, "hiv_status");
                assert_eq!(*property, FieldProperty::Exists);
            }
            other => panic!("expected Field, got {:?}", other),
        },
        other => panic!("expected Conditional, got {:?}", other),
    }
}
