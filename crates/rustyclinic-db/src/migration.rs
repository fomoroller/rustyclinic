//! Schema migrations for SQLite.

use anyhow::Result;
use rusqlite::Connection;

/// Run all pending migrations.
pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS _migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
    ")?;

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
];
