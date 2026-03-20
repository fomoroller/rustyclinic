//! SQLite implementation of QueueEntryRepo.


use rusqlite::Connection;
use uuid::Uuid;
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_clinical::queue::{QueueEntry, QueueEntryRepo, QueueStatus};

pub struct SqliteQueueRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteQueueRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl QueueEntryRepo for SqliteQueueRepo<'_> {
    fn create(&self, entry: &QueueEntry) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO queue_entries (id, facility_id, patient_id, service_type, status, assigned_to, position, arrived_at, called_at, service_started_at, completed_at, created_at, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                rusqlite::params![
                    entry.id.to_string(),
                    entry.facility_id.to_string(),
                    entry.patient_id.to_string(),
                    entry.service_type,
                    entry.status.to_string(),
                    entry.assigned_to.map(|u| u.to_string()),
                    entry.position,
                    entry.arrived_at.to_rfc3339(),
                    entry.called_at.map(|t| t.to_rfc3339()),
                    entry.service_started_at.map(|t| t.to_rfc3339()),
                    entry.completed_at.map(|t| t.to_rfc3339()),
                    entry.created_at.to_rfc3339(),
                    entry.version,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<QueueEntry>> {
        let result = self.conn
            .query_row(
                "SELECT id, facility_id, patient_id, service_type, status, assigned_to, position, arrived_at, called_at, service_started_at, completed_at, created_at, version
                 FROM queue_entries WHERE id = ?1",
                rusqlite::params![id.to_string()],
                |row| Ok(row_to_queue_entry(row)),
            );

        match result {
            Ok(entry) => Ok(Some(entry.map_err(|e| AppError::Database(e.to_string()))?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<QueueEntry>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, facility_id, patient_id, service_type, status, assigned_to, position, arrived_at, called_at, service_started_at, completed_at, created_at, version
                 FROM queue_entries
                 WHERE facility_id = ?1 AND status NOT IN ('completed', 'cancelled', 'no_show')
                 ORDER BY position ASC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![facility_id.to_string()], |row| {
                Ok(row_to_queue_entry(row))
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut entries = Vec::new();
        for row in rows {
            let entry = row.map_err(|e| AppError::Database(e.to_string()))?
                          .map_err(|e| AppError::Database(e.to_string()))?;
            entries.push(entry);
        }
        Ok(entries)
    }

    fn update(&self, entry: &QueueEntry) -> AppResult<()> {
        let affected = self.conn
            .execute(
                "UPDATE queue_entries SET status=?1, assigned_to=?2, called_at=?3, service_started_at=?4, completed_at=?5, version=?6
                 WHERE id=?7 AND version=?8",
                rusqlite::params![
                    entry.status.to_string(),
                    entry.assigned_to.map(|u| u.to_string()),
                    entry.called_at.map(|t| t.to_rfc3339()),
                    entry.service_started_at.map(|t| t.to_rfc3339()),
                    entry.completed_at.map(|t| t.to_rfc3339()),
                    entry.version,
                    entry.id.to_string(),
                    entry.version - 1,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if affected == 0 {
            return Err(AppError::Conflict {
                message: "queue entry was modified concurrently".to_string(),
            });
        }
        Ok(())
    }

    fn next_position(&self, facility_id: Uuid) -> AppResult<u32> {
        let max: Option<u32> = self.conn
            .query_row(
                "SELECT MAX(position) FROM queue_entries WHERE facility_id = ?1 AND date(arrived_at) = date('now')",
                rusqlite::params![facility_id.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(max.unwrap_or(0) + 1)
    }
}

fn row_to_queue_entry(row: &rusqlite::Row) -> Result<QueueEntry, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let facility_str: String = row.get(1)?;
    let patient_str: String = row.get(2)?;
    let status_str: String = row.get(4)?;
    let assigned_str: Option<String> = row.get(5)?;
    let arrived_str: String = row.get(7)?;
    let called_str: Option<String> = row.get(8)?;
    let started_str: Option<String> = row.get(9)?;
    let completed_str: Option<String> = row.get(10)?;
    let created_str: String = row.get(11)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    Ok(QueueEntry {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        patient_id: Uuid::parse_str(&patient_str).unwrap_or_default(),
        service_type: row.get(3)?,
        status: match status_str.as_str() {
            "created" => QueueStatus::Created,
            "waiting" => QueueStatus::Waiting,
            "called" => QueueStatus::Called,
            "in_service" => QueueStatus::InService,
            "transferred" => QueueStatus::Transferred,
            "completed" => QueueStatus::Completed,
            "no_show" => QueueStatus::NoShow,
            "cancelled" => QueueStatus::Cancelled,
            _ => QueueStatus::Created,
        },
        assigned_to: assigned_str.and_then(|s| Uuid::parse_str(&s).ok()),
        position: row.get(6)?,
        arrived_at: parse_dt(&arrived_str),
        called_at: called_str.as_deref().map(parse_dt),
        service_started_at: started_str.as_deref().map(parse_dt),
        completed_at: completed_str.as_deref().map(parse_dt),
        created_at: parse_dt(&created_str),
        version: row.get(12)?,
    })
}
