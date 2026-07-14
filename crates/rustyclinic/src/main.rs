//! RustyClinic — single executable, multi-role runtime.
//!
//! ```text
//! rustyclinic serve api|worker|sync|scheduler|mcp|all
//! rustyclinic admin ...
//! ```

use chrono::Utc;
use clap::{Parser, Subcommand};
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "rustyclinic",
    version,
    about = "Offline-first EMR for low-resource settings"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a runtime role
    Serve {
        #[command(subcommand)]
        role: ServeRole,
    },
    /// Administrative commands
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
}

#[derive(Subcommand)]
enum ServeRole {
    Api,
    Worker,
    Sync,
    Scheduler,
    Mcp,
    All,
}

#[derive(Subcommand)]
enum AdminAction {
    /// Run database migrations
    Migrate,
    /// Show system status
    Status,
    /// Download a public terminology source and import it
    DownloadTerminology {
        /// One of: icd11, loinc, ucum, fhir, snomed
        system: String,
        /// Replace existing rows for this system before import
        #[arg(long, default_value_t = false)]
        replace: bool,
    },
    /// Import a terminology dataset into the local database
    ImportTerminology {
        /// One of: icd11, loinc, ucum, fhir, snomed
        system: String,
        /// Local file path or HTTPS URL
        source: String,
        /// Replace existing rows for this system before import
        #[arg(long, default_value_t = false)]
        replace: bool,
    },
    Backup {
        path: String,
    },
    Restore {
        path: String,
    },
    SnapshotExport {
        path: String,
    },
    SnapshotImport {
        path: String,
    },
    /// Install a .rcpkg package file
    InstallPackage {
        /// Path to the .rcpkg file
        path: String,

        #[arg(
            long,
            default_value_t = false,
            help = "Skip Ed25519 signature verification (dev only)"
        )]
        skip_signature_check: bool,

        #[arg(
            long,
            help = "Verifying key (hex). If omitted, uses RUSTYCLINIC_PACKAGE_VERIFYING_KEY_HEX"
        )]
        verifying_key_hex: Option<String>,
    },

    ListPackages,

    TransitionPackage {
        id: String,
        transition: String,
    },
}

fn uuid_from_env(name: &str) -> Option<uuid::Uuid> {
    std::env::var(name)
        .ok()
        .and_then(|v| uuid::Uuid::parse_str(v.trim()).ok())
}

fn load_or_init_local_identity(
    conn: &rusqlite::Connection,
) -> anyhow::Result<(uuid::Uuid, uuid::Uuid)> {
    let existing: Option<(String, String)> = conn
        .query_row(
            "SELECT facility_id, device_id FROM local_identity WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    let env_facility = uuid_from_env("RUSTYCLINIC_FACILITY_ID");
    let env_device = uuid_from_env("RUSTYCLINIC_DEVICE_ID");

    let (facility_id, device_id) = match existing {
        Some((facility_str, device_str)) => {
            let facility_id = uuid::Uuid::parse_str(&facility_str).unwrap_or(uuid::Uuid::nil());
            let device_id = uuid::Uuid::parse_str(&device_str).unwrap_or(uuid::Uuid::nil());
            (
                env_facility.unwrap_or(facility_id),
                env_device.unwrap_or(device_id),
            )
        }
        None => (
            env_facility.unwrap_or_else(uuid::Uuid::now_v7),
            env_device.unwrap_or_else(uuid::Uuid::now_v7),
        ),
    };

    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO local_identity (id, facility_id, device_id, created_at, updated_at)
         VALUES (1, ?1, ?2, ?3, ?3)
         ON CONFLICT(id) DO UPDATE SET facility_id = excluded.facility_id, device_id = excluded.device_id, updated_at = excluded.updated_at",
        rusqlite::params![facility_id.to_string(), device_id.to_string(), now],
    )?;

    Ok((facility_id, device_id))
}

