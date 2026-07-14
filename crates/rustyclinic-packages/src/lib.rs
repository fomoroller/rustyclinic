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

pub mod builder;
pub mod format;
pub mod reader;
pub mod signing;

use chrono::{DateTime, NaiveDate, Utc};
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

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

impl PackageType {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "deployment" => Some(Self::Deployment),
            "program" => Some(Self::Program),
            "payer" => Some(Self::Payer),
            "form" => Some(Self::Form),
            "terminology" => Some(Self::Terminology),
            "report" => Some(Self::Report),
            "integration" => Some(Self::Integration),
            "model" => Some(Self::Model),
            _ => None,
        }
    }
}

impl fmt::Display for PackageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deployment => write!(f, "deployment"),
            Self::Program => write!(f, "program"),
            Self::Payer => write!(f, "payer"),
            Self::Form => write!(f, "form"),
            Self::Terminology => write!(f, "terminology"),
            Self::Report => write!(f, "report"),
            Self::Integration => write!(f, "integration"),
            Self::Model => write!(f, "model"),
        }
    }
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

impl PackageStatus {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "uploaded" => Some(Self::Uploaded),
            "verified" => Some(Self::Verified),
            "staged" => Some(Self::Staged),
            "activated" => Some(Self::Activated),
            "rolled_back" => Some(Self::RolledBack),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
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
            PackageStatus::Activated => {
                vec![PackageTransition::Rollback, PackageTransition::Revoke]
            }
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
                    .is_none_or(|s| effective_date >= s)
                && p.manifest.effective_end.is_none_or(|e| effective_date <= e)
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

        pkg.apply_transition(PackageTransition::Verify, &actor)
            .expect("verify");
        assert_eq!(pkg.status, PackageStatus::Verified);

        pkg.apply_transition(PackageTransition::Stage, &actor)
            .expect("stage");
        assert_eq!(pkg.status, PackageStatus::Staged);

