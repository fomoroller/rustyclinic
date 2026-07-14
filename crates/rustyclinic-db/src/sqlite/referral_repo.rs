//! SQLite implementation of ReferralRepo.

use rusqlite::Connection;
use rustyclinic_clinical::Priority;
use rustyclinic_clinical::referral::{Referral, ReferralRepo, ReferralStatus};
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

pub struct SqliteReferralRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteReferralRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl ReferralRepo for SqliteReferralRepo<'_> {
    fn create(&self, referral: &Referral) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO referrals (id, encounter_id, patient_id, facility_id, status, priority, referred_by, referred_to_facility, referred_to_department, reason, clinical_summary, sent_at, received_at, accepted_at, completed_at, notes, created_at, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
                rusqlite::params![
                    referral.id.to_string(),
                    referral.encounter_id.to_string(),
                    referral.patient_id.to_string(),
                    referral.facility_id.to_string(),
                    referral.status.to_string(),
                    referral.priority.to_string(),
                    referral.referred_by.to_string(),
                    referral.referred_to_facility,
                    referral.referred_to_department,
                    referral.reason,
                    referral.clinical_summary,
                    referral.sent_at.map(|t| t.to_rfc3339()),
                    referral.received_at.map(|t| t.to_rfc3339()),
                    referral.accepted_at.map(|t| t.to_rfc3339()),
                    referral.completed_at.map(|t| t.to_rfc3339()),
                    referral.notes,
                    referral.created_at.to_rfc3339(),
                    referral.version,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<Referral>> {
        let result = self.conn.query_row(
            "SELECT id, encounter_id, patient_id, facility_id, status, priority, referred_by, referred_to_facility, referred_to_department, reason, clinical_summary, sent_at, received_at, accepted_at, completed_at, notes, created_at, version
             FROM referrals WHERE id = ?1",
            rusqlite::params![id.to_string()],
            |row| Ok(row_to_referral(row)),
        );

        match result {
            Ok(r) => Ok(Some(r.map_err(|e| AppError::Database(e.to_string()))?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Referral>> {
        query_referrals(
            self.conn,
            "SELECT id, encounter_id, patient_id, facility_id, status, priority, referred_by, referred_to_facility, referred_to_department, reason, clinical_summary, sent_at, received_at, accepted_at, completed_at, notes, created_at, version
             FROM referrals WHERE patient_id = ?1 ORDER BY created_at DESC",
            rusqlite::params![patient_id.to_string()],
        )
    }

    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<Referral>> {
        query_referrals(
            self.conn,
            "SELECT id, encounter_id, patient_id, facility_id, status, priority, referred_by, referred_to_facility, referred_to_department, reason, clinical_summary, sent_at, received_at, accepted_at, completed_at, notes, created_at, version
             FROM referrals WHERE encounter_id = ?1 ORDER BY created_at ASC",
            rusqlite::params![encounter_id.to_string()],
        )
    }

    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<Referral>> {
        query_referrals(
            self.conn,
            "SELECT id, encounter_id, patient_id, facility_id, status, priority, referred_by, referred_to_facility, referred_to_department, reason, clinical_summary, sent_at, received_at, accepted_at, completed_at, notes, created_at, version
             FROM referrals WHERE facility_id = ?1 AND status NOT IN ('completed', 'declined', 'cancelled')
             ORDER BY created_at ASC",
            rusqlite::params![facility_id.to_string()],
        )
    }

    fn update(&self, referral: &Referral) -> AppResult<()> {
        let affected = self.conn
            .execute(
                "UPDATE referrals SET status=?1, sent_at=?2, received_at=?3, accepted_at=?4, completed_at=?5, notes=?6, version=?7
                 WHERE id=?8 AND version=?9",
                rusqlite::params![
                    referral.status.to_string(),
                    referral.sent_at.map(|t| t.to_rfc3339()),
                    referral.received_at.map(|t| t.to_rfc3339()),
                    referral.accepted_at.map(|t| t.to_rfc3339()),
                    referral.completed_at.map(|t| t.to_rfc3339()),
                    referral.notes,
                    referral.version,
                    referral.id.to_string(),
                    referral.version - 1,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if affected == 0 {
            return Err(AppError::Conflict {
                message: "referral was modified concurrently".to_string(),
            });
        }
        Ok(())
    }
}

fn query_referrals(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> AppResult<Vec<Referral>> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| AppError::Database(e.to_string()))?;
    let rows = stmt
        .query_map(params, |row| Ok(row_to_referral(row)))
        .map_err(|e| AppError::Database(e.to_string()))?;

    let mut results = Vec::new();
    for row in rows {
        let r = row
            .map_err(|e| AppError::Database(e.to_string()))?
            .map_err(|e| AppError::Database(e.to_string()))?;
        results.push(r);
    }
    Ok(results)
}

fn row_to_referral(row: &rusqlite::Row) -> Result<Referral, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let encounter_str: String = row.get(1)?;
    let patient_str: String = row.get(2)?;
    let facility_str: String = row.get(3)?;
    let status_str: String = row.get(4)?;
    let priority_str: String = row.get(5)?;
    let referred_by_str: String = row.get(6)?;
    let referred_to_facility: Option<String> = row.get(7)?;
    let referred_to_department: Option<String> = row.get(8)?;
    let reason: String = row.get(9)?;
    let clinical_summary: Option<String> = row.get(10)?;
    let sent_at_str: Option<String> = row.get(11)?;
    let received_at_str: Option<String> = row.get(12)?;
    let accepted_at_str: Option<String> = row.get(13)?;
    let completed_at_str: Option<String> = row.get(14)?;
    let notes: Option<String> = row.get(15)?;
    let created_str: String = row.get(16)?;
    let version: u32 = row.get(17)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    Ok(Referral {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        encounter_id: Uuid::parse_str(&encounter_str).unwrap_or_default(),
        patient_id: Uuid::parse_str(&patient_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        status: ReferralStatus::from_str_safe(&status_str),
        priority: Priority::from_str_safe(&priority_str),
        referred_by: Uuid::parse_str(&referred_by_str).unwrap_or_default(),
        referred_to_facility,
        referred_to_department,
        reason,
        clinical_summary,
        sent_at: sent_at_str.as_deref().map(parse_dt),
        received_at: received_at_str.as_deref().map(parse_dt),
        accepted_at: accepted_at_str.as_deref().map(parse_dt),
        completed_at: completed_at_str.as_deref().map(parse_dt),
        notes,
        created_at: parse_dt(&created_str),
        version,
    })
}