fn process_outbox_once(db_path: &str) -> anyhow::Result<u32> {
    let conn = rusqlite::Connection::open(db_path)?;

    let mut stmt = conn.prepare(
        "SELECT id, facility_id, aggregate_type, aggregate_id, event_type, payload, created_at
         FROM outbox_events
         WHERE published = 0
         ORDER BY created_at ASC
         LIMIT 100",
    )?;

    let events_iter = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let facility_id: String = row.get(1)?;
        let aggregate_type: String = row.get(2)?;
        let aggregate_id: String = row.get(3)?;
        let event_type: String = row.get(4)?;
        let payload_str: String = row.get(5)?;
        let created_at_str: String = row.get(6)?;

        let payload: serde_json::Value =
            serde_json::from_str(&payload_str).unwrap_or(serde_json::json!({}));

        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        Ok(rustyclinic_events::OutboxEvent {
            id: uuid::Uuid::parse_str(&id).unwrap_or_default(),
            facility_id: uuid::Uuid::parse_str(&facility_id).unwrap_or_default(),
            aggregate_type,
            aggregate_id: uuid::Uuid::parse_str(&aggregate_id).unwrap_or_default(),
            event_type,
            payload,
            created_at,
            published: false,
        })
    })?;

    let events = events_iter.collect::<Result<Vec<_>, _>>()?;

    if events.is_empty() {
        return Ok(0);
    }

    let mut processed = 0u32;
    for event in events {
        let tx = conn.unchecked_transaction()?;
        let applier = rustyclinic_projections::SqliteProjectionApplier::new(&tx);
        applier.apply_outbox_event(&event)?;

        let changed = tx.execute(
            "UPDATE outbox_events SET published = 1 WHERE id = ?1 AND published = 0",
            rusqlite::params![event.id.to_string()],
        )?;

        tx.commit()?;
        processed += changed as u32;
    }

    Ok(processed)
}

fn sqlite_string_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn sqlite_sidecar_paths(path: &Path) -> [PathBuf; 2] {
    let base = path.as_os_str().to_string_lossy().to_string();
    [
        PathBuf::from(format!("{base}-wal")),
        PathBuf::from(format!("{base}-shm")),
    ]
}

fn remove_sqlite_sidecars(path: &Path) -> anyhow::Result<()> {
    for sidecar in sqlite_sidecar_paths(path) {
        if sidecar.exists() {
            std::fs::remove_file(&sidecar)?;
        }
    }
    Ok(())
}

fn backup_sqlite_database(db_path: &Path, backup_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = backup_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if backup_path.exists() {
        std::fs::remove_file(backup_path)?;
    }

    let conn = rustyclinic_db::sqlite::connection::open_and_migrate(
        db_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("database path is not valid UTF-8"))?,
    )?;
    let backup_literal = sqlite_string_literal(
        backup_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("backup path is not valid UTF-8"))?,
    );
    conn.execute_batch(&format!("VACUUM INTO '{backup_literal}'"))?;
    Ok(())
}

