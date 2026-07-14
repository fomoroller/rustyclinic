//! Integration tests for service commands.

#[cfg(test)]
mod test_cases {
    use chrono::Utc;
    use rusqlite::Connection;

    use rustyclinic_auth::session::{Session, SessionRepo, SessionState};
    use rustyclinic_auth::users::{User, UserRepo};
    use rustyclinic_clinical::queue::{QueueEntryRepo, QueueStatus, QueueTransition};
    use rustyclinic_core::types::{ActorContext, Sex, new_id};
    use rustyclinic_db::migration::run_migrations;
    use rustyclinic_db::sqlite::patient_repo::SqlitePatientRepo;
    use rustyclinic_db::sqlite::queue_repo::SqliteQueueRepo;
    use rustyclinic_db::sqlite::session_repo::SqliteSessionRepo;
    use rustyclinic_db::sqlite::unit_of_work::UnitOfWork;
    use rustyclinic_db::sqlite::user_repo::SqliteUserRepo;
    use rustyclinic_identity::Patient;
    use rustyclinic_packages::{
        InstalledPackage, PackageDependency, PackageManifest, PackageScope, PackageStatus,
        PackageTransition, PackageType,
    };

    use rustyclinic_identity::PatientRepo;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.pragma_update(None, "foreign_keys", "on").expect("fk");
        run_migrations(&conn).expect("migrations");
        conn
    }

    fn test_actor() -> ActorContext {
        ActorContext {
            user_id: new_id(),
            facility_id: new_id(),
            device_id: new_id(),
            roles: vec!["nurse".to_string()],
            purpose: "clinical_care".to_string(),
            session_id: new_id(),
        }
    }

    fn test_patient(facility_id: uuid::Uuid) -> Patient {
        let now = Utc::now();
        Patient {
            id: new_id(),
            facility_id,
            given_name: "Uwimana".to_string(),
            family_name: "Marie".to_string(),
            sex: Sex::Female,
            date_of_birth: Some(chrono::NaiveDate::from_ymd_opt(1990, 5, 15).expect("date")),
            phone: Some("+250781234567".to_string()),
            address: None,
            national_id: None,
            created_at: now,
            updated_at: now,
            version: 0,
        }
    }

    fn test_package_manifest(package_id: &str, package_type: PackageType) -> PackageManifest {
        PackageManifest {
            package_id: package_id.to_string(),
            package_type,
            version: "1.0.0".to_string(),
            compatible_versions: "*".to_string(),
            dependencies: vec![],
            effective_start: None,
            effective_end: None,
            scope: PackageScope::Facility,
            checksum: "abc123".to_string(),
            localization_coverage: vec![],
        }
    }

    fn insert_installed_package(conn: &Connection, pkg: &InstalledPackage) {
        conn.execute(
            "INSERT INTO installed_packages (id, facility_id, package_id, package_type, version, status, manifest, installed_at, activated_at, rolled_back_at, installed_by, version_num)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                pkg.id.to_string(),
                pkg.facility_id.to_string(),
                &pkg.package_id,
                pkg.package_type.to_string(),
                &pkg.version,
                pkg.status.to_string(),
                serde_json::to_string(&pkg.manifest).expect("manifest json"),
                pkg.installed_at.to_rfc3339(),
                pkg.activated_at.map(|dt| dt.to_rfc3339()),
                pkg.rolled_back_at.map(|dt| dt.to_rfc3339()),
                pkg.installed_by.to_string(),
                pkg.version_num as i64,
            ],
        )
        .expect("insert package");
    }

    #[test]
    fn test_transition_queue_command() {
        let conn = setup_db();
        let patient_repo = SqlitePatientRepo::new(&conn);
        let queue_repo = SqliteQueueRepo::new(&conn);
        let actor = test_actor();

        let patient = test_patient(actor.facility_id);
        patient_repo.create(&patient).expect("create patient");

        // Enqueue
        let mut uow = UnitOfWork::new(&conn);
        let entry_id = crate::commands::enqueue_patient::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::enqueue_patient::EnqueuePatientInput {
                patient_id: patient.id,
                service_type: "consultation".to_string(),
            },
        )
        .expect("enqueue");
        uow.commit().expect("commit enqueue");

        // Call
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::transition_queue::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::transition_queue::TransitionQueueInput {
                queue_entry_id: entry_id,
                transition: QueueTransition::Call,
                assigned_to: None,
            },
        )
        .expect("call");
        uow.commit().expect("commit call");

        let entry = queue_repo
            .find_by_id(entry_id)
            .expect("find")
            .expect("exists");
        assert_eq!(entry.status, QueueStatus::Called);

        // BeginService
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::transition_queue::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::transition_queue::TransitionQueueInput {
                queue_entry_id: entry_id,
                transition: QueueTransition::BeginService,
                assigned_to: None,
            },
        )
        .expect("begin");
        uow.commit().expect("commit begin");

        let entry = queue_repo
            .find_by_id(entry_id)
            .expect("find")
            .expect("exists");
        assert_eq!(entry.status, QueueStatus::InService);
    }

    #[test]
    fn test_lock_and_unlock_session() {
        let conn = setup_db();
        let user_repo = SqliteUserRepo::new(&conn);
        let session_repo = SqliteSessionRepo::new(&conn);
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(),
            facility_id,
            username: "test_lock".to_string(),
            display_name: "Test Lock User".to_string(),
            roles: vec!["nurse".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        let password = "testpassword123";
        let password_hash = rustyclinic_auth::credentials::hash_credential(password).expect("hash");
        user_repo
            .create(&user, &password_hash)
            .expect("create user");

        let session = Session::new(
            user.id,
            facility_id,
            new_id(),
            vec!["nurse".to_string()],
            "password",
        );
        session_repo.create(&session).expect("create session");

        // Lock
        crate::commands::lock_session::execute(
            &session_repo,
            crate::commands::lock_session::LockSessionInput {
                session_id: session.id,
            },
        )
        .expect("lock");

        let locked = session_repo
            .find_by_id(session.id)
            .expect("find")
            .expect("exists");
        assert_eq!(locked.state, SessionState::Locked);

        // Unlock with password (PIN fallback)
        crate::commands::unlock_session::execute(
            &session_repo,
            &user_repo,
            crate::commands::unlock_session::UnlockSessionInput {
                session_id: session.id,
                pin: password.to_string(),
            },
        )
        .expect("unlock");

        let unlocked = session_repo
            .find_by_id(session.id)
            .expect("find")
            .expect("exists");
        assert_eq!(unlocked.state, SessionState::Active);
    }

    #[test]
    fn test_unlock_session_uses_configured_pin_hash() {
        let conn = setup_db();
        let user_repo = SqliteUserRepo::new(&conn);
        let session_repo = SqliteSessionRepo::new(&conn);
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(),
            facility_id,
            username: "pin_user".to_string(),
            display_name: "Pin User".to_string(),
            roles: vec!["nurse".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };

        let password_hash =
            rustyclinic_auth::credentials::hash_credential("password123").expect("hash");
        user_repo
            .create(&user, &password_hash)
            .expect("create user");

        let pin_hash = rustyclinic_auth::credentials::hash_credential("1234").expect("hash pin");
        user_repo
            .update_pin_hash(user.id, &pin_hash)
            .expect("set pin hash");

        let session = Session::new(
            user.id,
            facility_id,
            new_id(),
            vec!["nurse".to_string()],
            "password",
        );
        session_repo.create(&session).expect("create session");

        crate::commands::lock_session::execute(
            &session_repo,
            crate::commands::lock_session::LockSessionInput {
                session_id: session.id,
            },
        )
        .expect("lock");

        crate::commands::unlock_session::execute(
            &session_repo,
            &user_repo,
            crate::commands::unlock_session::UnlockSessionInput {
                session_id: session.id,
                pin: "1234".to_string(),
            },
        )
        .expect("unlock with pin");

        let unlocked = session_repo
            .find_by_id(session.id)
            .expect("find")
            .expect("exists");
        assert_eq!(unlocked.state, SessionState::Active);
    }

    #[test]
    fn test_unlock_session_rejects_password_when_pin_is_configured() {
        let conn = setup_db();
        let user_repo = SqliteUserRepo::new(&conn);
        let session_repo = SqliteSessionRepo::new(&conn);
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(),
            facility_id,
            username: "pin_user_reject".to_string(),
            display_name: "Pin Reject User".to_string(),
            roles: vec!["nurse".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };

        let password_hash =
            rustyclinic_auth::credentials::hash_credential("password123").expect("hash");
        user_repo
            .create(&user, &password_hash)
            .expect("create user");

        let pin_hash = rustyclinic_auth::credentials::hash_credential("5678").expect("hash pin");
        user_repo
            .update_pin_hash(user.id, &pin_hash)
            .expect("set pin hash");

        let session = Session::new(
            user.id,
            facility_id,
            new_id(),
            vec!["nurse".to_string()],
            "password",
        );
        session_repo.create(&session).expect("create session");

        crate::commands::lock_session::execute(
            &session_repo,
            crate::commands::lock_session::LockSessionInput {
                session_id: session.id,
            },
        )
        .expect("lock");

        let result = crate::commands::unlock_session::execute(
            &session_repo,
            &user_repo,
            crate::commands::unlock_session::UnlockSessionInput {
                session_id: session.id,
                pin: "password123".to_string(),
            },
        );

        assert!(result.is_err());

        let locked = session_repo
            .find_by_id(session.id)
            .expect("find")
            .expect("exists");
        assert_eq!(locked.state, SessionState::Locked);
    }

    #[test]
    fn test_login_reports_pin_setup_required_when_missing() {
        let conn = setup_db();
        let user_repo = SqliteUserRepo::new(&conn);
        let session_repo = SqliteSessionRepo::new(&conn);
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(),
            facility_id,
            username: "login_pin_missing".to_string(),
            display_name: "Login Missing Pin".to_string(),
            roles: vec!["nurse".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };

        let password = "pass1234";
        let password_hash = rustyclinic_auth::credentials::hash_credential(password).expect("hash");
        user_repo
            .create(&user, &password_hash)
            .expect("create user");

        let output = crate::commands::login::execute(
            &user_repo,
            &session_repo,
            crate::commands::login::LoginInput {
                facility_id,
                username: user.username.clone(),
                password: password.to_string(),
                device_id: new_id(),
            },
        )
        .expect("login");

        assert!(output.requires_pin_setup);
    }

    #[test]
    fn test_login_reports_pin_setup_not_required_when_pin_exists() {
        let conn = setup_db();
        let user_repo = SqliteUserRepo::new(&conn);
        let session_repo = SqliteSessionRepo::new(&conn);
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(),
            facility_id,
            username: "login_pin_exists".to_string(),
            display_name: "Login Has Pin".to_string(),
            roles: vec!["nurse".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };

        let password = "pass1234";
        let password_hash = rustyclinic_auth::credentials::hash_credential(password).expect("hash");
        user_repo
            .create(&user, &password_hash)
            .expect("create user");

        let pin_hash = rustyclinic_auth::credentials::hash_credential("1234").expect("pin hash");
        user_repo
            .update_pin_hash(user.id, &pin_hash)
            .expect("set pin hash");

        let output = crate::commands::login::execute(
            &user_repo,
            &session_repo,
            crate::commands::login::LoginInput {
                facility_id,
                username: user.username.clone(),
                password: password.to_string(),
                device_id: new_id(),
            },
        )
        .expect("login");

        assert!(!output.requires_pin_setup);
    }

    #[test]
    fn test_login_evicts_oldest_locked_session_when_cap_reached() {
        let conn = setup_db();
        let user_repo = SqliteUserRepo::new(&conn);
        let session_repo = SqliteSessionRepo::new(&conn);
        let now = Utc::now();
        let facility_id = new_id();
        let device_id = new_id();

        for idx in 0..3 {
            let user = User {
                id: new_id(),
                facility_id,
                username: format!("locked_user_{idx}"),
                display_name: format!("Locked User {idx}"),
                roles: vec!["nurse".to_string()],
                active: true,
                created_at: now,
                updated_at: now,
            };

            let password_hash =
                rustyclinic_auth::credentials::hash_credential(&format!("pass{idx}1234"))
                    .expect("hash");
            user_repo
                .create(&user, &password_hash)
                .expect("create user");

            let mut session = Session::new(
                user.id,
                facility_id,
                device_id,
                vec!["nurse".to_string()],
                "password",
            );
            session.created_at = now + chrono::Duration::minutes(idx as i64);
            session.last_active = session.created_at;
            session_repo.create(&session).expect("create session");
            session.lock();
            session.locked_at = Some(now + chrono::Duration::minutes(idx as i64));
            session_repo.update(&session).expect("lock session");
        }

        let new_user = User {
            id: new_id(),
            facility_id,
            username: "new_login_user".to_string(),
            display_name: "New Login User".to_string(),
            roles: vec!["clerk".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        let new_password = "newpass1234";
        let new_password_hash =
            rustyclinic_auth::credentials::hash_credential(new_password).expect("hash");
        user_repo
            .create(&new_user, &new_password_hash)
            .expect("create new user");

        let output = crate::commands::login::execute(
            &user_repo,
            &session_repo,
            crate::commands::login::LoginInput {
                facility_id,
                username: new_user.username.clone(),
                password: new_password.to_string(),
                device_id,
            },
        )
        .expect("login should succeed");

        let sessions = session_repo
            .find_active_by_device(device_id)
            .expect("sessions by device");
        let locked_sessions = sessions
            .iter()
            .filter(|session| session.state == SessionState::Locked)
            .count();
        assert_eq!(
            locked_sessions, 2,
            "oldest locked session should be evicted"
        );

        let terminated_count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE device_id = ?1 AND state = 'terminated'",
                rusqlite::params![device_id.to_string()],
                |row| row.get(0),
            )
            .expect("terminated count");
        assert_eq!(
            terminated_count, 1,
            "exactly one locked session should be terminated"
        );

        let oldest_state: String = conn
            .query_row(
                "SELECT state FROM sessions WHERE device_id = ?1 ORDER BY created_at ASC LIMIT 1",
                rusqlite::params![device_id.to_string()],
                |row| row.get(0),
            )
            .expect("oldest session state");
        assert_eq!(oldest_state, "terminated");

        let active_session = session_repo
            .find_by_id(output.session_id)
            .expect("find new session")
            .expect("new session exists");
        assert_eq!(active_session.state, SessionState::Active);
    }

    #[test]
    fn test_activate_package_rejects_missing_dependency() {
        let conn = setup_db();
        let actor = test_actor();
        let mut uow = UnitOfWork::new(&conn);

        let mut manifest = test_package_manifest("report-pack", PackageType::Report);
        manifest.dependencies.push(PackageDependency {
            package_id: "deployment-pack".to_string(),
            version_range: ">=1.0.0".to_string(),
        });

        let pkg = InstalledPackage {
            id: new_id(),
            facility_id: actor.facility_id,
            package_id: "report-pack".to_string(),
            package_type: PackageType::Report,
            version: "1.0.0".to_string(),
            status: PackageStatus::Staged,
            manifest,
            installed_at: Utc::now(),
            activated_at: None,
            rolled_back_at: None,
            installed_by: actor.user_id,
            version_num: 0,
        };
        insert_installed_package(&conn, &pkg);

        let err = crate::commands::transition_package::execute(
            &mut uow,
            &actor,
            crate::commands::transition_package::TransitionPackageInput {
                installed_package_row_id: pkg.id,
                transition: PackageTransition::Activate,
            },
        )
        .expect_err("activation should fail without dependency");

        assert!(format!("{err}").contains("missing dependency"));
    }

    #[test]
    fn test_activate_package_rejects_overlapping_active_package() {
        let conn = setup_db();
        let actor = test_actor();
        let mut uow = UnitOfWork::new(&conn);

        let mut active_manifest = test_package_manifest("queue-defaults", PackageType::Deployment);
        active_manifest.effective_start = chrono::NaiveDate::from_ymd_opt(2026, 1, 1);
        active_manifest.effective_end = chrono::NaiveDate::from_ymd_opt(2026, 12, 31);

        let active_pkg = InstalledPackage {
            id: new_id(),
            facility_id: actor.facility_id,
            package_id: "queue-defaults".to_string(),
            package_type: PackageType::Deployment,
            version: "1.0.0".to_string(),
            status: PackageStatus::Activated,
            manifest: active_manifest,
            installed_at: Utc::now(),
            activated_at: Some(Utc::now()),
            rolled_back_at: None,
            installed_by: actor.user_id,
            version_num: 1,
        };
        insert_installed_package(&conn, &active_pkg);

        let mut staged_manifest = test_package_manifest("queue-defaults", PackageType::Deployment);
        staged_manifest.effective_start = chrono::NaiveDate::from_ymd_opt(2026, 6, 1);
        staged_manifest.effective_end = chrono::NaiveDate::from_ymd_opt(2027, 6, 1);
        let staged_pkg = InstalledPackage {
            id: new_id(),
            facility_id: actor.facility_id,
            package_id: "queue-defaults".to_string(),
            package_type: PackageType::Deployment,
            version: "2.0.0".to_string(),
            status: PackageStatus::Staged,
            manifest: staged_manifest,
            installed_at: Utc::now(),
            activated_at: None,
            rolled_back_at: None,
            installed_by: actor.user_id,
            version_num: 0,
        };
        insert_installed_package(&conn, &staged_pkg);

        let err = crate::commands::transition_package::execute(
            &mut uow,
            &actor,
            crate::commands::transition_package::TransitionPackageInput {
                installed_package_row_id: staged_pkg.id,
                transition: PackageTransition::Activate,
            },
        )
        .expect_err("activation should fail for overlapping package");

        assert!(format!("{err}").contains("overlapping active version"));
    }

    #[test]
    fn test_create_and_complete_encounter() {
        let conn = setup_db();
        let patient_repo = SqlitePatientRepo::new(&conn);
        let queue_repo = SqliteQueueRepo::new(&conn);
        let actor = test_actor();

        // Create user for FK
        let user_repo = SqliteUserRepo::new(&conn);
        let now = Utc::now();
        let user = User {
            id: actor.user_id,
            facility_id: actor.facility_id,
            username: "provider".to_string(),
            display_name: "Provider".to_string(),
            roles: vec!["physician".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        let patient = test_patient(actor.facility_id);
        patient_repo.create(&patient).expect("create patient");

        // Enqueue
        let mut uow = UnitOfWork::new(&conn);
        let entry_id = crate::commands::enqueue_patient::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::enqueue_patient::EnqueuePatientInput {
                patient_id: patient.id,
                service_type: "consultation".to_string(),
            },
        )
        .expect("enqueue");
        uow.commit().expect("commit enqueue");

        // Call
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::transition_queue::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::transition_queue::TransitionQueueInput {
                queue_entry_id: entry_id,
                transition: QueueTransition::Call,
                assigned_to: None,
            },
        )
        .expect("call");
        uow.commit().expect("commit call");

        // Create encounter
        let mut uow = UnitOfWork::new(&conn);
        let output = crate::commands::create_encounter::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::create_encounter::CreateEncounterInput {
                queue_entry_id: entry_id,
                provider_id: actor.user_id,
            },
        )
        .expect("create encounter");
        uow.commit().expect("commit encounter");

        // Queue should be InService
        let entry = queue_repo
            .find_by_id(entry_id)
            .expect("find")
            .expect("exists");
        assert_eq!(entry.status, QueueStatus::InService);

        // Encounter exists
        let enc_status: String = conn
            .query_row(
                "SELECT status FROM encounters WHERE id = ?1",
                rusqlite::params![output.encounter_id.to_string()],
                |row| row.get(0),
            )
            .expect("find encounter");
        assert_eq!(enc_status, "in_progress");

        // Complete encounter
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::complete_encounter::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::complete_encounter::CompleteEncounterInput {
                encounter_id: output.encounter_id,
                queue_entry_id: entry_id,
                visit_notes: "Fever, prescribed antimalarials.".to_string(),
            },
        )
        .expect("complete encounter");
        uow.commit().expect("commit complete");

        // Encounter completed
        let enc_status: String = conn
            .query_row(
                "SELECT status FROM encounters WHERE id = ?1",
                rusqlite::params![output.encounter_id.to_string()],
                |row| row.get(0),
            )
            .expect("find encounter");
        assert_eq!(enc_status, "completed");

        // Queue completed
        let entry = queue_repo
            .find_by_id(entry_id)
            .expect("find")
            .expect("exists");
        assert_eq!(entry.status, QueueStatus::Completed);
    }

    // ===== Multi-department workflow tests =====

    #[test]
    fn test_create_lab_order_enqueues_patient_in_lab() {
        let conn = setup_db();
        let patient_repo = SqlitePatientRepo::new(&conn);
        let queue_repo = SqliteQueueRepo::new(&conn);
        let lab_order_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabOrderRepo::new(&conn);
        let lab_test_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabTestRepo::new(&conn);
        let actor = test_actor();

        // Create user for FK
        let user_repo = SqliteUserRepo::new(&conn);
        let now = Utc::now();
        let user = User {
            id: actor.user_id,
            facility_id: actor.facility_id,
            username: "labdoc".to_string(),
            display_name: "Lab Doctor".to_string(),
            roles: vec!["physician".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        let patient = test_patient(actor.facility_id);
        patient_repo.create(&patient).expect("create patient");

        // Enqueue and begin encounter
        let mut uow = UnitOfWork::new(&conn);
        let entry_id = crate::commands::enqueue_patient::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::enqueue_patient::EnqueuePatientInput {
                patient_id: patient.id,
                service_type: "consultation".to_string(),
            },
        )
        .expect("enqueue");
        uow.commit().expect("commit");

        // Call and begin service
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::transition_queue::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::transition_queue::TransitionQueueInput {
                queue_entry_id: entry_id,
                transition: QueueTransition::Call,
                assigned_to: None,
            },
        )
        .expect("call");
        uow.commit().expect("commit");

        // Create encounter
        let mut uow = UnitOfWork::new(&conn);
        let enc_output = crate::commands::create_encounter::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::create_encounter::CreateEncounterInput {
                queue_entry_id: entry_id,
                provider_id: actor.user_id,
            },
        )
        .expect("create encounter");
        uow.commit().expect("commit");

        // Create lab order
        let mut uow = UnitOfWork::new(&conn);
        let lab_output = crate::commands::create_lab_order::execute(
            &mut uow,
            &lab_order_repo,
            &queue_repo,
            &lab_test_repo,
            &actor,
            crate::commands::create_lab_order::CreateLabOrderInput {
                encounter_id: enc_output.encounter_id,
                patient_id: patient.id,
                tests: vec![rustyclinic_clinical::lab::LabTest {
                    test_code: "malaria_rdt".to_string(),
                    test_name: "Malaria RDT".to_string(),
                    result: None,
                    result_value: None,
                    unit: None,
                    reference_range: None,
                    is_abnormal: false,
                    resulted_at: None,
                    resulted_by: None,
                }],
                specimen_type: Some("Blood".to_string()),
                priority: rustyclinic_clinical::Priority::Routine,
                notes: None,
            },
        )
        .expect("create lab order");
        uow.commit().expect("commit");

        // Verify patient appears in lab queue
        let lab_entry = queue_repo
            .find_by_id(lab_output.queue_entry_id)
            .expect("find")
            .expect("exists");
        assert_eq!(lab_entry.department, "lab");
        assert_eq!(lab_entry.status, QueueStatus::Waiting);
        assert_eq!(lab_entry.encounter_id, Some(enc_output.encounter_id));

        // Verify lab order was created
        use rustyclinic_clinical::lab::LabOrderRepo;
        let order = lab_order_repo
            .find_by_id(lab_output.order_id)
            .expect("find")
            .expect("exists");
        assert_eq!(order.status, rustyclinic_clinical::lab::LabStatus::Ordered);
    }

    #[test]
    fn test_complete_lab_results_marks_order_done() {
        let conn = setup_db();
        let patient_repo = SqlitePatientRepo::new(&conn);
        let queue_repo = SqliteQueueRepo::new(&conn);
        let lab_order_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabOrderRepo::new(&conn);
        let lab_test_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabTestRepo::new(&conn);
        let actor = test_actor();

        let user_repo = SqliteUserRepo::new(&conn);
        let now = Utc::now();
        let user = User {
            id: actor.user_id,
            facility_id: actor.facility_id,
            username: "labtech".to_string(),
            display_name: "Lab Tech".to_string(),
            roles: vec!["lab_tech".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        let patient = test_patient(actor.facility_id);
        patient_repo.create(&patient).expect("create patient");

        // Enqueue, call, create encounter
        let mut uow = UnitOfWork::new(&conn);
        let entry_id = crate::commands::enqueue_patient::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::enqueue_patient::EnqueuePatientInput {
                patient_id: patient.id,
                service_type: "consultation".to_string(),
            },
        )
        .expect("enqueue");
        uow.commit().expect("commit");

        let mut uow = UnitOfWork::new(&conn);
        crate::commands::transition_queue::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::transition_queue::TransitionQueueInput {
                queue_entry_id: entry_id,
                transition: QueueTransition::Call,
                assigned_to: None,
            },
        )
        .expect("call");
        uow.commit().expect("commit");

        let mut uow = UnitOfWork::new(&conn);
        let enc_output = crate::commands::create_encounter::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::create_encounter::CreateEncounterInput {
                queue_entry_id: entry_id,
                provider_id: actor.user_id,
            },
        )
        .expect("encounter");
        uow.commit().expect("commit");

        // Create lab order
        let mut uow = UnitOfWork::new(&conn);
        let lab_output = crate::commands::create_lab_order::execute(
            &mut uow,
            &lab_order_repo,
            &queue_repo,
            &lab_test_repo,
            &actor,
            crate::commands::create_lab_order::CreateLabOrderInput {
                encounter_id: enc_output.encounter_id,
                patient_id: patient.id,
                tests: vec![rustyclinic_clinical::lab::LabTest {
                    test_code: "cbc".to_string(),
                    test_name: "Complete Blood Count".to_string(),
                    result: None,
                    result_value: None,
                    unit: None,
                    reference_range: None,
                    is_abnormal: false,
                    resulted_at: None,
                    resulted_by: None,
                }],
                specimen_type: Some("Blood".to_string()),
                priority: rustyclinic_clinical::Priority::Routine,
                notes: None,
            },
        )
        .expect("create lab order");
        uow.commit().expect("commit");

        // Complete lab order with results
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::complete_lab_order::execute(
            &mut uow,
            &lab_order_repo,
            &queue_repo,
            &lab_test_repo,
            &actor,
            crate::commands::complete_lab_order::CompleteLabOrderInput {
                order_id: lab_output.order_id,
                queue_entry_id: lab_output.queue_entry_id,
                results: vec![rustyclinic_clinical::lab::LabTest {
                    test_code: "cbc".to_string(),
                    test_name: "Complete Blood Count".to_string(),
                    result: Some("Normal".to_string()),
                    result_value: Some(14.5),
                    unit: Some("g/dL".to_string()),
                    reference_range: Some("12.0-17.5".to_string()),
                    is_abnormal: false,
                    resulted_at: Some(Utc::now()),
                    resulted_by: Some(actor.user_id),
                }],
            },
        )
        .expect("complete lab order");
        uow.commit().expect("commit");

        // Order should be verified (completed)
        use rustyclinic_clinical::lab::LabOrderRepo;
        let order = lab_order_repo
            .find_by_id(lab_output.order_id)
            .expect("find")
            .expect("exists");
        assert_eq!(order.status, rustyclinic_clinical::lab::LabStatus::Verified);

        // Lab queue entry should be completed
        let lab_entry = queue_repo
            .find_by_id(lab_output.queue_entry_id)
            .expect("find")
            .expect("exists");
        assert_eq!(lab_entry.status, QueueStatus::Completed);
    }

    #[test]
    fn test_create_prescription_enqueues_in_pharmacy() {
        let conn = setup_db();
        let patient_repo = SqlitePatientRepo::new(&conn);
        let queue_repo = SqliteQueueRepo::new(&conn);
        let dispense_repo =
            rustyclinic_db::sqlite::pharmacy_repo::SqliteMedicationDispenseRepo::new(&conn);
        let item_repo = rustyclinic_db::sqlite::pharmacy_repo::SqliteDispenseItemRepo::new(&conn);
        let actor = test_actor();

        let user_repo = SqliteUserRepo::new(&conn);
        let now = Utc::now();
        let user = User {
            id: actor.user_id,
            facility_id: actor.facility_id,
            username: "rxdoc".to_string(),
            display_name: "Rx Doctor".to_string(),
            roles: vec!["physician".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        let patient = test_patient(actor.facility_id);
        patient_repo.create(&patient).expect("create patient");

        // Enqueue, call, create encounter
        let mut uow = UnitOfWork::new(&conn);
        let entry_id = crate::commands::enqueue_patient::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::enqueue_patient::EnqueuePatientInput {
                patient_id: patient.id,
                service_type: "consultation".to_string(),
            },
        )
        .expect("enqueue");
        uow.commit().expect("commit");

        let mut uow = UnitOfWork::new(&conn);
        crate::commands::transition_queue::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::transition_queue::TransitionQueueInput {
                queue_entry_id: entry_id,
                transition: QueueTransition::Call,
                assigned_to: None,
            },
        )
        .expect("call");
        uow.commit().expect("commit");

        let mut uow = UnitOfWork::new(&conn);
        let enc_output = crate::commands::create_encounter::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::create_encounter::CreateEncounterInput {
                queue_entry_id: entry_id,
                provider_id: actor.user_id,
            },
        )
        .expect("encounter");
        uow.commit().expect("commit");

        // Create prescription
        let mut uow = UnitOfWork::new(&conn);
        let rx_output = crate::commands::create_prescription::execute(
            &mut uow,
            &dispense_repo,
            &queue_repo,
            &item_repo,
            &actor,
            crate::commands::create_prescription::CreatePrescriptionInput {
                encounter_id: enc_output.encounter_id,
                patient_id: patient.id,
                items: vec![rustyclinic_clinical::pharmacy::DispenseItem {
                    medication_name: "Amoxicillin".to_string(),
                    medication_system: Some("http://snomed.info/sct".to_string()),
                    medication_code: Some("27658006".to_string()),
                    medication_display: Some("Amoxicillin".to_string()),
                    dosage: "500mg".to_string(),
                    frequency: "3x daily".to_string(),
                    duration: "7 days".to_string(),
                    quantity: 21,
                    dispensed_quantity: None,
                    substituted: false,
                    substitution_reason: None,
                }],
                priority: rustyclinic_clinical::Priority::Routine,
                notes: None,
            },
        )
        .expect("create prescription");
        uow.commit().expect("commit");

        // Patient should appear in pharmacy queue
        let rx_entry = queue_repo
            .find_by_id(rx_output.queue_entry_id)
            .expect("find")
            .expect("exists");
        assert_eq!(rx_entry.department, "pharmacy");
        assert_eq!(rx_entry.status, QueueStatus::Waiting);

        let persisted_coding: (Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT medication_system, medication_code, medication_display
                 FROM dispense_items WHERE dispense_id = ?1",
                rusqlite::params![rx_output.order_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("dispense item coding");
        assert_eq!(
            persisted_coding,
            (
                Some("http://snomed.info/sct".to_string()),
                Some("27658006".to_string()),
                Some("Amoxicillin".to_string())
            )
        );
    }

    #[test]
    fn test_dispense_prescription_completes_order() {
        let conn = setup_db();
        let patient_repo = SqlitePatientRepo::new(&conn);
        let queue_repo = SqliteQueueRepo::new(&conn);
        let dispense_repo =
            rustyclinic_db::sqlite::pharmacy_repo::SqliteMedicationDispenseRepo::new(&conn);
        let item_repo = rustyclinic_db::sqlite::pharmacy_repo::SqliteDispenseItemRepo::new(&conn);
        let actor = test_actor();

        let user_repo = SqliteUserRepo::new(&conn);
        let now = Utc::now();
        let user = User {
            id: actor.user_id,
            facility_id: actor.facility_id,
            username: "pharmacist".to_string(),
            display_name: "Pharmacist".to_string(),
            roles: vec!["pharmacist".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        let patient = test_patient(actor.facility_id);
        patient_repo.create(&patient).expect("create patient");

        // Quick path: enqueue, call, encounter, prescribe
        let mut uow = UnitOfWork::new(&conn);
        let entry_id = crate::commands::enqueue_patient::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::enqueue_patient::EnqueuePatientInput {
                patient_id: patient.id,
                service_type: "consultation".to_string(),
            },
        )
        .expect("enqueue");
        uow.commit().expect("commit");

        let mut uow = UnitOfWork::new(&conn);
        crate::commands::transition_queue::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::transition_queue::TransitionQueueInput {
                queue_entry_id: entry_id,
                transition: QueueTransition::Call,
                assigned_to: None,
            },
        )
        .expect("call");
        uow.commit().expect("commit");

        let mut uow = UnitOfWork::new(&conn);
        let enc = crate::commands::create_encounter::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::create_encounter::CreateEncounterInput {
                queue_entry_id: entry_id,
                provider_id: actor.user_id,
            },
        )
        .expect("encounter");
        uow.commit().expect("commit");

        let mut uow = UnitOfWork::new(&conn);
        let rx = crate::commands::create_prescription::execute(
            &mut uow,
            &dispense_repo,
            &queue_repo,
            &item_repo,
            &actor,
            crate::commands::create_prescription::CreatePrescriptionInput {
                encounter_id: enc.encounter_id,
                patient_id: patient.id,
                items: vec![rustyclinic_clinical::pharmacy::DispenseItem {
                    medication_name: "Paracetamol".to_string(),
                    medication_system: None,
                    medication_code: None,
                    medication_display: None,
                    dosage: "500mg".to_string(),
                    frequency: "4x daily".to_string(),
                    duration: "3 days".to_string(),
                    quantity: 12,
                    dispensed_quantity: None,
                    substituted: false,
                    substitution_reason: None,
                }],
                priority: rustyclinic_clinical::Priority::Routine,
                notes: None,
            },
        )
        .expect("create prescription");
        uow.commit().expect("commit");

        // Dispense
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::dispense_prescription::execute(
            &mut uow,
            &dispense_repo,
            &queue_repo,
            &item_repo,
            &actor,
            crate::commands::dispense_prescription::DispensePrescriptionInput {
                order_id: rx.order_id,
                queue_entry_id: rx.queue_entry_id,
                items: vec![crate::commands::dispense_prescription::DispenseItemInput {
                    medication_name: "Paracetamol".to_string(),
                    dispensed_quantity: 12,
                    substituted: false,
                    substitution_reason: None,
                }],
            },
        )
        .expect("dispense");
        uow.commit().expect("commit");

        // Dispense should be completed (dispensed status)
        use rustyclinic_clinical::pharmacy::MedicationDispenseRepo;
        let dispense = dispense_repo
            .find_by_id(rx.order_id)
            .expect("find")
            .expect("exists");
        assert_eq!(
            dispense.status,
            rustyclinic_clinical::pharmacy::DispenseStatus::Dispensed
        );

        // Pharmacy queue entry should be completed
        let rx_entry = queue_repo
            .find_by_id(rx.queue_entry_id)
            .expect("find")
            .expect("exists");
        assert_eq!(rx_entry.status, QueueStatus::Completed);
    }

    #[test]
    fn test_full_patient_journey_consultation_lab_pharmacy() {
        let conn = setup_db();
        let patient_repo = SqlitePatientRepo::new(&conn);
        let queue_repo = SqliteQueueRepo::new(&conn);
        let lab_order_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabOrderRepo::new(&conn);
        let lab_test_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabTestRepo::new(&conn);
        let dispense_repo =
            rustyclinic_db::sqlite::pharmacy_repo::SqliteMedicationDispenseRepo::new(&conn);
        let item_repo = rustyclinic_db::sqlite::pharmacy_repo::SqliteDispenseItemRepo::new(&conn);
        let actor = test_actor();

        let user_repo = SqliteUserRepo::new(&conn);
        let now = Utc::now();
        let user = User {
            id: actor.user_id,
            facility_id: actor.facility_id,
            username: "clinician".to_string(),
            display_name: "Clinician".to_string(),
            roles: vec!["physician".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        let patient = test_patient(actor.facility_id);
        patient_repo.create(&patient).expect("create patient");

        // 1. Enqueue in consultation
        let mut uow = UnitOfWork::new(&conn);
        let consult_entry_id = crate::commands::enqueue_patient::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::enqueue_patient::EnqueuePatientInput {
                patient_id: patient.id,
                service_type: "consultation".to_string(),
            },
        )
        .expect("enqueue");
        uow.commit().expect("commit");

        let entry = queue_repo
            .find_by_id(consult_entry_id)
            .expect("find")
            .expect("exists");
        assert_eq!(entry.department, "consultation");

        // 2. Call and begin service
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::transition_queue::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::transition_queue::TransitionQueueInput {
                queue_entry_id: consult_entry_id,
                transition: QueueTransition::Call,
                assigned_to: None,
            },
        )
        .expect("call");
        uow.commit().expect("commit");

        // 3. Create encounter
        let mut uow = UnitOfWork::new(&conn);
        let enc = crate::commands::create_encounter::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::create_encounter::CreateEncounterInput {
                queue_entry_id: consult_entry_id,
                provider_id: actor.user_id,
            },
        )
        .expect("encounter");
        uow.commit().expect("commit");

        // 4. Order lab tests
        let mut uow = UnitOfWork::new(&conn);
        let lab = crate::commands::create_lab_order::execute(
            &mut uow,
            &lab_order_repo,
            &queue_repo,
            &lab_test_repo,
            &actor,
            crate::commands::create_lab_order::CreateLabOrderInput {
                encounter_id: enc.encounter_id,
                patient_id: patient.id,
                tests: vec![rustyclinic_clinical::lab::LabTest {
                    test_code: "malaria_rdt".to_string(),
                    test_name: "Malaria RDT".to_string(),
                    result: None,
                    result_value: None,
                    unit: None,
                    reference_range: None,
                    is_abnormal: false,
                    resulted_at: None,
                    resulted_by: None,
                }],
                specimen_type: Some("Blood".to_string()),
                priority: rustyclinic_clinical::Priority::Routine,
                notes: None,
            },
        )
        .expect("lab order");
        uow.commit().expect("commit");

        // 5. Prescribe medications
        let mut uow = UnitOfWork::new(&conn);
        let rx = crate::commands::create_prescription::execute(
            &mut uow,
            &dispense_repo,
            &queue_repo,
            &item_repo,
            &actor,
            crate::commands::create_prescription::CreatePrescriptionInput {
                encounter_id: enc.encounter_id,
                patient_id: patient.id,
                items: vec![rustyclinic_clinical::pharmacy::DispenseItem {
                    medication_name: "Coartem".to_string(),
                    medication_system: None,
                    medication_code: None,
                    medication_display: None,
                    dosage: "20/120mg".to_string(),
                    frequency: "2x daily".to_string(),
                    duration: "3 days".to_string(),
                    quantity: 12,
                    dispensed_quantity: None,
                    substituted: false,
                    substitution_reason: None,
                }],
                priority: rustyclinic_clinical::Priority::Urgent,
                notes: Some("Suspected malaria".to_string()),
            },
        )
        .expect("prescription");
        uow.commit().expect("commit");

        // Both lab and pharmacy queue entries should exist for this encounter
        let encounter_entries = queue_repo
            .find_by_encounter(enc.encounter_id)
            .expect("find");
        let lab_entries: Vec<_> = encounter_entries
            .iter()
            .filter(|e| e.department == "lab")
            .collect();
        let rx_entries: Vec<_> = encounter_entries
            .iter()
            .filter(|e| e.department == "pharmacy")
            .collect();
        assert_eq!(lab_entries.len(), 1);
        assert_eq!(rx_entries.len(), 1);

        // 6. Complete lab results
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::complete_lab_order::execute(
            &mut uow,
            &lab_order_repo,
            &queue_repo,
            &lab_test_repo,
            &actor,
            crate::commands::complete_lab_order::CompleteLabOrderInput {
                order_id: lab.order_id,
                queue_entry_id: lab.queue_entry_id,
                results: vec![rustyclinic_clinical::lab::LabTest {
                    test_code: "malaria_rdt".to_string(),
                    test_name: "Malaria RDT".to_string(),
                    result: Some("Positive".to_string()),
                    result_value: None,
                    unit: None,
                    reference_range: None,
                    is_abnormal: true,
                    resulted_at: Some(Utc::now()),
                    resulted_by: Some(actor.user_id),
                }],
            },
        )
        .expect("complete lab");
        uow.commit().expect("commit");

        // Lab entry should be completed
        let lab_entry = queue_repo
            .find_by_id(lab.queue_entry_id)
            .expect("find")
            .expect("exists");
        assert_eq!(lab_entry.status, QueueStatus::Completed);

        // 7. Dispense medications
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::dispense_prescription::execute(
            &mut uow,
            &dispense_repo,
            &queue_repo,
            &item_repo,
            &actor,
            crate::commands::dispense_prescription::DispensePrescriptionInput {
                order_id: rx.order_id,
                queue_entry_id: rx.queue_entry_id,
                items: vec![crate::commands::dispense_prescription::DispenseItemInput {
                    medication_name: "Coartem".to_string(),
                    dispensed_quantity: 12,
                    substituted: false,
                    substitution_reason: None,
                }],
            },
        )
        .expect("dispense");
        uow.commit().expect("commit");

        // Pharmacy entry should be completed
        let rx_entry = queue_repo
            .find_by_id(rx.queue_entry_id)
            .expect("find")
            .expect("exists");
        assert_eq!(rx_entry.status, QueueStatus::Completed);

        // All encounter entries should be completed
        let all_entries = queue_repo
            .find_by_encounter(enc.encounter_id)
            .expect("find");
        for entry in &all_entries {
            if entry.department == "lab" || entry.department == "pharmacy" {
                assert_eq!(
                    entry.status,
                    QueueStatus::Completed,
                    "entry in {} should be completed",
                    entry.department
                );
            }
        }
    }

    #[test]
    fn test_queue_filtering_by_department() {
        let conn = setup_db();
        let patient_repo = SqlitePatientRepo::new(&conn);
        let queue_repo = SqliteQueueRepo::new(&conn);
        let lab_order_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabOrderRepo::new(&conn);
        let lab_test_repo = rustyclinic_db::sqlite::lab_repo::SqliteLabTestRepo::new(&conn);
        let actor = test_actor();

        let user_repo = SqliteUserRepo::new(&conn);
        let now = Utc::now();
        let user = User {
            id: actor.user_id,
            facility_id: actor.facility_id,
            username: "filterdoc".to_string(),
            display_name: "Filter Doc".to_string(),
            roles: vec!["physician".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        let patient = test_patient(actor.facility_id);
        patient_repo.create(&patient).expect("create patient");

        // Enqueue in consultation
        let mut uow = UnitOfWork::new(&conn);
        let entry_id = crate::commands::enqueue_patient::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::enqueue_patient::EnqueuePatientInput {
                patient_id: patient.id,
                service_type: "consultation".to_string(),
            },
        )
        .expect("enqueue");
        uow.commit().expect("commit");

        // Call and create encounter
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::transition_queue::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::transition_queue::TransitionQueueInput {
                queue_entry_id: entry_id,
                transition: QueueTransition::Call,
                assigned_to: None,
            },
        )
        .expect("call");
        uow.commit().expect("commit");

        let mut uow = UnitOfWork::new(&conn);
        let enc = crate::commands::create_encounter::execute(
            &mut uow,
            &queue_repo,
            &actor,
            crate::commands::create_encounter::CreateEncounterInput {
                queue_entry_id: entry_id,
                provider_id: actor.user_id,
            },
        )
        .expect("encounter");
        uow.commit().expect("commit");

        // Create a lab order (adds to lab queue)
        let mut uow = UnitOfWork::new(&conn);
        crate::commands::create_lab_order::execute(
            &mut uow,
            &lab_order_repo,
            &queue_repo,
            &lab_test_repo,
            &actor,
            crate::commands::create_lab_order::CreateLabOrderInput {
                encounter_id: enc.encounter_id,
                patient_id: patient.id,
                tests: vec![rustyclinic_clinical::lab::LabTest {
                    test_code: "hiv_rapid".to_string(),
                    test_name: "HIV Rapid Test".to_string(),
                    result: None,
                    result_value: None,
                    unit: None,
                    reference_range: None,
                    is_abnormal: false,
                    resulted_at: None,
                    resulted_by: None,
                }],
                specimen_type: None,
                priority: rustyclinic_clinical::Priority::Routine,
                notes: None,
            },
        )
        .expect("lab order");
        uow.commit().expect("commit");

        // Filter by consultation department — should find 1 (the in_service consultation entry)
        let consult = queue_repo
            .find_active_by_facility_and_department(actor.facility_id, "consultation")
            .expect("filter consult");
        assert_eq!(consult.len(), 1);
        assert_eq!(consult[0].department, "consultation");

        // Filter by lab department — should find 1
        let lab = queue_repo
            .find_active_by_facility_and_department(actor.facility_id, "lab")
            .expect("filter lab");
        assert_eq!(lab.len(), 1);
        assert_eq!(lab[0].department, "lab");

        // Filter by pharmacy department — should find 0
        let pharmacy = queue_repo
            .find_active_by_facility_and_department(actor.facility_id, "pharmacy")
            .expect("filter pharmacy");
        assert_eq!(pharmacy.len(), 0);

        // All active — should find 2 (consultation + lab)
        let all = queue_repo
            .find_active_by_facility(actor.facility_id)
            .expect("all active");
        assert_eq!(all.len(), 2);
    }
}
