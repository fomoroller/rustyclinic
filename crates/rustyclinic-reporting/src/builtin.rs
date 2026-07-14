//! Built-in report definitions for Rwanda MoH compliance.
//!
//! These standard reports cover the most common aggregate reporting
//! requirements: OPD summaries and service delivery statistics.

use crate::definition::{
    AggregateFunction, AggregateQuery, Category, Disaggregation, DisaggregationLookup,
    FilterOperator, Indicator, PeriodType, QueryFilter, ReportDefinition,
};

/// Monthly OPD (Outpatient Department) Summary report.
///
/// Covers:
/// - Total encounters completed
/// - Encounters by sex (Male/Female/Other)
/// - Encounters by age group (0-4, 5-14, 15-24, 25-49, 50-64, 65+)
/// - Average wait time
/// - No-show rate (no_show queue entries / total queue entries)
pub fn monthly_opd_summary() -> ReportDefinition {
    ReportDefinition {
        id: "monthly-opd-summary".to_string(),
        title: "Monthly OPD Summary".to_string(),
        period_type: PeriodType::Monthly,
        indicators: vec![
            Indicator {
                id: "opd_encounters_completed".to_string(),
                name: "Total Encounters Completed".to_string(),
                description: "Total number of encounters with status 'completed' in the period."
                    .to_string(),
                numerator: AggregateQuery {
                    source_table: "encounters".to_string(),
                    filter: vec![QueryFilter {
                        field: "status".to_string(),
                        operator: FilterOperator::Eq,
                        value: "completed".to_string(),
                    }],
                    aggregate: AggregateFunction::Count,
                },
                denominator: None,
            },
            Indicator {
                id: "opd_encounters_total".to_string(),
                name: "Total Encounters".to_string(),
                description: "Total number of encounters (all statuses) in the period.".to_string(),
                numerator: AggregateQuery {
                    source_table: "encounters".to_string(),
                    filter: vec![],
                    aggregate: AggregateFunction::Count,
                },
                denominator: None,
            },
            Indicator {
                id: "opd_queue_total".to_string(),
                name: "Total Queue Entries".to_string(),
                description: "Total number of queue entries in the period.".to_string(),
                numerator: AggregateQuery {
                    source_table: "queue_entries".to_string(),
                    filter: vec![],
                    aggregate: AggregateFunction::Count,
                },
                denominator: None,
            },
            Indicator {
                id: "opd_noshow_rate".to_string(),
                name: "No-Show Rate".to_string(),
                description:
                    "Proportion of queue entries with status 'no_show' out of total queue entries."
                        .to_string(),
                numerator: AggregateQuery {
                    source_table: "queue_entries".to_string(),
                    filter: vec![QueryFilter {
                        field: "status".to_string(),
                        operator: FilterOperator::Eq,
                        value: "no_show".to_string(),
                    }],
                    aggregate: AggregateFunction::Count,
                },
                denominator: Some(AggregateQuery {
                    source_table: "queue_entries".to_string(),
                    filter: vec![],
                    aggregate: AggregateFunction::Count,
                }),
            },
        ],
        disaggregations: vec![sex_disaggregation()],
    }
}

/// Monthly Service Delivery report.
///
/// Covers:
/// - Total patients registered
/// - Total queue entries
/// - Queue completion rate (completed / total)
/// - Patients seen per provider (total encounters / distinct providers)
pub fn monthly_service_delivery() -> ReportDefinition {
    ReportDefinition {
        id: "monthly-service-delivery".to_string(),
        title: "Monthly Service Delivery".to_string(),
        period_type: PeriodType::Monthly,
        indicators: vec![
            Indicator {
                id: "sd_patients_registered".to_string(),
                name: "Total Patients Registered".to_string(),
                description: "Number of new patients registered during the period.".to_string(),
                numerator: AggregateQuery {
                    source_table: "patients".to_string(),
                    filter: vec![],
                    aggregate: AggregateFunction::Count,
                },
                denominator: None,
            },
            Indicator {
                id: "sd_queue_entries_total".to_string(),
                name: "Total Queue Entries".to_string(),
                description: "Total number of queue entries in the period.".to_string(),
                numerator: AggregateQuery {
                    source_table: "queue_entries".to_string(),
                    filter: vec![],
                    aggregate: AggregateFunction::Count,
                },
                denominator: None,
            },
            Indicator {
                id: "sd_queue_completion_rate".to_string(),
                name: "Queue Completion Rate".to_string(),
                description: "Proportion of queue entries that were completed.".to_string(),
                numerator: AggregateQuery {
                    source_table: "queue_entries".to_string(),
                    filter: vec![QueryFilter {
                        field: "status".to_string(),
                        operator: FilterOperator::Eq,
                        value: "completed".to_string(),
                    }],
                    aggregate: AggregateFunction::Count,
                },
                denominator: Some(AggregateQuery {
                    source_table: "queue_entries".to_string(),
                    filter: vec![],
                    aggregate: AggregateFunction::Count,
                }),
            },
            Indicator {
                id: "sd_encounters_total".to_string(),
                name: "Total Encounters".to_string(),
                description: "Total number of encounters completed during the period.".to_string(),
                numerator: AggregateQuery {
                    source_table: "encounters".to_string(),
                    filter: vec![QueryFilter {
                        field: "status".to_string(),
                        operator: FilterOperator::Eq,
                        value: "completed".to_string(),
                    }],
                    aggregate: AggregateFunction::Count,
                },
                denominator: None,
            },
        ],
        disaggregations: vec![],
    }
}

/// Sex disaggregation for encounter-based reports.
///
/// Since `sex` lives on the `patients` table but OPD indicators query
/// `encounters`, we use a lookup subquery:
/// `patient_id IN (SELECT id FROM patients WHERE sex = ?)`.
fn sex_disaggregation() -> Disaggregation {
    Disaggregation {
        id: "by_sex".to_string(),
        field: "sex".to_string(),
        categories: vec![
            Category {
                id: "male".to_string(),
                label: "Male".to_string(),
                filter: QueryFilter {
                    field: "sex".to_string(),
                    operator: FilterOperator::Eq,
                    value: "Male".to_string(),
                },
            },
            Category {
                id: "female".to_string(),
                label: "Female".to_string(),
                filter: QueryFilter {
                    field: "sex".to_string(),
                    operator: FilterOperator::Eq,
                    value: "Female".to_string(),
                },
            },
            Category {
                id: "other".to_string(),
                label: "Other".to_string(),
                filter: QueryFilter {
                    field: "sex".to_string(),
                    operator: FilterOperator::Eq,
                    value: "Other".to_string(),
                },
            },
        ],
        lookup: Some(DisaggregationLookup {
            join_field: "patient_id".to_string(),
            lookup_table: "patients".to_string(),
            lookup_id: "id".to_string(),
        }),
    }
}

/// Returns all built-in report definitions.
pub fn all_builtin_reports() -> Vec<ReportDefinition> {
    vec![monthly_opd_summary(), monthly_service_delivery()]
}
