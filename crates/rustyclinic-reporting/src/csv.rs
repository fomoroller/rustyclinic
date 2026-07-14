//! CSV export for offline report sharing.
//!
//! Produces a simple CSV string from a `GeneratedReport` that can be
//! saved to a file or downloaded via the web interface.

use crate::engine::GeneratedReport;

/// Convert a generated report to CSV format.
///
/// The output includes:
/// - A header row with metadata
/// - Indicator rows with id, name-placeholder, and value
/// - Disaggregated rows with indicator_id, disaggregation, category, and value
pub fn to_csv(report: &GeneratedReport) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Header comment
    lines.push(format!(
        "# Report: {} | Facility: {} | Period: {} to {} | Generated: {}",
        report.definition_id,
        report.facility_id,
        report.period_start,
        report.period_end,
        report.generated_at.format("%Y-%m-%dT%H:%M:%SZ"),
    ));

    // CSV header
    lines.push("indicator_id,disaggregation,category,value".to_string());

    for indicator in &report.indicators {
        // Total value row
        lines.push(format!(
            "{},total,total,{}",
            escape_csv(&indicator.indicator_id),
            format_csv_value(indicator.value),
        ));

        // Disaggregated rows
        let mut disagg_ids: Vec<&String> = indicator.disaggregated.keys().collect();
        disagg_ids.sort();
        for disagg_id in disagg_ids {
            if let Some(categories) = indicator.disaggregated.get(disagg_id) {
                let mut cat_ids: Vec<&String> = categories.keys().collect();
                cat_ids.sort();
                for cat_id in cat_ids {
                    if let Some(value) = categories.get(cat_id) {
                        lines.push(format!(
                            "{},{},{},{}",
                            escape_csv(&indicator.indicator_id),
                            escape_csv(disagg_id),
                            escape_csv(cat_id),
                            format_csv_value(*value),
                        ));
                    }
                }
            }
        }
    }

    lines.join("\n")
}

/// Escape a CSV field if it contains commas, quotes, or newlines.
fn escape_csv(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Format a numeric value for CSV.
fn format_csv_value(v: f64) -> String {
    if (v - v.round()).abs() < f64::EPSILON {
        format!("{}", v as i64)
    } else {
        format!("{:.2}", v)
    }
}
