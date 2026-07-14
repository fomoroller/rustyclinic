//! SQLite implementation of LabOrderRepo and LabTestRepo.

use rusqlite::Connection;
use rustyclinic_clinical::Priority;
use rustyclinic_clinical::lab::{LabOrder, LabOrderRepo, LabStatus, LabTest, LabTestRepo};
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

pub struct SqliteLabOrderRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteLabOrderRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl LabOrderRepo for SqliteLabOrderRepo<'_> {
    fn create(&self, order: &LabOrder) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO lab_orders (id, encounter_id, patient_id, facility_id, status, priority, ordered_by, specimen_type, collected_at, collected_by, resulted_at, resulted_by, verified_at, verified_by, notes, created_at, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                rusqlite::params![
                    order.id.to_string(),
                    order.encounter_id.to_string(),
                    order.patient_id.to_string(),
                    order.facility_id.to_string(),
                    order.status.to_string(),
                    order.priority.to_string(),
                    order.ordered_by.to_string(),
                    order.specimen_type,
                    order.collected_at.map(|t| t.to_rfc3339()),
                    order.collected_by.map(|u| u.to_string()),
                    order.resulted_at.map(|t| t.to_rfc3339()),
                    order.resulted_by.map(|u| u.to_string()),
                    order.verified_at.map(|t| t.to_rfc3339()),
                    order.verified_by.map(|u| u.to_string()),
                    order.notes,
                    order.created_at.to_rfc3339(),
                    order.version,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<LabOrder>> {
        let result = self.conn.query_row(
            "SELECT id, encounter_id, patient_id, facility_id, status, priority, ordered_by, specimen_type, collected_at, collected_by, resulted_at, resulted_by, verified_at, verified_by, notes, created_at, version
             FROM lab_orders WHERE id = ?1",
            rusqlite::params![id.to_string()],
            |row| Ok(row_to_lab_order(row)),
        );

        match result {
            Ok(order) => Ok(Some(order.map_err(|e| AppError::Database(e.to_string()))?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<LabOrder>> {
        query_lab_orders(
            self.conn,
            "SELECT id, encounter_id, patient_id, facility_id, status, priority, ordered_by, specimen_type, collected_at, collected_by, resulted_at, resulted_by, verified_at, verified_by, notes, created_at, version
             FROM lab_orders WHERE patient_id = ?1 ORDER BY created_at DESC",
            rusqlite::params![patient_id.to_string()],
        )
    }

    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<LabOrder>> {
        query_lab_orders(
            self.conn,
            "SELECT id, encounter_id, patient_id, facility_id, status, priority, ordered_by, specimen_type, collected_at, collected_by, resulted_at, resulted_by, verified_at, verified_by, notes, created_at, version
             FROM lab_orders WHERE encounter_id = ?1 ORDER BY created_at ASC",
            rusqlite::params![encounter_id.to_string()],
        )
    }

    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<LabOrder>> {
        query_lab_orders(
            self.conn,
            "SELECT id, encounter_id, patient_id, facility_id, status, priority, ordered_by, specimen_type, collected_at, collected_by, resulted_at, resulted_by, verified_at, verified_by, notes, created_at, version
             FROM lab_orders WHERE facility_id = ?1 AND status NOT IN ('verified', 'cancelled')
             ORDER BY CASE priority WHEN 'stat' THEN 0 WHEN 'urgent' THEN 1 ELSE 2 END, created_at ASC",
            rusqlite::params![facility_id.to_string()],
        )
    }

    fn update(&self, order: &LabOrder) -> AppResult<()> {
        let affected = self.conn
            .execute(
                "UPDATE lab_orders SET status=?1, collected_at=?2, collected_by=?3, resulted_at=?4, resulted_by=?5, verified_at=?6, verified_by=?7, notes=?8, specimen_type=?9, version=?10
                 WHERE id=?11 AND version=?12",
                rusqlite::params![
                    order.status.to_string(),
                    order.collected_at.map(|t| t.to_rfc3339()),
                    order.collected_by.map(|u| u.to_string()),
                    order.resulted_at.map(|t| t.to_rfc3339()),
                    order.resulted_by.map(|u| u.to_string()),
                    order.verified_at.map(|t| t.to_rfc3339()),
                    order.verified_by.map(|u| u.to_string()),
                    order.notes,
                    order.specimen_type,
                    order.version,
                    order.id.to_string(),
                    order.version - 1,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        if affected == 0 {
            return Err(AppError::Conflict {
                message: "lab order was modified concurrently".to_string(),
            });
        }
        Ok(())
    }
}

fn query_lab_orders(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> AppResult<Vec<LabOrder>> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| AppError::Database(e.to_string()))?;
    let rows = stmt
        .query_map(params, |row| Ok(row_to_lab_order(row)))
        .map_err(|e| AppError::Database(e.to_string()))?;

    let mut orders = Vec::new();
    for row in rows {
        let order = row
            .map_err(|e| AppError::Database(e.to_string()))?
            .map_err(|e| AppError::Database(e.to_string()))?;
        orders.push(order);
    }
    Ok(orders)
}

fn row_to_lab_order(row: &rusqlite::Row) -> Result<LabOrder, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let encounter_str: String = row.get(1)?;
    let patient_str: String = row.get(2)?;
    let facility_str: String = row.get(3)?;
    let status_str: String = row.get(4)?;
    let priority_str: String = row.get(5)?;
    let ordered_by_str: String = row.get(6)?;
    let specimen_type: Option<String> = row.get(7)?;
    let collected_at_str: Option<String> = row.get(8)?;
    let collected_by_str: Option<String> = row.get(9)?;
    let resulted_at_str: Option<String> = row.get(10)?;
    let resulted_by_str: Option<String> = row.get(11)?;
    let verified_at_str: Option<String> = row.get(12)?;
    let verified_by_str: Option<String> = row.get(13)?;
    let notes: Option<String> = row.get(14)?;
    let created_str: String = row.get(15)?;
    let version: u32 = row.get(16)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    Ok(LabOrder {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        encounter_id: Uuid::parse_str(&encounter_str).unwrap_or_default(),
        patient_id: Uuid::parse_str(&patient_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        status: LabStatus::from_str_safe(&status_str),
        priority: Priority::from_str_safe(&priority_str),
        ordered_by: Uuid::parse_str(&ordered_by_str).unwrap_or_default(),
        tests: vec![], // Tests loaded separately
        specimen_type,
        collected_at: collected_at_str.as_deref().map(parse_dt),
        collected_by: collected_by_str.and_then(|s| Uuid::parse_str(&s).ok()),
        resulted_at: resulted_at_str.as_deref().map(parse_dt),
        resulted_by: resulted_by_str.and_then(|s| Uuid::parse_str(&s).ok()),
        verified_at: verified_at_str.as_deref().map(parse_dt),
        verified_by: verified_by_str.and_then(|s| Uuid::parse_str(&s).ok()),
        notes,
        created_at: parse_dt(&created_str),
        version,
    })
}

// ===== Lab Test Repo =====

pub struct SqliteLabTestRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteLabTestRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl LabTestRepo for SqliteLabTestRepo<'_> {
    fn create_tests(&self, order_id: Uuid, tests: &[LabTest]) -> AppResult<()> {
        for test in tests {
            self.conn
                .execute(
                    "INSERT INTO lab_tests (order_id, test_code, test_name, result, result_value, unit, reference_range, is_abnormal, resulted_at, resulted_by)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        order_id.to_string(),
                        test.test_code,
                        test.test_name,
                        test.result,
                        test.result_value,
                        test.unit,
                        test.reference_range,
                        test.is_abnormal as i32,
                        test.resulted_at.map(|t| t.to_rfc3339()),
                        test.resulted_by.map(|u| u.to_string()),
                    ],
                )
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        Ok(())
    }

    fn find_by_order(&self, order_id: Uuid) -> AppResult<Vec<LabTest>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT test_code, test_name, result, result_value, unit, reference_range, is_abnormal, resulted_at, resulted_by
                 FROM lab_tests WHERE order_id = ?1",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![order_id.to_string()], |row| {
                let is_abnormal_int: i32 = row.get(6)?;
                let resulted_str: Option<String> = row.get(7)?;
                let resulted_by_str: Option<String> = row.get(8)?;
                Ok(LabTest {
                    test_code: row.get(0)?,
                    test_name: row.get(1)?,
                    result: row.get(2)?,
                    result_value: row.get(3)?,
                    unit: row.get(4)?,
                    reference_range: row.get(5)?,
                    is_abnormal: is_abnormal_int != 0,
                    resulted_at: resulted_str.map(|s| {
                        chrono::DateTime::parse_from_rfc3339(&s)
                            .map(|dt| dt.with_timezone(&chrono::Utc))
                            .unwrap_or_else(|_| chrono::Utc::now())
                    }),
                    resulted_by: resulted_by_str.and_then(|s| Uuid::parse_str(&s).ok()),
                })
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut tests = Vec::new();
        for row in rows {
            tests.push(row.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(tests)
    }

    fn update_test(&self, order_id: Uuid, test: &LabTest) -> AppResult<()> {
        self.conn
            .execute(
                "UPDATE lab_tests SET result=?1, result_value=?2, unit=?3, reference_range=?4, is_abnormal=?5, resulted_at=?6, resulted_by=?7
                 WHERE order_id=?8 AND test_code=?9",
                rusqlite::params![
                    test.result,
                    test.result_value,
                    test.unit,
                    test.reference_range,
                    test.is_abnormal as i32,
                    test.resulted_at.map(|t| t.to_rfc3339()),
                    test.resulted_by.map(|u| u.to_string()),
                    order_id.to_string(),
                    test.test_code,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}