fn restore_sqlite_database(db_path: &Path, backup_path: &Path) -> anyhow::Result<()> {
    if !backup_path.exists() {
        return Err(anyhow::anyhow!(
            "backup file '{}' does not exist",
            backup_path.display()
        ));
    }

    let restore_temp = db_path.with_extension(format!("restore-{}", uuid::Uuid::now_v7().simple()));
    std::fs::copy(backup_path, &restore_temp)?;

    rustyclinic_db::sqlite::connection::open_and_migrate(
        restore_temp
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("restore temp path is not valid UTF-8"))?,
    )?;

    remove_sqlite_sidecars(&restore_temp)?;
    remove_sqlite_sidecars(db_path)?;

    if db_path.exists() {
        std::fs::remove_file(db_path)?;
    }
    std::fs::rename(&restore_temp, db_path)?;
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotManifest {
    facility_id: String,
    snapshot_at: String,
    op_log_position: u64,
    db_file: String,
    db_sha256: String,
    total_size_bytes: u64,
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn export_snapshot_archive(db_path: &Path, snapshot_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = snapshot_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let temp_backup =
        snapshot_path.with_extension(format!("snapshot-{}", uuid::Uuid::now_v7().simple()));
    backup_sqlite_database(db_path, &temp_backup)?;
    let db_bytes = std::fs::read(&temp_backup)?;
    let conn = rusqlite::Connection::open(&temp_backup)?;
    let facility_id: String = conn
        .query_row(
            "SELECT facility_id FROM local_identity WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_else(|| uuid::Uuid::nil().to_string());
    let op_log_position: u64 =
        conn.query_row("SELECT COALESCE(MAX(sequence), 0) FROM op_log", [], |row| {
            row.get(0)
        })?;

    let manifest = SnapshotManifest {
        facility_id,
        snapshot_at: Utc::now().to_rfc3339(),
        op_log_position,
        db_file: "rustyclinic.sqlite".to_string(),
        db_sha256: hex_digest(&Sha256::digest(&db_bytes)),
        total_size_bytes: db_bytes.len() as u64,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;

    let file = std::fs::File::create(snapshot_path)?;
    let mut builder = tar::Builder::new(file);

    let mut manifest_header = tar::Header::new_gnu();
    manifest_header.set_size(manifest_bytes.len() as u64);
    manifest_header.set_mode(0o644);
    manifest_header.set_mtime(0);
    manifest_header.set_cksum();
    builder.append_data(
        &mut manifest_header,
        "manifest.json",
        Cursor::new(manifest_bytes),
    )?;

    let mut db_header = tar::Header::new_gnu();
    db_header.set_size(db_bytes.len() as u64);
    db_header.set_mode(0o644);
    db_header.set_mtime(0);
    db_header.set_cksum();
    builder.append_data(&mut db_header, &manifest.db_file, Cursor::new(db_bytes))?;
    builder.finish()?;

    let _ = std::fs::remove_file(&temp_backup);
    Ok(())
}

fn import_snapshot_archive(db_path: &Path, snapshot_path: &Path) -> anyhow::Result<()> {
    if !snapshot_path.exists() {
        return Err(anyhow::anyhow!(
            "snapshot file '{}' does not exist",
            snapshot_path.display()
        ));
    }

    let file = std::fs::File::open(snapshot_path)?;
    let mut archive = tar::Archive::new(file);
    let mut manifest_bytes: Option<Vec<u8>> = None;
    let mut db_bytes: Option<Vec<u8>> = None;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().to_string();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        if path == "manifest.json" {
            manifest_bytes = Some(bytes);
        } else if path == "rustyclinic.sqlite" {
            db_bytes = Some(bytes);
        }
    }

    let manifest: SnapshotManifest = serde_json::from_slice(
        &manifest_bytes.ok_or_else(|| anyhow::anyhow!("snapshot missing manifest.json"))?,
    )?;
    let db_bytes =
        db_bytes.ok_or_else(|| anyhow::anyhow!("snapshot missing rustyclinic.sqlite"))?;
    let actual_checksum = hex_digest(&Sha256::digest(&db_bytes));
    if actual_checksum != manifest.db_sha256 {
        return Err(anyhow::anyhow!("snapshot checksum mismatch"));
    }

    let temp_backup =
        snapshot_path.with_extension(format!("import-{}", uuid::Uuid::now_v7().simple()));
    std::fs::write(&temp_backup, db_bytes)?;
    restore_sqlite_database(db_path, &temp_backup)?;
    let _ = std::fs::remove_file(&temp_backup);
    Ok(())
}

async fn run_outbox_worker(db_path: String) -> anyhow::Result<()> {
    tracing::info!("worker started");
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("worker shutting down");
                break;
            }
            _ = tick.tick() => {
                match process_outbox_once(&db_path) {
                    Ok(n) if n > 0 => tracing::info!(processed = n, "outbox processed"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "outbox processing failed"),
                }
            }
        }
    }
    Ok(())
}

