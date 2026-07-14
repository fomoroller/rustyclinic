//! Integration tests for repository implementations.
//!
//! All repository tests are defined once as functions that accept a `TestBackend`,
//! then run against every enabled backend (SQLite always, PostgreSQL when the
//! `postgres` feature is active and `RUSTYCLINIC_PG_URL` is set).

#[cfg(test)]
mod test_cases {
    use chrono::Utc;
    use rusqlite::Connection;
    use uuid::Uuid;

    use rustyclinic_auth::session::{Session, SessionRepo, SessionState};
    use rustyclinic_auth::users::{User, UserRepo};
    use rustyclinic_clinical::queue::{QueueEntry, QueueEntryRepo, QueueStatus};
    use rustyclinic_core::types::{ActorContext, Sex, new_id};
    use rustyclinic_events::OpLogEntry;
    use rustyclinic_identity::{Patient, PatientRepo, PatientSearch};

    use crate::migration::run_migrations;
    use crate::sqlite::idempotency;
    use crate::sqlite::patient_repo::SqlitePatientRepo;
    use crate::sqlite::queue_repo::SqliteQueueRepo;
    use crate::sqlite::session_repo::SqliteSessionRepo;
    use crate::sqlite::sync_repo::SqliteSyncRepo;
    use crate::sqlite::unit_of_work::UnitOfWork;
    use crate::sqlite::user_repo::SqliteUserRepo;
    use crate::sync_repo::{
        OpLogSyncRepo, SyncConflictRecord, SyncConflictRepo, SyncConflictStatus, SyncCursorRecord,
        SyncCursorRepo,
    };

    // =========================================================================
    // TestBackend trait — backend-agnostic repository access
    // =========================================================================

    /// Provides repository instances for a given backend.
    trait TestBackend {
        fn patient_repo(&self) -> Box<dyn PatientRepo + '_>;
        fn queue_repo(&self) -> Box<dyn QueueEntryRepo + '_>;
        fn user_repo(&self) -> Box<dyn UserRepo + '_>;
        fn session_repo(&self) -> Box<dyn SessionRepo + '_>;
        fn op_log_sync_repo(&self) -> Box<dyn OpLogSyncRepo + '_>;
        fn sync_cursor_repo(&self) -> Box<dyn SyncCursorRepo + '_>;
        fn sync_conflict_repo(&self) -> Box<dyn SyncConflictRepo + '_>;
    }

    // ---- SQLite backend ----

    struct SqliteBackend {
        conn: Connection,
    }

    impl SqliteBackend {
        fn new() -> Self {
            let conn = Connection::open_in_memory().expect("in-memory db");
            conn.pragma_update(None, "foreign_keys", "on").expect("fk");
            run_migrations(&conn).expect("migrations");
            Self { conn }
        }
    }

