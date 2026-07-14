//! HTML renderer for form engine evaluation state.
//!
//! Takes a `FormDefinition` and `FormEvaluationState` and produces an HTML
//! form fragment using the DESIGN.md CSS classes. This is a Rust function,
//! not an Askama template, because the output is too dynamic for static templates.

use std::collections::HashMap;
use std::fmt::Write;

use rustyclinic_forms::definition::{FormDefinition, FormItem, ItemType, Severity};
use rustyclinic_forms::engine::{FormEvaluationState, ValidationResult};

/// Render a form definition with its current evaluation state to an HTML fragment.
///
/// The output is a series of form-group divs — no outer `<form>` tag, no layout
/// wrapper. The caller's template handles the form element and submit buttons.
///
/// `validate_url` is the htmx POST endpoint for live field validation,
/// e.g. `/web/encounters/{id}/validate`.
pub fn render_form(
    definition: &FormDefinition,
    state: &FormEvaluationState,
    validate_url: &str,
) -> String {
    let errors_by_field = index_validation_results(&state.validation_results);
    let mut html = String::with_capacity(4096);
    render_items(
        &definition.items,
        state,
        &errors_by_field,
        validate_url,
        &mut html,
    );
    html
}

/// Render a validation-only partial: updated computed fields, validation messages,
/// and visibility changes. Returns an HTML fragment suitable for htmx swap.
pub fn render_validation_partial(
    definition: &FormDefinition,
    state: &FormEvaluationState,
    validate_url: &str,
) -> String {
    // Re-render the entire form — htmx will swap the form-fields container.
    render_form(definition, state, validate_url)
}

fn index_validation_results(
    results: &[ValidationResult],
) -> HashMap<String, Vec<&ValidationResult>> {
    let mut map: HashMap<String, Vec<&ValidationResult>> = HashMap::new();
    for r in results {
        map.entry(r.link_id.clone()).or_default().push(r);
    }
    map
}

fn render_items(
    items: &[FormItem],
    state: &FormEvaluationState,
    errors: &HashMap<String, Vec<&ValidationResult>>,
    validate_url: &str,
    html: &mut String,
) {
    // Detect pairs of fields that should be side-by-side (2-column grid)
    let mut i = 0;
    while i < items.len() {
        let item = &items[i];
        // Check if this and next item should be paired in a grid row
        if i + 1 < items.len() && should_pair(item, &items[i + 1]) {
            html.push_str("<div class=\"grid grid-2 gap-md\">");
            render_item(item, state, errors, validate_url, html);
            render_item(&items[i + 1], state, errors, validate_url, html);
            html.push_str("</div>");
            i += 2;
        } else {
            render_item(item, state, errors, validate_url, html);
            i += 1;
        }
    }
}

/// Heuristic: pair fields side-by-side when they're both short numeric/date inputs.
fn should_pair(a: &FormItem, b: &FormItem) -> bool {
    let is_short = |item: &FormItem| {
        matches!(
            item.item_type,
            ItemType::Integer | ItemType::Decimal | ItemType::Date | ItemType::DateTime
        ) && !item.read_only
    };
    // Also pair known field combos by name
    let known_pair = |a_id: &str, b_id: &str| {
        matches!(
            (a_id, b_id),
            ("weight_kg", "height_cm")
                | ("blood_pressure_systolic", "blood_pressure_diastolic")
                | ("bp_systolic", "bp_diastolic")
                | ("temperature_c", "pulse_rate")
        )
    };
    known_pair(&a.link_id, &b.link_id)
        || (is_short(a)
            && is_short(b)
            && !matches!(a.item_type, ItemType::Group)
            && !matches!(b.item_type, ItemType::Group))
}

