use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
use rustyclinic_packages::{
    InstalledPackage, PackageManifest, PackageStatus, PackageTransition, PackageType,
    check_dependencies,
};
use uuid::Uuid;

pub struct TransitionPackageInput {
    pub installed_package_row_id: Uuid,
    pub transition: PackageTransition,
}

pub fn execute(
    uow: &mut UnitOfWork<'_>,
    actor: &ActorContext,
    input: TransitionPackageInput,
) -> AppResult<InstalledPackage> {
    let pkg = load_package(uow.conn(), input.installed_package_row_id)?;
    if pkg.facility_id != actor.facility_id {
        return Err(AppError::AuthorizationDenied {
            reason: "cross-facility package transition denied".to_string(),
        });
    }

    let mut pkg2 = pkg.clone();
    if matches!(input.transition, PackageTransition::Activate) {
        let installed = load_packages_by_facility(uow.conn(), actor.facility_id)?;
        check_dependencies(&pkg2.manifest, &installed)?;
        ensure_no_activation_conflict(&pkg2, &installed)?;
    }
    pkg2.apply_transition(input.transition.clone(), actor)?;

    uow.conn()
        .execute(
            "UPDATE installed_packages
             SET status = ?1,
                 activated_at = ?2,
                 rolled_back_at = ?3,
                 version_num = ?4
             WHERE id = ?5",
            rusqlite::params![
                pkg2.status.to_string(),
                pkg2.activated_at.map(|d| d.to_rfc3339()),
                pkg2.rolled_back_at.map(|d| d.to_rfc3339()),
                pkg2.version_num as i64,
                pkg2.id.to_string(),
            ],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

    let event_type = format!("package.{}", input.transition);

    uow.record_audit(
        actor,
        &event_type,
        "InstalledPackage",
        pkg2.id,
        serde_json::json!({
            "package_id": pkg2.package_id,
            "version": pkg2.version,
            "status": pkg2.status.to_string(),
        }),
    );

    uow.record_outbox(
        actor.facility_id,
        "InstalledPackage",
        pkg2.id,
        &event_type,
        serde_json::json!({
            "package_id": pkg2.package_id,
            "version": pkg2.version,
            "status": pkg2.status.to_string(),
        }),
    );

    uow.record_op_log(
        actor,
        "InstalledPackage",
        pkg2.id,
        serde_json::json!({
            "transition": input.transition.to_string(),
            "package_id": pkg2.package_id,
            "version": pkg2.version,
        }),
    );

    Ok(pkg2)
}

fn ensure_no_activation_conflict(
    package: &InstalledPackage,
    installed: &[InstalledPackage],
) -> AppResult<()> {
    let overlaps = |candidate: &InstalledPackage| {
        if candidate.id == package.id
            || candidate.status != PackageStatus::Activated
            || candidate.package_id != package.package_id
        {
            return false;
        }

        let package_start = package.manifest.effective_start;
        let package_end = package.manifest.effective_end;
        let candidate_start = candidate.manifest.effective_start;
        let candidate_end = candidate.manifest.effective_end;

        let starts_before_candidate_ends = match (package_start, candidate_end) {
            (Some(start), Some(end)) => start <= end,
            _ => true,
        };
        let candidate_starts_before_package_ends = match (candidate_start, package_end) {
            (Some(start), Some(end)) => start <= end,
            _ => true,
        };

        starts_before_candidate_ends && candidate_starts_before_package_ends
    };

    if installed.iter().any(overlaps) {
        return Err(AppError::Validation {
            message: format!(
                "cannot activate package '{}': overlapping active version already exists",
                package.package_id
            ),
        });
    }

    Ok(())
}

fn load_package(conn: &rusqlite::Connection, id: Uuid) -> AppResult<InstalledPackage> {
    conn.query_row(
        "SELECT id, facility_id, package_id, package_type, version, status, manifest,
                installed_at, activated_at, rolled_back_at, installed_by, version_num
         FROM installed_packages
         WHERE id = ?1",
        rusqlite::params![id.to_string()],
        |row| {
            let id_str: String = row.get(0)?;
            let facility_str: String = row.get(1)?;
            let package_id: String = row.get(2)?;
            let package_type_str: String = row.get(3)?;
            let version: String = row.get(4)?;
            let status_str: String = row.get(5)?;
            let manifest_str: String = row.get(6)?;
            let installed_at: String = row.get(7)?;
            let activated_at: Option<String> = row.get(8)?;
            let rolled_back_at: Option<String> = row.get(9)?;
            let installed_by: String = row.get(10)?;
            let version_num: u32 = row.get(11)?;

            let manifest: PackageManifest = serde_json::from_str(&manifest_str).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

            let parse_dt = |s: &str| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now())
            };

            let package_type = PackageType::parse(&package_type_str).ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(
                    3,
                    "package_type".into(),
                    rusqlite::types::Type::Text,
                )
            })?;
            let status = PackageStatus::parse(&status_str).ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(5, "status".into(), rusqlite::types::Type::Text)
            })?;

            Ok(InstalledPackage {
                id: Uuid::parse_str(&id_str).unwrap_or_default(),
                facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
                package_id,
                package_type,
                version,
                status,
                manifest,
                installed_at: parse_dt(&installed_at),
                activated_at: activated_at.as_deref().map(parse_dt),
                rolled_back_at: rolled_back_at.as_deref().map(parse_dt),
                installed_by: Uuid::parse_str(&installed_by).unwrap_or_default(),
                version_num,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound {
            entity: "InstalledPackage",
            id,
        },
        _ => AppError::Database(e.to_string()),
    })
}

fn load_packages_by_facility(
    conn: &rusqlite::Connection,
    facility_id: Uuid,
) -> AppResult<Vec<InstalledPackage>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, facility_id, package_id, package_type, version, status, manifest,
                    installed_at, activated_at, rolled_back_at, installed_by, version_num
             FROM installed_packages
             WHERE facility_id = ?1",
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

    let rows = stmt
        .query_map(rusqlite::params![facility_id.to_string()], |row| {
            let id_str: String = row.get(0)?;
            let facility_str: String = row.get(1)?;
            let package_id: String = row.get(2)?;
            let package_type_str: String = row.get(3)?;
            let version: String = row.get(4)?;
            let status_str: String = row.get(5)?;
            let manifest_str: String = row.get(6)?;
            let installed_at: String = row.get(7)?;
            let activated_at: Option<String> = row.get(8)?;
            let rolled_back_at: Option<String> = row.get(9)?;
            let installed_by: String = row.get(10)?;
            let version_num: u32 = row.get(11)?;

            let manifest: PackageManifest = serde_json::from_str(&manifest_str).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

            let parse_dt = |s: &str| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now())
            };

            let package_type = PackageType::parse(&package_type_str).ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(
                    3,
                    "package_type".into(),
                    rusqlite::types::Type::Text,
                )
            })?;
            let status = PackageStatus::parse(&status_str).ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(5, "status".into(), rusqlite::types::Type::Text)
            })?;

            Ok(InstalledPackage {
                id: Uuid::parse_str(&id_str).unwrap_or_default(),
                facility_id: Uuid::parse_str(&facility_str).unwrap_or_default(),
                package_id,
                package_type,
                version,
                status,
                manifest,
                installed_at: parse_dt(&installed_at),
                activated_at: activated_at.as_deref().map(parse_dt),
                rolled_back_at: rolled_back_at.as_deref().map(parse_dt),
                installed_by: Uuid::parse_str(&installed_by).unwrap_or_default(),
                version_num,
            })
        })
        .map_err(|e| AppError::Database(e.to_string()))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(e.to_string()))
}
