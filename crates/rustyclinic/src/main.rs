//! RustyClinic — single executable, multi-role runtime.
//!
//! ```text
//! rustyclinic serve api|worker|sync|scheduler|mcp|all
//! rustyclinic admin ...
//! rustyclinic dev          (dev environment with seed data)
//! rustyclinic diagnose     (health checks)
//! ```

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rustyclinic", version, about = "Offline-first EMR for low-resource settings")]
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
            let db_path = std::env::var("RUSTYCLINIC_DB")
                .unwrap_or_else(|_| "rustyclinic.db".to_string());
            let conn = rustyclinic_db::sqlite::connection::open_and_migrate(&db_path)?;
            tracing::info!(path = %db_path, "database ready");

            match role {
                ServeRole::Api | ServeRole::All => {
                    use rustyclinic_api::routes::patients::{AppState, AppStateInner};

                    let state: AppState = std::sync::Arc::new(AppStateInner {
                        db_path: db_path.clone(),
                    });

                    let app = axum::Router::new()
                        .route("/health", axum::routing::get(rustyclinic_api::health_check))
                        .route("/api/auth/login", axum::routing::post(rustyclinic_api::routes::auth::login))
                        .route("/api/auth/users", axum::routing::post(rustyclinic_api::routes::auth::create_user))
                        .route("/api/patients", axum::routing::post(rustyclinic_api::routes::patients::register_patient))
                        .route("/api/queue", axum::routing::post(rustyclinic_api::routes::queue::enqueue_patient))
                        .with_state(state);

                    let addr = "0.0.0.0:8080";
                    tracing::info!(addr, "listening");
                    let listener = tokio::net::TcpListener::bind(addr).await?;
                    axum::serve(listener, app).await?;
                }
                _ => {
                    tracing::info!(role = role_name, "role not yet implemented, waiting...");
                    tokio::signal::ctrl_c().await?;
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
                println!("rustyclinic v{}", env!("CARGO_PKG_VERSION"));
            }
        },
    }

    Ok(())
}
