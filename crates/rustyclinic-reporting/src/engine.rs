//! Report generation engine.
//!
//! Dynamically builds SQL queries from `AggregateQuery` definitions,
//! executes them against SQLite, and collects indicator results with
//! optional disaggregation.

use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::definition::{
    AggregateFunction, AggregateQuery, FilterOperator, Indicator, QueryFilter, ReportDefinition,
};

/// The report generation engine.
pub struct ReportEngine;

/// A fully generated report with computed indicator values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedReport {
    /// The definition id this report was generated from.
    pub definition_id: String,
    /// The facility this report covers.
    pub facility_id: Uuid,
    /// Start of the reporting period (inclusive).
    pub period_start: NaiveDate,
    /// End of the reporting period (inclusive).
    pub period_end: NaiveDate,
    /// When the report was generated.
    pub generated_at: DateTime<Utc>,
    /// Computed indicator results.
    pub indicators: Vec<IndicatorResult>,
}

/// Result of computing a single indicator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorResult {
    /// The indicator id.
    pub indicator_id: String,
    /// The aggregate value (or rate if denominator was present).
    pub value: f64,
    /// Disaggregated values: disaggregation_id -> category_id -> value.
    pub disaggregated: HashMap<String, HashMap<String, f64>>,
}

// ---- Allowed table/column names for SQL safety ----

const ALLOWED_TABLES: &[&str] = &["encounters", "queue_entries", "patients", "users"];

const ALLOWED_COLUMNS: &[&str] = &[
    "id",
    "facility_id",
    "patient_id",
    "provider_id",
    "queue_entry_id",
    "service_type",
    "status",
    "assigned_to",
    "position",
    "arrived_at",
    "called_at",
    "service_started_at",
    "completed_at",
    "created_at",
    "updated_at",
    "started_at",
    "ended_at",
    "visit_notes",
    "version",
    "sex",
    "date_of_birth",
    "given_name",
    "family_name",
    "national_id",
    "phone",
    "address",
    "active",
    "username",
    "display_name",
    "roles",
];

/// Validate that a table name is in the allow-list.
fn validate_table(name: &str) -> Result<()> {
    if ALLOWED_TABLES.contains(&name) {
        Ok(())
    } else {
        anyhow::bail!("table '{}' is not in the allow-list", name)
    }
}

/// Validate that a column name is in the allow-list.
fn validate_column(name: &str) -> Result<()> {
    if ALLOWED_COLUMNS.contains(&name) {
        Ok(())
    } else {
        anyhow::bail!("column '{}' is not in the allow-list", name)
    }
}

impl ReportEngine {
    /// Generate a report for a given facility and period.
    ///
    /// The engine iterates over each indicator in the definition, builds
    /// the appropriate SQL query, executes it, and collects results.
    /// Disaggregations add additional filtered queries per category.
    pub fn generate(
        conn: &Connection,
        definition: &ReportDefinition,
        facility_id: Uuid,
        period_start: NaiveDate,
        period_end: NaiveDate,
    ) -> Result<GeneratedReport> {
        let mut indicator_results = Vec::with_capacity(definition.indicators.len());

        for indicator in &definition.indicators {
            let result = Self::compute_indicator(
                conn,
                indicator,
                &definition.disaggregations,
                facility_id,
                period_start,
                period_end,
            )
            .with_context(|| format!("computing indicator '{}'", indicator.id))?;
            indicator_results.push(result);
        }

        Ok(GeneratedReport {
            definition_id: definition.id.clone(),
            facility_id,
            period_start,
            period_end,
            generated_at: Utc::now(),
            indicators: indicator_results,
        })
    }

