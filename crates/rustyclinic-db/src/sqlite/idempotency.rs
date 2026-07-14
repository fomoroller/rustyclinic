//! Idempotency record management.

use chrono::{Duration, Utc};
use rusqlite::Connection;
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

/// Check if an idempotency key has been used. If so, return the cached response.
pub fn check_idempotency(
    conn: &Connection,
    facility_id: Uuid,
    key: &str,
) -> AppResult<Option<serde_json::Value>> {
    let result = conn.query_row(
        "SELECT response, expires_at FROM idempotency_records WHERE key = ?1 AND facility_id = ?2",
        rusqlite::params![key, facility_id.to_string()],
        |row| {
            let response: String = row.get(0)?;
            let expires_str: String = row.get(1)?;
            Ok((response, expires_str))
        },
    );

    match result {
        Ok((response, expires_str)) => {
            // Check if expired
            if let Ok(expires) = chrono::DateTime::parse_from_rfc3339(&expires_str)
                && Utc::now() > expires.with_timezone(&Utc)
            {
                // Expired — delete and allow retry
                let _ = conn.execute(
                    "DELETE FROM idempotency_records WHERE key = ?1 AND facility_id = ?2",
                    rusqlite::params![key, facility_id.to_string()],
                );
                return Ok(None);
            }
            let value: serde_json::Value =
                serde_json::from_str(&response).map_err(|e| AppError::Database(e.to_string()))?;
            Ok(Some(value))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(AppError::Database(e.to_string())),
    }
}

/// Store an idempotency record (called as part of the unit-of-work commit).
pub fn store_idempotency(
    conn: &Connection,
    facility_id: Uuid,
    key: &str,
    response: &serde_json::Value,
) -> AppResult<()> {
    let now = Utc::now();
    let expires = now + Duration::hours(24);

    conn.execute(
        "INSERT OR REPLACE INTO idempotency_records (key, facility_id, response, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            key,
            facility_id.to_string(),
            response.to_string(),
            now.to_rfc3339(),
            expires.to_rfc3339(),
        ],
    ).map_err(|e| AppError::Database(e.to_string()))?;

    Ok(())
}