    impl TestBackend for SqliteBackend {
        fn patient_repo(&self) -> Box<dyn PatientRepo + '_> {
            Box::new(SqlitePatientRepo::new(&self.conn))
        }
        fn queue_repo(&self) -> Box<dyn QueueEntryRepo + '_> {
            Box::new(SqliteQueueRepo::new(&self.conn))
        }
        fn user_repo(&self) -> Box<dyn UserRepo + '_> {
            Box::new(SqliteUserRepo::new(&self.conn))
        }
        fn session_repo(&self) -> Box<dyn SessionRepo + '_> {
            Box::new(SqliteSessionRepo::new(&self.conn))
        }
        fn op_log_sync_repo(&self) -> Box<dyn OpLogSyncRepo + '_> {
            Box::new(SqliteSyncRepo::new(&self.conn))
        }
        fn sync_cursor_repo(&self) -> Box<dyn SyncCursorRepo + '_> {
            Box::new(SqliteSyncRepo::new(&self.conn))
        }
        fn sync_conflict_repo(&self) -> Box<dyn SyncConflictRepo + '_> {
            Box::new(SqliteSyncRepo::new(&self.conn))
        }
    }

    // =========================================================================
    // Macro to run a test against every available backend
    // =========================================================================

    /// Generate one test-per-backend for a given test function.
    ///
    /// The test function must have signature `fn(&dyn TestBackend)`.
    /// SQLite tests always run.  PostgreSQL tests compile only when the
    /// `postgres` feature is enabled and execute only when the
    /// `RUSTYCLINIC_PG_URL` environment variable is set (otherwise they
    /// are silently skipped at runtime).
    macro_rules! backend_test {
        ($name:ident, $test_fn:ident) => {
            mod $name {
                use super::*;

                #[test]
                fn sqlite() {
                    let backend = SqliteBackend::new();
                    $test_fn(&backend);
                }

                #[cfg(feature = "postgres")]
                #[test]
                fn postgres() {
                    // Skip at runtime when no PG URL is configured.
                    let pg_url = match std::env::var("RUSTYCLINIC_PG_URL") {
                        Ok(url) => url,
                        Err(_) => {
                            eprintln!("RUSTYCLINIC_PG_URL not set — skipping PostgreSQL test");
                            return;
                        }
                    };

                    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                    rt.block_on(async {
                        let (client, connection) =
                            tokio_postgres::connect(&pg_url, tokio_postgres::NoTls)
                                .await
                                .expect("PG connect");

                        // Spawn the connection future so it stays alive.
                        tokio::spawn(async move {
                            if let Err(e) = connection.await {
                                eprintln!("PG connection error: {e}");
                            }
                        });

                        crate::migration_pg::run_migrations(&client)
                            .await
                            .expect("PG migrations");

                        let backend = PgBackend { client };
                        $test_fn(&backend);
                    });
                }
            }
        };
    }

    // =========================================================================
    // PostgreSQL backend (compiled only with the `postgres` feature)
    // =========================================================================

    #[cfg(feature = "postgres")]
    struct PgBackend {
        client: tokio_postgres::Client,
    }

    #[cfg(feature = "postgres")]
    impl TestBackend for PgBackend {
        fn patient_repo(&self) -> Box<dyn PatientRepo + '_> {
            Box::new(crate::postgres::patient_repo::PgPatientRepo::new(
                &self.client,
            ))
        }
        fn queue_repo(&self) -> Box<dyn QueueEntryRepo + '_> {
            Box::new(crate::postgres::queue_repo::PgQueueRepo::new(&self.client))
        }
        fn user_repo(&self) -> Box<dyn UserRepo + '_> {
            Box::new(crate::postgres::user_repo::PgUserRepo::new(&self.client))
        }
        fn session_repo(&self) -> Box<dyn SessionRepo + '_> {
            Box::new(crate::postgres::session_repo::PgSessionRepo::new(
                &self.client,
            ))
        }
        fn op_log_sync_repo(&self) -> Box<dyn OpLogSyncRepo + '_> {
            Box::new(crate::postgres::sync_repo::PgSyncRepo::new(&self.client))
        }
        fn sync_cursor_repo(&self) -> Box<dyn SyncCursorRepo + '_> {
            Box::new(crate::postgres::sync_repo::PgSyncRepo::new(&self.client))
        }
        fn sync_conflict_repo(&self) -> Box<dyn SyncConflictRepo + '_> {
            Box::new(crate::postgres::sync_repo::PgSyncRepo::new(&self.client))
        }
    }

    // =========================================================================
    // Shared test helpers
    // =========================================================================

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

    fn test_patient(facility_id: Uuid) -> Patient {
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
            national_id: Some("1199080012345678".to_string()),
            created_at: now,
            updated_at: now,
            version: 0,
        }
    }

    fn test_op_entry(
        sequence: u64,
        facility_id: Uuid,
        device_id: Uuid,
        aggregate_id: Uuid,
    ) -> OpLogEntry {
        OpLogEntry {
            id: new_id(),
            sequence,
            facility_id,
            device_id,
            actor_id: new_id(),
            created_at: Utc::now(),
            aggregate_type: "Patient".to_string(),
            aggregate_id,
            payload: serde_json::json!({"event": "test", "sequence": sequence}),
            prev_hash: Vec::new(),
            entry_hash: vec![sequence as u8],
        }
    }

    // =========================================================================
    // Backend-agnostic repo test functions
    // =========================================================================

    // ---- Patient ----

    fn patient_create_and_find_impl(b: &dyn TestBackend) {
        let repo = b.patient_repo();
        let facility_id = new_id();
        let patient = test_patient(facility_id);
        let patient_id = patient.id;

        repo.create(&patient).expect("create patient");

        let found = repo
            .find_by_id(patient_id)
            .expect("find")
            .expect("should exist");
        assert_eq!(found.given_name, "Uwimana");
        assert_eq!(found.family_name, "Marie");
        assert_eq!(found.sex, Sex::Female);
        assert_eq!(found.national_id.as_deref(), Some("1199080012345678"));
    }

    fn patient_not_found_impl(b: &dyn TestBackend) {
        let repo = b.patient_repo();
        let result = repo.find_by_id(new_id()).expect("find");
        assert!(result.is_none());
    }

    fn patient_search_by_name_impl(b: &dyn TestBackend) {
        let repo = b.patient_repo();
        let facility_id = new_id();

        let mut p1 = test_patient(facility_id);
        p1.given_name = "Jean".to_string();
        p1.family_name = "Habimana".to_string();
        repo.create(&p1).expect("create p1");

        let mut p2 = test_patient(facility_id);
        p2.id = new_id();
        p2.given_name = "Claudine".to_string();
        p2.family_name = "Ingabire".to_string();
        repo.create(&p2).expect("create p2");

        let results = repo
            .search(&PatientSearch {
                family_name: Some("Habimana".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].given_name, "Jean");
    }

    fn patient_search_by_national_id_impl(b: &dyn TestBackend) {
        let repo = b.patient_repo();
        let facility_id = new_id();
        let patient = test_patient(facility_id);
        repo.create(&patient).expect("create");

        let results = repo
            .search(&PatientSearch {
                national_id: Some("1199080012345678".to_string()),
                ..Default::default()
            })
            .expect("search");
        assert_eq!(results.len(), 1);
    }

    fn patient_update_with_optimistic_lock_impl(b: &dyn TestBackend) {
        let repo = b.patient_repo();
        let facility_id = new_id();
        let patient = test_patient(facility_id);
        repo.create(&patient).expect("create");

        let mut updated = patient.clone();
        updated.phone = Some("+250789999999".to_string());
        updated.version = 1;
        updated.updated_at = Utc::now();
        repo.update(&updated).expect("update");

        let found = repo.find_by_id(patient.id).expect("find").expect("exists");
        assert_eq!(found.phone.as_deref(), Some("+250789999999"));
        assert_eq!(found.version, 1);
    }

    fn patient_update_conflict_impl(b: &dyn TestBackend) {
        let repo = b.patient_repo();
        let facility_id = new_id();
        let patient = test_patient(facility_id);
        repo.create(&patient).expect("create");

        let mut stale = patient.clone();
        stale.version = 5;
        stale.updated_at = Utc::now();
        let result = repo.update(&stale);
        assert!(result.is_err());
    }

    // ---- Queue ----

    fn queue_create_and_find_impl(b: &dyn TestBackend) {
        let patient_repo = b.patient_repo();
        let queue_repo = b.queue_repo();
        let facility_id = new_id();

        let patient = test_patient(facility_id);
        patient_repo.create(&patient).expect("create patient");

        let now = Utc::now();
        let entry = QueueEntry {
            id: new_id(),
            facility_id,
            patient_id: patient.id,
            service_type: "consultation".to_string(),
            department: "consultation".to_string(),
            encounter_id: None,
            status: QueueStatus::Waiting,
            assigned_to: None,
            position: 1,
            arrived_at: now,
            called_at: None,
            service_started_at: None,
            completed_at: None,
            created_at: now,
            version: 0,
        };

        queue_repo.create(&entry).expect("create queue entry");

        let found = queue_repo
            .find_by_id(entry.id)
            .expect("find")
            .expect("exists");
        assert_eq!(found.status, QueueStatus::Waiting);
        assert_eq!(found.patient_id, patient.id);
    }

    fn queue_find_active_excludes_completed_impl(b: &dyn TestBackend) {
        let patient_repo = b.patient_repo();
        let queue_repo = b.queue_repo();
        let facility_id = new_id();

        let patient = test_patient(facility_id);
        patient_repo.create(&patient).expect("create patient");

        let now = Utc::now();
        let e1 = QueueEntry {
            id: new_id(),
            facility_id,
            patient_id: patient.id,
            service_type: "consult".to_string(),
            department: "consultation".to_string(),
            encounter_id: None,
            status: QueueStatus::Waiting,
            assigned_to: None,
            position: 1,
            arrived_at: now,
            called_at: None,
            service_started_at: None,
            completed_at: None,
            created_at: now,
            version: 0,
        };
        queue_repo.create(&e1).expect("create e1");

        let mut e2 = e1.clone();
        e2.id = new_id();
        e2.status = QueueStatus::Completed;
        e2.position = 2;
        queue_repo.create(&e2).expect("create e2");

        let active = queue_repo
            .find_active_by_facility(facility_id)
            .expect("find active");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, QueueStatus::Waiting);
    }

    fn queue_next_position_impl(b: &dyn TestBackend) {
        let queue_repo = b.queue_repo();
        let facility_id = new_id();

        let pos = queue_repo.next_position(facility_id).expect("next pos");
        assert_eq!(pos, 1);
    }

    fn queue_update_with_optimistic_lock_impl(b: &dyn TestBackend) {
        let patient_repo = b.patient_repo();
        let queue_repo = b.queue_repo();
        let facility_id = new_id();

        let patient = test_patient(facility_id);
        patient_repo.create(&patient).expect("create patient");

        let now = Utc::now();
        let entry = QueueEntry {
            id: new_id(),
            facility_id,
            patient_id: patient.id,
            service_type: "consult".to_string(),
            department: "consultation".to_string(),
            encounter_id: None,
            status: QueueStatus::Waiting,
            assigned_to: None,
            position: 1,
            arrived_at: now,
            called_at: None,
            service_started_at: None,
            completed_at: None,
            created_at: now,
            version: 0,
        };
        queue_repo.create(&entry).expect("create");

        let mut updated = entry.clone();
        updated.status = QueueStatus::Called;
        updated.assigned_to = Some(new_id());
        updated.called_at = Some(Utc::now());
        updated.version = 1;
        queue_repo.update(&updated).expect("update");

        let found = queue_repo
            .find_by_id(entry.id)
            .expect("find")
            .expect("exists");
        assert_eq!(found.status, QueueStatus::Called);
        assert!(found.assigned_to.is_some());
    }

    // ---- User ----

    fn user_create_and_find_impl(b: &dyn TestBackend) {
        let repo = b.user_repo();
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(),
            facility_id,
            username: "nurse.diane".to_string(),
            display_name: "Mukamana Diane".to_string(),
            roles: vec!["nurse".to_string(), "queue_manager".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };

        repo.create(&user, "$argon2id$fakehash")
            .expect("create user");

        let found = repo.find_by_id(user.id).expect("find").expect("exists");
        assert_eq!(found.username, "nurse.diane");
        assert_eq!(found.roles, vec!["nurse", "queue_manager"]);
    }

    fn user_find_by_username_impl(b: &dyn TestBackend) {
        let repo = b.user_repo();
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(),
            facility_id,
            username: "dr.habimana".to_string(),
            display_name: "Dr Habimana".to_string(),
            roles: vec!["physician".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        repo.create(&user, "hash123").expect("create");

        let (found, hash, pin_hash) = repo
            .find_by_username(facility_id, "dr.habimana")
            .expect("find")
            .expect("exists");
        assert_eq!(found.display_name, "Dr Habimana");
        assert_eq!(hash, "hash123");
        assert_eq!(pin_hash, None);
    }

    fn user_update_pin_hash_impl(b: &dyn TestBackend) {
        let repo = b.user_repo();
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(),
            facility_id,
            username: "pin.user".to_string(),
            display_name: "Pin User".to_string(),
            roles: vec!["nurse".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        repo.create(&user, "password_hash").expect("create");
        repo.update_pin_hash(user.id, "pin_hash").expect("set pin");

        let (_found, _password_hash, pin_hash) = repo
            .find_by_username(facility_id, "pin.user")
            .expect("find")
            .expect("exists");
        assert_eq!(pin_hash, Some("pin_hash".to_string()));
    }

    fn user_not_found_wrong_facility_impl(b: &dyn TestBackend) {
        let repo = b.user_repo();
        let now = Utc::now();

        let user = User {
            id: new_id(),
            facility_id: new_id(),
            username: "nurse".to_string(),
            display_name: "Nurse".to_string(),
            roles: vec![],
            active: true,
            created_at: now,
            updated_at: now,
        };
        repo.create(&user, "hash").expect("create");

        let result = repo.find_by_username(new_id(), "nurse").expect("find");
        assert!(result.is_none());
    }

    // ---- Session ----

    fn session_create_and_find_impl(b: &dyn TestBackend) {
        let user_repo = b.user_repo();
        let session_repo = b.session_repo();
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(),
            facility_id,
            username: "test".to_string(),
            display_name: "Test".to_string(),
            roles: vec!["nurse".to_string()],
            active: true,
            created_at: now,
            updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        let session = Session::new(
            user.id,
            facility_id,
            new_id(),
            vec!["nurse".to_string()],
            "password",
        );
        session_repo.create(&session).expect("create session");

        let found = session_repo
            .find_by_id(session.id)
            .expect("find")
            .expect("exists");
        assert_eq!(found.state, SessionState::Active);
        assert_eq!(found.user_id, user.id);
    }

    fn session_update_lock_impl(b: &dyn TestBackend) {
        let user_repo = b.user_repo();
        let session_repo = b.session_repo();
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(),
            facility_id,
            username: "test2".to_string(),
            display_name: "Test2".to_string(),
            roles: vec![],
            active: true,
            created_at: now,
            updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        let mut session = Session::new(user.id, facility_id, new_id(), vec![], "pin");
        session_repo.create(&session).expect("create");

        session.lock();
        session_repo.update(&session).expect("update");

        let found = session_repo
            .find_by_id(session.id)
            .expect("find")
            .expect("exists");
        assert_eq!(found.state, SessionState::Locked);
        assert!(found.locked_at.is_some());
    }

    fn session_count_locked_impl(b: &dyn TestBackend) {
        let user_repo = b.user_repo();
        let session_repo = b.session_repo();
        let now = Utc::now();
        let facility_id = new_id();
        let device_id = new_id();

        let user = User {
            id: new_id(),
            facility_id,
            username: "test3".to_string(),
            display_name: "Test3".to_string(),
            roles: vec![],
            active: true,
            created_at: now,
            updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        for _ in 0..2 {
            let mut s = Session::new(user.id, facility_id, device_id, vec![], "password");
            session_repo.create(&s).expect("create");
            s.lock();
            session_repo.update(&s).expect("lock");
        }

        let count = session_repo
            .count_locked_by_device(device_id)
            .expect("count");
        assert_eq!(count, 2);
    }

    fn sync_cursor_upsert_and_get_impl(b: &dyn TestBackend) {
        let repo = b.sync_cursor_repo();
        let device_id = new_id();
        let facility_id = new_id();

        assert!(
            repo.get(device_id, facility_id)
                .expect("get before")
                .is_none()
        );

        let first = SyncCursorRecord {
            device_id,
            facility_id,
            last_pulled_sequence: 10,
            last_pushed_sequence: 7,
            updated_at: Utc::now(),
        };
        repo.upsert(&first).expect("upsert first");

        let found = repo
            .get(device_id, facility_id)
            .expect("get after first")
            .expect("cursor exists");
        assert_eq!(found.last_pulled_sequence, 10);
        assert_eq!(found.last_pushed_sequence, 7);

        let second = SyncCursorRecord {
            last_pulled_sequence: 14,
            last_pushed_sequence: 13,
            updated_at: Utc::now(),
            ..first
        };
        repo.upsert(&second).expect("upsert second");

        let found2 = repo
            .get(device_id, facility_id)
            .expect("get after second")
            .expect("cursor exists");
        assert_eq!(found2.last_pulled_sequence, 14);
        assert_eq!(found2.last_pushed_sequence, 13);
    }

    fn sync_conflict_insert_list_resolve_impl(b: &dyn TestBackend) {
        let repo = b.sync_conflict_repo();
        let facility_id = new_id();
        let now = Utc::now();

        let conflict1 = SyncConflictRecord {
            id: new_id(),
            facility_id,
            aggregate_type: "Patient".to_string(),
            aggregate_id: new_id(),
            local_entry_id: new_id(),
            remote_entry_id: new_id(),
            conflict_type: serde_json::json!({"type": "field", "field_name": "date_of_birth"}),
            status: SyncConflictStatus::Pending,
            created_at: now,
            resolved_at: None,
            resolved_by: None,
            resolution: None,
        };
        let conflict2 = SyncConflictRecord {
            id: new_id(),
            aggregate_id: new_id(),
            local_entry_id: new_id(),
            remote_entry_id: new_id(),
            created_at: now + chrono::Duration::milliseconds(1),
            ..conflict1.clone()
        };

        repo.insert(&conflict1).expect("insert conflict 1");
        repo.insert(&conflict2).expect("insert conflict 2");

        let pending = repo.list_pending(facility_id).expect("list pending");
        assert_eq!(pending.len(), 2);

        repo.mark_resolved(
            conflict1.id,
            new_id(),
            serde_json::json!({"resolution": "accept_remote"}),
            Utc::now(),
        )
        .expect("resolve");

        let pending_after = repo.list_pending(facility_id).expect("list after resolve");
        assert_eq!(pending_after.len(), 1);
        assert_eq!(pending_after[0].id, conflict2.id);
    }

    fn op_log_sync_contract_impl(b: &dyn TestBackend) {
        let repo = b.op_log_sync_repo();
        let facility_id = new_id();
        let device_a = new_id();
        let device_b = new_id();

        let pending_1 = test_op_entry(1, facility_id, device_a, new_id());
        let pending_2 = test_op_entry(2, facility_id, device_a, new_id());
        let ack_remote = test_op_entry(3, facility_id, device_b, new_id());

        assert!(
            repo.insert_pending_if_missing(&pending_1)
                .expect("insert pending 1")
        );
        assert!(
            repo.insert_pending_if_missing(&pending_2)
                .expect("insert pending 2")
        );
        assert!(
            repo.insert_acknowledged_if_missing(&ack_remote)
                .expect("insert ack")
        );
        assert!(
            !repo
                .insert_acknowledged_if_missing(&ack_remote)
                .expect("dedup insert ack")
        );

        assert!(repo.exists(pending_1.id).expect("exists true"));
        assert!(!repo.exists(new_id()).expect("exists false"));

        let pending = repo.list_pending(facility_id, 10).expect("list pending");
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].sequence, 1);
        assert_eq!(pending[1].sequence, 2);

        repo.mark_pushed_through(facility_id, 1)
            .expect("mark pushed");
        let pending_after_push = repo
            .list_pending(facility_id, 10)
            .expect("list pending after pushed");
        assert_eq!(pending_after_push.len(), 1);
        assert_eq!(pending_after_push[0].sequence, 2);

        repo.mark_acknowledged_through(facility_id, 2)
            .expect("mark acknowledged");
        let pending_after_ack = repo
            .list_pending(facility_id, 10)
            .expect("list pending after ack");
        assert!(pending_after_ack.is_empty());

        let pull = repo
            .list_since_excluding_device(facility_id, 0, device_a, 10)
            .expect("pull range");
        assert_eq!(pull.len(), 1);
        assert_eq!(pull[0].id, ack_remote.id);
    }

    // =========================================================================
    // Wire up every test to every backend via the macro
    // =========================================================================

    backend_test!(test_patient_create_and_find, patient_create_and_find_impl);
    backend_test!(test_patient_not_found, patient_not_found_impl);
    backend_test!(test_patient_search_by_name, patient_search_by_name_impl);
    backend_test!(
        test_patient_search_by_national_id,
        patient_search_by_national_id_impl
    );
    backend_test!(
        test_patient_update_with_optimistic_lock,
        patient_update_with_optimistic_lock_impl
    );
    backend_test!(test_patient_update_conflict, patient_update_conflict_impl);

    backend_test!(test_queue_create_and_find, queue_create_and_find_impl);
    backend_test!(
        test_queue_find_active_excludes_completed,
        queue_find_active_excludes_completed_impl
    );
    backend_test!(test_queue_next_position, queue_next_position_impl);
    backend_test!(
        test_queue_update_with_optimistic_lock,
        queue_update_with_optimistic_lock_impl
    );

    backend_test!(test_user_create_and_find, user_create_and_find_impl);
    backend_test!(test_user_find_by_username, user_find_by_username_impl);
    backend_test!(test_user_update_pin_hash, user_update_pin_hash_impl);
    backend_test!(
        test_user_not_found_wrong_facility,
        user_not_found_wrong_facility_impl
    );

    backend_test!(test_session_create_and_find, session_create_and_find_impl);
    backend_test!(test_session_update_lock, session_update_lock_impl);
    backend_test!(test_session_count_locked, session_count_locked_impl);

    backend_test!(
        test_sync_cursor_upsert_and_get,
        sync_cursor_upsert_and_get_impl
    );
    backend_test!(
        test_sync_conflict_insert_list_resolve,
        sync_conflict_insert_list_resolve_impl
    );
    backend_test!(test_op_log_sync_contract, op_log_sync_contract_impl);

    // =========================================================================
    // SQLite-specific tests (Unit of Work, idempotency, hash chains)
    //
    // These are inherently tied to the SQLite UoW implementation and do not
    // need dual-backend coverage — the repo-level tests above cover the
    // behavior parity that matters.
    // =========================================================================

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        conn.pragma_update(None, "foreign_keys", "on").expect("fk");
        run_migrations(&conn).expect("migrations");
        conn
    }

    #[test]
    fn test_uow_commits_audit_and_outbox_and_oplog() {
        let conn = setup_db();
        let actor = test_actor();
        let patient = test_patient(actor.facility_id);
        let patient_repo = SqlitePatientRepo::new(&conn);

        patient_repo.create(&patient).expect("create patient");

        let mut uow = UnitOfWork::new(&conn);
        uow.record_audit(
            &actor,
            "patient.registered",
            "Patient",
            patient.id,
            serde_json::json!({"name": "test"}),
        );
        uow.record_outbox(
            actor.facility_id,
            "Patient",
            patient.id,
            "patient.registered",
            serde_json::json!({}),
        );
        uow.record_op_log(
            &actor,
            "Patient",
            patient.id,
            serde_json::json!({"action": "register"}),
        );
        uow.commit().expect("commit");

        let audit_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))
            .expect("count");
        assert_eq!(audit_count, 1);

        let hash: Vec<u8> = conn
            .query_row("SELECT entry_hash FROM audit_log LIMIT 1", [], |r| r.get(0))
            .expect("hash");
        assert!(!hash.is_empty());

        let outbox_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM outbox_events", [], |r| r.get(0))
            .expect("count");
        assert_eq!(outbox_count, 1);

        let (seq, sync_state): (u64, String) = conn
            .query_row("SELECT sequence, sync_state FROM op_log LIMIT 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .expect("op_log");
        assert_eq!(seq, 1);
        assert_eq!(sync_state, "pending");
    }

    #[test]
    fn test_uow_audit_hash_chain() {
        let conn = setup_db();
        let actor = test_actor();

        let mut uow = UnitOfWork::new(&conn);
        uow.record_audit(&actor, "action1", "Test", new_id(), serde_json::json!({}));
        uow.record_audit(&actor, "action2", "Test", new_id(), serde_json::json!({}));
        uow.commit().expect("commit");

        let rows: Vec<(Vec<u8>, Vec<u8>)> = {
            let mut stmt = conn
                .prepare("SELECT prev_hash, entry_hash FROM audit_log ORDER BY rowid")
                .expect("prepare");
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .expect("query")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect")
        };

        assert_eq!(rows.len(), 2);
        assert!(rows[0].0.is_empty());
        assert_eq!(rows[1].0, rows[0].1);
    }

    #[test]
    fn test_uow_op_log_sequence_increments() {
        let conn = setup_db();
        let actor = test_actor();

        let mut uow1 = UnitOfWork::new(&conn);
        uow1.record_op_log(&actor, "Patient", new_id(), serde_json::json!({}));
        uow1.commit().expect("commit1");

        let mut uow2 = UnitOfWork::new(&conn);
        uow2.record_op_log(&actor, "Patient", new_id(), serde_json::json!({}));
        uow2.commit().expect("commit2");

        let seqs: Vec<u64> = {
            let mut stmt = conn
                .prepare("SELECT sequence FROM op_log ORDER BY sequence")
                .expect("p");
            stmt.query_map([], |r| r.get(0))
                .expect("q")
                .collect::<Result<Vec<_>, _>>()
                .expect("c")
        };
        assert_eq!(seqs, vec![1, 2]);
    }

    #[test]
    fn test_idempotency_check_miss() {
        let conn = setup_db();
        let result = idempotency::check_idempotency(&conn, new_id(), "key-1").expect("check");
        assert!(result.is_none());
    }

    #[test]
    fn test_idempotency_store_and_replay() {
        let conn = setup_db();
        let facility_id = new_id();
        let response = serde_json::json!({"id": "abc-123", "message": "created"});

        idempotency::store_idempotency(&conn, facility_id, "key-2", &response).expect("store");

        let replayed = idempotency::check_idempotency(&conn, facility_id, "key-2")
            .expect("check")
            .expect("should exist");
        assert_eq!(replayed["id"], "abc-123");
    }

    #[test]
    fn test_idempotency_different_facility_no_replay() {
        let conn = setup_db();
        let facility_a = new_id();
        let facility_b = new_id();
        let response = serde_json::json!({"ok": true});

        idempotency::store_idempotency(&conn, facility_a, "key-3", &response).expect("store");

        let result = idempotency::check_idempotency(&conn, facility_b, "key-3").expect("check");
        assert!(result.is_none(), "different facility should not replay");
    }

    #[test]
    fn test_idempotency_via_uow() {
        let conn = setup_db();
        let actor = test_actor();
        let response = serde_json::json!({"patient_id": "xyz"});

        let mut uow = UnitOfWork::new(&conn);
        uow.record_idempotency(
            actor.facility_id,
            "register-xyz".to_string(),
            response.clone(),
        );
        uow.commit().expect("commit");

        let replayed = idempotency::check_idempotency(&conn, actor.facility_id, "register-xyz")
            .expect("check")
            .expect("should exist");
        assert_eq!(replayed["patient_id"], "xyz");
    }
}