fn render_item(
    item: &FormItem,
    state: &FormEvaluationState,
    errors: &HashMap<String, Vec<&ValidationResult>>,
    validate_url: &str,
    html: &mut String,
) {
    let visible = state.visibility.get(&item.link_id).copied().unwrap_or(true);
    let display = if visible {
        ""
    } else {
        " style=\"display:none\""
    };

    if item.link_id == "primary_diagnosis" && matches!(item.item_type, ItemType::String) {
        render_primary_diagnosis_terminology_field(
            item,
            state,
            errors,
            validate_url,
            html,
            display,
        );
        return;
    }

    match &item.item_type {
        ItemType::Group => {
            let _ = write!(
                html,
                "<fieldset class=\"form-section\" id=\"group-{link_id}\"{display}>\
                 <legend class=\"form-section-title\">{text}</legend>",
                link_id = escape_attr(&item.link_id),
                text = escape_html(&item.text),
            );
            render_items(&item.items, state, errors, validate_url, html);
            html.push_str("</fieldset>");
        }
        ItemType::String => {
            render_string_field(item, state, errors, validate_url, html, display);
        }
        ItemType::Integer => {
            render_number_field(item, state, errors, validate_url, html, display, "0", "1");
        }
        ItemType::Decimal => {
            render_number_field(
                item,
                state,
                errors,
                validate_url,
                html,
                display,
                "0.0",
                "any",
            );
        }
        ItemType::Boolean => {
            render_boolean_field(item, state, errors, validate_url, html, display);
        }
        ItemType::Date => {
            render_date_field(item, state, errors, validate_url, html, display);
        }
        ItemType::DateTime => {
            render_datetime_field(item, state, errors, validate_url, html, display);
        }
        ItemType::Choice { options } => {
            render_choice_field(item, state, errors, validate_url, html, display, options);
        }
    }
}

fn render_primary_diagnosis_terminology_field(
    item: &FormItem,
    state: &FormEvaluationState,
    errors: &HashMap<String, Vec<&ValidationResult>>,
    validate_url: &str,
    html: &mut String,
    display: &str,
) {
    let display_value = {
        let v = field_value_str(state, "primary_diagnosis_display");
        if v.is_empty() {
            field_value_str(state, &item.link_id)
        } else {
            v
        }
    };
    let system_value = field_value_str(state, "primary_diagnosis_system");
    let code_value = field_value_str(state, "primary_diagnosis_code");

    let has_errors = has_error_severity(errors, &item.link_id);
    let error_class = if has_errors { " error" } else { "" };
    let required_attr = if item.required { " required" } else { "" };
    let readonly_attr = if item.read_only { " readonly" } else { "" };

    let _ = write!(
        html,
        "<div class=\"form-group\" id=\"field-{link_id}\"{display}>",
        link_id = escape_attr(&item.link_id),
    );
    render_label(html, item);

    html.push_str("<div data-terminology-search=\"icd11\">");
    html.push_str("<div class=\"search-input-wrap\">");
    html.push_str(
        "<svg viewBox=\"0 0 20 20\" fill=\"none\" aria-hidden=\"true\">\
          <path d=\"M8.5 3a5.5 5.5 0 104.07 9.21l3.11 3.11a1 1 0 001.42-1.42l-3.11-3.11A5.5 5.5 0 008.5 3z\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\"/>\
        </svg>",
    );
    let _ = write!(
        html,
        "<input type=\"text\" class=\"form-input{error_class}\" id=\"{link_id}\" name=\"{link_id}\" \
         value=\"{value}\" placeholder=\"Search ICD-11…\" autocomplete=\"off\" data-terminology-input \
         {required_attr}{readonly_attr} \
         hx-post=\"{validate_url}\" hx-trigger=\"change\" hx-target=\"#form-fields\" hx-swap=\"innerHTML\" \
         hx-include=\"closest form\">",
        link_id = escape_attr(&item.link_id),
        value = escape_attr(&display_value),
        validate_url = escape_attr(validate_url),
    );
    html.push_str("</div>");

    let _ = write!(
        html,
        "<input type=\"hidden\" name=\"primary_diagnosis_system\" value=\"{system}\" data-terminology-system>\
         <input type=\"hidden\" name=\"primary_diagnosis_code\" value=\"{code}\" data-terminology-code>\
         <input type=\"hidden\" name=\"primary_diagnosis_display\" value=\"{display_val}\" data-terminology-display>",
        system = escape_attr(&system_value),
        code = escape_attr(&code_value),
        display_val = escape_attr(&display_value),
    );

    html.push_str("<div class=\"card p-0 mt-sm hidden\" data-terminology-panel></div>");

    if !code_value.is_empty() {
        let _ = write!(
            html,
            "<div class=\"text-sm text-secondary mt-sm\">Code: <span class=\"tabular\">{code}</span></div>",
            code = escape_html(&code_value),
        );
    }

    html.push_str("</div>");
    render_field_errors(html, errors, &item.link_id);
    html.push_str("</div>");
}

