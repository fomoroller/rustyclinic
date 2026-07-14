//! Form definitions for encounter capture.
//!
//! Provides hardcoded `FormDefinition` instances for built-in clinical forms
//! as fallback, and supports loading forms from installed packages.

use rusqlite::Connection;

use rustyclinic_forms::definition::{
    ChoiceOption, FormDefinition, FormItem, ItemType, Severity, ValidationRule,
};
use rustyclinic_forms::expression::{BinaryOperator, DataType, Expression, FieldProperty};

/// Try to load a form definition from installed packages in the database.
///
/// Looks for an activated package containing a form with the given `form_id`.
/// Returns `None` if no matching form is found (caller should fall back to default).
pub fn load_form_from_packages(conn: &Connection, form_id: &str) -> Option<FormDefinition> {
    // Query for form JSON stored in the package_forms table
    let result: Result<String, _> = conn.query_row(
        "SELECT form_json FROM package_forms
         INNER JOIN installed_packages ON package_forms.package_row_id = installed_packages.id
         WHERE package_forms.form_id = ?1
           AND installed_packages.status = 'activated'
         ORDER BY installed_packages.activated_at DESC
         LIMIT 1",
        rusqlite::params![form_id],
        |row| row.get(0),
    );

    match result {
        Ok(json) => serde_json::from_str(&json)
            .map_err(|e| {
                tracing::warn!(form_id, error = %e, "failed to parse installed form definition");
                e
            })
            .ok(),
        Err(_) => None,
    }
}

fn load_form_from_packages_with_version(
    conn: &Connection,
    form_id: &str,
    package_version: &str,
) -> Option<FormDefinition> {
    let result: Result<String, _> = conn.query_row(
        "SELECT form_json FROM package_forms
         INNER JOIN installed_packages ON package_forms.package_row_id = installed_packages.id
         WHERE package_forms.form_id = ?1
           AND installed_packages.status = 'activated'
           AND installed_packages.version = ?2
         ORDER BY installed_packages.activated_at DESC
         LIMIT 1",
        rusqlite::params![form_id, package_version],
        |row| row.get(0),
    );

    match result {
        Ok(json) => serde_json::from_str(&json)
            .map_err(|e| {
                tracing::warn!(form_id, package_version, error = %e, "failed to parse installed form definition");
                e
            })
            .ok(),
        Err(_) => None,
    }
}

fn load_pinned_form_version(
    conn: &Connection,
    encounter_id: &str,
    form_family: &str,
) -> Option<String> {
    let persisted_pin: Option<String> = conn
        .query_row(
            "SELECT pinned_form_version FROM encounters
             WHERE id = ?1 AND pinned_form_family = ?2 AND pinned_form_version IS NOT NULL",
            rusqlite::params![encounter_id, form_family],
            |row| row.get(0),
        )
        .ok();

    if persisted_pin.is_some() {
        return persisted_pin;
    }

    conn.query_row(
        "SELECT form_version FROM form_drafts
         WHERE encounter_id = ?1 AND form_family = ?2
         ORDER BY saved_at DESC
         LIMIT 1",
        rusqlite::params![encounter_id, form_family],
        |row| row.get(0),
    )
    .ok()
}

/// Resolve a form definition for the given form ID.
///
/// Tries installed packages first, falls back to the hardcoded default.
pub fn resolve_form(conn: &Connection, form_id: &str) -> FormDefinition {
    if let Some(form) = load_form_from_packages(conn, form_id) {
        return form;
    }
    // Fall back to the hardcoded default
    default_encounter_form()
}

