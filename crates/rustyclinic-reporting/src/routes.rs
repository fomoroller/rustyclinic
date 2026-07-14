//! HTTP routes for report generation and export.
//!
//! Provides endpoints for listing available reports, generating them,
//! and exporting in DHIS2 JSON or CSV format.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::NaiveDate;
use serde::Deserialize;

use crate::builtin::all_builtin_reports;
use crate::definition::ReportDefinition;
use crate::engine::ReportEngine;

/// Shared state for reporting routes.
#[derive(Clone)]
pub struct ReportingState {
    inner: Arc<ReportingStateInner>,
}

struct ReportingStateInner {
    db_path: String,
}

impl ReportingState {
    /// Create a new reporting state with the given database path.
    pub fn new(db_path: String) -> Self {
        Self {
            inner: Arc::new(ReportingStateInner { db_path }),
        }
    }
}

/// Query parameters for report generation.
#[derive(Deserialize)]
pub struct GenerateParams {
    pub definition_id: String,
    pub facility_id: String,
    pub period_start: String,
    pub period_end: String,
}

/// Query parameters for DHIS2 export.
#[derive(Deserialize)]
pub struct Dhis2ExportParams {
    pub definition_id: String,
    pub facility_id: String,
    pub period_start: String,
    pub period_end: String,
    /// JSON-encoded DHIS2 mapping configuration.
    pub mapping: String,
}

/// Query parameters for CSV export.
#[derive(Deserialize)]
pub struct CsvExportParams {
    pub definition_id: String,
    pub facility_id: String,
    pub period_start: String,
    pub period_end: String,
}

fn load_active_report_definitions(conn: &rusqlite::Connection) -> Vec<ReportDefinition> {
    let mut definitions = all_builtin_reports();

    let mut stmt = match conn.prepare(
        "SELECT package_report_artifacts.report_json
         FROM package_report_artifacts
         INNER JOIN installed_packages ON installed_packages.id = package_report_artifacts.package_row_id
         WHERE installed_packages.status = 'activated'
         ORDER BY installed_packages.activated_at DESC, package_report_artifacts.report_id ASC",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return definitions,
    };

    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
        Ok(rows) => rows,
        Err(_) => return definitions,
    };

    for json in rows.flatten() {
        match serde_json::from_str::<ReportDefinition>(&json) {
            Ok(definition) => definitions.push(definition),
            Err(error) => {
                tracing::warn!(error = %error, "failed to parse packaged report definition")
            }
        }
    }

    definitions
}

/// List all available report definitions.
pub async fn list_definitions(State(state): State<ReportingState>) -> impl IntoResponse {
    let conn = match rusqlite::Connection::open(&state.inner.db_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("db error: {e}") })),
            );
        }
    };

    let definitions = load_active_report_definitions(&conn);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "definitions": definitions })),
    )
}

/// Generate a report for the given parameters.
pub async fn generate_report(
    State(state): State<ReportingState>,
    Query(params): Query<GenerateParams>,
) -> impl IntoResponse {
    let facility_id = match uuid::Uuid::parse_str(&params.facility_id) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid facility_id: {e}") })),
            );
        }
    };

    let period_start = match NaiveDate::parse_from_str(&params.period_start, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid period_start: {e}") })),
            );
        }
    };

    let period_end = match NaiveDate::parse_from_str(&params.period_end, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid period_end: {e}") })),
            );
        }
    };

    let conn = match rusqlite::Connection::open(&state.inner.db_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("db error: {e}") })),
            );
        }
    };

    let definitions = load_active_report_definitions(&conn);
    let definition = match definitions.iter().find(|d| d.id == params.definition_id) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "report definition not found" })),
            );
        }
    };

    match ReportEngine::generate(&conn, definition, facility_id, period_start, period_end) {
        Ok(report) => (
            StatusCode::OK,
            Json(serde_json::json!({ "report": report })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("report generation failed: {e}") })),
        ),
    }
}

/// Export a report in DHIS2 JSON format.
pub async fn export_dhis2(
    State(state): State<ReportingState>,
    Query(params): Query<Dhis2ExportParams>,
) -> impl IntoResponse {
    let facility_id = match uuid::Uuid::parse_str(&params.facility_id) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid facility_id: {e}") })),
            );
        }
    };

    let period_start = match NaiveDate::parse_from_str(&params.period_start, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid period_start: {e}") })),
            );
        }
    };

    let period_end = match NaiveDate::parse_from_str(&params.period_end, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid period_end: {e}") })),
            );
        }
    };

    let mapping: crate::dhis2::Dhis2Mapping = match serde_json::from_str(&params.mapping) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid mapping JSON: {e}") })),
            );
        }
    };

    let conn = match rusqlite::Connection::open(&state.inner.db_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("db error: {e}") })),
            );
        }
    };

    let definitions = load_active_report_definitions(&conn);
    let definition = match definitions.iter().find(|d| d.id == params.definition_id) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "report definition not found" })),
            );
        }
    };

    let report =
        match ReportEngine::generate(&conn, definition, facility_id, period_start, period_end) {
            Ok(r) => r,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("report generation failed: {e}") })),
                );
            }
        };

    match crate::dhis2::to_dhis2(&report, &mapping, &definition.period_type) {
        Ok(dvs) => (StatusCode::OK, Json(serde_json::json!(dvs))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("DHIS2 export failed: {e}") })),
        ),
    }
}

