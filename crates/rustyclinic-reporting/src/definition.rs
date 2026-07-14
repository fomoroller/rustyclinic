//! Report definition types.
//!
//! A `ReportDefinition` describes what data to aggregate, how to disaggregate it,
//! and over what period. These definitions are data-driven so that new reports
//! can be added without changing engine code.

use serde::{Deserialize, Serialize};

/// A complete report definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportDefinition {
    /// Unique identifier, e.g. "monthly-opd-summary".
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// The reporting period granularity.
    pub period_type: PeriodType,
    /// Indicators to compute.
    pub indicators: Vec<Indicator>,
    /// Disaggregation axes (e.g. by sex, age group).
    pub disaggregations: Vec<Disaggregation>,
}

/// Reporting period granularity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeriodType {
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    Annual,
}

/// A single indicator within a report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Indicator {
    /// Unique identifier, e.g. "opd_visits_total".
    pub id: String,
    /// Display name.
    pub name: String,
    /// Description of what this indicator measures.
    pub description: String,
    /// The numerator query.
    pub numerator: AggregateQuery,
    /// Optional denominator — if `None`, the indicator is a raw count/sum.
    /// If `Some`, the indicator is a rate (numerator / denominator).
    pub denominator: Option<AggregateQuery>,
}

/// Describes an aggregate SQL query to run against the local database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateQuery {
    /// Source table name, e.g. "encounters", "queue_entries".
    pub source_table: String,
    /// Filters to apply (ANDed together).
    pub filter: Vec<QueryFilter>,
    /// Aggregation function.
    pub aggregate: AggregateFunction,
}

/// SQL aggregation functions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AggregateFunction {
    Count,
    Sum { field: String },
    Avg { field: String },
    Min { field: String },
    Max { field: String },
}

/// A single filter condition in a query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryFilter {
    /// Column name.
    pub field: String,
    /// Comparison operator.
    pub operator: FilterOperator,
    /// Value to compare against (as a string; will be parameterized).
    pub value: String,
}

/// Comparison operators for query filters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterOperator {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
    In,
    Like,
}

/// A disaggregation axis applied to indicators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Disaggregation {
    /// Unique identifier, e.g. "by_sex".
    pub id: String,
    /// The database column to disaggregate on.
    pub field: String,
    /// The categories within this disaggregation.
    pub categories: Vec<Category>,
    /// Optional lookup for cross-table disaggregation.
    ///
    /// When the disaggregation field lives on a different table (e.g. `sex`
    /// on `patients` while the indicator queries `encounters`), set this to
    /// describe how to resolve the lookup via a subquery:
    /// `source_table.join_field IN (SELECT lookup_table.lookup_id FROM lookup_table WHERE ...)`
    #[serde(default)]
    pub lookup: Option<DisaggregationLookup>,
}

/// Cross-table lookup for disaggregation.
///
/// Generates a subquery like:
/// `{join_field} IN (SELECT {lookup_id} FROM {lookup_table} WHERE {filter})`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisaggregationLookup {
    /// The column on the source table used to join (e.g. "patient_id").
    pub join_field: String,
    /// The lookup table name (e.g. "patients").
    pub lookup_table: String,
    /// The column on the lookup table that matches `join_field` (e.g. "id").
    pub lookup_id: String,
}

/// A single category within a disaggregation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    /// Unique identifier, e.g. "male".
    pub id: String,
    /// Display label, e.g. "Male".
    pub label: String,
    /// Filter that selects rows belonging to this category.
    pub filter: QueryFilter,
}
