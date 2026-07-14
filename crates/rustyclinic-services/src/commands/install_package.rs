//! Install a `.rcpkg` package into the system.
//!
//! Parses the binary package, verifies checksums, extracts form definitions,
//! and creates an `InstalledPackage` record.

use chrono::Utc;
use uuid::Uuid;

use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use rustyclinic_forms::definition::FormDefinition;
use rustyclinic_packages::reader::PackageReader;
use rustyclinic_packages::{InstalledPackage, PackageStatus, PackageTransition, PackageType};

/// Input for the install_package command.
pub struct InstallPackageInput {
    /// Raw `.rcpkg` bytes.
    pub package_bytes: Vec<u8>,
    /// Skip Ed25519 signature verification (for dev mode).
    pub skip_signature_check: bool,
    pub verifying_key_hex: Option<String>,
}

/// Output of a successful package install.
pub struct InstallPackageOutput {
    /// The ID assigned to this installed package record.
    pub package_id: Uuid,
    /// The form IDs found and validated in the package.
    pub forms_installed: Vec<String>,
    /// The raw form JSON keyed by form ID (for storage).
    pub form_definitions: Vec<(String, String)>,
}

/// Execute the install_package command.
///
/// This:
/// 1. Parses the `.rcpkg` binary via `PackageReader`
/// 2. Verifies checksums
/// 3. Extracts and validates all form JSON files
pub fn execute(
    uow: &mut UnitOfWork<'_>,
    actor: &rustyclinic_core::types::ActorContext,
    input: InstallPackageInput,
) -> AppResult<(InstalledPackage, InstallPackageOutput)> {
    // Parse the package
    let reader = PackageReader::open(&input.package_bytes).map_err(|e| AppError::Validation {
        message: format!("failed to open package: {e}"),
    })?;

    if !input.skip_signature_check {
        let verifying_key = verifying_key_bytes(input.verifying_key_hex.as_deref())?;
        let sig_ok = reader.verify_signature(&verifying_key)?;
        if !sig_ok {
            return Err(AppError::Validation {
                message: "package signature verification failed".to_string(),
            });
        }
    }

    // Verify checksums
    let checksums_ok = reader
        .verify_checksums()
        .map_err(|e| AppError::Validation {
            message: format!("checksum verification failed: {e}"),
        })?;
    if !checksums_ok {
        return Err(AppError::Validation {
            message: "package checksum verification failed — file may be corrupted".to_string(),
        });
    }

    let header = reader.header();
    let manifest = header.manifest.clone();

    let mut forms_installed = Vec::new();
    let mut form_definitions = Vec::new();
    let mut artifacts = PersistedArtifacts::default();

    match &manifest.package_type {
        PackageType::Form => {
            let form_entries = reader.extract_forms().map_err(|e| AppError::Validation {
                message: format!("failed to extract forms: {e}"),
            })?;

            for (form_id, form_json) in &form_entries {
                // Validate that each form JSON is a valid FormDefinition
                let definition: FormDefinition =
                    serde_json::from_str(form_json).map_err(|e| AppError::Validation {
                        message: format!("form '{form_id}' has invalid definition: {e}"),
                    })?;

                // Validate that the form engine can load it (catches DAG cycles, etc.)
                rustyclinic_forms::engine::FormEngine::new(definition).map_err(|e| {
                    AppError::Validation {
                        message: format!("form '{form_id}' failed engine validation: {e}"),
                    }
                })?;

                forms_installed.push(form_id.clone());
                form_definitions.push((form_id.clone(), form_json.clone()));
            }

            artifacts.form_definitions = form_definitions.clone();
        }
        PackageType::Report => {
            artifacts.report_artifacts = extract_report_artifacts(&reader)?;
        }
        PackageType::Terminology => {
            artifacts.terminology_artifacts = extract_terminology_artifacts(&reader)?;
        }
        PackageType::Deployment => {
            artifacts.deployment_settings = extract_deployment_settings(&reader)?;
        }
        _ => {}
    }
    let id = Uuid::now_v7();

    let mut installed = InstalledPackage {
        id,
        facility_id: actor.facility_id,
        package_id: manifest.package_id.clone(),
        package_type: manifest.package_type.clone(),
        version: manifest.version.clone(),
        status: PackageStatus::Uploaded,
        manifest,
        installed_at: Utc::now(),
        activated_at: None,
        rolled_back_at: None,
        installed_by: actor.user_id,
        version_num: 0,
    };

    installed.apply_transition(PackageTransition::Verify, actor)?;
    installed.apply_transition(PackageTransition::Stage, actor)?;

    let output = InstallPackageOutput {
        package_id: id,
        forms_installed,
        form_definitions,
    };

    persist_package(uow, &installed, &artifacts)?;

    uow.record_audit(
        actor,
        "package.installed",
        "InstalledPackage",
        installed.id,
        serde_json::json!({
            "package_id": installed.package_id,
            "version": installed.version,
            "forms": output.forms_installed,
        }),
    );
    uow.record_outbox(
        actor.facility_id,
        "InstalledPackage",
        installed.id,
        "package.installed",
        serde_json::json!({
            "package_row_id": installed.id,
            "package_id": installed.package_id,
            "version": installed.version,
            "status": installed.status.to_string(),
        }),
    );
    uow.record_op_log(
        actor,
        "InstalledPackage",
        installed.id,
        serde_json::json!({
            "action": "install",
            "package_id": installed.package_id,
            "version": installed.version,
            "status": installed.status.to_string(),
        }),
    );

    Ok((installed, output))
}