/// Export a report as CSV.
pub async fn export_csv(
    State(state): State<ReportingState>,
    Query(params): Query<CsvExportParams>,
) -> impl IntoResponse {
    let facility_id = match uuid::Uuid::parse_str(&params.facility_id) {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::response::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(axum::body::Body::from(format!("invalid facility_id: {e}")))
                    .expect("response"),
            );
        }
    };

    let period_start = match NaiveDate::parse_from_str(&params.period_start, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::response::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(axum::body::Body::from(format!("invalid period_start: {e}")))
                    .expect("response"),
            );
        }
    };

    let period_end = match NaiveDate::parse_from_str(&params.period_end, "%Y-%m-%d") {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::response::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(axum::body::Body::from(format!("invalid period_end: {e}")))
                    .expect("response"),
            );
        }
    };

    let conn = match rusqlite::Connection::open(&state.inner.db_path) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::response::Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(axum::body::Body::from(format!("db error: {e}")))
                    .expect("response"),
            );
        }
    };

    let definitions = load_active_report_definitions(&conn);
    let definition = match definitions.iter().find(|d| d.id == params.definition_id) {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                axum::response::Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(axum::body::Body::from("report definition not found"))
                    .expect("response"),
            );
        }
    };

    let report =
        match ReportEngine::generate(&conn, definition, facility_id, period_start, period_end) {
            Ok(r) => r,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::response::Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(axum::body::Body::from(format!(
                            "report generation failed: {e}"
                        )))
                        .expect("response"),
                );
            }
        };

    let csv_content = crate::csv::to_csv(&report);

    (
        StatusCode::OK,
        axum::response::Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/csv")
            .header(
                "Content-Disposition",
                format!(
                    "attachment; filename=\"{}_{}_{}.csv\"",
                    report.definition_id, report.period_start, report.period_end
                ),
            )
            .body(axum::body::Body::from(csv_content))
            .expect("response"),
    )
}

/// Build the reporting router with all routes.
pub fn reporting_router(state: ReportingState) -> axum::Router {
    axum::Router::new()
        .route(
            "/api/reports/definitions",
            axum::routing::get(list_definitions),
        )
        .route("/api/reports/generate", axum::routing::get(generate_report))
        .route(
            "/api/reports/export/dhis2",
            axum::routing::get(export_dhis2),
        )
        .route("/api/reports/export/csv", axum::routing::get(export_csv))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        rustyclinic_db::migration::run_migrations(&conn).expect("migrations");
        conn
    }

    #[test]
    fn load_active_report_definitions_includes_only_activated_packages() {
        let conn = setup_db();
        let package_row_id = uuid::Uuid::now_v7();
        let staged_row_id = uuid::Uuid::now_v7();
        let facility_id = uuid::Uuid::now_v7();
        let installed_by = uuid::Uuid::now_v7();
        let now = chrono::Utc::now().to_rfc3339();

        let activated_manifest = serde_json::json!({
            "package_id": "report-pack-activated",
            "package_type": "report",
            "version": "1.0.0",
            "compatible_versions": "*",
            "dependencies": [],
            "effective_start": null,
            "effective_end": null,
            "scope": "facility",
            "checksum": "abc",
            "localization_coverage": []
        })
        .to_string();
        let staged_manifest = serde_json::json!({
            "package_id": "report-pack-staged",
            "package_type": "report",
            "version": "1.0.1",
            "compatible_versions": "*",
            "dependencies": [],
            "effective_start": null,
            "effective_end": null,
            "scope": "facility",
            "checksum": "def",
            "localization_coverage": []
        })
        .to_string();

        conn.execute(
            "INSERT INTO installed_packages (id, facility_id, package_id, package_type, version, status, manifest, installed_at, activated_at, rolled_back_at, installed_by, version_num)
             VALUES (?1, ?2, 'report-pack-activated', 'report', '1.0.0', 'activated', ?3, ?4, ?4, NULL, ?5, 1)",
            rusqlite::params![package_row_id.to_string(), facility_id.to_string(), activated_manifest, now, installed_by.to_string()],
        )
        .expect("insert activated package");
        conn.execute(
            "INSERT INTO installed_packages (id, facility_id, package_id, package_type, version, status, manifest, installed_at, activated_at, rolled_back_at, installed_by, version_num)
             VALUES (?1, ?2, 'report-pack-staged', 'report', '1.0.1', 'staged', ?3, ?4, NULL, NULL, ?5, 1)",
            rusqlite::params![staged_row_id.to_string(), facility_id.to_string(), staged_manifest, now, installed_by.to_string()],
        )
        .expect("insert staged package");

        let packaged_report = serde_json::json!({
            "id": "packaged-opd-report",
            "title": "Packaged OPD Report",
            "period_type": "Monthly",
            "indicators": [],
            "disaggregations": []
        })
        .to_string();
        let staged_report = serde_json::json!({
            "id": "staged-report",
            "title": "Should Stay Hidden",
            "period_type": "Monthly",
            "indicators": [],
            "disaggregations": []
        })
        .to_string();

        conn.execute(
            "INSERT INTO package_report_artifacts (package_row_id, report_id, report_family, report_version, report_json)
             VALUES (?1, 'packaged-opd-report', NULL, '1.0.0', ?2)",
            rusqlite::params![package_row_id.to_string(), packaged_report],
        )
        .expect("insert active report artifact");
        conn.execute(
            "INSERT INTO package_report_artifacts (package_row_id, report_id, report_family, report_version, report_json)
             VALUES (?1, 'staged-report', NULL, '1.0.1', ?2)",
            rusqlite::params![staged_row_id.to_string(), staged_report],
        )
        .expect("insert staged report artifact");

        let definitions = load_active_report_definitions(&conn);
        assert!(
            definitions
                .iter()
                .any(|definition| definition.id == "packaged-opd-report")
        );
        assert!(
            !definitions
                .iter()
                .any(|definition| definition.id == "staged-report")
        );
    }
}
