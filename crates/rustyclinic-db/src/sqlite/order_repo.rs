//! SQLite implementation of LabOrderRepo, LabTestRepo, MedicationDispenseRepo, and DispenseItemRepo.

use rusqlite::Connection;
use rustyclinic_clinical::Priority;
use rustyclinic_clinical::lab::{LabOrder, LabOrderRepo, LabStatus, LabTest, LabTestRepo};
use rustyclinic_clinical::pharmacy::{
    DispenseItem, DispenseItemRepo, DispenseStatus, MedicationDispense, MedicationDispenseRepo,
};
use rustyclinic_core::error::{AppError, AppResult};
use uuid::Uuid;

// ===== Lab Order Repo =====

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
        let mut stmt = self.conn
            .prepare(
                "SELECT id, encounter_id, patient_id, facility_id, status, priority, ordered_by, specimen_type, collected_at, collected_by, resulted_at, resulted_by, verified_at, verified_by, notes, created_at, version
                 FROM lab_orders WHERE patient_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_lab_orders(&mut stmt, rusqlite::params![patient_id.to_string()])
    }

    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<LabOrder>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, encounter_id, patient_id, facility_id, status, priority, ordered_by, specimen_type, collected_at, collected_by, resulted_at, resulted_by, verified_at, verified_by, notes, created_at, version
                 FROM lab_orders WHERE encounter_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_lab_orders(&mut stmt, rusqlite::params![encounter_id.to_string()])
    }

    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<LabOrder>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, encounter_id, patient_id, facility_id, status, priority, ordered_by, specimen_type, collected_at, collected_by, resulted_at, resulted_by, verified_at, verified_by, notes, created_at, version
                 FROM lab_orders
                 WHERE facility_id = ?1 AND status NOT IN ('verified', 'cancelled')
                 ORDER BY created_at ASC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_lab_orders(&mut stmt, rusqlite::params![facility_id.to_string()])
    }

    fn update(&self, order: &LabOrder) -> AppResult<()> {
        let affected = self.conn
            .execute(
                "UPDATE lab_orders SET status=?1, specimen_type=?2, collected_at=?3, collected_by=?4, resulted_at=?5, resulted_by=?6, verified_at=?7, verified_by=?8, notes=?9, version=?10
                 WHERE id=?11 AND version=?12",
                rusqlite::params![
                    order.status.to_string(),
                    order.specimen_type,
                    order.collected_at.map(|t| t.to_rfc3339()),
                    order.collected_by.map(|u| u.to_string()),
                    order.resulted_at.map(|t| t.to_rfc3339()),
                    order.resulted_by.map(|u| u.to_string()),
                    order.verified_at.map(|t| t.to_rfc3339()),
                    order.verified_by.map(|u| u.to_string()),
                    order.notes,
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

fn collect_lab_orders(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> AppResult<Vec<LabOrder>> {
    let rows = stmt
        .query_map(params, |row| Ok(row_to_lab_order(row)))
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut result = Vec::new();
    for row in rows {
        let order = row
            .map_err(|e| AppError::Database(e.to_string()))?
            .map_err(|e| AppError::Database(e.to_string()))?;
        result.push(order);
    }
    Ok(result)
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
    let collected_str: Option<String> = row.get(8)?;
    let collected_by_str: Option<String> = row.get(9)?;
    let resulted_str: Option<String> = row.get(10)?;
    let resulted_by_str: Option<String> = row.get(11)?;
    let verified_str: Option<String> = row.get(12)?;
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
        tests: vec![], // Tests loaded separately via LabTestRepo
        specimen_type,
        collected_at: collected_str.as_deref().map(parse_dt),
        collected_by: collected_by_str.and_then(|s| Uuid::parse_str(&s).ok()),
        resulted_at: resulted_str.as_deref().map(parse_dt),
        resulted_by: resulted_by_str.and_then(|s| Uuid::parse_str(&s).ok()),
        verified_at: verified_str.as_deref().map(parse_dt),
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

// ===== Medication Dispense Repo =====

pub struct SqliteMedicationDispenseRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteMedicationDispenseRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl MedicationDispenseRepo for SqliteMedicationDispenseRepo<'_> {
    fn create(&self, dispense: &MedicationDispense) -> AppResult<()> {
        self.conn
            .execute(
                "INSERT INTO medication_dispenses (id, encounter_id, patient_id, facility_id, status, priority, prescribed_by, dispensed_by, notes, created_at, prepared_at, dispensed_at, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                rusqlite::params![
                    dispense.id.to_string(),
                    dispense.encounter_id.to_string(),
                    dispense.patient_id.to_string(),
                    dispense.facility_id.to_string(),
                    dispense.status.to_string(),
                    dispense.priority.to_string(),
                    dispense.prescribed_by.to_string(),
                    dispense.dispensed_by.map(|u| u.to_string()),
                    dispense.notes,
                    dispense.created_at.to_rfc3339(),
                    dispense.prepared_at.map(|t| t.to_rfc3339()),
                    dispense.dispensed_at.map(|t| t.to_rfc3339()),
                    dispense.version,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    fn find_by_id(&self, id: Uuid) -> AppResult<Option<MedicationDispense>> {
        let result = self.conn.query_row(
            "SELECT id, encounter_id, patient_id, facility_id, status, priority, prescribed_by, dispensed_by, notes, created_at, prepared_at, dispensed_at, version
             FROM medication_dispenses WHERE id = ?1",
            rusqlite::params![id.to_string()],
            |row| Ok(row_to_dispense(row)),
        );
        match result {
            Ok(d) => Ok(Some(d.map_err(|e| AppError::Database(e.to_string()))?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(AppError::Database(e.to_string())),
        }
    }

    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<MedicationDispense>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, encounter_id, patient_id, facility_id, status, priority, prescribed_by, dispensed_by, notes, created_at, prepared_at, dispensed_at, version
                 FROM medication_dispenses WHERE patient_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_dispenses(&mut stmt, rusqlite::params![patient_id.to_string()])
    }

    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<MedicationDispense>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, encounter_id, patient_id, facility_id, status, priority, prescribed_by, dispensed_by, notes, created_at, prepared_at, dispensed_at, version
                 FROM medication_dispenses WHERE encounter_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_dispenses(&mut stmt, rusqlite::params![encounter_id.to_string()])
    }

    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<MedicationDispense>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT id, encounter_id, patient_id, facility_id, status, priority, prescribed_by, dispensed_by, notes, created_at, prepared_at, dispensed_at, version
                 FROM medication_dispenses
                 WHERE facility_id = ?1 AND status NOT IN ('dispensed', 'returned', 'voided')
                 ORDER BY created_at ASC",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        collect_dispenses(&mut stmt, rusqlite::params![facility_id.to_string()])
    }

    fn update(&self, dispense: &MedicationDispense) -> AppResult<()> {
        let affected = self.conn
            .execute(
                "UPDATE medication_dispenses SET status=?1, dispensed_by=?2, notes=?3, prepared_at=?4, dispensed_at=?5, version=?6
                 WHERE id=?7 AND version=?8",
                rusqlite::params![
                    dispense.status.to_string(),
                    dispense.dispensed_by.map(|u| u.to_string()),
                    dispense.notes,
                    dispense.prepared_at.map(|t| t.to_rfc3339()),
                    dispense.dispensed_at.map(|t| t.to_rfc3339()),
                    dispense.version,
                    dispense.id.to_string(),
                    dispense.version - 1,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        if affected == 0 {
            return Err(AppError::Conflict {
                message: "medication dispense was modified concurrently".to_string(),
            });
        }
        Ok(())
    }
}

fn collect_dispenses(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> AppResult<Vec<MedicationDispense>> {
    let rows = stmt
        .query_map(params, |row| Ok(row_to_dispense(row)))
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut result = Vec::new();
    for row in rows {
        let d = row
            .map_err(|e| AppError::Database(e.to_string()))?
            .map_err(|e| AppError::Database(e.to_string()))?;
        result.push(d);
    }
    Ok(result)
}

fn row_to_dispense(row: &rusqlite::Row) -> Result<MedicationDispense, rusqlite::Error> {
    let id_str: String = row.get(0)?;
    let encounter_str: String = row.get(1)?;
    let patient_str: String = row.get(2)?;
    let facility_str: String = row.get(3)?;
    let status_str: String = row.get(4)?;
    let priority_str: String = row.get(5)?;
    let prescribed_by_str: String = row.get(6)?;
    let dispensed_by_str: Option<String> = row.get(7)?;
    let notes: Option<String> = row.get(8)?;
    let created_str: String = row.get(9)?;
    let prepared_str: Option<String> = row.get(10)?;
    let dispensed_str: Option<String> = row.get(11)?;
    let version: u32 = row.get(12)?;

    let parse_dt = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };

    Ok(MedicationDispense {
        id: Uuid::parse_str(&id_str).unwrap_or_default(),
        encounter_id: Uuid::parse_str(&encounter_str).unwrap_or_default(),
        patient_id: Uuid::parse_str(&patient_str).unwrap_or_default(),
        facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
        status: DispenseStatus::from_str_safe(&status_str),
        priority: Priority::from_str_safe(&priority_str),
        prescribed_by: Uuid::parse_str(&prescribed_by_str).unwrap_or_default(),
        dispensed_by: dispensed_by_str.and_then(|s| Uuid::parse_str(&s).ok()),
        items: vec![], // Items loaded separately via DispenseItemRepo
        notes,
        created_at: parse_dt(&created_str),
        prepared_at: prepared_str.as_deref().map(parse_dt),
        dispensed_at: dispensed_str.as_deref().map(parse_dt),
        version,
    })
}

// ===== Dispense Item Repo =====

pub struct SqliteDispenseItemRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteDispenseItemRepo<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

impl DispenseItemRepo for SqliteDispenseItemRepo<'_> {
    fn create_items(&self, dispense_id: Uuid, items: &[DispenseItem]) -> AppResult<()> {
        for item in items {
            self.conn
                .execute(
                    "INSERT INTO dispense_items (dispense_id, medication_name, medication_system, medication_code, medication_display, dosage, frequency, duration, quantity, dispensed_quantity, substituted, substitution_reason)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    rusqlite::params![
                        dispense_id.to_string(),
                        item.medication_name,
                        item.medication_system,
                        item.medication_code,
                        item.medication_display,
                        item.dosage,
                        item.frequency,
                        item.duration,
                        item.quantity,
                        item.dispensed_quantity,
                        item.substituted as i32,
                        item.substitution_reason,
                    ],
                )
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        Ok(())
    }

    fn find_by_dispense(&self, dispense_id: Uuid) -> AppResult<Vec<DispenseItem>> {
        let mut stmt = self.conn
            .prepare(
                "SELECT medication_name, medication_system, medication_code, medication_display, dosage, frequency, duration, quantity, dispensed_quantity, substituted, substitution_reason
                 FROM dispense_items WHERE dispense_id = ?1",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![dispense_id.to_string()], |row| {
                let substituted_int: i32 = row.get(9)?;
                Ok(DispenseItem {
                    medication_name: row.get(0)?,
                    medication_system: row.get(1)?,
                    medication_code: row.get(2)?,
                    medication_display: row.get(3)?,
                    dosage: row.get(4)?,
                    frequency: row.get(5)?,
                    duration: row.get(6)?,
                    quantity: row.get(7)?,
                    dispensed_quantity: row.get(8)?,
                    substituted: substituted_int != 0,
                    substitution_reason: row.get(10)?,
                })
            })
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row.map_err(|e| AppError::Database(e.to_string()))?);
        }
        Ok(items)
    }

    fn update_dispensed(
        &self,
        dispense_id: Uuid,
        medication_name: &str,
        dispensed_quantity: u32,
        substituted: bool,
        substitution_reason: Option<&str>,
    ) -> AppResult<()> {
        self.conn
            .execute(
                "UPDATE dispense_items SET dispensed_quantity=?1, substituted=?2, substitution_reason=?3
                 WHERE dispense_id=?4 AND medication_name=?5",
                rusqlite::params![
                    dispensed_quantity,
                    substituted as i32,
                    substitution_reason,
                    dispense_id.to_string(),
                    medication_name,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}
