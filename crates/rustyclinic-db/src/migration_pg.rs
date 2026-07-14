//! Schema migrations for PostgreSQL.

use tokio_postgres::Client;

/// Run all pending migrations against a PostgreSQL database.
pub async fn run_migrations(client: &Client) -> Result<(), String> {
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS _migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );",
        )
        .await
        .map_err(|e| format!("create _migrations table: {e}"))?;

    let rows = client
        .query("SELECT version FROM _migrations ORDER BY version", &[])
        .await
        .map_err(|e| format!("query _migrations: {e}"))?;

    let applied: Vec<i64> = rows
        .iter()
        .map(|r| r.try_get::<_, i32>(0).map(|v| v as i64).unwrap_or_default())
        .collect();

    for (i, m) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as i64;
        if !applied.contains(&version) {
            tracing::info!(version, name = m.name, "applying PG migration");
            client
                .batch_execute(m.sql)
                .await
                .map_err(|e| format!("migration {} ({}): {e}", version, m.name))?;
            client
                .execute(
                    "INSERT INTO _migrations (version, name) VALUES ($1, $2)",
                    &[&(version as i32), &m.name],
                )
                .await
                .map_err(|e| format!("record migration {version}: {e}"))?;
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
                id UUID PRIMARY KEY NOT NULL,
                facility_id UUID NOT NULL,
                given_name TEXT NOT NULL,
                family_name TEXT NOT NULL,
                sex TEXT NOT NULL,
                date_of_birth DATE,
                phone TEXT,
                address TEXT,
                national_id TEXT,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL,
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
                id UUID PRIMARY KEY NOT NULL,
                facility_id UUID NOT NULL,
                patient_id UUID NOT NULL REFERENCES patients(id),
                service_type TEXT NOT NULL,
                status TEXT NOT NULL,
                assigned_to UUID,
                position INTEGER NOT NULL,
                arrived_at TIMESTAMPTZ NOT NULL,
                called_at TIMESTAMPTZ,
                service_started_at TIMESTAMPTZ,
                completed_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_queue_facility_status ON queue_entries(facility_id, status);
        ",
    },
    Migration {
        name: "create_audit_log",
        sql: "
            CREATE TABLE audit_log (
                id UUID PRIMARY KEY NOT NULL,
                facility_id UUID NOT NULL,
                actor_id UUID NOT NULL,
                device_id UUID NOT NULL,
                timestamp TIMESTAMPTZ NOT NULL,
                action TEXT NOT NULL,
                aggregate_type TEXT NOT NULL,
                aggregate_id UUID NOT NULL,
                payload JSONB NOT NULL,
                prev_hash BYTEA,
                entry_hash BYTEA NOT NULL
            );
            CREATE INDEX idx_audit_facility ON audit_log(facility_id, timestamp);
        ",
    },
    Migration {
        name: "create_outbox",
        sql: "
            CREATE TABLE outbox_events (
                id UUID PRIMARY KEY NOT NULL,
                facility_id UUID NOT NULL,
                aggregate_type TEXT NOT NULL,
                aggregate_id UUID NOT NULL,
                event_type TEXT NOT NULL,
                payload JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                published BOOLEAN NOT NULL DEFAULT false
            );
            CREATE INDEX idx_outbox_unpublished ON outbox_events(published) WHERE published = false;
        ",
    },
    Migration {
        name: "create_idempotency",
        sql: "
            CREATE TABLE idempotency_records (
                key TEXT NOT NULL,
                facility_id UUID NOT NULL,
                response JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                PRIMARY KEY (key, facility_id)
            );
        ",
    },
    Migration {
        name: "create_op_log",
        sql: "
            CREATE TABLE op_log (
                id UUID PRIMARY KEY NOT NULL,
                sequence BIGINT NOT NULL,
                facility_id UUID NOT NULL,
                device_id UUID NOT NULL,
                actor_id UUID NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                aggregate_type TEXT NOT NULL,
                aggregate_id UUID NOT NULL,
                payload JSONB NOT NULL,
                prev_hash BYTEA,
                entry_hash BYTEA NOT NULL,
                sync_state TEXT NOT NULL DEFAULT 'pending'
            );
            CREATE UNIQUE INDEX idx_op_log_sequence ON op_log(facility_id, sequence);
        ",
    },
    Migration {
        name: "create_form_drafts",
        sql: "
            CREATE TABLE form_drafts (
                user_id UUID NOT NULL,
                encounter_id UUID NOT NULL,
                form_family TEXT NOT NULL,
                form_version TEXT NOT NULL,
                field_values JSONB NOT NULL,
                saved_at TIMESTAMPTZ NOT NULL,
                PRIMARY KEY (user_id, encounter_id, form_family, form_version)
            );
        ",
    },
    Migration {
        name: "create_users",
        sql: "
            CREATE TABLE users (
                id UUID PRIMARY KEY NOT NULL,
                facility_id UUID NOT NULL,
                username TEXT NOT NULL,
                display_name TEXT NOT NULL,
                password_hash TEXT NOT NULL,
                roles TEXT NOT NULL DEFAULT '[]',
                active BOOLEAN NOT NULL DEFAULT true,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            );
            CREATE UNIQUE INDEX idx_users_facility_username ON users(facility_id, username);
        ",
    },
    Migration {
        name: "create_sessions",
        sql: "
            CREATE TABLE sessions (
                id UUID PRIMARY KEY NOT NULL,
                user_id UUID NOT NULL REFERENCES users(id),
                facility_id UUID NOT NULL,
                device_id UUID NOT NULL,
                roles TEXT NOT NULL DEFAULT '[]',
                auth_method TEXT NOT NULL,
                state TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ NOT NULL,
                last_active TIMESTAMPTZ NOT NULL,
                locked_at TIMESTAMPTZ
            );
            CREATE INDEX idx_sessions_device ON sessions(device_id, state);
        ",
    },
    Migration {
        name: "create_local_identity",
        sql: "
            CREATE TABLE local_identity (
                id SMALLINT PRIMARY KEY NOT NULL CHECK (id = 1),
                facility_id UUID NOT NULL,
                device_id UUID NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            );
        ",
    },
    Migration {
        name: "create_jobs",
        sql: "
            CREATE TABLE jobs (
                id UUID PRIMARY KEY NOT NULL,
                facility_id UUID,
                job_type TEXT NOT NULL,
                payload JSONB NOT NULL DEFAULT '{}'::jsonb,
                status TEXT NOT NULL,
                run_at TIMESTAMPTZ NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 10,
                leased_by_device_id UUID,
                lease_expires_at TIMESTAMPTZ,
                last_heartbeat_at TIMESTAMPTZ,
                last_error TEXT,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            );
            CREATE INDEX idx_jobs_ready ON jobs(status, run_at);
            CREATE INDEX idx_jobs_lease_expires ON jobs(lease_expires_at);
        ",
    },
    Migration {
        name: "create_installed_packages",
        sql: "
            CREATE TABLE installed_packages (
                id UUID PRIMARY KEY NOT NULL,
                facility_id UUID NOT NULL,
                package_id TEXT NOT NULL,
                package_type TEXT NOT NULL,
                version TEXT NOT NULL,
                status TEXT NOT NULL,
                manifest JSONB NOT NULL,
                installed_at TIMESTAMPTZ NOT NULL,
                activated_at TIMESTAMPTZ,
                rolled_back_at TIMESTAMPTZ,
                installed_by UUID NOT NULL,
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
                last_pulled_sequence BIGINT NOT NULL DEFAULT 0,
                last_pushed_sequence BIGINT NOT NULL DEFAULT 0,
                updated_at TIMESTAMPTZ NOT NULL,
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
                created_at TIMESTAMPTZ NOT NULL,
                resolved_at TIMESTAMPTZ,
                resolved_by TEXT,
                resolution TEXT
            );
            CREATE INDEX idx_sync_conflicts_facility ON sync_conflicts(facility_id, status);
        ",
    },
    Migration {
        name: "create_encounters",
        sql: "
            CREATE TABLE encounters (
                id UUID PRIMARY KEY NOT NULL,
                facility_id UUID NOT NULL,
                patient_id UUID NOT NULL REFERENCES patients(id),
                queue_entry_id UUID REFERENCES queue_entries(id),
                provider_id UUID NOT NULL REFERENCES users(id),
                started_at TIMESTAMPTZ NOT NULL,
                ended_at TIMESTAMPTZ,
                visit_notes TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT 'in_progress',
                created_at TIMESTAMPTZ NOT NULL,
                version INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_encounters_patient ON encounters(patient_id);
            CREATE INDEX idx_encounters_queue ON encounters(queue_entry_id);
        ",
    },
    Migration {
        name: "create_package_forms",
        sql: "
            CREATE TABLE package_forms (
                package_row_id UUID NOT NULL REFERENCES installed_packages(id),
                form_id TEXT NOT NULL,
                form_json JSONB NOT NULL,
                PRIMARY KEY (package_row_id, form_id)
            );
            CREATE INDEX idx_package_forms_form_id ON package_forms(form_id);
        ",
    },
    Migration {
        name: "create_projection_queue_board_v1",
        sql: "
            CREATE TABLE projection_queue_board_v1 (
                facility_id UUID NOT NULL,
                queue_entry_id UUID NOT NULL,
                patient_id UUID NOT NULL,
                patient_name TEXT NOT NULL,
                patient_mrn TEXT,
                service_type TEXT NOT NULL,
                department TEXT NOT NULL,
                status TEXT NOT NULL,
                position INTEGER NOT NULL,
                arrived_at TIMESTAMPTZ NOT NULL,
                assigned_to UUID,
                assigned_to_name TEXT,
                updated_at TIMESTAMPTZ NOT NULL,
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
                facility_id UUID NOT NULL,
                patient_id UUID NOT NULL,
                given_name TEXT NOT NULL,
                family_name TEXT NOT NULL,
                sex TEXT NOT NULL,
                age INTEGER,
                national_id TEXT,
                last_visit TIMESTAMPTZ,
                active_programs JSONB NOT NULL DEFAULT '[]'::jsonb,
                updated_at TIMESTAMPTZ NOT NULL,
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
                facility_id UUID NOT NULL,
                patient_id UUID NOT NULL,
                source_aggregate_type TEXT NOT NULL,
                source_aggregate_id UUID NOT NULL,
                entry_type TEXT NOT NULL,
                title TEXT NOT NULL,
                detail TEXT,
                occurred_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL,
                PRIMARY KEY (facility_id, source_aggregate_type, source_aggregate_id)
            );

            CREATE INDEX idx_proj_timeline_patient_occurred
                ON projection_longitudinal_timeline_v1(facility_id, patient_id, occurred_at);
            CREATE INDEX idx_proj_timeline_entry_type
                ON projection_longitudinal_timeline_v1(facility_id, entry_type, occurred_at);
        ",
    },
    Migration {
        name: "create_package_runtime_artifact_tables_and_encounter_form_pinning",
        sql: "
            CREATE TABLE package_report_artifacts (
                package_row_id UUID NOT NULL REFERENCES installed_packages(id),
                report_id TEXT NOT NULL,
                report_family TEXT,
                report_version TEXT,
                report_json JSONB NOT NULL,
                PRIMARY KEY (package_row_id, report_id)
            );
            CREATE INDEX idx_package_report_artifacts_report_id ON package_report_artifacts(report_id);

            CREATE TABLE package_terminology_artifacts (
                package_row_id UUID NOT NULL REFERENCES installed_packages(id),
                artifact_id TEXT NOT NULL,
                terminology_system TEXT NOT NULL,
                artifact_type TEXT NOT NULL,
                artifact_json JSONB NOT NULL,
                PRIMARY KEY (package_row_id, artifact_id)
            );
            CREATE INDEX idx_package_terminology_artifacts_system
                ON package_terminology_artifacts(terminology_system, artifact_type);

            CREATE TABLE package_deployment_settings (
                package_row_id UUID NOT NULL REFERENCES installed_packages(id),
                setting_key TEXT NOT NULL,
                setting_value JSONB NOT NULL,
                PRIMARY KEY (package_row_id, setting_key)
            );

            ALTER TABLE encounters ADD COLUMN pinned_form_family TEXT;
            ALTER TABLE encounters ADD COLUMN pinned_form_version TEXT;
            ALTER TABLE encounters ADD COLUMN pinned_form_package_row_id UUID;
            ALTER TABLE encounters ADD COLUMN pinned_form_source_form_id TEXT;
            CREATE INDEX idx_encounters_pinned_form_package_row
                ON encounters(pinned_form_package_row_id)
                WHERE pinned_form_package_row_id IS NOT NULL;
        ",
    },
];
