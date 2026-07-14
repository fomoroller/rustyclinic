use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rustyclinic_core::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub facility_id: Option<Uuid>,
    pub job_type: String,
    pub payload: JsonValue,
    pub status: JobStatus,
    pub run_at: DateTime<Utc>,
    pub attempts: u32,
    pub max_attempts: u32,
    pub leased_by_device_id: Option<Uuid>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct SqliteJobRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteJobRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn enqueue(
        &self,
        facility_id: Option<Uuid>,
        job_type: &str,
        payload: JsonValue,
        run_at: DateTime<Utc>,
        max_attempts: u32,
    ) -> AppResult<Uuid> {
        let id = Uuid::now_v7();
        let now = Utc::now().to_rfc3339();

        self.conn
            .execute(
                "INSERT INTO jobs (id, facility_id, job_type, payload, status, run_at, attempts, max_attempts, leased_by_device_id, lease_expires_at, last_heartbeat_at, last_error, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, NULL, NULL, NULL, NULL, ?8, ?8)",
                rusqlite::params![
                    id.to_string(),
                    facility_id.map(|u| u.to_string()),
                    job_type,
                    payload.to_string(),
                    JobStatus::Pending.as_str(),
                    run_at.to_rfc3339(),
                    max_attempts as i64,
                    now,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(id)
    }

    pub fn ensure_singleton_due(
        &self,
        facility_id: Option<Uuid>,
        job_type: &str,
        run_at: DateTime<Utc>,
        payload: JsonValue,
        max_attempts: u32,
    ) -> AppResult<()> {
        let fid = facility_id.map(|u| u.to_string());
        let exists: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM jobs
                 WHERE job_type = ?1
                   AND facility_id IS ?2
                   AND status IN ('pending', 'running')
                 ORDER BY created_at DESC
                 LIMIT 1",
                rusqlite::params![job_type, fid],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        if exists.is_some() {
            return Ok(());
        }

        let _ = self.enqueue(facility_id, job_type, payload, run_at, max_attempts)?;
        Ok(())
    }

    pub fn try_lease_next(
        &self,
        device_id: Uuid,
        lease_duration: Duration,
    ) -> AppResult<Option<Job>> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let lease_until = (now + lease_duration).to_rfc3339();

        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| AppError::Database(e.to_string()))?;

        let candidate: Option<(String, u32)> = tx
            .query_row(
                "SELECT id, attempts FROM jobs
                 WHERE
                   (status = 'pending' AND run_at <= ?1)
                   OR
                   (status = 'running' AND lease_expires_at IS NOT NULL AND lease_expires_at < ?1)
                 ORDER BY run_at ASC
                 LIMIT 1",
                rusqlite::params![now_str],
                |row| Ok((row.get(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()
            .map_err(|e| AppError::Database(e.to_string()))?;

        let Some((job_id, attempts)) = candidate else {
            tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
            return Ok(None);
        };

        let changed = tx
            .execute(
                "UPDATE jobs
                 SET status = 'running',
                     leased_by_device_id = ?2,
                     lease_expires_at = ?3,
                     last_heartbeat_at = ?1,
                     attempts = ?4,
                     updated_at = ?1
                 WHERE id = ?5
                   AND attempts = ?6
                   AND (status = 'pending' OR (status = 'running' AND lease_expires_at < ?1))",
                rusqlite::params![
                    now_str,
                    device_id.to_string(),
                    lease_until,
                    (attempts + 1) as i64,
                    job_id,
                    attempts as i64,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if changed == 0 {
            tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
            return Ok(None);
        }

        let job = tx
            .query_row(
                "SELECT id, facility_id, job_type, payload, status, run_at, attempts, max_attempts,
                        leased_by_device_id, lease_expires_at, last_heartbeat_at, last_error,
                        created_at, updated_at
                 FROM jobs WHERE id = ?1",
                rusqlite::params![job_id],
                row_to_job,
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
        Ok(Some(job))
    }

    pub fn heartbeat(
        &self,
        job_id: Uuid,
        device_id: Uuid,
        lease_duration: Duration,
    ) -> AppResult<()> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let lease_until = (now + lease_duration).to_rfc3339();

        let changed = self
            .conn
            .execute(
                "UPDATE jobs
                 SET lease_expires_at = ?1, last_heartbeat_at = ?2, updated_at = ?2
                 WHERE id = ?3 AND status = 'running' AND leased_by_device_id = ?4",
                rusqlite::params![
                    lease_until,
                    now_str,
                    job_id.to_string(),
                    device_id.to_string(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(AppError::Conflict {
                message: "job not leased by this device".to_string(),
            });
        }

        Ok(())
    }

    pub fn succeed(&self, job_id: Uuid, device_id: Uuid) -> AppResult<()> {
        self.finish(job_id, device_id, JobStatus::Succeeded, None)
    }

    pub fn fail(&self, job_id: Uuid, device_id: Uuid, error: String) -> AppResult<()> {
        self.finish(job_id, device_id, JobStatus::Failed, Some(error))
    }

    fn finish(
        &self,
        job_id: Uuid,
        device_id: Uuid,
        status: JobStatus,
        error: Option<String>,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();

        let changed = self
            .conn
            .execute(
                "UPDATE jobs
                 SET status = ?1,
                     leased_by_device_id = NULL,
                     lease_expires_at = NULL,
                     last_error = ?2,
                     updated_at = ?3
                 WHERE id = ?4 AND status = 'running' AND leased_by_device_id = ?5",
                rusqlite::params![
                    status.as_str(),
                    error,
                    now,
                    job_id.to_string(),
                    device_id.to_string(),
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(AppError::Conflict {
                message: "job not leased by this device".to_string(),
            });
        }

        Ok(())
    }
}

fn row_to_job(row: &rusqlite::Row) -> Result<Job, rusqlite::Error> {
    let id: String = row.get(0)?;
    let facility_id: Option<String> = row.get(1)?;
    let job_type: String = row.get(2)?;
    let payload: String = row.get(3)?;
    let status: String = row.get(4)?;
    let run_at: String = row.get(5)?;
    let attempts: u32 = row.get(6)?;
    let max_attempts: u32 = row.get(7)?;
    let leased_by: Option<String> = row.get(8)?;
    let lease_expires_at: Option<String> = row.get(9)?;
    let last_heartbeat_at: Option<String> = row.get(10)?;
    let last_error: Option<String> = row.get(11)?;
    let created_at: String = row.get(12)?;
    let updated_at: String = row.get(13)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now())
    };

    let payload: JsonValue = serde_json::from_str(&payload).unwrap_or(JsonValue::Null);

    Ok(Job {
        id: Uuid::parse_str(&id).unwrap_or_default(),
        facility_id: facility_id.and_then(|s| Uuid::parse_str(&s).ok()),
        job_type,
        payload,
        status: JobStatus::parse(&status),
        run_at: parse_dt(&run_at),
        attempts,
        max_attempts,
        leased_by_device_id: leased_by.and_then(|s| Uuid::parse_str(&s).ok()),
        lease_expires_at: lease_expires_at.as_deref().map(parse_dt),
        last_heartbeat_at: last_heartbeat_at.as_deref().map(parse_dt),
        last_error,
        created_at: parse_dt(&created_at),
        updated_at: parse_dt(&updated_at),
    })
}