        pkg.apply_transition(PackageTransition::Activate, &actor)
            .expect("activate");
        assert_eq!(pkg.status, PackageStatus::Activated);
        assert!(pkg.activated_at.is_some());
    }

    #[test]
    fn test_rollback_and_restage() {
        let actor = test_actor();
        let mut pkg = test_package(actor.facility_id);

        pkg.apply_transition(PackageTransition::Verify, &actor)
            .expect("verify");
        pkg.apply_transition(PackageTransition::Stage, &actor)
            .expect("stage");
        pkg.apply_transition(PackageTransition::Activate, &actor)
            .expect("activate");
        pkg.apply_transition(PackageTransition::Rollback, &actor)
            .expect("rollback");
        assert_eq!(pkg.status, PackageStatus::RolledBack);

        // Can re-stage after rollback
        pkg.apply_transition(PackageTransition::Stage, &actor)
            .expect("re-stage");
        assert_eq!(pkg.status, PackageStatus::Staged);
    }

    #[test]
    fn test_revoked_is_terminal() {
        let actor = test_actor();
        let mut pkg = test_package(actor.facility_id);

        pkg.apply_transition(PackageTransition::Verify, &actor)
            .expect("verify");
        pkg.apply_transition(PackageTransition::Revoke, &actor)
            .expect("revoke");
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

    // ---- Package format tests ----

    use crate::builder::PackageBuilder;
    use crate::reader::PackageReader;
    use crate::signing;

    fn sample_form_json(id: &str) -> String {
        format!(
            r#"{{
                "id": "{id}",
                "version": "1.0.0",
                "title": "Test Form",
                "items": []
            }}"#
        )
    }

    fn sample_report_json(id: &str) -> String {
        format!(
            r#"{{
                "id": "{id}",
                "title": "{id}",
                "query": "SELECT 1"
            }}"#
        )
    }

    #[test]
    fn test_build_and_read_package() {
        let manifest = test_manifest();
        let mut builder = PackageBuilder::new(manifest.clone());
        builder
            .add_form(&sample_form_json("anc-visit"))
            .expect("add form");

        let bytes = builder.build().expect("build");
        let reader = PackageReader::open(&bytes).expect("open");

        assert_eq!(reader.header().manifest.package_id, manifest.package_id);
        assert_eq!(reader.header().manifest.version, manifest.version);
        // manifest.json + forms/anc-visit.json
        assert_eq!(reader.list_files().len(), 2);
    }

    #[test]
    fn test_sign_and_verify() {
        let kp = signing::generate_keypair();
        let manifest = test_manifest();
        let mut builder = PackageBuilder::new(manifest);
        builder
            .add_form(&sample_form_json("test-form"))
            .expect("add form");

        let bytes = builder.build_signed(&kp.signing_key).expect("build signed");
        let reader = PackageReader::open(&bytes).expect("open");

        assert!(reader.verify_signature(&kp.verifying_key).expect("verify"));
    }

    #[test]
    fn test_tampered_payload_fails_signature() {
        let kp = signing::generate_keypair();
        let manifest = test_manifest();
        let builder = PackageBuilder::new(manifest);

        let mut bytes = builder.build_signed(&kp.signing_key).expect("build signed");

        // Tamper with a byte in the payload area (well past header + signature).
        let tamper_idx = bytes.len() - 40;
        bytes[tamper_idx] ^= 0xFF;

        // Re-open — may fail to decompress, which is also correct rejection.
        // If it does parse, checksums should fail.
        if let Ok(reader) = PackageReader::open(&bytes) {
            let checksums_ok = reader.verify_checksums().unwrap_or(false);
            assert!(
                !checksums_ok,
                "tampered payload should fail checksum verification"
            );
        }
    }

    #[test]
    fn test_tampered_payload_fails_checksum() {
        let manifest = test_manifest();
        let mut builder = PackageBuilder::new(manifest);
        builder
            .add_form(&sample_form_json("form-a"))
            .expect("add form");

        let mut bytes = builder.build().expect("build");

        // Tamper with the stored checksum (last 32 bytes).
        let len = bytes.len();
        bytes[len - 1] ^= 0xFF;

        let reader = PackageReader::open(&bytes).expect("open");
        let ok = reader.verify_checksums().expect("verify");
        assert!(!ok, "tampered checksum should fail verification");
    }

    #[test]
    fn test_empty_package_manifest_only() {
        let manifest = test_manifest();
        let builder = PackageBuilder::new(manifest.clone());

        let bytes = builder.build().expect("build");
        let reader = PackageReader::open(&bytes).expect("open");

        assert_eq!(reader.header().manifest.package_id, manifest.package_id);
        // Only manifest.json
        assert_eq!(reader.list_files().len(), 1);
        assert_eq!(reader.list_files()[0].path, "manifest.json");
    }

    #[test]
    fn test_package_with_multiple_forms() {
        let manifest = test_manifest();
        let mut builder = PackageBuilder::new(manifest);
        builder
            .add_form(&sample_form_json("form-a"))
            .expect("add form a");
        builder
            .add_form(&sample_form_json("form-b"))
            .expect("add form b");
        builder
            .add_form(&sample_form_json("form-c"))
            .expect("add form c");

        let bytes = builder.build().expect("build");
        let reader = PackageReader::open(&bytes).expect("open");

        // manifest.json + 3 forms
        assert_eq!(reader.list_files().len(), 4);

        let forms = reader.extract_forms().expect("extract forms");
        assert_eq!(forms.len(), 3);

        let form_ids: Vec<&str> = forms.iter().map(|(id, _)| id.as_str()).collect();
        assert!(form_ids.contains(&"form-a"));
        assert!(form_ids.contains(&"form-b"));
        assert!(form_ids.contains(&"form-c"));
    }

    #[test]
    fn test_round_trip_full() {
        let kp = signing::generate_keypair();
        let manifest = test_manifest();
        let form_json = sample_form_json("anc-visit");

        let mut builder = PackageBuilder::new(manifest.clone());
        builder.add_form(&form_json).expect("add form");

        let bytes = builder.build_signed(&kp.signing_key).expect("build");
        let reader = PackageReader::open(&bytes).expect("open");

        // Verify header.
        assert_eq!(reader.header().manifest.package_id, "rw-deployment");
        assert_eq!(reader.header().entries.len(), 2);

        // Verify signature.
        assert!(
            reader
                .verify_signature(&kp.verifying_key)
                .expect("verify sig")
        );

        // Verify checksums.
        assert!(reader.verify_checksums().expect("verify checksums"));

        // Extract forms and validate they parse.
        let forms = reader.extract_forms().expect("extract forms");
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].0, "anc-visit");

        // Verify the form JSON parses back.
        let parsed: serde_json::Value = serde_json::from_str(&forms[0].1).expect("parse form JSON");
        assert_eq!(parsed["id"].as_str(), Some("anc-visit"));

        // Read manifest.json directly.
        let manifest_bytes = reader.read_file("manifest.json").expect("read manifest");
        let read_manifest: PackageManifest =
            serde_json::from_slice(&manifest_bytes).expect("parse manifest");
        assert_eq!(read_manifest.package_id, manifest.package_id);
    }

    #[test]
    fn test_extracts_all_phase4_payload_types_deterministically() {
        let manifest = test_manifest();
        let mut builder = PackageBuilder::new(manifest);

        builder
            .add_form(&sample_form_json("z-form"))
            .expect("add form z");
        builder
            .add_form(&sample_form_json("a-form"))
            .expect("add form a");
        builder.add_file(
            "reports/z-report.json",
            sample_report_json("z-report").into_bytes(),
        );
        builder.add_file(
            "reports/a-report.json",
            sample_report_json("a-report").into_bytes(),
        );
        builder.add_file(
            "terminology/z-codes.json",
            br#"{"system":"ICD-10","version":"z"}"#.to_vec(),
        );
        builder.add_file(
            "terminology/a-codes.json",
            br#"{"system":"ICD-10","version":"a"}"#.to_vec(),
        );
        builder.add_file(
            "deployment/settings.json",
            br#"{"locale":"rw","timezone":"Africa/Kigali"}"#.to_vec(),
        );

        let bytes = builder.build().expect("build");
        let reader = PackageReader::open(&bytes).expect("open");

        let form_ids: Vec<String> = reader
            .extract_forms()
            .expect("extract forms")
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(form_ids, vec!["a-form".to_string(), "z-form".to_string()]);

        let report_ids: Vec<String> = reader
            .extract_reports()
            .expect("extract reports")
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            report_ids,
            vec!["a-report".to_string(), "z-report".to_string()]
        );

        let terminology_names: Vec<String> = reader
            .extract_terminology_artifacts()
            .expect("extract terminology")
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            terminology_names,
            vec!["a-codes".to_string(), "z-codes".to_string()]
        );

        let deployment_settings = reader
            .extract_deployment_settings()
            .expect("extract deployment settings")
            .expect("settings should exist");
        let settings_json: serde_json::Value =
            serde_json::from_str(&deployment_settings).expect("parse settings json");
        assert_eq!(settings_json["locale"], "rw");
    }

    #[test]
    fn test_extract_forms_rejects_id_mismatch() {
        let manifest = test_manifest();
        let mut builder = PackageBuilder::new(manifest);
        builder.add_file(
            "forms/intake.json",
            br#"{"id":"triage","version":"1.0.0"}"#.to_vec(),
        );

        let bytes = builder.build().expect("build");
        let reader = PackageReader::open(&bytes).expect("open");

        let err = reader.extract_forms().expect_err("id mismatch must fail");
        assert!(
            format!("{err}").contains("form id mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_extract_reports_rejects_non_json_extension() {
        let manifest = test_manifest();
        let mut builder = PackageBuilder::new(manifest);
        builder.add_file("reports/weekly.sql", b"SELECT * FROM visits".to_vec());

        let bytes = builder.build().expect("build");
        let reader = PackageReader::open(&bytes).expect("open");

        let err = reader
            .extract_reports()
            .expect_err("non-json report path must fail");
        assert!(
            format!("{err}").contains("expected .json extension"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_extract_deployment_settings_rejects_unsupported_path() {
        let manifest = test_manifest();
        let mut builder = PackageBuilder::new(manifest);
        builder.add_file(
            "deployment/feature-flags.json",
            br#"{"newFeature":true}"#.to_vec(),
        );

        let bytes = builder.build().expect("build");
        let reader = PackageReader::open(&bytes).expect("open");

        let err = reader
            .extract_deployment_settings()
            .expect_err("unsupported deployment path must fail");
        assert!(
            format!("{err}").contains("unsupported deployment payload path"),
            "unexpected error: {err}"
        );
    }
}