fn render_string_field(
    item: &FormItem,
    state: &FormEvaluationState,
    errors: &HashMap<String, Vec<&ValidationResult>>,
    validate_url: &str,
    html: &mut String,
    display: &str,
) {
    let value = field_value_str(state, &item.link_id);
    let has_errors = has_error_severity(errors, &item.link_id);
    let error_class = if has_errors { " error" } else { "" };
    let required_attr = if item.required { " required" } else { "" };
    let readonly_attr = if item.read_only { " readonly" } else { "" };

    // Use textarea for fields that benefit from multi-line input
    let multiline_keywords = [
        "notes",
        "complaint",
        "history",
        "findings",
        "examination",
        "plan",
        "treatment",
        "medications",
        "instructions",
        "comments",
        "description",
        "narrative",
        "assessment",
    ];
    let is_multiline = multiline_keywords
        .iter()
        .any(|kw| item.link_id.contains(kw) || item.text.to_lowercase().contains(kw));

    let _ = write!(
        html,
        "<div class=\"form-group\" id=\"field-{link_id}\"{display}>",
        link_id = escape_attr(&item.link_id),
    );
    render_label(html, item);

    if is_multiline {
        let _ = write!(
            html,
            "<textarea class=\"form-input{error_class}\" id=\"{link_id}\" name=\"{link_id}\" \
             rows=\"4\"{required_attr}{readonly_attr} \
             hx-post=\"{validate_url}\" hx-trigger=\"change\" hx-target=\"#form-fields\" hx-swap=\"innerHTML\" \
             hx-include=\"closest form\">{value}</textarea>",
            link_id = escape_attr(&item.link_id),
            value = escape_html(&value),
            validate_url = escape_attr(validate_url),
        );
    } else {
        let _ = write!(
            html,
            "<input type=\"text\" class=\"form-input{error_class}\" id=\"{link_id}\" name=\"{link_id}\" \
             value=\"{value}\"{required_attr}{readonly_attr} \
             hx-post=\"{validate_url}\" hx-trigger=\"change\" hx-target=\"#form-fields\" hx-swap=\"innerHTML\" \
             hx-include=\"closest form\">",
            link_id = escape_attr(&item.link_id),
            value = escape_attr(&value),
            validate_url = escape_attr(validate_url),
        );
    }

    render_field_errors(html, errors, &item.link_id);
    html.push_str("</div>");
}

#[allow(clippy::too_many_arguments)]
fn render_number_field(
    item: &FormItem,
    state: &FormEvaluationState,
    errors: &HashMap<String, Vec<&ValidationResult>>,
    validate_url: &str,
    html: &mut String,
    display: &str,
    _placeholder: &str,
    step: &str,
) {
    let has_errors = has_error_severity(errors, &item.link_id);
    let error_class = if has_errors { " error" } else { "" };
    let required_attr = if item.required { " required" } else { "" };
    let readonly_attr = if item.read_only { " readonly" } else { "" };

    // For computed fields, show the computed value
    let value = if item.read_only {
        computed_value_str(state, &item.link_id)
    } else {
        field_value_str(state, &item.link_id)
    };

    let _ = write!(
        html,
        "<div class=\"form-group\" id=\"field-{link_id}\"{display}>",
        link_id = escape_attr(&item.link_id),
    );
    render_label(html, item);

    if item.read_only {
        // Computed read-only field: display as a styled read-only input
        let _ = write!(
            html,
            "<input type=\"text\" class=\"form-input computed\" id=\"{link_id}\" name=\"{link_id}\" \
             value=\"{value}\" readonly tabindex=\"-1\">",
            link_id = escape_attr(&item.link_id),
            value = escape_attr(&value),
        );
    } else {
        let _ = write!(
            html,
            "<input type=\"number\" class=\"form-input{error_class}\" id=\"{link_id}\" name=\"{link_id}\" \
             value=\"{value}\" step=\"{step}\"{required_attr}{readonly_attr} \
             hx-post=\"{validate_url}\" hx-trigger=\"change\" hx-target=\"#form-fields\" hx-swap=\"innerHTML\" \
             hx-include=\"closest form\">",
            link_id = escape_attr(&item.link_id),
            value = escape_attr(&value),
            validate_url = escape_attr(validate_url),
        );
    }

    render_field_errors(html, errors, &item.link_id);
    html.push_str("</div>");
}

