//! Runtime configuration.

use serde::{Deserialize, Serialize};

/// Deployment tier determines resource budgets and feature availability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeploymentTier {
    Nano,
    Micro,
    Standard,
    Enterprise,
}

/// Runtime role — which services this process runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeRole {
    Api,
    Worker,
    Sync,
    Scheduler,
    Mcp,
    All,
}

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub tier: DeploymentTier,
    pub role: RuntimeRole,
    pub listen_addr: String,
    pub listen_port: u16,
    pub database_url: String,
    pub facility_name: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tier: DeploymentTier::Micro,
            role: RuntimeRole::All,
            listen_addr: "0.0.0.0".to_string(),
            listen_port: 8080,
            database_url: "rustyclinic.db".to_string(),
            facility_name: "RustyClinic".to_string(),
        }
    }
}
