//! Package registry, resolution, and activation.
//!
//! Packages deliver deployment-specific clinical and administrative content:
//! country rules, payer logic, form schemas, terminology, report mappings,
//! prompts, and model packs.
//!
//! ```text
//! PACKAGE INSTALL STATE MACHINE:
//!
//!   uploaded → verified → staged → activated
//!                                    │
//!                                    └──▶ rolled_back
//!                         │
//!                         └──▶ revoked
//! ```

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use std::fmt;

/// Package types delivered through the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageType {
    Deployment,
    Program,
    Payer,
    Form,
    Terminology,
    Report,
    Integration,
    Model,
}

/// Package installation status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageStatus {
    Uploaded,
    Verified,
    Staged,
    Activated,
    RolledBack,
    Revoked,
}

impl fmt::Display for PackageStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uploaded => write!(f, "uploaded"),
            Self::Verified => write!(f, "verified"),
            Self::Staged => write!(f, "staged"),
            Self::Activated => write!(f, "activated"),
            Self::RolledBack => write!(f, "rolled_back"),
            Self::Revoked => write!(f, "revoked"),
        }
    }
}

/// Transitions for the package install state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PackageTransition {
    Verify,
    Stage,
    Activate,
    Rollback,
    Revoke,
}

impl fmt::Display for PackageTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Verify => write!(f, "verify"),
            Self::Stage => write!(f, "stage"),
            Self::Activate => write!(f, "activate"),
            Self::Rollback => write!(f, "rollback"),
            Self::Revoke => write!(f, "revoke"),
        }
    }
}

/// A package manifest — metadata about an installable package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    pub package_id: String,
    pub package_type: PackageType,
    pub version: String,
    pub compatible_versions: String,
    pub dependencies: Vec<PackageDependency>,
    pub effective_start: Option<NaiveDate>,
    pub effective_end: Option<NaiveDate>,
    pub scope: PackageScope,
    pub checksum: String,
    pub localization_coverage: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageDependency {
    pub package_id: String,
    pub version_range: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageScope {
    Facility,
    District,
    Network,
    Global,
}

/// An installed package record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub package_id: String,
    pub package_type: PackageType,
    pub version: String,
    pub status: PackageStatus,
    pub manifest: PackageManifest,
    pub installed_at: DateTime<Utc>,
    pub activated_at: Option<DateTime<Utc>>,
    pub rolled_back_at: Option<DateTime<Utc>>,
    pub installed_by: Uuid,
    pub version_num: u32,
}

impl StateMachine for InstalledPackage {
    type State = PackageStatus;
    type Transition = PackageTransition;

    fn current_state(&self) -> &PackageStatus {
        &self.status
    }

    fn allowed_transitions(&self, _actor: &ActorContext) -> Vec<PackageTransition> {
        match &self.status {
            PackageStatus::Uploaded => vec![PackageTransition::Verify],
            PackageStatus::Verified => vec![PackageTransition::Stage, PackageTransition::Revoke],
            PackageStatus::Staged => vec![PackageTransition::Activate, PackageTransition::Revoke],
            PackageStatus::Activated => vec![PackageTransition::Rollback, PackageTransition::Revoke],
            PackageStatus::RolledBack => vec![PackageTransition::Stage],
            PackageStatus::Revoked => vec![],
        }
    }

    fn apply_transition(
        &mut self,
        transition: PackageTransition,
        actor: &ActorContext,
    ) -> AppResult<()> {
        self.validate_transition(&transition, actor)?;
        let now = Utc::now();
        match transition {
            PackageTransition::Verify => {
                self.status = PackageStatus::Verified;
            }
            PackageTransition::Stage => {
                self.status = PackageStatus::Staged;
            }
            PackageTransition::Activate => {
                self.status = PackageStatus::Activated;
                self.activated_at = Some(now);
            }
            PackageTransition::Rollback => {
                self.status = PackageStatus::RolledBack;
                self.rolled_back_at = Some(now);
            }
            PackageTransition::Revoke => {
                self.status = PackageStatus::Revoked;
            }
        }
        self.version_num += 1;
        Ok(())
    }
}

/// Resolve the active package set for a facility at a given date.
/// In production, this is cached in memory and invalidated on activation events.
pub fn resolve_active_packages(
    packages: &[InstalledPackage],
    effective_date: NaiveDate,
) -> Vec<&InstalledPackage> {
    packages
        .iter()
        .filter(|p| {
            p.status == PackageStatus::Activated
                && p.manifest
                    .effective_start
                    .map_or(true, |s| effective_date >= s)
                && p.manifest
                    .effective_end
                    .map_or(true, |e| effective_date <= e)
        })
        .collect()
}