fn render_boolean_field(
    item: &FormItem,
    state: &FormEvaluationState,
    errors: &HashMap<String, Vec<&ValidationResult>>,
    validate_url: &str,
    html: &mut String,
    display: &str,
) {
    let checked = match state.field_values.get(&item.link_id) {
        Some(serde_json::Value::Bool(true)) => true,
        Some(serde_json::Value::String(s)) if s == "true" || s == "on" => true,
        _ => false,
    };
    let checked_attr = if checked { " checked" } else { "" };

    let _ = write!(
        html,
        "<div class=\"form-group\" id=\"field-{link_id}\"{display}>\
         <label class=\"form-checkbox-label\" for=\"{link_id}\">\
         <input type=\"hidden\" name=\"{link_id}\" value=\"false\">\
         <input type=\"checkbox\" class=\"form-checkbox\" id=\"{link_id}\" name=\"{link_id}\" \
         value=\"true\"{checked_attr} \
         hx-post=\"{validate_url}\" hx-trigger=\"change\" hx-target=\"#form-fields\" hx-swap=\"innerHTML\" \
         hx-include=\"closest form\">\
         <span>{text}</span>\
         </label>",
        link_id = escape_attr(&item.link_id),
        text = escape_html(&item.text),
        validate_url = escape_attr(validate_url),
    );

    render_field_errors(html, errors, &item.link_id);
    html.push_str("</div>");
}

fn render_date_field(
    item: &FormItem,
    state: &FormEvaluationState,
    errors: &HashMap<String, Vec<&ValidationResult>>,
    validate_url: &str,
    html: &mut String,
    display: &str,
) {
    let value = field_value_str(state, &item.link_id);
    let has_errors = has_error_severity(errors, &item.link_id);
    let error_class = if has_errors { " error" } else { "" };
    let required_attr = if item.required { " required" } else { "" };

    let _ = write!(
        html,
        "<div class=\"form-group\" id=\"field-{link_id}\"{display}>",
        link_id = escape_attr(&item.link_id),
    );
    render_label(html, item);

    let _ = write!(
        html,
        "<input type=\"date\" class=\"form-input{error_class}\" id=\"{link_id}\" name=\"{link_id}\" \
         value=\"{value}\"{required_attr} \
         hx-post=\"{validate_url}\" hx-trigger=\"change\" hx-target=\"#form-fields\" hx-swap=\"innerHTML\" \
         hx-include=\"closest form\">",
        link_id = escape_attr(&item.link_id),
        value = escape_attr(&value),
        validate_url = escape_attr(validate_url),
    );

    render_field_errors(html, errors, &item.link_id);
    html.push_str("</div>");
}

fn render_datetime_field(
    item: &FormItem,
    state: &FormEvaluationState,
    errors: &HashMap<String, Vec<&ValidationResult>>,
    validate_url: &str,
    html: &mut String,
    display: &str,
) {
    let value = field_value_str(state, &item.link_id);
    let has_errors = has_error_severity(errors, &item.link_id);
    let error_class = if has_errors { " error" } else { "" };
    let required_attr = if item.required { " required" } else { "" };

    let _ = write!(
        html,
        "<div class=\"form-group\" id=\"field-{link_id}\"{display}>",
        link_id = escape_attr(&item.link_id),
    );
    render_label(html, item);

    let _ = write!(
        html,
        "<input type=\"datetime-local\" class=\"form-input{error_class}\" id=\"{link_id}\" name=\"{link_id}\" \
         value=\"{value}\"{required_attr} \
         hx-post=\"{validate_url}\" hx-trigger=\"change\" hx-target=\"#form-fields\" hx-swap=\"innerHTML\" \
         hx-include=\"closest form\">",
        link_id = escape_attr(&item.link_id),
        value = escape_attr(&value),
        validate_url = escape_attr(validate_url),
    );

    render_field_errors(html, errors, &item.link_id);
    html.push_str("</div>");
}

fn render_choice_field(
    item: &FormItem,
    state: &FormEvaluationState,
    errors: &HashMap<String, Vec<&ValidationResult>>,
    validate_url: &str,
    html: &mut String,
    display: &str,
    options: &[rustyclinic_forms::definition::ChoiceOption],
) {
    let value = field_value_str(state, &item.link_id);
    let has_errors = has_error_severity(errors, &item.link_id);
    let error_class = if has_errors { " error" } else { "" };
    let required_attr = if item.required { " required" } else { "" };

    let _ = write!(
        html,
        "<div class=\"form-group\" id=\"field-{link_id}\"{display}>",
        link_id = escape_attr(&item.link_id),
    );
    render_label(html, item);

    let _ = write!(
        html,
        "<select class=\"form-input{error_class}\" id=\"{link_id}\" name=\"{link_id}\"{required_attr} \
         hx-post=\"{validate_url}\" hx-trigger=\"change\" hx-target=\"#form-fields\" hx-swap=\"innerHTML\" \
         hx-include=\"closest form\">\
         <option value=\"\">Select...</option>",
        link_id = escape_attr(&item.link_id),
        validate_url = escape_attr(validate_url),
    );

    for opt in options {
        let selected = if opt.value == value { " selected" } else { "" };
        let _ = write!(
            html,
            "<option value=\"{value}\"{selected}>{label}</option>",
            value = escape_attr(&opt.value),
            label = escape_html(&opt.label),
        );
    }

    html.push_str("</select>");
    render_field_errors(html, errors, &item.link_id);
    html.push_str("</div>");
}