    /// Compute a single indicator, including its disaggregated values.
    fn compute_indicator(
        conn: &Connection,
        indicator: &Indicator,
        disaggregations: &[crate::definition::Disaggregation],
        facility_id: Uuid,
        period_start: NaiveDate,
        period_end: NaiveDate,
    ) -> Result<IndicatorResult> {
        let numerator = Self::execute_query(
            conn,
            &indicator.numerator,
            facility_id,
            period_start,
            period_end,
        )?;

        let value = match &indicator.denominator {
            Some(denom_query) => {
                let denominator =
                    Self::execute_query(conn, denom_query, facility_id, period_start, period_end)?;
                if denominator == 0.0 {
                    0.0
                } else {
                    numerator / denominator
                }
            }
            None => numerator,
        };

        // Compute disaggregated values
        let mut disaggregated: HashMap<String, HashMap<String, f64>> = HashMap::new();

        for disagg in disaggregations {
            let mut category_values: HashMap<String, f64> = HashMap::new();

            for category in &disagg.categories {
                let cat_value = if disagg.lookup.is_some() {
                    // Cross-table disaggregation: use subquery
                    Self::execute_query_with_lookup(
                        conn,
                        &indicator.numerator,
                        facility_id,
                        period_start,
                        period_end,
                        disagg,
                        &category.filter,
                    )?
                } else {
                    // Same-table disaggregation: add filter directly
                    let mut extended_filters = indicator.numerator.filter.clone();
                    extended_filters.push(category.filter.clone());
                    let extended_query = AggregateQuery {
                        source_table: indicator.numerator.source_table.clone(),
                        filter: extended_filters,
                        aggregate: indicator.numerator.aggregate.clone(),
                    };
                    Self::execute_query(
                        conn,
                        &extended_query,
                        facility_id,
                        period_start,
                        period_end,
                    )?
                };

                // If there's a denominator, compute rate per category
                let cat_result = match &indicator.denominator {
                    Some(denom_query) => {
                        let denom_val = if disagg.lookup.is_some() {
                            Self::execute_query_with_lookup(
                                conn,
                                denom_query,
                                facility_id,
                                period_start,
                                period_end,
                                disagg,
                                &category.filter,
                            )?
                        } else {
                            let mut denom_filters = denom_query.filter.clone();
                            denom_filters.push(category.filter.clone());
                            let denom_ext = AggregateQuery {
                                source_table: denom_query.source_table.clone(),
                                filter: denom_filters,
                                aggregate: denom_query.aggregate.clone(),
                            };
                            Self::execute_query(
                                conn,
                                &denom_ext,
                                facility_id,
                                period_start,
                                period_end,
                            )?
                        };
                        if denom_val == 0.0 {
                            0.0
                        } else {
                            cat_value / denom_val
                        }
                    }
                    None => cat_value,
                };

                category_values.insert(category.id.clone(), cat_result);
            }

            disaggregated.insert(disagg.id.clone(), category_values);
        }

        Ok(IndicatorResult {
            indicator_id: indicator.id.clone(),
            value,
            disaggregated,
        })
    }

    /// Build and execute a single aggregate SQL query.
    ///
    /// All values are parameterized to prevent SQL injection.
    /// Table and column names are validated against allow-lists.
    fn execute_query(
        conn: &Connection,
        query: &AggregateQuery,
        facility_id: Uuid,
        period_start: NaiveDate,
        period_end: NaiveDate,
    ) -> Result<f64> {
        validate_table(&query.source_table)?;

        let agg_expr = match &query.aggregate {
            AggregateFunction::Count => "COUNT(*)".to_string(),
            AggregateFunction::Sum { field } => {
                validate_column(field)?;
                format!("COALESCE(SUM({field}), 0)")
            }
            AggregateFunction::Avg { field } => {
                validate_column(field)?;
                format!("COALESCE(AVG({field}), 0)")
            }
            AggregateFunction::Min { field } => {
                validate_column(field)?;
                format!("COALESCE(MIN({field}), 0)")
            }
            AggregateFunction::Max { field } => {
                validate_column(field)?;
                format!("COALESCE(MAX({field}), 0)")
            }
        };

        let table = &query.source_table;

        // Build WHERE clause: always filter by facility_id and created_at period
        let mut conditions = vec![
            "facility_id = ?1".to_string(),
            "created_at >= ?2".to_string(),
            "created_at <= ?3".to_string(),
        ];
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(facility_id.to_string()),
            Box::new(period_start.to_string()),
            Box::new(format!("{}T23:59:59", period_end)),
        ];

        // Add user-defined filters
        for filter in &query.filter {
            validate_column(&filter.field)?;
            let param_idx = params.len() + 1;
            let op = match filter.operator {
                FilterOperator::Eq => "=",
                FilterOperator::Ne => "!=",
                FilterOperator::Gt => ">",
                FilterOperator::Lt => "<",
                FilterOperator::Gte => ">=",
                FilterOperator::Lte => "<=",
                FilterOperator::Like => "LIKE",
                FilterOperator::In => "IN",
            };

            if filter.operator == FilterOperator::In {
                // For IN operator, we split the value by comma and generate placeholders
                let values: Vec<&str> = filter.value.split(',').collect();
                let placeholders: Vec<String> = values
                    .iter()
                    .enumerate()
                    .map(|(j, _)| format!("?{}", param_idx + j))
                    .collect();
                conditions.push(format!("{} IN ({})", filter.field, placeholders.join(",")));
                for v in values {
                    params.push(Box::new(v.trim().to_string()));
                }
            } else {
                conditions.push(format!("{} {op} ?{param_idx}", filter.field));
                params.push(Box::new(filter.value.clone()));
            }
        }

