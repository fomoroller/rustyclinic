//! Queue entry aggregate with state machine.
//!
//! ```text
//! QUEUE STATE MACHINE:
//!
//!   created → waiting → called → in_service → completed
//!                │         │          │
//!                │         │          └──▶ transferred
//!                │         └──▶ no_show
//!                └──▶ cancelled
//!
//!   The `called` transition uses assigned_to + optimistic lock
//!   to prevent two nurses from calling the same patient.
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use rustyclinic_core::error::{AppError, AppResult};
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueueStatus {
    Created,
    Waiting,
    Called,
    InService,
    Transferred,
    Completed,
    NoShow,
    Cancelled,
}

impl fmt::Display for QueueStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Waiting => write!(f, "waiting"),
            Self::Called => write!(f, "called"),
            Self::InService => write!(f, "in_service"),
            Self::Transferred => write!(f, "transferred"),
            Self::Completed => write!(f, "completed"),
            Self::NoShow => write!(f, "no_show"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueueTransition {
    Enqueue,
    Call,
    BeginService,
    Transfer,
    Complete,
    MarkNoShow,
    Cancel,
}

impl fmt::Display for QueueTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enqueue => write!(f, "enqueue"),
            Self::Call => write!(f, "call"),
            Self::BeginService => write!(f, "begin_service"),
            Self::Transfer => write!(f, "transfer"),
            Self::Complete => write!(f, "complete"),
            Self::MarkNoShow => write!(f, "mark_no_show"),
            Self::Cancel => write!(f, "cancel"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueEntry {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub patient_id: Uuid,
    pub service_type: String,
    pub status: QueueStatus,
    pub assigned_to: Option<Uuid>,
    pub position: u32,
    pub arrived_at: DateTime<Utc>,
    pub called_at: Option<DateTime<Utc>>,
    pub service_started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub version: u32,
}

impl StateMachine for QueueEntry {
    type State = QueueStatus;
    type Transition = QueueTransition;

    fn current_state(&self) -> &QueueStatus {
        &self.status
    }

    fn allowed_transitions(&self, _actor: &ActorContext) -> Vec<QueueTransition> {
        match &self.status {
            QueueStatus::Created => vec![QueueTransition::Enqueue, QueueTransition::Cancel],
            QueueStatus::Waiting => vec![QueueTransition::Call, QueueTransition::Cancel],
            QueueStatus::Called => vec![
                QueueTransition::BeginService,
                QueueTransition::MarkNoShow,
                QueueTransition::Cancel,
            ],
            QueueStatus::InService => vec![
                QueueTransition::Complete,
                QueueTransition::Transfer,
            ],
            QueueStatus::Transferred => vec![QueueTransition::Complete],
            _ => vec![],
        }
    }

    fn apply_transition(
        &mut self,
        transition: QueueTransition,
        actor: &ActorContext,
    ) -> AppResult<()> {
        self.validate_transition(&transition, actor)?;

        let now = Utc::now();
        match transition {
            QueueTransition::Enqueue => {
                self.status = QueueStatus::Waiting;
            }
            QueueTransition::Call => {
                // Claim mechanism: assigned_to + version for optimistic lock
                if self.assigned_to.is_some() {
                    return Err(AppError::Conflict {
                        message: "patient already called by another user".to_string(),
                    });
                }
                self.status = QueueStatus::Called;
                self.assigned_to = Some(actor.user_id);
                self.called_at = Some(now);
            }
            QueueTransition::BeginService => {
                self.status = QueueStatus::InService;
                self.service_started_at = Some(now);
            }
            QueueTransition::Transfer => {
                self.status = QueueStatus::Transferred;
                self.assigned_to = None;
            }
            QueueTransition::Complete => {
                self.status = QueueStatus::Completed;
                self.completed_at = Some(now);
            }
            QueueTransition::MarkNoShow => {
                self.status = QueueStatus::NoShow;
                self.completed_at = Some(now);
            }
            QueueTransition::Cancel => {
                self.status = QueueStatus::Cancelled;
                self.completed_at = Some(now);
            }
        }
        self.version += 1;
        Ok(())
    }
}

/// Repository trait for queue entry persistence.
pub trait QueueEntryRepo {
    fn create(&self, entry: &QueueEntry) -> AppResult<()>;
    fn find_by_id(&self, id: Uuid) -> AppResult<Option<QueueEntry>>;
    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<QueueEntry>>;
    fn update(&self, entry: &QueueEntry) -> AppResult<()>;
    fn next_position(&self, facility_id: Uuid) -> AppResult<u32>;
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
            roles: vec!["nurse".to_string()],
            purpose: "clinical_care".to_string(),
            session_id: new_id(),
        }
    }

    fn new_queue_entry(facility_id: Uuid, patient_id: Uuid) -> QueueEntry {
        let now = Utc::now();
        QueueEntry {
            id: new_id(),
            facility_id,
            patient_id,
            service_type: "consultation".to_string(),
            status: QueueStatus::Created,
            assigned_to: None,
            position: 1,
            arrived_at: now,
            called_at: None,
            service_started_at: None,
            completed_at: None,
            created_at: now,
            version: 0,
        }
    }

    #[test]
    fn test_happy_path_queue_flow() {
        let actor = test_actor();
        let mut entry = new_queue_entry(actor.facility_id, new_id());

        // created → waiting → called → in_service → completed
        entry.apply_transition(QueueTransition::Enqueue, &actor).expect("enqueue");
        assert_eq!(entry.status, QueueStatus::Waiting);

        entry.apply_transition(QueueTransition::Call, &actor).expect("call");
        assert_eq!(entry.status, QueueStatus::Called);
        assert_eq!(entry.assigned_to, Some(actor.user_id));

        entry.apply_transition(QueueTransition::BeginService, &actor).expect("begin");
        assert_eq!(entry.status, QueueStatus::InService);

        entry.apply_transition(QueueTransition::Complete, &actor).expect("complete");
        assert_eq!(entry.status, QueueStatus::Completed);
        assert_eq!(entry.version, 4);
    }

    #[test]
    fn test_invalid_transition_rejected() {
        let actor = test_actor();
        let mut entry = new_queue_entry(actor.facility_id, new_id());

        // Cannot call from Created (must enqueue first)
        let result = entry.apply_transition(QueueTransition::Call, &actor);
        assert!(result.is_err());
    }

    #[test]
    fn test_double_call_prevented() {
        let actor1 = test_actor();
        let actor2 = test_actor();
        let mut entry = new_queue_entry(actor1.facility_id, new_id());

        entry.apply_transition(QueueTransition::Enqueue, &actor1).expect("enqueue");
        entry.apply_transition(QueueTransition::Call, &actor1).expect("first call");

        // Second call should fail — patient already claimed
        let mut entry2 = entry.clone();
        entry2.status = QueueStatus::Waiting;  // simulate racing state
        entry2.assigned_to = Some(actor1.user_id); // but assigned_to is set
        let result = entry2.apply_transition(QueueTransition::Call, &actor2);
        assert!(result.is_err());
    }

    #[test]
    fn test_no_show_from_called() {
        let actor = test_actor();
        let mut entry = new_queue_entry(actor.facility_id, new_id());

        entry.apply_transition(QueueTransition::Enqueue, &actor).expect("enqueue");
        entry.apply_transition(QueueTransition::Call, &actor).expect("call");
        entry.apply_transition(QueueTransition::MarkNoShow, &actor).expect("no_show");
        assert_eq!(entry.status, QueueStatus::NoShow);
    }

    #[test]
    fn test_no_transitions_from_terminal_states() {
        let actor = test_actor();
        let mut entry = new_queue_entry(actor.facility_id, new_id());

        entry.apply_transition(QueueTransition::Enqueue, &actor).expect("enqueue");
        entry.apply_transition(QueueTransition::Cancel, &actor).expect("cancel");

        assert!(entry.allowed_transitions(&actor).is_empty());
    }
}
