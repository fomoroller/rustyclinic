//! Schema migrations for SQLite.

use anyhow::Result;
use rusqlite::Connection;

/// Run all pending migrations.
pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS _migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
    ",
    )?;

    let applied: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT version FROM _migrations ORDER BY version")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    for (i, m) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as i64;
        if !applied.contains(&version) {
            tracing::info!(version, name = m.name, "applying migration");
            conn.execute_batch(m.sql)?;
            conn.execute(
                "INSERT INTO _migrations (version, name) VALUES (?1, ?2)",
                rusqlite::params![version, m.name],
            )?;
        }
    }

    Ok(())
}

struct Migration {
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        name: "create_patients",
        sql: "
            CREATE TABLE patients (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                given_name TEXT NOT NULL,
                family_name TEXT NOT NULL,
                sex TEXT NOT NULL,
                date_of_birth TEXT,
                phone TEXT,
                address TEXT,
                national_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_patients_facility ON patients(facility_id);
            CREATE INDEX idx_patients_name ON patients(family_name, given_name);
            CREATE INDEX idx_patients_national_id ON patients(national_id) WHERE national_id IS NOT NULL;
        ",
    },
    Migration {
        name: "create_queue_entries",
        sql: "
            CREATE TABLE queue_entries (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                service_type TEXT NOT NULL,
                status TEXT NOT NULL,
                assigned_to TEXT,
                position INTEGER NOT NULL,
                arrived_at TEXT NOT NULL,
                called_at TEXT,
                service_started_at TEXT,
                completed_at TEXT,
                created_at TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_queue_facility_status ON queue_entries(facility_id, status);
        ",
    },
    Migration {
        name: "create_audit_log",
        sql: "
            CREATE TABLE audit_log (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                device_id TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                action TEXT NOT NULL,
                aggregate_type TEXT NOT NULL,
                aggregate_id TEXT NOT NULL,
                payload TEXT NOT NULL,
                prev_hash BLOB,
                entry_hash BLOB NOT NULL
            );
            CREATE INDEX idx_audit_facility ON audit_log(facility_id, timestamp);
        ",
    },
    Migration {
        name: "create_outbox",
        sql: "
            CREATE TABLE outbox_events (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                aggregate_type TEXT NOT NULL,
                aggregate_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at TEXT NOT NULL,
                published INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_outbox_unpublished ON outbox_events(published) WHERE published = 0;
        ",
    },
    Migration {
        name: "create_idempotency",
        sql: "
            CREATE TABLE idempotency_records (
                key TEXT NOT NULL,
                facility_id TEXT NOT NULL,
                response TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                PRIMARY KEY (key, facility_id)
            );
        ",
    },
    Migration {
        name: "create_op_log",
        sql: "
            CREATE TABLE op_log (
                id TEXT PRIMARY KEY NOT NULL,
                sequence INTEGER NOT NULL,
                facility_id TEXT NOT NULL,
                device_id TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                aggregate_type TEXT NOT NULL,
                aggregate_id TEXT NOT NULL,
                payload TEXT NOT NULL,
                prev_hash BLOB,
                entry_hash BLOB NOT NULL,
                sync_state TEXT NOT NULL DEFAULT 'pending'
            );
            CREATE UNIQUE INDEX idx_op_log_sequence ON op_log(facility_id, sequence);
        ",
    },
    Migration {
        name: "create_form_drafts",
        sql: "
            CREATE TABLE form_drafts (
                user_id TEXT NOT NULL,
                encounter_id TEXT NOT NULL,
                form_family TEXT NOT NULL,
                form_version TEXT NOT NULL,
                field_values TEXT NOT NULL,
                saved_at TEXT NOT NULL,
                PRIMARY KEY (user_id, encounter_id, form_family, form_version)
            );
        ",
    },
    Migration {
        name: "create_users",
        sql: "
            CREATE TABLE users (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                username TEXT NOT NULL,
                display_name TEXT NOT NULL,
                password_hash TEXT NOT NULL,
                roles TEXT NOT NULL DEFAULT '[]',
                active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE UNIQUE INDEX idx_users_facility_username ON users(facility_id, username);
        ",
    },
    Migration {
        name: "create_sessions",
        sql: "
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY NOT NULL,
                user_id TEXT NOT NULL REFERENCES users(id),
                facility_id TEXT NOT NULL,
                device_id TEXT NOT NULL,
                roles TEXT NOT NULL DEFAULT '[]',
                auth_method TEXT NOT NULL,
                state TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                last_active TEXT NOT NULL,
                locked_at TEXT
            );
            CREATE INDEX idx_sessions_device ON sessions(device_id, state);
        ",
    },
    Migration {
        name: "create_local_identity",
        sql: "
            CREATE TABLE local_identity (
                id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
                facility_id TEXT NOT NULL,
                device_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
        ",
    },
    Migration {
        name: "create_installed_packages",
        sql: "
            CREATE TABLE installed_packages (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                package_id TEXT NOT NULL,
                package_type TEXT NOT NULL,
                version TEXT NOT NULL,
                status TEXT NOT NULL,
                manifest TEXT NOT NULL,
                installed_at TEXT NOT NULL,
                activated_at TEXT,
                rolled_back_at TEXT,
                installed_by TEXT NOT NULL,
                version_num INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_packages_facility ON installed_packages(facility_id, status);
            CREATE UNIQUE INDEX idx_packages_active ON installed_packages(facility_id, package_id, version)
                WHERE status = 'activated';
        ",
    },
    Migration {
        name: "add_user_pin_hash",
        sql: "ALTER TABLE users ADD COLUMN pin_hash TEXT;",
    },
    Migration {
        name: "create_sync_cursors",
        sql: "
            CREATE TABLE sync_cursors (
                device_id TEXT NOT NULL,
                facility_id TEXT NOT NULL,
                last_pulled_sequence INTEGER NOT NULL DEFAULT 0,
                last_pushed_sequence INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (device_id, facility_id)
            );
        ",
    },
    Migration {
        name: "create_sync_conflicts",
        sql: "
            CREATE TABLE sync_conflicts (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                aggregate_type TEXT NOT NULL,
                aggregate_id TEXT NOT NULL,
                local_entry_id TEXT NOT NULL,
                remote_entry_id TEXT NOT NULL,
                conflict_type TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL,
                resolved_at TEXT,
                resolved_by TEXT,
                resolution TEXT
            );
            CREATE INDEX idx_sync_conflicts_facility ON sync_conflicts(facility_id, status);
        ",
    },
    Migration {
        name: "create_jobs",
        sql: "
            CREATE TABLE jobs (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT,
                job_type TEXT NOT NULL,
                payload TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL,
                run_at TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 10,
                leased_by_device_id TEXT,
                lease_expires_at TEXT,
                last_heartbeat_at TEXT,
                last_error TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX idx_jobs_ready ON jobs(status, run_at);
            CREATE INDEX idx_jobs_lease_expires ON jobs(lease_expires_at) WHERE lease_expires_at IS NOT NULL;
        ",
    },
    Migration {
        name: "create_encounters",
        sql: "
            CREATE TABLE encounters (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                queue_entry_id TEXT REFERENCES queue_entries(id),
                provider_id TEXT NOT NULL REFERENCES users(id),
                started_at TEXT NOT NULL,
                ended_at TEXT,
                visit_notes TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'in_progress',
                created_at TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_encounters_patient ON encounters(patient_id);
            CREATE INDEX idx_encounters_queue ON encounters(queue_entry_id);
        ",
    },
    Migration {
        name: "create_generated_reports",
        sql: "
            CREATE TABLE generated_reports (
                id TEXT PRIMARY KEY NOT NULL,
                definition_id TEXT NOT NULL,
                facility_id TEXT NOT NULL,
                period_start TEXT NOT NULL,
                period_end TEXT NOT NULL,
                generated_at TEXT NOT NULL,
                data TEXT NOT NULL,
                created_by TEXT
            );
            CREATE INDEX idx_generated_reports_facility ON generated_reports(facility_id, definition_id);
        ",
    },
    Migration {
        name: "add_queue_department_and_encounter",
        sql: "
            ALTER TABLE queue_entries ADD COLUMN department TEXT NOT NULL DEFAULT 'consultation';
            ALTER TABLE queue_entries ADD COLUMN encounter_id TEXT;
            CREATE INDEX idx_queue_department ON queue_entries(facility_id, department, status);
            CREATE INDEX idx_queue_encounter ON queue_entries(encounter_id);
        ",
    },
    Migration {
        name: "create_service_orders",
        sql: "
            CREATE TABLE service_orders (
                id TEXT PRIMARY KEY NOT NULL,
                encounter_id TEXT NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                facility_id TEXT NOT NULL,
                order_type TEXT NOT NULL,
                status TEXT NOT NULL,
                priority TEXT NOT NULL DEFAULT 'routine',
                ordered_by TEXT NOT NULL,
                details TEXT NOT NULL DEFAULT '{}',
                notes TEXT,
                created_at TEXT NOT NULL,
                completed_at TEXT,
                completed_by TEXT,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_service_orders_encounter ON service_orders(encounter_id);
            CREATE INDEX idx_service_orders_facility_type ON service_orders(facility_id, order_type, status);
            CREATE INDEX idx_service_orders_patient ON service_orders(patient_id);
        ",
    },
    Migration {
        name: "create_lab_results",
        sql: "
            CREATE TABLE lab_results (
                order_id TEXT NOT NULL REFERENCES service_orders(id),
                test_code TEXT NOT NULL,
                test_name TEXT NOT NULL,
                result TEXT,
                result_value REAL,
                unit TEXT,
                reference_range TEXT,
                is_abnormal INTEGER NOT NULL DEFAULT 0,
                resulted_at TEXT,
                resulted_by TEXT,
                PRIMARY KEY (order_id, test_code)
            );
        ",
    },
    Migration {
        name: "create_prescription_items",
        sql: "
            CREATE TABLE prescription_items (
                order_id TEXT NOT NULL REFERENCES service_orders(id),
                medication_name TEXT NOT NULL,
                dosage TEXT NOT NULL,
                frequency TEXT NOT NULL,
                duration TEXT NOT NULL,
                quantity INTEGER NOT NULL,
                dispensed_quantity INTEGER,
                substituted INTEGER NOT NULL DEFAULT 0,
                substitution_reason TEXT,
                PRIMARY KEY (order_id, medication_name)
            );
        ",
    },
    Migration {
        name: "create_coverage",
        sql: "
            CREATE TABLE coverage (
                id TEXT PRIMARY KEY NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                facility_id TEXT NOT NULL,
                payer_id TEXT NOT NULL,
                payer_name TEXT NOT NULL,
                member_id TEXT NOT NULL,
                plan_name TEXT,
                effective_start TEXT NOT NULL,
                effective_end TEXT,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX idx_coverage_patient ON coverage(patient_id);
            CREATE INDEX idx_coverage_facility ON coverage(facility_id, payer_id);
        ",
    },
    Migration {
        name: "create_eligibility_checks",
        sql: "
            CREATE TABLE eligibility_checks (
                id TEXT PRIMARY KEY NOT NULL,
                coverage_id TEXT NOT NULL REFERENCES coverage(id),
                patient_id TEXT NOT NULL REFERENCES patients(id),
                facility_id TEXT NOT NULL,
                checked_at TEXT NOT NULL,
                is_eligible INTEGER NOT NULL,
                denial_reason TEXT,
                checked_by TEXT NOT NULL
            );
            CREATE INDEX idx_eligibility_coverage ON eligibility_checks(coverage_id);
        ",
    },
    Migration {
        name: "create_tariffs",
        sql: "
            CREATE TABLE tariffs (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                service_code TEXT NOT NULL,
                service_name TEXT NOT NULL,
                unit_price REAL NOT NULL,
                currency TEXT NOT NULL DEFAULT 'RWF',
                effective_start TEXT NOT NULL,
                effective_end TEXT,
                payer_id TEXT
            );
            CREATE INDEX idx_tariffs_facility ON tariffs(facility_id, service_code);
            CREATE INDEX idx_tariffs_payer ON tariffs(facility_id, payer_id) WHERE payer_id IS NOT NULL;
        ",
    },
    Migration {
        name: "create_claim_cases",
        sql: "
            CREATE TABLE claim_cases (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                encounter_id TEXT,
                payer_id TEXT NOT NULL,
                claim_number TEXT,
                status TEXT NOT NULL,
                total_amount REAL NOT NULL,
                approved_amount REAL,
                items TEXT NOT NULL DEFAULT '[]',
                submitted_at TEXT,
                adjudicated_at TEXT,
                paid_at TEXT,
                rejection_reason TEXT,
                created_at TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_claims_patient ON claim_cases(patient_id);
            CREATE INDEX idx_claims_facility_status ON claim_cases(facility_id, status);
            CREATE INDEX idx_claims_encounter ON claim_cases(encounter_id) WHERE encounter_id IS NOT NULL;
        ",
    },
    Migration {
        name: "create_payments",
        sql: "
            CREATE TABLE payments (
                id TEXT PRIMARY KEY NOT NULL,
                facility_id TEXT NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                encounter_id TEXT,
                claim_id TEXT,
                amount REAL NOT NULL,
                currency TEXT NOT NULL DEFAULT 'RWF',
                method TEXT NOT NULL,
                reference_number TEXT,
                received_by TEXT NOT NULL,
                received_at TEXT NOT NULL,
                notes TEXT
            );
            CREATE INDEX idx_payments_patient ON payments(patient_id);
            CREATE INDEX idx_payments_encounter ON payments(encounter_id) WHERE encounter_id IS NOT NULL;
            CREATE INDEX idx_payments_claim ON payments(claim_id) WHERE claim_id IS NOT NULL;
        ",
    },
    Migration {
        name: "create_waivers",
        sql: "
            CREATE TABLE waivers (
                id TEXT PRIMARY KEY NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                facility_id TEXT NOT NULL,
                encounter_id TEXT,
                amount_waived REAL NOT NULL,
                reason TEXT NOT NULL,
                approved_by TEXT NOT NULL,
                approved_at TEXT NOT NULL,
                notes TEXT
            );
            CREATE INDEX idx_waivers_patient ON waivers(patient_id);
        ",
    },
    Migration {
        name: "create_lab_orders",
        sql: "
            CREATE TABLE lab_orders (
                id TEXT PRIMARY KEY NOT NULL,
                encounter_id TEXT NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                facility_id TEXT NOT NULL,
                status TEXT NOT NULL,
                priority TEXT NOT NULL DEFAULT 'routine',
                ordered_by TEXT NOT NULL,
                specimen_type TEXT,
                collected_at TEXT,
                collected_by TEXT,
                resulted_at TEXT,
                resulted_by TEXT,
                verified_at TEXT,
                verified_by TEXT,
                notes TEXT,
                created_at TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_lab_orders_encounter ON lab_orders(encounter_id);
            CREATE INDEX idx_lab_orders_patient ON lab_orders(patient_id);
            CREATE INDEX idx_lab_orders_facility_status ON lab_orders(facility_id, status);
        ",
    },
    Migration {
        name: "create_lab_tests",
        sql: "
            CREATE TABLE lab_tests (
                order_id TEXT NOT NULL REFERENCES lab_orders(id),
                test_code TEXT NOT NULL,
                test_name TEXT NOT NULL,
                result TEXT,
                result_value REAL,
                unit TEXT,
                reference_range TEXT,
                is_abnormal INTEGER NOT NULL DEFAULT 0,
                resulted_at TEXT,
                resulted_by TEXT,
                PRIMARY KEY (order_id, test_code)
            );
        ",
    },
    Migration {
        name: "create_medication_dispenses",
        sql: "
            CREATE TABLE medication_dispenses (
                id TEXT PRIMARY KEY NOT NULL,
                encounter_id TEXT NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                facility_id TEXT NOT NULL,
                status TEXT NOT NULL,
                priority TEXT NOT NULL DEFAULT 'routine',
                prescribed_by TEXT NOT NULL,
                dispensed_by TEXT,
                notes TEXT,
                created_at TEXT NOT NULL,
                prepared_at TEXT,
                dispensed_at TEXT,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_med_dispenses_encounter ON medication_dispenses(encounter_id);
            CREATE INDEX idx_med_dispenses_patient ON medication_dispenses(patient_id);
            CREATE INDEX idx_med_dispenses_facility_status ON medication_dispenses(facility_id, status);
        ",
    },
    Migration {
        name: "create_dispense_items",
        sql: "
            CREATE TABLE dispense_items (
                dispense_id TEXT NOT NULL REFERENCES medication_dispenses(id),
                medication_name TEXT NOT NULL,
                dosage TEXT NOT NULL,
                frequency TEXT NOT NULL,
                duration TEXT NOT NULL,
                quantity INTEGER NOT NULL,
                dispensed_quantity INTEGER,
                substituted INTEGER NOT NULL DEFAULT 0,
                substitution_reason TEXT,
                PRIMARY KEY (dispense_id, medication_name)
            );
        ",
    },
    Migration {
        name: "create_admissions",
        sql: "
            CREATE TABLE admissions (
                id TEXT PRIMARY KEY NOT NULL,
                encounter_id TEXT NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                facility_id TEXT NOT NULL,
                status TEXT NOT NULL,
                ward TEXT NOT NULL,
                bed TEXT,
                admitted_by TEXT NOT NULL,
                admitted_at TEXT,
                transferred_to_ward TEXT,
                transferred_at TEXT,
                discharged_at TEXT,
                discharged_by TEXT,
                discharge_reason TEXT,
                notes TEXT,
                created_at TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_admissions_encounter ON admissions(encounter_id);
            CREATE INDEX idx_admissions_patient ON admissions(patient_id);
            CREATE INDEX idx_admissions_facility_status ON admissions(facility_id, status);
        ",
    },
    Migration {
        name: "create_referrals",
        sql: "
            CREATE TABLE referrals (
                id TEXT PRIMARY KEY NOT NULL,
                encounter_id TEXT NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                facility_id TEXT NOT NULL,
                status TEXT NOT NULL,
                priority TEXT NOT NULL DEFAULT 'routine',
                referred_by TEXT NOT NULL,
                referred_to_facility TEXT,
                referred_to_department TEXT,
                reason TEXT NOT NULL,
                clinical_summary TEXT,
                sent_at TEXT,
                received_at TEXT,
                accepted_at TEXT,
                completed_at TEXT,
                notes TEXT,
                created_at TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_referrals_encounter ON referrals(encounter_id);
            CREATE INDEX idx_referrals_patient ON referrals(patient_id);
            CREATE INDEX idx_referrals_facility_status ON referrals(facility_id, status);
        ",
    },
    Migration {
        name: "create_facility_settings",
        sql: "
            CREATE TABLE facility_settings (
                facility_id TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (facility_id, key)
            );
        ",
    },
    Migration {
        name: "create_program_enrollments",
        sql: "
            CREATE TABLE program_enrollments (
                id TEXT PRIMARY KEY NOT NULL,
                patient_id TEXT NOT NULL REFERENCES patients(id),
                facility_id TEXT NOT NULL,
                program_code TEXT NOT NULL,
                program_name TEXT NOT NULL,
                status TEXT NOT NULL,
                enrolled_by TEXT NOT NULL,
                enrolled_at TEXT,
                activated_at TEXT,
                paused_at TEXT,
                completed_at TEXT,
                withdrawn_at TEXT,
                notes TEXT,
                created_at TEXT NOT NULL,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_enrollments_patient ON program_enrollments(patient_id);
            CREATE INDEX idx_enrollments_facility_status ON program_enrollments(facility_id, status);
        ",
    },
    Migration {
        name: "create_terminology_tables",
        sql: "
            CREATE TABLE terminology_concepts (
                system TEXT NOT NULL,
                code TEXT NOT NULL,
                display TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                properties TEXT NOT NULL DEFAULT '{}',
                imported_at TEXT NOT NULL,
                PRIMARY KEY (system, code)
            );
            CREATE INDEX idx_terminology_display ON terminology_concepts(system, display);

            CREATE TABLE terminology_designations (
                system TEXT NOT NULL,
                code TEXT NOT NULL,
                language TEXT NOT NULL DEFAULT 'en',
                use_type TEXT NOT NULL DEFAULT 'synonym',
                value TEXT NOT NULL,
                PRIMARY KEY (system, code, language, use_type, value)
            );
            CREATE INDEX idx_terminology_designation_value ON terminology_designations(system, value);

            CREATE TABLE terminology_artifacts (
                system TEXT NOT NULL,
                resource_type TEXT NOT NULL,
                canonical_url TEXT NOT NULL,
                version TEXT,
                payload TEXT NOT NULL,
                imported_at TEXT NOT NULL,
                PRIMARY KEY (system, resource_type, canonical_url, version)
            );

            CREATE TABLE terminology_import_runs (
                id TEXT PRIMARY KEY NOT NULL,
                system TEXT NOT NULL,
                source TEXT NOT NULL,
                imported_at TEXT NOT NULL,
                concept_count INTEGER NOT NULL DEFAULT 0,
                designation_count INTEGER NOT NULL DEFAULT 0,
                artifact_count INTEGER NOT NULL DEFAULT 0,
                notes TEXT
            );
            CREATE INDEX idx_terminology_import_runs_system ON terminology_import_runs(system, imported_at);
        ",
    },
    Migration {
        name: "create_package_forms",
        sql: "
            CREATE TABLE package_forms (
                package_row_id TEXT NOT NULL REFERENCES installed_packages(id),
                form_id TEXT NOT NULL,
                form_json TEXT NOT NULL,
                PRIMARY KEY (package_row_id, form_id)
            );
            CREATE INDEX idx_package_forms_form_id ON package_forms(form_id);
        ",
    },
    Migration {
        name: "create_projection_queue_board_v1",
        sql: "
            CREATE TABLE projection_queue_board_v1 (
                facility_id TEXT NOT NULL,
                queue_entry_id TEXT NOT NULL,
                patient_id TEXT NOT NULL,
                patient_name TEXT NOT NULL,
                patient_mrn TEXT,
                service_type TEXT NOT NULL,
                department TEXT NOT NULL,
                status TEXT NOT NULL,
                position INTEGER NOT NULL,
                arrived_at TEXT NOT NULL,
                assigned_to TEXT,
                assigned_to_name TEXT,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (facility_id, queue_entry_id)
            );

            CREATE INDEX idx_proj_queue_board_facility_status
                ON projection_queue_board_v1(facility_id, status);
            CREATE INDEX idx_proj_queue_board_facility_dept_status
                ON projection_queue_board_v1(facility_id, department, status);
            CREATE INDEX idx_proj_queue_board_facility_arrived
                ON projection_queue_board_v1(facility_id, arrived_at);
        ",
    },
    Migration {
        name: "create_projection_patient_summary_v1",
        sql: "
            CREATE TABLE projection_patient_summary_v1 (
                facility_id TEXT NOT NULL,
                patient_id TEXT NOT NULL,
                given_name TEXT NOT NULL,
                family_name TEXT NOT NULL,
                sex TEXT NOT NULL,
                age INTEGER,
                national_id TEXT,
                last_visit TEXT,
                active_programs TEXT NOT NULL DEFAULT '[]',
                updated_at TEXT NOT NULL,
                PRIMARY KEY (facility_id, patient_id)
            );

            CREATE INDEX idx_proj_patient_summary_facility_name
                ON projection_patient_summary_v1(facility_id, family_name, given_name);
            CREATE INDEX idx_proj_patient_summary_last_visit
                ON projection_patient_summary_v1(facility_id, last_visit);
        ",
    },
    Migration {
        name: "create_projection_longitudinal_timeline_v1",
        sql: "
            CREATE TABLE projection_longitudinal_timeline_v1 (
                facility_id TEXT NOT NULL,
                patient_id TEXT NOT NULL,
                source_aggregate_type TEXT NOT NULL,
                source_aggregate_id TEXT NOT NULL,
                entry_type TEXT NOT NULL,
                title TEXT NOT NULL,
                detail TEXT,
                occurred_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (facility_id, source_aggregate_type, source_aggregate_id)
            );

            CREATE INDEX idx_proj_timeline_patient_occurred
                ON projection_longitudinal_timeline_v1(facility_id, patient_id, occurred_at);
            CREATE INDEX idx_proj_timeline_entry_type
                ON projection_longitudinal_timeline_v1(facility_id, entry_type, occurred_at);
        ",
    },
    Migration {
        name: "add_dispense_item_medication_coding",
        sql: "
            ALTER TABLE dispense_items ADD COLUMN medication_system TEXT;
            ALTER TABLE dispense_items ADD COLUMN medication_code TEXT;
            ALTER TABLE dispense_items ADD COLUMN medication_display TEXT;
        ",
    },
    Migration {
        name: "create_package_runtime_artifact_tables_and_encounter_form_pinning",
        sql: "
            CREATE TABLE package_report_artifacts (
                package_row_id TEXT NOT NULL REFERENCES installed_packages(id),
                report_id TEXT NOT NULL,
                report_family TEXT,
                report_version TEXT,
                report_json TEXT NOT NULL,
                PRIMARY KEY (package_row_id, report_id)
            );
            CREATE INDEX idx_package_report_artifacts_report_id ON package_report_artifacts(report_id);

            CREATE TABLE package_terminology_artifacts (
                package_row_id TEXT NOT NULL REFERENCES installed_packages(id),
                artifact_id TEXT NOT NULL,
                terminology_system TEXT NOT NULL,
                artifact_type TEXT NOT NULL,
                artifact_json TEXT NOT NULL,
                PRIMARY KEY (package_row_id, artifact_id)
            );
            CREATE INDEX idx_package_terminology_artifacts_system
                ON package_terminology_artifacts(terminology_system, artifact_type);

            CREATE TABLE package_deployment_settings (
                package_row_id TEXT NOT NULL REFERENCES installed_packages(id),
                setting_key TEXT NOT NULL,
                setting_value TEXT NOT NULL,
                PRIMARY KEY (package_row_id, setting_key)
            );

            ALTER TABLE encounters ADD COLUMN pinned_form_family TEXT;
            ALTER TABLE encounters ADD COLUMN pinned_form_version TEXT;
            ALTER TABLE encounters ADD COLUMN pinned_form_package_row_id TEXT;
            ALTER TABLE encounters ADD COLUMN pinned_form_source_form_id TEXT;
            CREATE INDEX idx_encounters_pinned_form_package_row
                ON encounters(pinned_form_package_row_id)
                WHERE pinned_form_package_row_id IS NOT NULL;
        ",
    },
];