fn persist_package(
    uow: &mut UnitOfWork<'_>,
    installed: &InstalledPackage,
    artifacts: &PersistedArtifacts,
) -> AppResult<()> {
    let manifest_json =
        serde_json::to_string(&installed.manifest).map_err(|e| AppError::Validation {
            message: format!("manifest serialize failed: {e}"),
        })?;

    uow.conn()
        .execute(
            "INSERT INTO installed_packages (id, facility_id, package_id, package_type, version, status, manifest, installed_at, activated_at, rolled_back_at, installed_by, version_num)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                installed.id.to_string(),
                installed.facility_id.to_string(),
                &installed.package_id,
                installed.package_type.to_string(),
                &installed.version,
                installed.status.to_string(),
                manifest_json,
                installed.installed_at.to_rfc3339(),
                installed.activated_at.map(|d| d.to_rfc3339()),
                installed.rolled_back_at.map(|d| d.to_rfc3339()),
                installed.installed_by.to_string(),
                installed.version_num as i64,
            ],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

    for (form_id, form_json) in &artifacts.form_definitions {
        uow.conn()
            .execute(
                "INSERT INTO package_forms (package_row_id, form_id, form_json) VALUES (?1, ?2, ?3)",
                rusqlite::params![installed.id.to_string(), form_id, form_json],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
    }

    for report in &artifacts.report_artifacts {
        uow.conn()
            .execute(
                "INSERT INTO package_report_artifacts (package_row_id, report_id, report_family, report_version, report_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    installed.id.to_string(),
                    &report.report_id,
                    &report.report_family,
                    &report.report_version,
                    &report.report_json,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
    }

    for artifact in &artifacts.terminology_artifacts {
        uow.conn()
            .execute(
                "INSERT INTO package_terminology_artifacts (package_row_id, artifact_id, terminology_system, artifact_type, artifact_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    installed.id.to_string(),
                    &artifact.artifact_id,
                    &artifact.terminology_system,
                    &artifact.artifact_type,
                    &artifact.artifact_json,
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
    }

    for (setting_key, setting_value) in &artifacts.deployment_settings {
        uow.conn()
            .execute(
                "INSERT INTO package_deployment_settings (package_row_id, setting_key, setting_value) VALUES (?1, ?2, ?3)",
                rusqlite::params![installed.id.to_string(), setting_key, setting_value],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
    }

    Ok(())
}

#[derive(Default)]
struct PersistedArtifacts {
    form_definitions: Vec<(String, String)>,
    report_artifacts: Vec<ReportArtifactRecord>,
    terminology_artifacts: Vec<TerminologyArtifactRecord>,
    deployment_settings: Vec<(String, String)>,
}

struct ReportArtifactRecord {
    report_id: String,
    report_family: Option<String>,
    report_version: Option<String>,
    report_json: String,
}

struct TerminologyArtifactRecord {
    artifact_id: String,
    terminology_system: String,
    artifact_type: String,
    artifact_json: String,
}

fn extract_report_artifacts(reader: &PackageReader) -> AppResult<Vec<ReportArtifactRecord>> {
    let reports = reader.extract_reports().map_err(|e| AppError::Validation {
        message: format!("failed to extract reports: {e}"),
    })?;

    reports
        .into_iter()
        .map(|(report_id, report_json)| {
            let value: serde_json::Value =
                serde_json::from_str(&report_json).map_err(|e| AppError::Validation {
                    message: format!("report '{report_id}' has invalid definition: {e}"),
                })?;

            Ok(ReportArtifactRecord {
                report_id,
                report_family: value
                    .get("family")
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string()),
                report_version: value
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string()),
                report_json,
            })
        })
        .collect()
}

fn extract_terminology_artifacts(
    reader: &PackageReader,
) -> AppResult<Vec<TerminologyArtifactRecord>> {
    let artifacts = reader
        .extract_terminology_artifacts()
        .map_err(|e| AppError::Validation {
            message: format!("failed to extract terminology artifacts: {e}"),
        })?;

    artifacts
        .into_iter()
        .map(|(artifact_id, artifact_json)| {
            let value: serde_json::Value =
                serde_json::from_str(&artifact_json).map_err(|e| AppError::Validation {
                    message: format!("terminology artifact '{artifact_id}' has invalid JSON: {e}"),
                })?;

            Ok(TerminologyArtifactRecord {
                artifact_id,
                terminology_system: value
                    .get("terminology_system")
                    .or_else(|| value.get("system"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                artifact_type: value
                    .get("artifact_type")
                    .or_else(|| value.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("artifact")
                    .to_string(),
                artifact_json,
            })
        })
        .collect()
}

fn extract_deployment_settings(reader: &PackageReader) -> AppResult<Vec<(String, String)>> {
    let Some(settings_json) =
        reader
            .extract_deployment_settings()
            .map_err(|e| AppError::Validation {
                message: format!("failed to extract deployment settings: {e}"),
            })?
    else {
        return Err(AppError::Validation {
            message: "deployment package missing deployment/settings.json artifact".to_string(),
        });
    };

    let value: serde_json::Value =
        serde_json::from_str(&settings_json).map_err(|e| AppError::Validation {
            message: format!("deployment settings JSON is invalid: {e}"),
        })?;
    let object = value.as_object().ok_or_else(|| AppError::Validation {
        message: "deployment settings must be a JSON object".to_string(),
    })?;

    let mut settings = object
        .iter()
        .map(|(key, setting_value)| {
            serde_json::to_string(setting_value)
                .map(|serialized| (key.clone(), serialized))
                .map_err(|e| AppError::Validation {
                    message: format!("failed to serialize deployment setting '{key}': {e}"),
                })
        })
        .collect::<AppResult<Vec<_>>>()?;

    settings.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(settings)
}

fn verifying_key_bytes(explicit_hex: Option<&str>) -> AppResult<[u8; 32]> {
    let hex = match explicit_hex {
        Some(h) => h.to_string(),
        None => std::env::var("RUSTYCLINIC_PACKAGE_VERIFYING_KEY_HEX").map_err(|_| {
            AppError::Validation {
                message: "missing verifying key; set RUSTYCLINIC_PACKAGE_VERIFYING_KEY_HEX or pass verifying_key_hex".to_string(),
            }
        })?,
    };

    let hex = hex.trim();
    if hex.len() != 64 {
        return Err(AppError::Validation {
            message: "verifying key hex must be 64 characters (32 bytes)".to_string(),
        });
    }

    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let idx = i * 2;
        let b = u8::from_str_radix(&hex[idx..idx + 2], 16).map_err(|_| AppError::Validation {
            message: "verifying key hex contains invalid characters".to_string(),
        })?;
        *byte = b;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclinic_core::types::{ActorContext, new_id};
    use rustyclinic_db::migration::run_migrations;
    use rustyclinic_packages::builder::PackageBuilder;
    use rustyclinic_packages::{PackageManifest, PackageScope, PackageType};

    fn test_manifest(pkg_id: &str) -> PackageManifest {
        test_manifest_with_type(pkg_id, PackageType::Form)
    }

    fn test_manifest_with_type(pkg_id: &str, package_type: PackageType) -> PackageManifest {
        PackageManifest {
            package_id: pkg_id.to_string(),
            package_type,
            version: "1.0.0".to_string(),
            compatible_versions: ">=0.1.0".to_string(),
            dependencies: vec![],
            effective_start: None,
            effective_end: None,
            scope: PackageScope::Facility,
            checksum: "test".to_string(),
            localization_coverage: vec!["en".to_string()],
        }
    }

    fn setup_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.pragma_update(None, "foreign_keys", "on").expect("fk");
        run_migrations(&conn).expect("migrations");
        conn
    }

    fn test_actor() -> ActorContext {
        ActorContext {
            user_id: new_id(),
            facility_id: new_id(),
            device_id: new_id(),
            roles: vec!["system_admin".to_string()],
            purpose: "clinical_care".to_string(),
            session_id: new_id(),
        }
    }

    #[test]
    fn contract_install_does_not_auto_activate_package() {
        let conn = setup_db();
        let actor = test_actor();
        let form_json = include_str!("../../../rustyclinic-packages/forms/general-encounter.json");

        let manifest = test_manifest("phase4-install-vs-activate");
        let mut builder = PackageBuilder::new(manifest);
        builder.add_form(form_json).expect("add form");
        let bytes = builder.build().expect("build");

        let mut uow = UnitOfWork::new(&conn);
        let input = InstallPackageInput {
            package_bytes: bytes,
            skip_signature_check: true,
            verifying_key_hex: None,
        };

        let (installed, _output) = execute(&mut uow, &actor, input).expect("install");
        uow.commit().expect("commit install");

        assert_eq!(
            installed.status,
            PackageStatus::Staged,
            "contract: install must stage artifacts but not activate runtime"
        );

        let mut uow = UnitOfWork::new(&conn);
        let activated = crate::commands::transition_package::execute(
            &mut uow,
            &actor,
            crate::commands::transition_package::TransitionPackageInput {
                installed_package_row_id: installed.id,
                transition: PackageTransition::Activate,
            },
        )
        .expect("explicit activation should succeed");
        uow.commit().expect("commit activation");

        assert_eq!(activated.status, PackageStatus::Activated);
    }

    #[test]
    fn install_general_encounter_package() {
        let conn = setup_db();
        let actor = test_actor();
        let form_json = include_str!("../../../rustyclinic-packages/forms/general-encounter.json");

        let manifest = test_manifest("general-encounter-pkg");
        let mut builder = PackageBuilder::new(manifest);
        builder
            .add_form(form_json)
            .expect("should add general encounter form");

        let bytes = builder.build().expect("should build package");

        let mut uow = UnitOfWork::new(&conn);
        let input = InstallPackageInput {
            package_bytes: bytes,
            skip_signature_check: true,
            verifying_key_hex: None,
        };

        let (installed, output) = execute(&mut uow, &actor, input).expect("should install package");
        uow.commit().expect("commit");
        assert_eq!(installed.package_id, "general-encounter-pkg");
        assert_eq!(installed.status, PackageStatus::Staged);
        assert_eq!(output.forms_installed, vec!["general-encounter"]);
        assert_eq!(output.form_definitions.len(), 1);

        let pkg_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM installed_packages", [], |r| r.get(0))
            .expect("count");
        assert_eq!(pkg_count, 1);

        let form_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM package_forms", [], |r| r.get(0))
            .expect("count");
        assert_eq!(form_count, 1);
    }

    #[test]
    fn install_anc_visit_package() {
        let conn = setup_db();
        let actor = test_actor();
        let form_json = include_str!("../../../rustyclinic-packages/forms/anc-visit.json");

        let manifest = test_manifest("anc-visit-pkg");
        let mut builder = PackageBuilder::new(manifest);
        builder
            .add_form(form_json)
            .expect("should add ANC visit form");

        let bytes = builder.build().expect("should build package");

        let mut uow = UnitOfWork::new(&conn);
        let input = InstallPackageInput {
            package_bytes: bytes,
            skip_signature_check: true,
            verifying_key_hex: None,
        };

        let (installed, output) = execute(&mut uow, &actor, input).expect("should install package");
        uow.commit().expect("commit");
        assert_eq!(installed.package_id, "anc-visit-pkg");
        assert_eq!(output.forms_installed, vec!["anc-visit"]);
    }

    #[test]
    fn install_report_package_persists_report_artifacts() {
        let conn = setup_db();
        let actor = test_actor();

        let manifest = test_manifest_with_type("reports-pkg", PackageType::Report);
        let mut builder = PackageBuilder::new(manifest);
        builder.add_file(
            "reports/weekly-summary.json",
            br#"{"id":"weekly-summary","family":"weekly","version":"2026.03","sql":"SELECT 1"}"#
                .to_vec(),
        );

        let bytes = builder.build().expect("should build package");

        let mut uow = UnitOfWork::new(&conn);
        let input = InstallPackageInput {
            package_bytes: bytes,
            skip_signature_check: true,
            verifying_key_hex: None,
        };

        let (installed, output) = execute(&mut uow, &actor, input).expect("should install package");
        uow.commit().expect("commit");

        assert_eq!(installed.status, PackageStatus::Staged);
        assert!(output.forms_installed.is_empty());

        let report_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM package_report_artifacts", [], |r| {
                r.get(0)
            })
            .expect("count reports");
        assert_eq!(report_count, 1);

        let (family, version): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT report_family, report_version FROM package_report_artifacts WHERE package_row_id = ?1 AND report_id = ?2",
                rusqlite::params![installed.id.to_string(), "weekly-summary"],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("load report row");
        assert_eq!(family.as_deref(), Some("weekly"));
        assert_eq!(version.as_deref(), Some("2026.03"));
    }

    #[test]
    fn install_terminology_package_persists_terminology_artifacts() {
        let conn = setup_db();
        let actor = test_actor();

        let manifest = test_manifest_with_type("terminology-pkg", PackageType::Terminology);
        let mut builder = PackageBuilder::new(manifest);
        builder.add_file(
            "terminology/icd10-core.json",
            br#"{"system":"ICD-10","artifact_type":"code_system","version":"2026"}"#.to_vec(),
        );

        let bytes = builder.build().expect("should build package");

        let mut uow = UnitOfWork::new(&conn);
        let input = InstallPackageInput {
            package_bytes: bytes,
            skip_signature_check: true,
            verifying_key_hex: None,
        };

        let (installed, _output) =
            execute(&mut uow, &actor, input).expect("should install package");
        uow.commit().expect("commit");

        assert_eq!(installed.status, PackageStatus::Staged);

        let (system, artifact_type): (String, String) = conn
            .query_row(
                "SELECT terminology_system, artifact_type FROM package_terminology_artifacts WHERE package_row_id = ?1 AND artifact_id = ?2",
                rusqlite::params![installed.id.to_string(), "icd10-core"],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("load terminology row");
        assert_eq!(system, "ICD-10");
        assert_eq!(artifact_type, "code_system");
    }

    #[test]
    fn install_deployment_package_persists_deployment_settings() {
        let conn = setup_db();
        let actor = test_actor();

        let manifest = test_manifest_with_type("deployment-pkg", PackageType::Deployment);
        let mut builder = PackageBuilder::new(manifest);
        builder.add_file(
            "deployment/settings.json",
            br#"{"locale":"rw","feature_flags":{"new_ui":true}}"#.to_vec(),
        );

        let bytes = builder.build().expect("should build package");

        let mut uow = UnitOfWork::new(&conn);
        let input = InstallPackageInput {
            package_bytes: bytes,
            skip_signature_check: true,
            verifying_key_hex: None,
        };

        let (installed, _output) =
            execute(&mut uow, &actor, input).expect("should install package");
        uow.commit().expect("commit");

        assert_eq!(installed.status, PackageStatus::Staged);

        let settings_count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM package_deployment_settings",
                [],
                |r| r.get(0),
            )
            .expect("count deployment settings");
        assert_eq!(settings_count, 2);

        let locale_value: String = conn
            .query_row(
                "SELECT setting_value FROM package_deployment_settings WHERE package_row_id = ?1 AND setting_key = ?2",
                rusqlite::params![installed.id.to_string(), "locale"],
                |r| r.get(0),
            )
            .expect("load locale setting");
        assert_eq!(locale_value, "\"rw\"");
    }

    #[test]
    fn install_rejects_invalid_form_json() {
        let conn = setup_db();
        let actor = test_actor();
        let bad_json = r#"{"id": "bad-form", "version": "1.0.0"}"#;

        let manifest = test_manifest("bad-pkg");
        let mut builder = PackageBuilder::new(manifest);
        builder
            .add_form(bad_json)
            .expect("builder accepts any JSON with id");

        let bytes = builder.build().expect("should build package");

        let mut uow = UnitOfWork::new(&conn);
        let input = InstallPackageInput {
            package_bytes: bytes,
            skip_signature_check: true,
            verifying_key_hex: None,
        };

        let result = execute(&mut uow, &actor, input);
        assert!(result.is_err());
    }

    #[test]
    fn install_rejects_corrupted_package() {
        let conn = setup_db();
        let actor = test_actor();
        let form_json = r#"{"id": "test", "version": "1.0.0", "title": "Test", "items": []}"#;

        let manifest = test_manifest("corrupt-pkg");
        let mut builder = PackageBuilder::new(manifest);
        builder.add_form(form_json).expect("add form");

        let mut bytes = builder.build().expect("build");
        // Corrupt the trailing checksum
        let len = bytes.len();
        bytes[len - 1] ^= 0xFF;

        let mut uow = UnitOfWork::new(&conn);
        let input = InstallPackageInput {
            package_bytes: bytes,
            skip_signature_check: true,
            verifying_key_hex: None,
        };

        let result = execute(&mut uow, &actor, input);
        assert!(result.is_err());
    }

    #[test]
    fn general_encounter_form_json_parses() {
        let form_json = include_str!("../../../rustyclinic-packages/forms/general-encounter.json");
        let definition: FormDefinition =
            serde_json::from_str(form_json).expect("should parse general encounter form");
        assert_eq!(definition.id, "general-encounter");
        assert_eq!(definition.version, "1.0.0");
        assert!(!definition.items.is_empty());
    }

    #[test]
    fn anc_visit_form_json_parses() {
        let form_json = include_str!("../../../rustyclinic-packages/forms/anc-visit.json");
        let definition: FormDefinition =
            serde_json::from_str(form_json).expect("should parse ANC visit form");
        assert_eq!(definition.id, "anc-visit");
        assert_eq!(definition.version, "1.0.0");
        assert!(!definition.items.is_empty());
    }

    #[test]
    fn general_encounter_engine_evaluates() {
        let form_json = include_str!("../../../rustyclinic-packages/forms/general-encounter.json");
        let definition: FormDefinition = serde_json::from_str(form_json).expect("should parse");
        let engine =
            rustyclinic_forms::engine::FormEngine::new(definition).expect("should create engine");

        // Evaluate with some sample data
        let mut values = std::collections::HashMap::new();
        values.insert("weight_kg".to_string(), serde_json::json!(65));
        values.insert("height_cm".to_string(), serde_json::json!(165));
        values.insert(
            "chief_complaint".to_string(),
            serde_json::json!("Fever and headache"),
        );
        values.insert(
            "treatment_plan".to_string(),
            serde_json::json!("Malaria test, antipyretics"),
        );
        values.insert(
            "primary_diagnosis".to_string(),
            serde_json::json!("malaria"),
        );

        let state = engine.evaluate(&values);

        // BMI should be computed: 65 / (1.65^2) = 23.9
        let bmi = state
            .computed_values
            .get("bmi")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        assert!((bmi - 23.9).abs() < 0.1);

        // other_diagnosis should be hidden (primary is "malaria", not "other")
        assert_eq!(state.visibility.get("other_diagnosis"), Some(&false));

        // follow_up_date should be hidden (follow_up_needed not set)
        assert_eq!(state.visibility.get("follow_up_date"), Some(&false));

        // Should be submittable (required fields filled)
        assert!(state.is_submittable);
    }

    #[test]
    fn general_encounter_skip_logic() {
        let form_json = include_str!("../../../rustyclinic-packages/forms/general-encounter.json");
        let definition: FormDefinition = serde_json::from_str(form_json).expect("should parse");
        let engine =
            rustyclinic_forms::engine::FormEngine::new(definition).expect("should create engine");

        let mut values = std::collections::HashMap::new();
        values.insert("weight_kg".to_string(), serde_json::json!(65));
        values.insert("height_cm".to_string(), serde_json::json!(165));
        values.insert(
            "chief_complaint".to_string(),
            serde_json::json!("Back pain"),
        );
        values.insert("treatment_plan".to_string(), serde_json::json!("Rest"));
        values.insert("primary_diagnosis".to_string(), serde_json::json!("other"));
        values.insert("follow_up_needed".to_string(), serde_json::json!(true));

        let state = engine.evaluate(&values);

        // "other" selected => other_diagnosis should be visible
        assert_eq!(state.visibility.get("other_diagnosis"), Some(&true));

        // follow_up_needed = true => follow_up_date should be visible
        assert_eq!(state.visibility.get("follow_up_date"), Some(&true));
    }

    #[test]
    fn anc_visit_engine_evaluates() {
        let form_json = include_str!("../../../rustyclinic-packages/forms/anc-visit.json");
        let definition: FormDefinition = serde_json::from_str(form_json).expect("should parse");
        let engine =
            rustyclinic_forms::engine::FormEngine::new(definition).expect("should create engine");

        let mut values = std::collections::HashMap::new();
        values.insert("weight_kg".to_string(), serde_json::json!(65));
        values.insert("height_cm".to_string(), serde_json::json!(160));
        values.insert("lmp_date".to_string(), serde_json::json!("2025-12-01"));
        values.insert("hiv_status".to_string(), serde_json::json!("negative"));

        let state = engine.evaluate(&values);

        // BMI should be computed: 65 / (1.6^2) = 25.4
        let bmi = state
            .computed_values
            .get("bmi")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        assert!((bmi - 25.4).abs() < 0.1);

        // Gestational age should be computed
        assert!(state.computed_values.contains_key("gestational_age_weeks"));

        // HIV negative => hiv_section hidden
        assert_eq!(state.visibility.get("hiv_section"), Some(&false));

        // PMTCT counseling should be hidden
        assert_eq!(state.visibility.get("pmtct_counseling_done"), Some(&false));
    }

    #[test]
    fn anc_visit_hiv_skip_logic() {
        let form_json = include_str!("../../../rustyclinic-packages/forms/anc-visit.json");
        let definition: FormDefinition = serde_json::from_str(form_json).expect("should parse");
        let engine =
            rustyclinic_forms::engine::FormEngine::new(definition).expect("should create engine");

        let mut values = std::collections::HashMap::new();
        values.insert("weight_kg".to_string(), serde_json::json!(65));
        values.insert("height_cm".to_string(), serde_json::json!(160));
        values.insert("lmp_date".to_string(), serde_json::json!("2025-12-01"));
        values.insert("hiv_status".to_string(), serde_json::json!("positive"));

        let state = engine.evaluate(&values);

        // HIV positive => hiv_section visible
        assert_eq!(state.visibility.get("hiv_section"), Some(&true));

        // on_art, arv_regimen, viral_load should be visible (within hiv_section)
        assert_eq!(state.visibility.get("on_art"), Some(&true));

        // arv_regimen conditional on on_art = true (not set yet, so hidden)
        assert_eq!(state.visibility.get("arv_regimen"), Some(&false));

        // PMTCT counseling should be visible for HIV positive
        assert_eq!(state.visibility.get("pmtct_counseling_done"), Some(&true));

        // Now set on_art = true
        let mut state2_values = values.clone();
        state2_values.insert("on_art".to_string(), serde_json::json!(true));
        let state2 = engine.evaluate(&state2_values);

        // arv_regimen and viral_load should now be visible
        assert_eq!(state2.visibility.get("arv_regimen"), Some(&true));
        assert_eq!(state2.visibility.get("viral_load"), Some(&true));
    }

    #[test]
    fn anc_visit_hemoglobin_warning() {
        let form_json = include_str!("../../../rustyclinic-packages/forms/anc-visit.json");
        let definition: FormDefinition = serde_json::from_str(form_json).expect("should parse");
        let engine =
            rustyclinic_forms::engine::FormEngine::new(definition).expect("should create engine");

        let mut values = std::collections::HashMap::new();
        values.insert("weight_kg".to_string(), serde_json::json!(65));
        values.insert("height_cm".to_string(), serde_json::json!(160));
        values.insert("lmp_date".to_string(), serde_json::json!("2025-12-01"));
        values.insert("hiv_status".to_string(), serde_json::json!("negative"));
        values.insert("hemoglobin".to_string(), serde_json::json!(8.5));

        let state = engine.evaluate(&values);

        // Low hemoglobin should produce a warning
        let hb_warnings: Vec<_> = state
            .validation_results
            .iter()
            .filter(|r| {
                r.link_id == "hemoglobin"
                    && r.severity == rustyclinic_forms::definition::Severity::Warning
            })
            .collect();
        assert!(
            !hb_warnings.is_empty(),
            "expected hemoglobin warning for value 8.5"
        );

        // Should still be submittable (warning, not error)
        assert!(state.is_submittable);
    }

    #[test]
    fn round_trip_general_encounter_package() {
        let form_json = include_str!("../../../rustyclinic-packages/forms/general-encounter.json");

        // Build package
        let manifest = test_manifest("general-encounter-pkg");
        let mut builder = PackageBuilder::new(manifest);
        builder.add_form(form_json).expect("add form");
        let bytes = builder.build().expect("build");

        // Read package back
        let reader = PackageReader::open(&bytes).expect("open");
        assert!(reader.verify_checksums().expect("verify"));

        let forms = reader.extract_forms().expect("extract");
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].0, "general-encounter");

        // Parse extracted form back to FormDefinition
        let definition: FormDefinition =
            serde_json::from_str(&forms[0].1).expect("parse extracted form");
        assert_eq!(definition.id, "general-encounter");

        // Load into engine
        let engine =
            rustyclinic_forms::engine::FormEngine::new(definition).expect("engine from extracted");
        let state = engine.evaluate(&std::collections::HashMap::new());
        // Empty form => not submittable (required fields)
        assert!(!state.is_submittable);
    }

    #[test]
    fn round_trip_anc_visit_package() {
        let form_json = include_str!("../../../rustyclinic-packages/forms/anc-visit.json");

        let manifest = test_manifest("anc-visit-pkg");
        let mut builder = PackageBuilder::new(manifest);
        builder.add_form(form_json).expect("add form");
        let bytes = builder.build().expect("build");

        let reader = PackageReader::open(&bytes).expect("open");
        assert!(reader.verify_checksums().expect("verify"));

        let forms = reader.extract_forms().expect("extract");
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].0, "anc-visit");

        let definition: FormDefinition =
            serde_json::from_str(&forms[0].1).expect("parse extracted form");
        assert_eq!(definition.id, "anc-visit");

        let engine =
            rustyclinic_forms::engine::FormEngine::new(definition).expect("engine from extracted");
        let state = engine.evaluate(&std::collections::HashMap::new());
        assert!(!state.is_submittable);
    }
}