        let where_clause = conditions.join(" AND ");
        let sql = format!("SELECT {agg_expr} FROM {table} WHERE {where_clause}");

        tracing::debug!(sql = %sql, "executing report query");

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let result: f64 = conn
            .query_row(&sql, param_refs.as_slice(), |row| row.get(0))
            .with_context(|| format!("executing query: {sql}"))?;

        Ok(result)
    }

    /// Execute a query with a cross-table disaggregation lookup (subquery).
    ///
    /// Generates SQL like:
    /// `SELECT COUNT(*) FROM encounters WHERE ... AND patient_id IN
    ///  (SELECT id FROM patients WHERE sex = ?)`
    fn execute_query_with_lookup(
        conn: &Connection,
        query: &AggregateQuery,
        facility_id: Uuid,
        period_start: NaiveDate,
        period_end: NaiveDate,
        disagg: &crate::definition::Disaggregation,
        category_filter: &QueryFilter,
    ) -> Result<f64> {
        let lookup = disagg
            .lookup
            .as_ref()
            .context("disaggregation lookup is None")?;

        validate_table(&query.source_table)?;
        validate_table(&lookup.lookup_table)?;
        validate_column(&lookup.join_field)?;
        validate_column(&lookup.lookup_id)?;
        validate_column(&category_filter.field)?;

        let agg_expr = match &query.aggregate {
            AggregateFunction::Count => "COUNT(*)".to_string(),
            AggregateFunction::Sum { field } => {
                validate_column(field)?;
                format!("COALESCE(SUM({field}), 0)")
            }
            AggregateFunction::Avg { field } => {
                validate_column(field)?;
                format!("COALESCE(AVG({field}), 0)")
            }
            AggregateFunction::Min { field } => {
                validate_column(field)?;
                format!("COALESCE(MIN({field}), 0)")
            }
            AggregateFunction::Max { field } => {
                validate_column(field)?;
                format!("COALESCE(MAX({field}), 0)")
            }
        };

        let table = &query.source_table;

        let mut conditions = vec![
            "facility_id = ?1".to_string(),
            "created_at >= ?2".to_string(),
            "created_at <= ?3".to_string(),
        ];
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(facility_id.to_string()),
            Box::new(period_start.to_string()),
            Box::new(format!("{}T23:59:59", period_end)),
        ];

        // Add user-defined filters from the indicator query
        for filter in &query.filter {
            validate_column(&filter.field)?;
            let param_idx = params.len() + 1;
            let op = match filter.operator {
                FilterOperator::Eq => "=",
                FilterOperator::Ne => "!=",
                FilterOperator::Gt => ">",
                FilterOperator::Lt => "<",
                FilterOperator::Gte => ">=",
                FilterOperator::Lte => "<=",
                FilterOperator::Like => "LIKE",
                FilterOperator::In => "IN",
            };
            if filter.operator == FilterOperator::In {
                let values: Vec<&str> = filter.value.split(',').collect();
                let placeholders: Vec<String> = values
                    .iter()
                    .enumerate()
                    .map(|(j, _)| format!("?{}", param_idx + j))
                    .collect();
                conditions.push(format!("{} IN ({})", filter.field, placeholders.join(",")));
                for v in values {
                    params.push(Box::new(v.trim().to_string()));
                }
            } else {
                conditions.push(format!("{} {op} ?{param_idx}", filter.field));
                params.push(Box::new(filter.value.clone()));
            }
        }

        // Add subquery for cross-table disaggregation
        let subquery_param_idx = params.len() + 1;
        let cat_op = match category_filter.operator {
            FilterOperator::Eq => "=",
            FilterOperator::Ne => "!=",
            FilterOperator::Gt => ">",
            FilterOperator::Lt => "<",
            FilterOperator::Gte => ">=",
            FilterOperator::Lte => "<=",
            FilterOperator::Like => "LIKE",
            FilterOperator::In => "IN",
        };
        conditions.push(format!(
            "{} IN (SELECT {} FROM {} WHERE {} {} ?{})",
            lookup.join_field,
            lookup.lookup_id,
            lookup.lookup_table,
            category_filter.field,
            cat_op,
            subquery_param_idx,
        ));
        params.push(Box::new(category_filter.value.clone()));

        let where_clause = conditions.join(" AND ");
        let sql = format!("SELECT {agg_expr} FROM {table} WHERE {where_clause}");

        tracing::debug!(sql = %sql, "executing report query with lookup");

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let result: f64 = conn
            .query_row(&sql, param_refs.as_slice(), |row| row.get(0))
            .with_context(|| format!("executing lookup query: {sql}"))?;

        Ok(result)
    }
}