async fn run_scheduler(db_path: String) -> anyhow::Result<()> {
    tracing::info!("scheduler started");
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("scheduler shutting down");
                break;
            }
            _ = tick.tick() => {
                let conn = match rustyclinic_db::sqlite::connection::open_and_migrate(&db_path) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(error = %e, "scheduler db open failed");
                        continue;
                    }
                };

                let (facility_id, device_id) = match load_or_init_local_identity(&conn) {
                    Ok(ids) => ids,
                    Err(e) => {
                        tracing::warn!(error = %e, "scheduler identity load failed");
                        continue;
                    }
                };

                let repo = rustyclinic_jobs::SqliteJobRepo::new(&conn);
                let _ = repo.ensure_singleton_due(
                    Some(facility_id),
                    "maintenance.prune_form_drafts",
                    chrono::Utc::now(),
                    serde_json::json!({"retention_days": 7}),
                    3,
                );

                let job = match repo.try_lease_next(device_id, chrono::Duration::minutes(2)) {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::warn!(error = %e, "job lease failed");
                        continue;
                    }
                };

                let Some(job) = job else {
                    continue;
                };

                match job.job_type.as_str() {
                    "maintenance.prune_form_drafts" => {
                        let changed = conn.execute(
                            "DELETE FROM form_drafts WHERE datetime(saved_at) < datetime('now', '-7 days')",
                            [],
                        ).unwrap_or(0);
                        tracing::info!(pruned = changed, "form drafts pruned");
                        let _ = repo.succeed(job.id, device_id);
                    }
                    other => {
                        tracing::warn!(job_type = other, "unknown job type");
                        let _ = repo.fail(job.id, device_id, format!("unknown job type: {other}"));
                    }
                }
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Structured JSON logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(true)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { role } => {
            let role_name = match &role {
                ServeRole::Api => "api",
                ServeRole::Worker => "worker",
                ServeRole::Sync => "sync",
                ServeRole::Scheduler => "scheduler",
                ServeRole::Mcp => "mcp",
                ServeRole::All => "all",
            };
            tracing::info!(role = role_name, "starting rustyclinic");

            // Open database and run migrations
            let db_path =
                std::env::var("RUSTYCLINIC_DB").unwrap_or_else(|_| "rustyclinic.db".to_string());
            let conn = rustyclinic_db::sqlite::connection::open_and_migrate(&db_path)?;
            tracing::info!(path = %db_path, "database ready");

            let (facility_id, device_id) = load_or_init_local_identity(&conn)?;

            match role {
                ServeRole::Api | ServeRole::All => {
                    use rustyclinic_api::state::{AppState, AppStateInner};

                    let api_state: AppState = std::sync::Arc::new(AppStateInner {
                        db_path: db_path.clone(),
                        facility_id,
                        device_id,
                    });

                    let web_state = rustyclinic_web::WebAppState {
                        db_path: db_path.clone(),
                        device_id,
                        facility_id,
                    };

                    let sync_state = rustyclinic_sync::SyncState::new(db_path.clone());

                    let reporting_state =
                        rustyclinic_reporting::routes::ReportingState::new(db_path.clone());
                    let interop_state = rustyclinic_interop::InteropState::new(db_path.clone());

                    let api_routes = axum::Router::new()
                        .route("/health", axum::routing::get(rustyclinic_api::health_check))
                        .route(
                            "/api/auth/login",
                            axum::routing::post(rustyclinic_api::routes::auth::login),
                        )
                        .route(
                            "/api/auth/bootstrap",
                            axum::routing::post(rustyclinic_api::routes::auth::bootstrap_user),
                        )
                        .route(
                            "/api/auth/users",
                            axum::routing::post(rustyclinic_api::routes::auth::create_user),
                        )
                        .route(
                            "/api/patients",
                            axum::routing::post(
                                rustyclinic_api::routes::patients::register_patient,
                            ),
                        )
                        .route(
                            "/api/queue",
                            axum::routing::post(rustyclinic_api::routes::queue::enqueue_patient),
                        )
                        .with_state(api_state);

                    let web_routes = rustyclinic_web::web_router(web_state);
                    let sync_routes = rustyclinic_sync::sync_router(sync_state);
                    let reporting_routes =
                        rustyclinic_reporting::routes::reporting_router(reporting_state);
                    let interop_routes = rustyclinic_interop::interop_router(interop_state);

                    let mcp_routes = rustyclinic_mcp::mcp_router(rustyclinic_mcp::McpState {
                        db_path: db_path.clone(),
                    });

                    if matches!(role, ServeRole::All) {
                        let _worker = tokio::spawn(run_outbox_worker(db_path.clone()));
                        let _scheduler = tokio::spawn(run_scheduler(db_path.clone()));
                    }

                    let app = api_routes
                        .merge(web_routes)
                        .merge(sync_routes)
                        .merge(reporting_routes)
                        .merge(interop_routes);

                    let app = if matches!(role, ServeRole::All) {
                        app.merge(mcp_routes)
                    } else {
                        app
                    };

                    let addr = "0.0.0.0:8080";
                    tracing::info!(addr, "listening");
                    let listener = tokio::net::TcpListener::bind(addr).await?;
                    axum::serve(listener, app).await?;
                }
                ServeRole::Sync => {
                    let sync_state = rustyclinic_sync::SyncState::new(db_path.clone());
                    let sync_routes = rustyclinic_sync::sync_router(sync_state);

                    let app = axum::Router::new()
                        .route("/health", axum::routing::get(rustyclinic_api::health_check))
                        .merge(sync_routes);

                    let addr = "0.0.0.0:8080";
                    tracing::info!(addr, "listening");
                    let listener = tokio::net::TcpListener::bind(addr).await?;
                    axum::serve(listener, app).await?;
                }
                ServeRole::Worker => {
                    run_outbox_worker(db_path.clone()).await?;
                }
                ServeRole::Scheduler => {
                    run_scheduler(db_path.clone()).await?;
                }
                ServeRole::Mcp => {
                    let app = axum::Router::new()
                        .route("/health", axum::routing::get(rustyclinic_api::health_check))
                        .merge(rustyclinic_mcp::mcp_router(rustyclinic_mcp::McpState {
                            db_path: db_path.clone(),
                        }));

                    let addr = "0.0.0.0:8080";
                    tracing::info!(addr, "listening");
                    let listener = tokio::net::TcpListener::bind(addr).await?;
                    axum::serve(listener, app).await?;
                }
            }

            drop(conn);
        }
        Commands::Admin { action } => match action {
            AdminAction::Migrate => {
                let db_path = std::env::var("RUSTYCLINIC_DB")
                    .unwrap_or_else(|_| "rustyclinic.db".to_string());
                let _conn = rustyclinic_db::sqlite::connection::open_and_migrate(&db_path)?;
                tracing::info!("migrations complete");
            }
            AdminAction::Status => {
                let db_path = std::env::var("RUSTYCLINIC_DB")
                    .unwrap_or_else(|_| "rustyclinic.db".to_string());
                let conn = rustyclinic_db::sqlite::connection::open_and_migrate(&db_path)?;

                let (facility_id, device_id) = load_or_init_local_identity(&conn)?;
                let counts = rustyclinic_terminology::import::concept_counts(&conn)?;
                let import_runs = rustyclinic_terminology::import::latest_import_runs(&conn)?;

                println!("rustyclinic v{}", env!("CARGO_PKG_VERSION"));
                println!("facility_id: {facility_id}");
                println!("device_id: {device_id}");

                if counts.is_empty() && import_runs.is_empty() {
                    println!("terminology: none imported");
                } else {
                    println!("terminology:");
                    let mut all_systems: Vec<String> = Vec::new();
                    for system in counts.keys() {
                        let normalized = normalize_terminology_system_label(system);
                        if !all_systems.contains(&normalized) {
                            all_systems.push(normalized);
                        }
                    }
                    for run in &import_runs {
                        let normalized = normalize_terminology_system_label(&run.system);
                        if !all_systems.contains(&normalized) {
                            all_systems.push(normalized);
                        }
                    }
                    all_systems.sort();

                    for system in all_systems {
                        let count = counts
                            .iter()
                            .find(|(raw_system, _)| {
                                normalize_terminology_system_label(raw_system) == system
                            })
                            .map(|(_, count)| *count)
                            .unwrap_or(0);
                        let run_info = import_runs
                            .iter()
                            .find(|r| normalize_terminology_system_label(&r.system) == system);
                        if let Some(run) = run_info {
                            println!(
                                "  {system}: {count} concepts | source={} | imported_at={} | designations={} | artifacts={}",
                                run.source,
                                run.imported_at,
                                run.designation_count,
                                run.artifact_count
                            );
                        } else {
                            println!("  {system}: {count}");
                        }
                    }
                }
            }
            AdminAction::ImportTerminology {
                system,
                source,
                replace,
            } => {
                let Some(system_enum) =
                    rustyclinic_terminology::import::ImportSystem::parse(&system)
                else {
                    return Err(anyhow::anyhow!(
                        "unknown terminology system '{}'; expected one of: icd11, loinc, ucum, fhir, snomed",
                        system
                    ));
                };

                let db_path = std::env::var("RUSTYCLINIC_DB")
                    .unwrap_or_else(|_| "rustyclinic.db".to_string());
                let conn = rustyclinic_db::sqlite::connection::open_and_migrate(&db_path)?;
                let summary = rustyclinic_terminology::import::import_from_source(
                    &conn,
                    system_enum,
                    &source,
                    replace,
                )?;

                tracing::info!(
                    system = system_enum.as_str(),
                    source,
                    concepts = summary.concept_count,
                    designations = summary.designation_count,
                    artifacts = summary.artifact_count,
                    "terminology import complete"
                );

                println!(
                    "Imported {} concepts, {} designations, {} artifacts for {}",
                    summary.concept_count,
                    summary.designation_count,
                    summary.artifact_count,
                    system_enum.as_str()
                );
            }
            AdminAction::Backup { path } => {
                let db_path = std::env::var("RUSTYCLINIC_DB")
                    .unwrap_or_else(|_| "rustyclinic.db".to_string());
                backup_sqlite_database(Path::new(&db_path), Path::new(&path))?;
                println!("backup created: {}", path);
            }
            AdminAction::Restore { path } => {
                let db_path = std::env::var("RUSTYCLINIC_DB")
                    .unwrap_or_else(|_| "rustyclinic.db".to_string());
                restore_sqlite_database(Path::new(&db_path), Path::new(&path))?;
                println!("restore complete: {}", path);
            }
            AdminAction::SnapshotExport { path } => {
                let db_path = std::env::var("RUSTYCLINIC_DB")
                    .unwrap_or_else(|_| "rustyclinic.db".to_string());
                export_snapshot_archive(Path::new(&db_path), Path::new(&path))?;
                println!("snapshot exported: {}", path);
            }
            AdminAction::SnapshotImport { path } => {
                let db_path = std::env::var("RUSTYCLINIC_DB")
                    .unwrap_or_else(|_| "rustyclinic.db".to_string());
                import_snapshot_archive(Path::new(&db_path), Path::new(&path))?;
                println!("snapshot imported: {}", path);
            }
            AdminAction::DownloadTerminology { system, replace } => {
                let Some(system_enum) =
                    rustyclinic_terminology::import::ImportSystem::parse(&system)
                else {
                    return Err(anyhow::anyhow!(
                        "unknown terminology system '{}'; expected one of: icd11, loinc, ucum, fhir, snomed",
                        system
                    ));
                };

                let source = rustyclinic_terminology::import::preset_source(system_enum)?;
                let db_path = std::env::var("RUSTYCLINIC_DB")
                    .unwrap_or_else(|_| "rustyclinic.db".to_string());
                let conn = rustyclinic_db::sqlite::connection::open_and_migrate(&db_path)?;
                let summary = rustyclinic_terminology::import::import_from_source(
                    &conn,
                    system_enum,
                    source,
                    replace,
                )?;

                tracing::info!(
                    system = system_enum.as_str(),
                    source,
                    concepts = summary.concept_count,
                    designations = summary.designation_count,
                    artifacts = summary.artifact_count,
                    "preset terminology download/import complete"
                );

                println!(
                    "Downloaded and imported {} concepts, {} designations, {} artifacts for {} from {}",
                    summary.concept_count,
                    summary.designation_count,
                    summary.artifact_count,
                    system_enum.as_str(),
                    source
                );
            }
            AdminAction::InstallPackage {
                path,
                skip_signature_check,
                verifying_key_hex,
            } => {
                let db_path = std::env::var("RUSTYCLINIC_DB")
                    .unwrap_or_else(|_| "rustyclinic.db".to_string());
                let conn = rustyclinic_db::sqlite::connection::open_and_migrate(&db_path)?;

                let (facility_id, device_id) = load_or_init_local_identity(&conn)?;

                let package_bytes = std::fs::read(&path).map_err(|e| {
                    anyhow::anyhow!("failed to read package file '{}': {}", path, e)
                })?;

                let actor = rustyclinic_core::types::ActorContext {
                    user_id: uuid::Uuid::nil(),
                    facility_id,
                    device_id,
                    roles: vec!["system_admin".to_string()],
                    purpose: "system".to_string(),
                    session_id: uuid::Uuid::nil(),
                };

                let mut uow = rustyclinic_db::sqlite::unit_of_work::UnitOfWork::new(&conn);

                let input = rustyclinic_services::commands::install_package::InstallPackageInput {
                    package_bytes,
                    skip_signature_check,
                    verifying_key_hex,
                };

                let (installed, output) = rustyclinic_services::commands::install_package::execute(
                    &mut uow, &actor, input,
                )?;
                uow.commit()?;

                tracing::info!(
                    package_id = %installed.package_id,
                    version = %installed.version,
                    forms = ?output.forms_installed,
                    "package installed successfully"
                );

                println!(
                    "Installed package '{}' v{} with {} form(s): {}",
                    installed.package_id,
                    installed.version,
                    output.forms_installed.len(),
                    output.forms_installed.join(", "),
                );
            }
            AdminAction::ListPackages => {
                let db_path = std::env::var("RUSTYCLINIC_DB")
                    .unwrap_or_else(|_| "rustyclinic.db".to_string());
                let conn = rustyclinic_db::sqlite::connection::open_and_migrate(&db_path)?;

                let mut stmt = conn
                    .prepare(
                        "SELECT id, package_id, package_type, version, status, installed_at, activated_at, rolled_back_at
                         FROM installed_packages
                         ORDER BY installed_at DESC",
                    )
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;

                let rows = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, Option<String>>(7)?,
                        ))
                    })
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;

                for row in rows.flatten() {
                    let (
                        id,
                        pkg_id,
                        pkg_type,
                        version,
                        status,
                        installed_at,
                        activated_at,
                        rolled_back_at,
                    ) = row;
                    println!(
                        "{id}  {pkg_id}  {pkg_type}  v{version}  {status}  installed={installed_at}  activated={}  rolled_back={}",
                        activated_at.unwrap_or_else(|| "—".to_string()),
                        rolled_back_at.unwrap_or_else(|| "—".to_string())
                    );
                }
            }
            AdminAction::TransitionPackage { id, transition } => {
                let db_path = std::env::var("RUSTYCLINIC_DB")
                    .unwrap_or_else(|_| "rustyclinic.db".to_string());
                let conn = rustyclinic_db::sqlite::connection::open_and_migrate(&db_path)?;

                let (facility_id, device_id) = load_or_init_local_identity(&conn)?;

                let installed_package_row_id = uuid::Uuid::parse_str(id.trim())
                    .map_err(|_| anyhow::anyhow!("invalid package row id"))?;

                let transition = match transition.as_str() {
                    "verify" => rustyclinic_packages::PackageTransition::Verify,
                    "stage" => rustyclinic_packages::PackageTransition::Stage,
                    "activate" => rustyclinic_packages::PackageTransition::Activate,
                    "rollback" => rustyclinic_packages::PackageTransition::Rollback,
                    "revoke" => rustyclinic_packages::PackageTransition::Revoke,
                    _ => {
                        return Err(anyhow::anyhow!(
                            "unknown transition '{}'; expected: verify|stage|activate|rollback|revoke",
                            transition
                        ));
                    }
                };

                let actor = rustyclinic_core::types::ActorContext {
                    user_id: uuid::Uuid::nil(),
                    facility_id,
                    device_id,
                    roles: vec!["system_admin".to_string()],
                    purpose: "system".to_string(),
                    session_id: uuid::Uuid::nil(),
                };

                let mut uow = rustyclinic_db::sqlite::unit_of_work::UnitOfWork::new(&conn);
                let pkg = rustyclinic_services::commands::transition_package::execute(
                    &mut uow,
                    &actor,
                    rustyclinic_services::commands::transition_package::TransitionPackageInput {
                        installed_package_row_id,
                        transition,
                    },
                )?;
                uow.commit()?;

                println!(
                    "package transitioned: {} {} {} -> {}",
                    pkg.id, pkg.package_id, pkg.version, pkg.status
                );
            }
        },
    }

    Ok(())
}

