//! Aggregate reporting and DHIS2-compatible data export.
//!
//! This crate provides:
//! - Data-driven report definitions (indicators, disaggregations)
//! - A SQL-based report engine that generates aggregate reports
//! - Built-in report templates for Rwanda MoH compliance
//! - DHIS2 Web API JSON export for national HMIS submission
//! - CSV export for offline use
//! - HTTP routes for report generation and export

pub mod builtin;
pub mod csv;
pub mod definition;
pub mod dhis2;
pub mod engine;
pub mod routes;

#[cfg(test)]
mod tests;
