//! Facility settings: read and upsert key-value configuration.

use chrono::Utc;
use rusqlite::Connection;
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

/// Upsert a facility setting.
pub fn update_setting(
    conn: &Connection,
    facility_id: Uuid,
    key: &str,
    value: &str,
) -> AppResult<()> {
    let now = Utc::now();
    conn.execute(
        "INSERT INTO facility_settings (facility_id, key, value, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(facility_id, key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        rusqlite::params![
            facility_id.to_string(),
            key,
            value,
            now.to_rfc3339(),
        ],
    )
    .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(())
}

/// Read a facility setting. Returns None if not set.
pub fn get_setting(conn: &Connection, facility_id: Uuid, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM facility_settings WHERE facility_id = ?1 AND key = ?2",
        rusqlite::params![facility_id.to_string(), key],
        |row| row.get(0),
    )
    .ok()
}