fn normalize_terminology_system_label(system: &str) -> String {
    match system {
        rustyclinic_terminology::ICD11_SYSTEM => "icd11".to_string(),
        rustyclinic_terminology::LOINC_SYSTEM => "loinc".to_string(),
        rustyclinic_terminology::SNOMED_SYSTEM => "snomed".to_string(),
        rustyclinic_terminology::UCUM_SYSTEM => "ucum".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_and_restore_round_trip_sqlite_database() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!(
            "rustyclinic-backup-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let backup_path = temp_dir.join(format!(
            "rustyclinic-backup-test-{}.sqlite3",
            uuid::Uuid::now_v7()
        ));

        let conn = rustyclinic_db::sqlite::connection::open_and_migrate(
            db_path.to_str().expect("db path"),
        )
        .expect("create db");
        let (facility_id, _) = load_or_init_local_identity(&conn).expect("identity");
        let now = Utc::now().to_rfc3339();
        let patient_id = uuid::Uuid::now_v7();

        conn.execute(
            "INSERT INTO patients (id, facility_id, given_name, family_name, sex, date_of_birth, phone, address, national_id, created_at, updated_at, version)
             VALUES (?1, ?2, 'Backup', 'Patient', 'female', NULL, NULL, NULL, NULL, ?3, ?3, 0)",
            rusqlite::params![patient_id.to_string(), facility_id.to_string(), now],
        )
        .expect("insert patient");

        backup_sqlite_database(&db_path, &backup_path).expect("backup");

        conn.execute(
            "DELETE FROM patients WHERE id = ?1",
            rusqlite::params![patient_id.to_string()],
        )
        .expect("delete patient");
        drop(conn);

        restore_sqlite_database(&db_path, &backup_path).expect("restore");

        let restored = rusqlite::Connection::open(&db_path).expect("open restored db");
        let count: u32 = restored
            .query_row(
                "SELECT COUNT(*) FROM patients WHERE id = ?1",
                rusqlite::params![patient_id.to_string()],
                |row| row.get(0),
            )
            .expect("count restored patient");
        assert_eq!(count, 1);

        let _ = std::fs::remove_file(&backup_path);
        let _ = std::fs::remove_file(&db_path);
        let _ = remove_sqlite_sidecars(&db_path);
    }

    #[test]
    fn snapshot_export_and_import_round_trip_sqlite_database() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!(
            "rustyclinic-snapshot-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let snapshot_path = temp_dir.join(format!(
            "rustyclinic-snapshot-test-{}.rcsnap",
            uuid::Uuid::now_v7()
        ));

        let conn = rustyclinic_db::sqlite::connection::open_and_migrate(
            db_path.to_str().expect("db path"),
        )
        .expect("create db");
        let (facility_id, _) = load_or_init_local_identity(&conn).expect("identity");
        let now = Utc::now().to_rfc3339();
        let patient_id = uuid::Uuid::now_v7();

        conn.execute(
            "INSERT INTO patients (id, facility_id, given_name, family_name, sex, date_of_birth, phone, address, national_id, created_at, updated_at, version)
             VALUES (?1, ?2, 'Snapshot', 'Patient', 'female', NULL, NULL, NULL, NULL, ?3, ?3, 0)",
            rusqlite::params![patient_id.to_string(), facility_id.to_string(), now],
        )
        .expect("insert patient");
        drop(conn);

        export_snapshot_archive(&db_path, &snapshot_path).expect("export snapshot");

        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        conn.execute(
            "DELETE FROM patients WHERE id = ?1",
            rusqlite::params![patient_id.to_string()],
        )
        .expect("delete patient");
        drop(conn);

        import_snapshot_archive(&db_path, &snapshot_path).expect("import snapshot");

        let restored = rusqlite::Connection::open(&db_path).expect("open restored db");
        let count: u32 = restored
            .query_row(
                "SELECT COUNT(*) FROM patients WHERE id = ?1",
                rusqlite::params![patient_id.to_string()],
                |row| row.get(0),
            )
            .expect("count restored patient");
        assert_eq!(count, 1);

        let _ = std::fs::remove_file(&snapshot_path);
        let _ = std::fs::remove_file(&db_path);
        let _ = remove_sqlite_sidecars(&db_path);
    }
}
