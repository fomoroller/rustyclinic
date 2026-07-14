//! Tests for report generation, DHIS2 export, and CSV export.

#[cfg(test)]
mod test_cases {
    use std::collections::HashMap;

    use chrono::NaiveDate;
    use rusqlite::Connection;
    use uuid::Uuid;

    use crate::builtin::{monthly_opd_summary, monthly_service_delivery};
    use crate::dhis2::{Dhis2IndicatorMapping, Dhis2Mapping};
    use crate::engine::ReportEngine;

    /// Set up an in-memory database with migrations and seed data.
    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.pragma_update(None, "foreign_keys", "on").expect("fk");
        rustyclinic_db::migration::run_migrations(&conn).expect("migrations");
        conn
    }

    fn test_facility_id() -> Uuid {
        // Use a fixed UUID so all test data shares the same facility
        Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").expect("uuid")
    }

    fn test_provider_id() -> Uuid {
        Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").expect("uuid")
    }

    /// Insert test patients, queue entries, and encounters for March 2026.
    fn seed_test_data(conn: &Connection) {
        let facility_id = test_facility_id().to_string();
        let provider_id = test_provider_id().to_string();

        // Insert provider user (needed for FK on encounters)
        conn.execute(
            "INSERT INTO users (id, facility_id, username, display_name, password_hash, roles, active, created_at, updated_at)
             VALUES (?1, ?2, 'dr.test', 'Dr Test', 'hash', '[\"physician\"]', 1, '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z')",
            rusqlite::params![provider_id, facility_id],
        ).expect("insert user");

        // Insert patients: 2 male, 3 female, 1 other
        let patients = [
            ("p1", "Male", "1990-01-01"),
            ("p2", "Male", "1985-06-15"),
            ("p3", "Female", "1992-03-20"),
            ("p4", "Female", "2000-11-10"),
            ("p5", "Female", "1978-08-25"),
            ("p6", "Other", "1995-04-12"),
        ];

        for (i, (suffix, sex, dob)) in patients.iter().enumerate() {
            let pid = format!("00000000-0000-0000-0000-{:012}", i + 1);
            conn.execute(
                "INSERT INTO patients (id, facility_id, given_name, family_name, sex, date_of_birth, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, '2026-03-05T10:00:00Z', '2026-03-05T10:00:00Z')",
                rusqlite::params![pid, facility_id, format!("Patient{suffix}"), format!("Family{suffix}"), sex, dob],
            ).expect("insert patient");
        }

        // Insert queue entries: 5 completed, 1 no_show
        let queue_statuses = [
            ("completed", "q1"),
            ("completed", "q2"),
            ("completed", "q3"),
            ("completed", "q4"),
            ("completed", "q5"),
            ("no_show", "q6"),
        ];

        for (i, (status, _suffix)) in queue_statuses.iter().enumerate() {
            let qid = format!("10000000-0000-0000-0000-{:012}", i + 1);
            let pid = format!("00000000-0000-0000-0000-{:012}", i + 1);
            conn.execute(
                "INSERT INTO queue_entries (id, facility_id, patient_id, service_type, status, position, arrived_at, created_at)
                 VALUES (?1, ?2, ?3, 'consultation', ?4, ?5, '2026-03-10T08:00:00Z', '2026-03-10T08:00:00Z')",
                rusqlite::params![qid, facility_id, pid, status, i + 1],
            ).expect("insert queue entry");
        }

        // Insert encounters: 4 completed, 1 in_progress
        // Encounters for patients p1 (Male), p2 (Male), p3 (Female), p4 (Female), p5 (Female)
        let encounter_data = vec![
            (
                "e1",
                "00000000-0000-0000-0000-000000000001",
                "10000000-0000-0000-0000-000000000001",
                "completed",
            ),
            (
                "e2",
                "00000000-0000-0000-0000-000000000002",
                "10000000-0000-0000-0000-000000000002",
                "completed",
            ),
            (
                "e3",
                "00000000-0000-0000-0000-000000000003",
                "10000000-0000-0000-0000-000000000003",
                "completed",
            ),
            (
                "e4",
                "00000000-0000-0000-0000-000000000004",
                "10000000-0000-0000-0000-000000000004",
                "completed",
            ),
            (
                "e5",
                "00000000-0000-0000-0000-000000000005",
                "10000000-0000-0000-0000-000000000005",
                "in_progress",
            ),
        ];

        for (suffix, patient_id, queue_id, status) in &encounter_data {
            let eid = format!("20000000-0000-0000-0000-{:0>12}", suffix.replace('e', ""));
            conn.execute(
                "INSERT INTO encounters (id, facility_id, patient_id, queue_entry_id, provider_id, started_at, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, '2026-03-10T09:00:00Z', ?6, '2026-03-10T09:00:00Z')",
                rusqlite::params![eid, facility_id, patient_id, queue_id, provider_id, status],
            ).expect("insert encounter");
        }
    }

    // =========================================================================
    // Monthly OPD Summary tests
    // =========================================================================

    #[test]
    fn test_opd_encounters_completed_count() {
        let conn = setup_db();
        seed_test_data(&conn);

        let definition = monthly_opd_summary();
        let report = ReportEngine::generate(
            &conn,
            &definition,
            test_facility_id(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
        )
        .expect("generate report");

        let completed = report
            .indicators
            .iter()
            .find(|i| i.indicator_id == "opd_encounters_completed")
            .expect("find indicator");
        assert_eq!(completed.value, 4.0, "should have 4 completed encounters");
    }

    #[test]
    fn test_opd_encounters_total_count() {
        let conn = setup_db();
        seed_test_data(&conn);

        let definition = monthly_opd_summary();
        let report = ReportEngine::generate(
            &conn,
            &definition,
            test_facility_id(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
        )
        .expect("generate report");

        let total = report
            .indicators
            .iter()
            .find(|i| i.indicator_id == "opd_encounters_total")
            .expect("find indicator");
        assert_eq!(total.value, 5.0, "should have 5 total encounters");
    }

    #[test]
    fn test_opd_noshow_rate() {
        let conn = setup_db();
        seed_test_data(&conn);

        let definition = monthly_opd_summary();
        let report = ReportEngine::generate(
            &conn,
            &definition,
            test_facility_id(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
        )
        .expect("generate report");

        let noshow = report
            .indicators
            .iter()
            .find(|i| i.indicator_id == "opd_noshow_rate")
            .expect("find indicator");
        // 1 no_show / 6 total queue entries
        let expected = 1.0 / 6.0;
        assert!(
            (noshow.value - expected).abs() < 0.001,
            "no-show rate should be ~0.167, got {}",
            noshow.value
        );
    }

    #[test]
    fn test_opd_disaggregation_by_sex() {
        let conn = setup_db();
        seed_test_data(&conn);

        let definition = monthly_opd_summary();
        let report = ReportEngine::generate(
            &conn,
            &definition,
            test_facility_id(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
        )
        .expect("generate report");

        // Check sex disaggregation on total encounters indicator
        let total = report
            .indicators
            .iter()
            .find(|i| i.indicator_id == "opd_encounters_total")
            .expect("find indicator");

        let by_sex = total
            .disaggregated
            .get("by_sex")
            .expect("should have by_sex disaggregation");

        // 2 male patients have encounters (p1, p2)
        assert_eq!(
            by_sex.get("male").copied().unwrap_or(0.0),
            2.0,
            "should have 2 male encounters"
        );
        // 3 female patients have encounters (p3, p4, p5)
        assert_eq!(
            by_sex.get("female").copied().unwrap_or(0.0),
            3.0,
            "should have 3 female encounters"
        );
        // 0 other patients have encounters (p6 has no encounter)
        assert_eq!(
            by_sex.get("other").copied().unwrap_or(0.0),
            0.0,
            "should have 0 other encounters"
        );
    }

    // =========================================================================
    // Monthly Service Delivery tests
    // =========================================================================

    #[test]
    fn test_service_delivery_patients_registered() {
        let conn = setup_db();
        seed_test_data(&conn);

        let definition = monthly_service_delivery();
        let report = ReportEngine::generate(
            &conn,
            &definition,
            test_facility_id(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
        )
        .expect("generate report");

        let patients = report
            .indicators
            .iter()
            .find(|i| i.indicator_id == "sd_patients_registered")
            .expect("find indicator");
        assert_eq!(patients.value, 6.0, "should have 6 registered patients");
    }

    #[test]
    fn test_service_delivery_queue_completion_rate() {
        let conn = setup_db();
        seed_test_data(&conn);

        let definition = monthly_service_delivery();
        let report = ReportEngine::generate(
            &conn,
            &definition,
            test_facility_id(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
        )
        .expect("generate report");

        let rate = report
            .indicators
            .iter()
            .find(|i| i.indicator_id == "sd_queue_completion_rate")
            .expect("find indicator");
        // 5 completed / 6 total
        let expected = 5.0 / 6.0;
        assert!(
            (rate.value - expected).abs() < 0.001,
            "completion rate should be ~0.833, got {}",
            rate.value
        );
    }

    // =========================================================================
    // Empty data tests
    // =========================================================================

    #[test]
    fn test_opd_with_empty_data() {
        let conn = setup_db();
        // No seed data — empty tables

        let definition = monthly_opd_summary();
        let report = ReportEngine::generate(
            &conn,
            &definition,
            test_facility_id(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
        )
        .expect("generate report");

        for indicator in &report.indicators {
            assert_eq!(
                indicator.value, 0.0,
                "indicator {} should be 0 with no data",
                indicator.indicator_id
            );
        }
    }

    #[test]
    fn test_different_facility_sees_no_data() {
        let conn = setup_db();
        seed_test_data(&conn);

        let other_facility = Uuid::parse_str("ffffffff-ffff-ffff-ffff-ffffffffffff").expect("uuid");
        let definition = monthly_opd_summary();
        let report = ReportEngine::generate(
            &conn,
            &definition,
            other_facility,
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
        )
        .expect("generate report");

        let total = report
            .indicators
            .iter()
            .find(|i| i.indicator_id == "opd_encounters_total")
            .expect("find indicator");
        assert_eq!(total.value, 0.0, "different facility should see no data");
    }

    // =========================================================================
    // DHIS2 export tests
    // =========================================================================

    #[test]
    fn test_dhis2_export_json_structure() {
        let conn = setup_db();
        seed_test_data(&conn);

        let definition = monthly_opd_summary();
        let report = ReportEngine::generate(
            &conn,
            &definition,
            test_facility_id(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
        )
        .expect("generate report");

        let mut indicator_mappings = HashMap::new();
        indicator_mappings.insert(
            "opd_encounters_completed".to_string(),
            Dhis2IndicatorMapping {
                data_element: "DE_OPD_COMPLETED".to_string(),
                category_mappings: {
                    let mut cm = HashMap::new();
                    cm.insert("by_sex:male".to_string(), "COC_MALE".to_string());
                    cm.insert("by_sex:female".to_string(), "COC_FEMALE".to_string());
                    cm
                },
            },
        );
        indicator_mappings.insert(
            "opd_encounters_total".to_string(),
            Dhis2IndicatorMapping {
                data_element: "DE_OPD_TOTAL".to_string(),
                category_mappings: HashMap::new(),
            },
        );

        let mapping = Dhis2Mapping {
            data_set: "DS_MONTHLY_OPD".to_string(),
            org_unit: "OU_FACILITY_001".to_string(),
            indicator_mappings,
        };

        let dvs =
            crate::dhis2::to_dhis2(&report, &mapping, &definition.period_type).expect("to_dhis2");

        assert_eq!(dvs.data_set, "DS_MONTHLY_OPD");
        assert_eq!(dvs.period, "202603");
        assert_eq!(dvs.org_unit, "OU_FACILITY_001");

        // Should have data values: completed total + male + female, plus total
        assert!(
            dvs.data_values.len() >= 3,
            "should have at least 3 data values, got {}",
            dvs.data_values.len()
        );

        // Verify JSON serialization roundtrips
        let json_str = serde_json::to_string_pretty(&dvs).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("parse");
        assert!(parsed.get("dataSet").is_some());
        assert!(parsed.get("period").is_some());
        assert!(parsed.get("orgUnit").is_some());
        assert!(parsed.get("dataValues").is_some());
    }

    #[test]
    fn test_dhis2_period_format_monthly() {
        let report = crate::engine::GeneratedReport {
            definition_id: "test".to_string(),
            facility_id: test_facility_id(),
            period_start: NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            period_end: NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
            generated_at: chrono::Utc::now(),
            indicators: vec![],
        };

        let period = crate::dhis2::format_period(&report, &crate::definition::PeriodType::Monthly);
        assert_eq!(period, "202603");
    }

    #[test]
    fn test_dhis2_period_format_annual() {
        let report = crate::engine::GeneratedReport {
            definition_id: "test".to_string(),
            facility_id: test_facility_id(),
            period_start: NaiveDate::from_ymd_opt(2026, 1, 1).expect("date"),
            period_end: NaiveDate::from_ymd_opt(2026, 12, 31).expect("date"),
            generated_at: chrono::Utc::now(),
            indicators: vec![],
        };

        let period = crate::dhis2::format_period(&report, &crate::definition::PeriodType::Annual);
        assert_eq!(period, "2026");
    }

    // =========================================================================
    // CSV export tests
    // =========================================================================

    #[test]
    fn test_csv_export_content() {
        let conn = setup_db();
        seed_test_data(&conn);

        let definition = monthly_opd_summary();
        let report = ReportEngine::generate(
            &conn,
            &definition,
            test_facility_id(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
        )
        .expect("generate report");

        let csv_output = crate::csv::to_csv(&report);

        // Should contain header comment
        assert!(csv_output.contains("# Report: monthly-opd-summary"));
        // Should contain CSV header
        assert!(csv_output.contains("indicator_id,disaggregation,category,value"));
        // Should contain indicator values
        assert!(csv_output.contains("opd_encounters_completed,total,total,4"));
        assert!(csv_output.contains("opd_encounters_total,total,total,5"));
        // Should contain disaggregated values
        assert!(csv_output.contains("by_sex"));
        assert!(csv_output.contains("male"));
        assert!(csv_output.contains("female"));
    }

    #[test]
    fn test_csv_export_empty_data() {
        let conn = setup_db();

        let definition = monthly_opd_summary();
        let report = ReportEngine::generate(
            &conn,
            &definition,
            test_facility_id(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("date"),
            NaiveDate::from_ymd_opt(2026, 3, 31).expect("date"),
        )
        .expect("generate report");

        let csv_output = crate::csv::to_csv(&report);

        // All values should be 0
        assert!(csv_output.contains("opd_encounters_completed,total,total,0"));
        assert!(csv_output.contains("opd_encounters_total,total,total,0"));
    }
}
