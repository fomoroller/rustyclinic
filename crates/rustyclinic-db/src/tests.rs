//! Integration tests for SQLite repositories, unit of work, and idempotency.
//! All tests use in-memory SQLite databases.

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use rusqlite::Connection;
    use uuid::Uuid;

    use rustyclinic_core::types::{new_id, ActorContext, Sex};
    use rustyclinic_identity::{Patient, PatientRepo, PatientSearch};
    use rustyclinic_clinical::queue::{QueueEntry, QueueEntryRepo, QueueStatus};
    use rustyclinic_auth::users::{User, UserRepo};
    use rustyclinic_auth::session::{Session, SessionRepo, SessionState};

    use crate::migration::run_migrations;
    use crate::sqlite::patient_repo::SqlitePatientRepo;
    use crate::sqlite::queue_repo::SqliteQueueRepo;
    use crate::sqlite::user_repo::SqliteUserRepo;
    use crate::sqlite::session_repo::SqliteSessionRepo;
    use crate::sqlite::unit_of_work::UnitOfWork;
    use crate::sqlite::idempotency;

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

    // ---- Patient Repo Tests ----

    #[test]
    fn test_patient_create_and_find() {
        let conn = setup_db();
        let repo = SqlitePatientRepo::new(&conn);
        let facility_id = new_id();
        let patient = test_patient(facility_id);
        let patient_id = patient.id;

        repo.create(&patient).expect("create patient");

        let found = repo.find_by_id(patient_id).expect("find").expect("should exist");
        assert_eq!(found.given_name, "Uwimana");
        assert_eq!(found.family_name, "Marie");
        assert_eq!(found.sex, Sex::Female);
        assert_eq!(found.national_id.as_deref(), Some("1199080012345678"));
    }

    #[test]
    fn test_patient_not_found() {
        let conn = setup_db();
        let repo = SqlitePatientRepo::new(&conn);

        let result = repo.find_by_id(new_id()).expect("find");
        assert!(result.is_none());
    }

    #[test]
    fn test_patient_search_by_name() {
        let conn = setup_db();
        let repo = SqlitePatientRepo::new(&conn);
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

        let results = repo.search(&PatientSearch {
            family_name: Some("Habimana".to_string()),
            ..Default::default()
        }).expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].given_name, "Jean");
    }

    #[test]
    fn test_patient_search_by_national_id() {
        let conn = setup_db();
        let repo = SqlitePatientRepo::new(&conn);
        let facility_id = new_id();

        let patient = test_patient(facility_id);
        repo.create(&patient).expect("create");

        let results = repo.search(&PatientSearch {
            national_id: Some("1199080012345678".to_string()),
            ..Default::default()
        }).expect("search");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_patient_update_with_optimistic_lock() {
        let conn = setup_db();
        let repo = SqlitePatientRepo::new(&conn);
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

    #[test]
    fn test_patient_update_conflict() {
        let conn = setup_db();
        let repo = SqlitePatientRepo::new(&conn);
        let facility_id = new_id();

        let patient = test_patient(facility_id);
        repo.create(&patient).expect("create");

        // Try updating with wrong version
        let mut stale = patient.clone();
        stale.version = 5; // wrong version — expects version 4 in WHERE clause
        stale.updated_at = Utc::now();
        let result = repo.update(&stale);
        assert!(result.is_err());
    }

    // ---- Queue Repo Tests ----

    #[test]
    fn test_queue_create_and_find() {
        let conn = setup_db();
        let patient_repo = SqlitePatientRepo::new(&conn);
        let queue_repo = SqliteQueueRepo::new(&conn);
        let facility_id = new_id();

        // Must create patient first (FK constraint)
        let patient = test_patient(facility_id);
        patient_repo.create(&patient).expect("create patient");

        let now = Utc::now();
        let entry = QueueEntry {
            id: new_id(),
            facility_id,
            patient_id: patient.id,
            service_type: "consultation".to_string(),
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

        let found = queue_repo.find_by_id(entry.id).expect("find").expect("exists");
        assert_eq!(found.status, QueueStatus::Waiting);
        assert_eq!(found.patient_id, patient.id);
    }

    #[test]
    fn test_queue_find_active_excludes_completed() {
        let conn = setup_db();
        let patient_repo = SqlitePatientRepo::new(&conn);
        let queue_repo = SqliteQueueRepo::new(&conn);
        let facility_id = new_id();

        let patient = test_patient(facility_id);
        patient_repo.create(&patient).expect("create patient");

        let now = Utc::now();
        let mut e1 = QueueEntry {
            id: new_id(), facility_id, patient_id: patient.id,
            service_type: "consult".to_string(), status: QueueStatus::Waiting,
            assigned_to: None, position: 1, arrived_at: now,
            called_at: None, service_started_at: None, completed_at: None,
            created_at: now, version: 0,
        };
        queue_repo.create(&e1).expect("create e1");

        let mut e2 = e1.clone();
        e2.id = new_id();
        e2.status = QueueStatus::Completed;
        e2.position = 2;
        queue_repo.create(&e2).expect("create e2");

        let active = queue_repo.find_active_by_facility(facility_id).expect("find active");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, QueueStatus::Waiting);
    }

    #[test]
    fn test_queue_next_position() {
        let conn = setup_db();
        let queue_repo = SqliteQueueRepo::new(&conn);
        let facility_id = new_id();

        let pos = queue_repo.next_position(facility_id).expect("next pos");
        assert_eq!(pos, 1);
    }

    #[test]
    fn test_queue_update_with_optimistic_lock() {
        let conn = setup_db();
        let patient_repo = SqlitePatientRepo::new(&conn);
        let queue_repo = SqliteQueueRepo::new(&conn);
        let facility_id = new_id();

        let patient = test_patient(facility_id);
        patient_repo.create(&patient).expect("create patient");

        let now = Utc::now();
        let entry = QueueEntry {
            id: new_id(), facility_id, patient_id: patient.id,
            service_type: "consult".to_string(), status: QueueStatus::Waiting,
            assigned_to: None, position: 1, arrived_at: now,
            called_at: None, service_started_at: None, completed_at: None,
            created_at: now, version: 0,
        };
        queue_repo.create(&entry).expect("create");

        let mut updated = entry.clone();
        updated.status = QueueStatus::Called;
        updated.assigned_to = Some(new_id());
        updated.called_at = Some(Utc::now());
        updated.version = 1;
        queue_repo.update(&updated).expect("update");

        let found = queue_repo.find_by_id(entry.id).expect("find").expect("exists");
        assert_eq!(found.status, QueueStatus::Called);
        assert!(found.assigned_to.is_some());
    }

    // ---- User Repo Tests ----

    #[test]
    fn test_user_create_and_find() {
        let conn = setup_db();
        let repo = SqliteUserRepo::new(&conn);
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

        repo.create(&user, "$argon2id$fakehash").expect("create user");

        let found = repo.find_by_id(user.id).expect("find").expect("exists");
        assert_eq!(found.username, "nurse.diane");
        assert_eq!(found.roles, vec!["nurse", "queue_manager"]);
    }

    #[test]
    fn test_user_find_by_username() {
        let conn = setup_db();
        let repo = SqliteUserRepo::new(&conn);
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(), facility_id,
            username: "dr.habimana".to_string(),
            display_name: "Dr Habimana".to_string(),
            roles: vec!["physician".to_string()],
            active: true, created_at: now, updated_at: now,
        };
        repo.create(&user, "hash123").expect("create");

        let (found, hash) = repo.find_by_username(facility_id, "dr.habimana")
            .expect("find").expect("exists");
        assert_eq!(found.display_name, "Dr Habimana");
        assert_eq!(hash, "hash123");
    }

    #[test]
    fn test_user_not_found_wrong_facility() {
        let conn = setup_db();
        let repo = SqliteUserRepo::new(&conn);
        let now = Utc::now();

        let user = User {
            id: new_id(), facility_id: new_id(),
            username: "nurse".to_string(),
            display_name: "Nurse".to_string(),
            roles: vec![], active: true, created_at: now, updated_at: now,
        };
        repo.create(&user, "hash").expect("create");

        // Different facility
        let result = repo.find_by_username(new_id(), "nurse").expect("find");
        assert!(result.is_none());
    }

    // ---- Session Repo Tests ----

    #[test]
    fn test_session_create_and_find() {
        let conn = setup_db();
        let user_repo = SqliteUserRepo::new(&conn);
        let session_repo = SqliteSessionRepo::new(&conn);
        let now = Utc::now();
        let facility_id = new_id();

        // Create user first (FK)
        let user = User {
            id: new_id(), facility_id,
            username: "test".to_string(), display_name: "Test".to_string(),
            roles: vec!["nurse".to_string()], active: true,
            created_at: now, updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        let session = Session::new(user.id, facility_id, new_id(), vec!["nurse".to_string()], "password");
        session_repo.create(&session).expect("create session");

        let found = session_repo.find_by_id(session.id).expect("find").expect("exists");
        assert_eq!(found.state, SessionState::Active);
        assert_eq!(found.user_id, user.id);
    }

    #[test]
    fn test_session_update_lock() {
        let conn = setup_db();
        let user_repo = SqliteUserRepo::new(&conn);
        let session_repo = SqliteSessionRepo::new(&conn);
        let now = Utc::now();
        let facility_id = new_id();

        let user = User {
            id: new_id(), facility_id,
            username: "test2".to_string(), display_name: "Test2".to_string(),
            roles: vec![], active: true, created_at: now, updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        let mut session = Session::new(user.id, facility_id, new_id(), vec![], "pin");
        session_repo.create(&session).expect("create");

        session.lock();
        session_repo.update(&session).expect("update");

        let found = session_repo.find_by_id(session.id).expect("find").expect("exists");
        assert_eq!(found.state, SessionState::Locked);
        assert!(found.locked_at.is_some());
    }

    #[test]
    fn test_session_count_locked() {
        let conn = setup_db();
        let user_repo = SqliteUserRepo::new(&conn);
        let session_repo = SqliteSessionRepo::new(&conn);
        let now = Utc::now();
        let facility_id = new_id();
        let device_id = new_id();

        let user = User {
            id: new_id(), facility_id,
            username: "test3".to_string(), display_name: "Test3".to_string(),
            roles: vec![], active: true, created_at: now, updated_at: now,
        };
        user_repo.create(&user, "hash").expect("create user");

        // Create 2 locked sessions
        for _ in 0..2 {
            let mut s = Session::new(user.id, facility_id, device_id, vec![], "password");
            session_repo.create(&s).expect("create");
            s.lock();
            session_repo.update(&s).expect("lock");
        }

        let count = session_repo.count_locked_by_device(device_id).expect("count");
        assert_eq!(count, 2);
    }

    // ---- Unit of Work Tests ----

    #[test]
    fn test_uow_commits_audit_and_outbox_and_oplog() {
        let conn = setup_db();
        let actor = test_actor();
        let patient = test_patient(actor.facility_id);
        let patient_repo = SqlitePatientRepo::new(&conn);

        // Domain write
        patient_repo.create(&patient).expect("create patient");

        // Side-effect records via UoW
        let mut uow = UnitOfWork::new(&conn);
        uow.record_audit(&actor, "patient.registered", "Patient", patient.id,
            serde_json::json!({"name": "test"}));
        uow.record_outbox(actor.facility_id, "Patient", patient.id,
            "patient.registered", serde_json::json!({}));
        uow.record_op_log(&actor, "Patient", patient.id,
            serde_json::json!({"action": "register"}));
        uow.commit().expect("commit");

        // Verify audit
        let audit_count: u32 = conn.query_row(
            "SELECT COUNT(*) FROM audit_log", [], |r| r.get(0)
        ).expect("count");
        assert_eq!(audit_count, 1);

        // Verify hash chain is non-empty
        let hash: Vec<u8> = conn.query_row(
            "SELECT entry_hash FROM audit_log LIMIT 1", [], |r| r.get(0)
        ).expect("hash");
        assert!(!hash.is_empty());

        // Verify outbox
        let outbox_count: u32 = conn.query_row(
            "SELECT COUNT(*) FROM outbox_events", [], |r| r.get(0)
        ).expect("count");
        assert_eq!(outbox_count, 1);

        // Verify op-log
        let (seq, sync_state): (u64, String) = conn.query_row(
            "SELECT sequence, sync_state FROM op_log LIMIT 1", [],
            |r| Ok((r.get(0)?, r.get(1)?))
        ).expect("op_log");
        assert_eq!(seq, 1);
        assert_eq!(sync_state, "pending");
    }

    #[test]
    fn test_uow_audit_hash_chain() {
        let conn = setup_db();
        let actor = test_actor();

        // Write two audit entries — verify chain
        let mut uow = UnitOfWork::new(&conn);
        uow.record_audit(&actor, "action1", "Test", new_id(), serde_json::json!({}));
        uow.record_audit(&actor, "action2", "Test", new_id(), serde_json::json!({}));
        uow.commit().expect("commit");

        let rows: Vec<(Vec<u8>, Vec<u8>)> = {
            let mut stmt = conn.prepare(
                "SELECT prev_hash, entry_hash FROM audit_log ORDER BY rowid"
            ).expect("prepare");
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .expect("query")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect")
        };

        assert_eq!(rows.len(), 2);
        // First entry's prev_hash is empty (no predecessor)
        assert!(rows[0].0.is_empty());
        // Second entry's prev_hash equals first entry's entry_hash
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
            let mut stmt = conn.prepare("SELECT sequence FROM op_log ORDER BY sequence").expect("p");
            stmt.query_map([], |r| r.get(0)).expect("q")
                .collect::<Result<Vec<_>, _>>().expect("c")
        };
        assert_eq!(seqs, vec![1, 2]);
    }

    // ---- Idempotency Tests ----

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
            .expect("check").expect("should exist");
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
        uow.record_idempotency(actor.facility_id, "register-xyz".to_string(), response.clone());
        uow.commit().expect("commit");

        let replayed = idempotency::check_idempotency(&conn, actor.facility_id, "register-xyz")
            .expect("check").expect("should exist");
        assert_eq!(replayed["patient_id"], "xyz");
    }
}