pub fn resolve_form_for_encounter(
    conn: &Connection,
    encounter_id: &str,
    form_id: &str,
) -> FormDefinition {
    let pinned_row: Option<(String, Option<String>, String)> = conn
        .query_row(
            "SELECT pinned_form_version,
                    pinned_form_package_row_id,
                    COALESCE(pinned_form_source_form_id, pinned_form_family, ?2)
             FROM encounters
             WHERE id = ?1",
            rusqlite::params![encounter_id, form_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();

    if let Some((_, Some(package_row_id), source_form_id)) = pinned_row.as_ref()
        && let Ok(json) = conn.query_row(
            "SELECT form_json FROM package_forms WHERE package_row_id = ?1 AND form_id = ?2",
            rusqlite::params![package_row_id, source_form_id],
            |row| row.get::<_, String>(0),
        )
        && let Ok(form) = serde_json::from_str(&json)
    {
        return form;
    }

    if let Some((pinned_version, _, source_form_id)) = pinned_row.as_ref()
        && let Some(form) =
            load_form_from_packages_with_version(conn, source_form_id, pinned_version)
    {
        return form;
    }

    if let Some(pinned_version) = load_pinned_form_version(conn, encounter_id, form_id)
        && let Some(form) = load_form_from_packages_with_version(conn, form_id, &pinned_version)
    {
        return form;
    }

    resolve_form(conn, form_id)
}

/// Returns a triage-only form definition with vitals fields only.
///
/// Used during the triage step when a nurse records vitals before
/// the doctor picks up the encounter for full consultation.
pub fn triage_form() -> FormDefinition {
    FormDefinition {
        id: "triage-vitals".to_string(),
        version: "1.0.0".to_string(),
        title: "Triage — Vital Signs".to_string(),
        items: vec![
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
                        text: "Weight".to_string(),
                        hint: Some("In kilograms".to_string()),
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
                                message: "Weight must be greater than 0".to_string(),
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
                                severity: Severity::Error,
                                message: "Weight must be less than 300 kg".to_string(),
                            },
                        ],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "height_cm".to_string(),
                        item_type: ItemType::Decimal,
                        text: "Height".to_string(),
                        hint: Some("In centimeters".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "temperature_c".to_string(),
                        item_type: ItemType::Decimal,
                        text: "Temperature".to_string(),
                        hint: Some("In \u{00B0}C (normal: 36.1\u{2013}37.2)".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![
                            ValidationRule {
                                expression: Expression::Op {
                                    op: BinaryOperator::Ge,
                                    left: Box::new(Expression::Field {
                                        link_id: "temperature_c".to_string(),
                                        property: FieldProperty::Value,
                                    }),
                                    right: Box::new(Expression::Literal {
                                        value: serde_json::json!(30),
                                        data_type: DataType::Decimal,
                                    }),
                                },
                                severity: Severity::Error,
                                message: "Temperature must be at least 30\u{00B0}C".to_string(),
                            },
                            ValidationRule {
                                expression: Expression::Op {
                                    op: BinaryOperator::Le,
                                    left: Box::new(Expression::Field {
                                        link_id: "temperature_c".to_string(),
                                        property: FieldProperty::Value,
                                    }),
                                    right: Box::new(Expression::Literal {
                                        value: serde_json::json!(45),
                                        data_type: DataType::Decimal,
                                    }),
                                },
                                severity: Severity::Error,
                                message: "Temperature must be at most 45\u{00B0}C".to_string(),
                            },
                        ],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "pulse_rate".to_string(),
                        item_type: ItemType::Integer,
                        text: "Pulse Rate".to_string(),
                        hint: Some("Beats per minute".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "blood_pressure_systolic".to_string(),
                        item_type: ItemType::Integer,
                        text: "BP Systolic".to_string(),
                        hint: Some("mmHg".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "blood_pressure_diastolic".to_string(),
                        item_type: ItemType::Integer,
                        text: "BP Diastolic".to_string(),
                        hint: Some("mmHg".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                ],
            },
            // BMI computed
            FormItem {
                link_id: "bmi".to_string(),
                item_type: ItemType::Decimal,
                text: "BMI".to_string(),
                hint: Some("Calculated from weight and height".to_string()),
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

/// Returns the default encounter capture form definition.
///
/// Contains: vitals (weight, height, temperature, BP, pulse), computed BMI,
/// chief complaint, diagnosis with skip logic, treatment, visit notes,
/// and follow-up with conditional date field.
pub fn default_encounter_form() -> FormDefinition {
    FormDefinition {
        id: "encounter-capture".to_string(),
        version: "1.1.0".to_string(),
        title: "General Outpatient Encounter".to_string(),
        items: vec![
            // ===== Vitals section =====
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
                        text: "Weight".to_string(),
                        hint: Some("In kilograms".to_string()),
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
                                message: "Weight must be greater than 0".to_string(),
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
                                severity: Severity::Error,
                                message: "Weight must be less than 300 kg".to_string(),
                            },
                        ],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "height_cm".to_string(),
                        item_type: ItemType::Decimal,
                        text: "Height".to_string(),
                        hint: Some("In centimeters".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "temperature_c".to_string(),
                        item_type: ItemType::Decimal,
                        text: "Temperature".to_string(),
                        hint: Some("In \u{00B0}C (normal: 36.1\u{2013}37.2)".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![
                            ValidationRule {
                                expression: Expression::Op {
                                    op: BinaryOperator::Ge,
                                    left: Box::new(Expression::Field {
                                        link_id: "temperature_c".to_string(),
                                        property: FieldProperty::Value,
                                    }),
                                    right: Box::new(Expression::Literal {
                                        value: serde_json::json!(30),
                                        data_type: DataType::Decimal,
                                    }),
                                },
                                severity: Severity::Error,
                                message: "Temperature must be at least 30\u{00B0}C".to_string(),
                            },
                            ValidationRule {
                                expression: Expression::Op {
                                    op: BinaryOperator::Le,
                                    left: Box::new(Expression::Field {
                                        link_id: "temperature_c".to_string(),
                                        property: FieldProperty::Value,
                                    }),
                                    right: Box::new(Expression::Literal {
                                        value: serde_json::json!(45),
                                        data_type: DataType::Decimal,
                                    }),
                                },
                                severity: Severity::Error,
                                message: "Temperature must be at most 45\u{00B0}C".to_string(),
                            },
                        ],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "pulse_rate".to_string(),
                        item_type: ItemType::Integer,
                        text: "Pulse Rate".to_string(),
                        hint: Some("Beats per minute".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "blood_pressure_systolic".to_string(),
                        item_type: ItemType::Integer,
                        text: "BP Systolic".to_string(),
                        hint: Some("mmHg".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "blood_pressure_diastolic".to_string(),
                        item_type: ItemType::Integer,
                        text: "BP Diastolic".to_string(),
                        hint: Some("mmHg".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                ],
            },
            // ===== BMI (computed) =====
            FormItem {
                link_id: "bmi".to_string(),
                item_type: ItemType::Decimal,
                text: "BMI".to_string(),
                hint: Some("Calculated from weight and height".to_string()),
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
            // ===== Clinical Assessment section =====
            FormItem {
                link_id: "assessment".to_string(),
                item_type: ItemType::Group,
                text: "Clinical Assessment".to_string(),
                hint: None,
                required: false,
                read_only: false,
                enable_when: None,
                computed_value: None,
                validation: vec![],
                items: vec![
                    FormItem {
                        link_id: "chief_complaint".to_string(),
                        item_type: ItemType::String,
                        text: "Chief Complaint".to_string(),
                        hint: Some(
                            "Primary reason for visit, in the patient's own words".to_string(),
                        ),
                        required: true,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "history_of_present_illness".to_string(),
                        item_type: ItemType::String,
                        text: "History of Present Illness".to_string(),
                        hint: Some("Onset, duration, severity, associated symptoms".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "physical_examination".to_string(),
                        item_type: ItemType::String,
                        text: "Physical Examination Findings".to_string(),
                        hint: Some("Relevant positive and negative findings".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                ],
            },
            // ===== Diagnosis section =====
            FormItem {
                link_id: "diagnosis".to_string(),
                item_type: ItemType::Group,
                text: "Diagnosis".to_string(),
                hint: None,
                required: false,
                read_only: false,
                enable_when: None,
                computed_value: None,
                validation: vec![],
                items: vec![
                    FormItem {
                        link_id: "primary_diagnosis".to_string(),
                        item_type: ItemType::String,
                        text: "Primary Diagnosis".to_string(),
                        hint: Some(
                            "Search ICD-11 and select a diagnosis (or enter free text)."
                                .to_string(),
                        ),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "other_diagnosis".to_string(),
                        item_type: ItemType::String,
                        text: "Specify Other Diagnosis".to_string(),
                        hint: None,
                        required: false,
                        read_only: false,
                        enable_when: Some(Expression::Op {
                            op: BinaryOperator::Eq,
                            left: Box::new(Expression::Field {
                                link_id: "primary_diagnosis".to_string(),
                                property: FieldProperty::Value,
                            }),
                            right: Box::new(Expression::Literal {
                                value: serde_json::json!("other"),
                                data_type: DataType::String,
                            }),
                        }),
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                ],
            },
            // ===== Treatment & Plan section =====
            FormItem {
                link_id: "treatment_plan".to_string(),
                item_type: ItemType::Group,
                text: "Treatment & Plan".to_string(),
                hint: None,
                required: false,
                read_only: false,
                enable_when: None,
                computed_value: None,
                validation: vec![],
                items: vec![
                    FormItem {
                        link_id: "treatment".to_string(),
                        item_type: ItemType::String,
                        text: "Treatment Plan".to_string(),
                        hint: Some("Management approach, interventions performed".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "medications".to_string(),
                        item_type: ItemType::String,
                        text: "Medications Prescribed".to_string(),
                        hint: Some("Drug name, dose, frequency, duration".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "patient_instructions".to_string(),
                        item_type: ItemType::String,
                        text: "Patient Instructions".to_string(),
                        hint: Some(
                            "Home care, warning signs to watch for, when to return".to_string(),
                        ),
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                ],
            },
            // ===== Disposition =====
            FormItem {
                link_id: "disposition".to_string(),
                item_type: ItemType::Group,
                text: "Disposition".to_string(),
                hint: None,
                required: false,
                read_only: false,
                enable_when: None,
                computed_value: None,
                validation: vec![],
                items: vec![
                    FormItem {
                        link_id: "outcome".to_string(),
                        item_type: ItemType::Choice {
                            options: vec![
                                ChoiceOption {
                                    value: "discharged".to_string(),
                                    label: "Discharged home".to_string(),
                                },
                                ChoiceOption {
                                    value: "referred".to_string(),
                                    label: "Referred to higher facility".to_string(),
                                },
                                ChoiceOption {
                                    value: "admitted".to_string(),
                                    label: "Admitted".to_string(),
                                },
                                ChoiceOption {
                                    value: "observation".to_string(),
                                    label: "Under observation".to_string(),
                                },
                            ],
                        },
                        text: "Visit Outcome".to_string(),
                        hint: None,
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "referral_facility".to_string(),
                        item_type: ItemType::String,
                        text: "Referral Facility".to_string(),
                        hint: Some("Name of the facility being referred to".to_string()),
                        required: false,
                        read_only: false,
                        enable_when: Some(Expression::Op {
                            op: BinaryOperator::Eq,
                            left: Box::new(Expression::Field {
                                link_id: "outcome".to_string(),
                                property: FieldProperty::Value,
                            }),
                            right: Box::new(Expression::Literal {
                                value: serde_json::json!("referred"),
                                data_type: DataType::String,
                            }),
                        }),
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "follow_up_needed".to_string(),
                        item_type: ItemType::Boolean,
                        text: "Follow-up visit needed".to_string(),
                        hint: None,
                        required: false,
                        read_only: false,
                        enable_when: None,
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                    FormItem {
                        link_id: "follow_up_date".to_string(),
                        item_type: ItemType::Date,
                        text: "Follow-up Date".to_string(),
                        hint: None,
                        required: false,
                        read_only: false,
                        enable_when: Some(Expression::Op {
                            op: BinaryOperator::Eq,
                            left: Box::new(Expression::Field {
                                link_id: "follow_up_needed".to_string(),
                                property: FieldProperty::Value,
                            }),
                            right: Box::new(Expression::Literal {
                                value: serde_json::json!(true),
                                data_type: DataType::Boolean,
                            }),
                        }),
                        computed_value: None,
                        validation: vec![],
                        items: vec![],
                    },
                ],
            },
            // ===== Additional notes =====
            FormItem {
                link_id: "visit_notes".to_string(),
                item_type: ItemType::String,
                text: "Additional Notes".to_string(),
                hint: Some("Any additional observations or comments".to_string()),
                required: false,
                read_only: false,
                enable_when: None,
                computed_value: None,
                validation: vec![],
                items: vec![],
            },
        ],
        mappings: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rustyclinic_core::types::new_id;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.pragma_update(None, "foreign_keys", "on").expect("fk");
        rustyclinic_db::migration::run_migrations(&conn).expect("migrations");
        conn
    }

    fn form_json(form_id: &str, version: &str, title: &str) -> String {
        format!(r#"{{"id":"{form_id}","version":"{version}","title":"{title}","items":[]}}"#)
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_form_package(
        conn: &Connection,
        facility_id: uuid::Uuid,
        package_id: &str,
        package_version: &str,
        status: &str,
        activated_at: Option<&str>,
        form_id: &str,
        form_payload: &str,
    ) -> uuid::Uuid {
        let row_id = new_id();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO installed_packages
             (id, facility_id, package_id, package_type, version, status, manifest, installed_at, activated_at, rolled_back_at, installed_by, version_num)
             VALUES (?1, ?2, ?3, 'form', ?4, ?5, '{}', ?6, ?7, NULL, ?8, 0)",
            rusqlite::params![
                row_id.to_string(),
                facility_id.to_string(),
                package_id,
                package_version,
                status,
                now,
                activated_at,
                new_id().to_string(),
            ],
        )
        .expect("insert installed package");

        conn.execute(
            "INSERT INTO package_forms (package_row_id, form_id, form_json) VALUES (?1, ?2, ?3)",
            rusqlite::params![row_id.to_string(), form_id, form_payload],
        )
        .expect("insert package form");

        row_id
    }

    #[test]
    fn contract_activation_gates_package_form_resolution() {
        let conn = setup_db();
        let facility_id = new_id();
        let payload = form_json("encounter-capture", "1.0.0", "Staged Encounter Form");

        insert_form_package(
            &conn,
            facility_id,
            "encounter-runtime",
            "1.0.0",
            "staged",
            None,
            "encounter-capture",
            &payload,
        );

        assert!(
            load_form_from_packages(&conn, "encounter-capture").is_none(),
            "contract: staged install must not be used at runtime before activation"
        );

        conn.execute(
            "UPDATE installed_packages SET status = 'activated', activated_at = ?1 WHERE package_id = 'encounter-runtime'",
            rusqlite::params!["2026-01-01T00:00:00Z"],
        )
        .expect("activate package");

        let resolved = load_form_from_packages(&conn, "encounter-capture")
            .expect("activated package form should resolve");
        assert_eq!(resolved.title, "Staged Encounter Form");
    }

    #[test]
    fn contract_rollback_changes_runtime_resolution_for_new_work() {
        let conn = setup_db();
        let facility_id = new_id();

        let v1 = form_json("encounter-capture", "1.0.0", "Encounter Form V1");
        let v2 = form_json("encounter-capture", "2.0.0", "Encounter Form V2");

        insert_form_package(
            &conn,
            facility_id,
            "encounter-runtime",
            "1.0.0",
            "activated",
            Some("2026-01-01T00:00:00Z"),
            "encounter-capture",
            &v1,
        );

        let v2_row = insert_form_package(
            &conn,
            facility_id,
            "encounter-runtime",
            "2.0.0",
            "activated",
            Some("2026-02-01T00:00:00Z"),
            "encounter-capture",
            &v2,
        );

        let before_rollback = resolve_form(&conn, "encounter-capture");
        assert_eq!(before_rollback.version, "2.0.0");

        conn.execute(
            "UPDATE installed_packages SET status = 'rolled_back', rolled_back_at = ?1 WHERE id = ?2",
            rusqlite::params!["2026-02-02T00:00:00Z", v2_row.to_string()],
        )
        .expect("rollback newer package");

        let after_rollback = resolve_form(&conn, "encounter-capture");
        assert_eq!(
            after_rollback.version, "1.0.0",
            "contract: rollback must change what newly resolved runtime work uses"
        );
    }

    #[test]
    fn contract_encounter_remains_pinned_after_newer_activation() {
        let conn = setup_db();
        let facility_id = new_id();
        let encounter_id = "encounter-pin-contract";

        let v1 = form_json("encounter-capture", "1.0.0", "Encounter Form V1");
        let v2 = form_json("encounter-capture", "2.0.0", "Encounter Form V2");

        insert_form_package(
            &conn,
            facility_id,
            "encounter-runtime",
            "1.0.0",
            "activated",
            Some("2026-01-01T00:00:00Z"),
            "encounter-capture",
            &v1,
        );

        conn.execute(
            "INSERT INTO form_drafts (user_id, encounter_id, form_family, form_version, field_values, saved_at)
             VALUES (?1, ?2, 'encounter-capture', '1.0.0', '{}', ?3)",
            rusqlite::params![new_id().to_string(), encounter_id, "2026-01-05T00:00:00Z"],
        )
        .expect("insert pinned draft metadata");

        insert_form_package(
            &conn,
            facility_id,
            "encounter-runtime",
            "2.0.0",
            "activated",
            Some("2026-02-01T00:00:00Z"),
            "encounter-capture",
            &v2,
        );

        let resolved = resolve_form_for_encounter(&conn, encounter_id, "encounter-capture");
        assert_eq!(
            resolved.version, "1.0.0",
            "contract: encounter should keep its pinned form version after newer activation"
        );
    }
}