/// Check if a package's dependencies are satisfied by the installed set.
pub fn check_dependencies(
    manifest: &PackageManifest,
    installed: &[InstalledPackage],
) -> AppResult<()> {
    for dep in &manifest.dependencies {
        let satisfied = installed.iter().any(|p| {
            p.package_id == dep.package_id
                && (p.status == PackageStatus::Activated || p.status == PackageStatus::Staged)
        });
        if !satisfied {
            return Err(AppError::Validation {
                message: format!(
                    "missing dependency: {} {}",
                    dep.package_id, dep.version_range
                ),
            });
        }
    }
    Ok(())
}

/// Repository trait for package persistence.
pub trait PackageRepo {
    fn create(&self, pkg: &InstalledPackage) -> AppResult<()>;
    fn find_by_id(&self, id: Uuid) -> AppResult<Option<InstalledPackage>>;
    fn find_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<InstalledPackage>>;
    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<InstalledPackage>>;
    fn update(&self, pkg: &InstalledPackage) -> AppResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclinic_core::types::new_id;

    fn test_actor() -> ActorContext {
        ActorContext {
            user_id: new_id(),
            facility_id: new_id(),
            device_id: new_id(),
            roles: vec!["admin".to_string()],
            purpose: "admin".to_string(),
            session_id: new_id(),
        }
    }

    fn test_manifest() -> PackageManifest {
        PackageManifest {
            package_id: "rw-deployment".to_string(),
            package_type: PackageType::Deployment,
            version: "1.0.0".to_string(),
            compatible_versions: ">=0.1.0".to_string(),
            dependencies: vec![],
            effective_start: None,
            effective_end: None,
            scope: PackageScope::Network,
            checksum: "abc123".to_string(),
            localization_coverage: vec!["en".to_string(), "rw".to_string()],
        }
    }

    fn test_package(facility_id: Uuid) -> InstalledPackage {
        InstalledPackage {
            id: new_id(),
            facility_id,
            package_id: "rw-deployment".to_string(),
            package_type: PackageType::Deployment,
            version: "1.0.0".to_string(),
            status: PackageStatus::Uploaded,
            manifest: test_manifest(),
            installed_at: Utc::now(),
            activated_at: None,
            rolled_back_at: None,
            installed_by: new_id(),
            version_num: 0,
        }
    }

    #[test]
    fn test_package_lifecycle() {
        let actor = test_actor();
        let mut pkg = test_package(actor.facility_id);

        pkg.apply_transition(PackageTransition::Verify, &actor).expect("verify");
        assert_eq!(pkg.status, PackageStatus::Verified);

        pkg.apply_transition(PackageTransition::Stage, &actor).expect("stage");
        assert_eq!(pkg.status, PackageStatus::Staged);

        pkg.apply_transition(PackageTransition::Activate, &actor).expect("activate");
        assert_eq!(pkg.status, PackageStatus::Activated);
        assert!(pkg.activated_at.is_some());
    }

    #[test]
    fn test_rollback_and_restage() {
        let actor = test_actor();
        let mut pkg = test_package(actor.facility_id);

        pkg.apply_transition(PackageTransition::Verify, &actor).expect("verify");
        pkg.apply_transition(PackageTransition::Stage, &actor).expect("stage");
        pkg.apply_transition(PackageTransition::Activate, &actor).expect("activate");
        pkg.apply_transition(PackageTransition::Rollback, &actor).expect("rollback");
        assert_eq!(pkg.status, PackageStatus::RolledBack);

        // Can re-stage after rollback
        pkg.apply_transition(PackageTransition::Stage, &actor).expect("re-stage");
        assert_eq!(pkg.status, PackageStatus::Staged);
    }

    #[test]
    fn test_revoked_is_terminal() {
        let actor = test_actor();
        let mut pkg = test_package(actor.facility_id);

        pkg.apply_transition(PackageTransition::Verify, &actor).expect("verify");
        pkg.apply_transition(PackageTransition::Revoke, &actor).expect("revoke");
        assert_eq!(pkg.status, PackageStatus::Revoked);
        assert!(pkg.allowed_transitions(&actor).is_empty());
    }

    #[test]
    fn test_dependency_check() {
        let manifest = PackageManifest {
            dependencies: vec![PackageDependency {
                package_id: "terminology-icd10".to_string(),
                version_range: ">=1.0".to_string(),
            }],
            ..test_manifest()
        };

        // No packages installed — should fail
        let result = check_dependencies(&manifest, &[]);
        assert!(result.is_err());

        // With dependency installed and activated
        let mut dep = test_package(new_id());
        dep.package_id = "terminology-icd10".to_string();
        dep.status = PackageStatus::Activated;
        let result = check_dependencies(&manifest, &[dep]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_resolve_active_packages() {
        let fid = new_id();
        let mut pkg1 = test_package(fid);
        pkg1.status = PackageStatus::Activated;

        let mut pkg2 = test_package(fid);
        pkg2.status = PackageStatus::Staged; // not activated

        let today = Utc::now().date_naive();
        let packages = [pkg1, pkg2];
        let active = resolve_active_packages(&packages, today);
        assert_eq!(active.len(), 1);
    }
}