// ===== Helpers =====

fn render_label(html: &mut String, item: &FormItem) {
    // Boolean fields render their own label inline
    if matches!(item.item_type, ItemType::Boolean) {
        return;
    }
    let required_marker = if item.required {
        " <span class=\"text-accent\">*</span>"
    } else {
        ""
    };
    let computed_hint = if item.read_only {
        " <span class=\"text-muted text-sm\">(computed)</span>"
    } else {
        ""
    };
    let _ = write!(
        html,
        "<label class=\"form-label\" for=\"{link_id}\">{text}{required_marker}{computed_hint}</label>",
        link_id = escape_attr(&item.link_id),
        text = escape_html(&item.text),
    );
    // Render hint text if present
    if let Some(hint) = &item.hint {
        let _ = write!(
            html,
            "<span class=\"form-hint\">{hint}</span>",
            hint = escape_html(hint),
        );
    }
}

fn render_field_errors(
    html: &mut String,
    errors: &HashMap<String, Vec<&ValidationResult>>,
    link_id: &str,
) {
    if let Some(results) = errors.get(link_id) {
        for r in results {
            let css_class = match r.severity {
                Severity::Error => "form-error",
                Severity::Warning => "form-warning",
                Severity::Info => "form-info",
            };
            let _ = write!(
                html,
                "<div class=\"{css_class}\">{message}</div>",
                message = escape_html(&r.message),
            );
        }
    }
}

fn field_value_str(state: &FormEvaluationState, link_id: &str) -> String {
    match state.field_values.get(link_id) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn computed_value_str(state: &FormEvaluationState, link_id: &str) -> String {
    match state.computed_values.get(link_id) {
        Some(serde_json::Value::Number(n)) => {
            // Format computed numbers nicely (1 decimal place for BMI etc.)
            if let Some(f) = n.as_f64() {
                format!("{:.1}", f)
            } else {
                n.to_string()
            }
        }
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn has_error_severity(errors: &HashMap<String, Vec<&ValidationResult>>, link_id: &str) -> bool {
    errors
        .get(link_id)
        .is_some_and(|results| results.iter().any(|r| r.severity == Severity::Error))
}

/// Minimal HTML escaping for text content.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Minimal attribute value escaping.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::Value;

    use super::*;

    #[test]
    fn render_form_includes_terminology_diagnosis_widget() {
        let form = crate::forms::default_encounter_form();
        let engine = rustyclinic_forms::engine::FormEngine::new(form.clone()).expect("engine");

        let mut field_values = HashMap::new();
        field_values.insert(
            "primary_diagnosis".to_string(),
            Value::String("Headache".to_string()),
        );
        field_values.insert(
            "primary_diagnosis_system".to_string(),
            Value::String(rustyclinic_terminology::ICD11_SYSTEM.to_string()),
        );
        field_values.insert(
            "primary_diagnosis_code".to_string(),
            Value::String("MG30".to_string()),
        );
        field_values.insert(
            "primary_diagnosis_display".to_string(),
            Value::String("Headache".to_string()),
        );

        let state = engine.evaluate(&field_values);
        let html = render_form(&form, &state, "/web/encounters/test/validate");

        println!("{html}");

        assert!(html.contains("data-terminology-search=\"icd11\""));
        assert!(html.contains("data-terminology-input"));
        assert!(html.contains("name=\"primary_diagnosis_system\""));
        assert!(html.contains("name=\"primary_diagnosis_code\" value=\"MG30\""));
        assert!(html.contains("name=\"primary_diagnosis_display\" value=\"Headache\""));
        assert!(html.contains("Code: <span class=\"tabular\">MG30</span>"));
    }
}
