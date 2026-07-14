//! DHIS2 data export format.
//!
//! Converts generated reports into the DHIS2 Web API JSON format
//! (`dataValueSets`) so that facilities can submit aggregate data
//! to the national HMIS.

use std::collections::HashMap;

use anyhow::Result;
use chrono::Datelike;
use serde::{Deserialize, Serialize};

use crate::definition::PeriodType;
use crate::engine::GeneratedReport;

/// A DHIS2-compatible data value set, ready for submission via the Web API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dhis2DataValueSet {
    /// DHIS2 dataset UID.
    pub data_set: String,
    /// Period string, e.g. "202603" for March 2026.
    pub period: String,
    /// DHIS2 organisation unit UID.
    pub org_unit: String,
    /// Individual data values.
    pub data_values: Vec<Dhis2DataValue>,
}

/// A single data value within a DHIS2 data value set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dhis2DataValue {
    /// DHIS2 data element UID.
    pub data_element: String,
    /// Optional category option combo UID (for disaggregation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_option_combo: Option<String>,
    /// The value as a string (DHIS2 convention).
    pub value: String,
}

/// Mapping configuration from RustyClinic indicator IDs to DHIS2 UIDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dhis2Mapping {
    /// The DHIS2 dataset UID to submit to.
    pub data_set: String,
    /// The DHIS2 organisation unit UID.
    pub org_unit: String,
    /// Mapping of indicator IDs to DHIS2 data element config.
    pub indicator_mappings: HashMap<String, Dhis2IndicatorMapping>,
}

/// Mapping for a single indicator to its DHIS2 data element.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dhis2IndicatorMapping {
    /// The DHIS2 data element UID.
    pub data_element: String,
    /// Mapping of category IDs to DHIS2 category option combo UIDs.
    /// Key format: "disaggregation_id:category_id" -> category option combo UID.
    pub category_mappings: HashMap<String, String>,
}

/// Format a DHIS2 period string from a report's period dates and type.
pub fn format_period(report: &GeneratedReport, period_type: &PeriodType) -> String {
    let start = report.period_start;
    match period_type {
        PeriodType::Daily => start.format("%Y%m%d").to_string(),
        PeriodType::Weekly => {
            let week = start.format("%W").to_string();
            let week_num: u32 = week.parse().unwrap_or_default();
            let week_num = if week_num == 0 { 1 } else { week_num };
            format!("{}W{}", start.format("%Y"), week_num)
        }
        PeriodType::Monthly => start.format("%Y%m").to_string(),
        PeriodType::Quarterly => {
            let quarter = (start.month0() / 3) + 1;
            format!("{}Q{}", start.format("%Y"), quarter)
        }
        PeriodType::Annual => start.format("%Y").to_string(),
    }
}

/// Convert a `GeneratedReport` to a DHIS2 data value set.
///
/// Only indicators that have a mapping entry will be included.
/// Unmapped indicators are silently skipped.
pub fn to_dhis2(
    report: &GeneratedReport,
    mapping: &Dhis2Mapping,
    period_type: &PeriodType,
) -> Result<Dhis2DataValueSet> {
    let period = format_period(report, period_type);
    let mut data_values = Vec::new();

    for indicator in &report.indicators {
        let ind_mapping = match mapping.indicator_mappings.get(&indicator.indicator_id) {
            Some(m) => m,
            None => continue, // skip unmapped indicators
        };

        // Add the aggregate (total) value
        data_values.push(Dhis2DataValue {
            data_element: ind_mapping.data_element.clone(),
            category_option_combo: None,
            value: format_value(indicator.value),
        });

        // Add disaggregated values
        for (disagg_id, categories) in &indicator.disaggregated {
            for (cat_id, cat_value) in categories {
                let mapping_key = format!("{disagg_id}:{cat_id}");
                if let Some(coc_uid) = ind_mapping.category_mappings.get(&mapping_key) {
                    data_values.push(Dhis2DataValue {
                        data_element: ind_mapping.data_element.clone(),
                        category_option_combo: Some(coc_uid.clone()),
                        value: format_value(*cat_value),
                    });
                }
            }
        }
    }

    if data_values.is_empty() {
        anyhow::bail!("no mapped indicators found — check DHIS2 mapping configuration");
    }

    Ok(Dhis2DataValueSet {
        data_set: mapping.data_set.clone(),
        period,
        org_unit: mapping.org_unit.clone(),
        data_values,
    })
}

/// Format a numeric value for DHIS2 (integer if whole, otherwise 2 decimal places).
fn format_value(v: f64) -> String {
    if (v - v.round()).abs() < f64::EPSILON {
        format!("{}", v as i64)
    } else {
        format!("{:.2}", v)
    }
}
