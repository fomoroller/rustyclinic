//! Program enrollment aggregate with state machine.
//!
//! ```text
//! PROGRAM ENROLLMENT STATE MACHINE:
//!
//!   eligible → enrolled → active → completed
//!                            │
//!                            ├──▶ paused (can resume to active)
//!                            └──▶ withdrawn
//!
//!   eligible or enrolled ──▶ withdrawn
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

use rustyclinic_core::error::AppResult;
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnrollmentStatus {
    Eligible,
    Enrolled,
    Active,
    Paused,
    Completed,
    Withdrawn,
}

impl fmt::Display for EnrollmentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eligible => write!(f, "eligible"),
            Self::Enrolled => write!(f, "enrolled"),
            Self::Active => write!(f, "active"),
            Self::Paused => write!(f, "paused"),
            Self::Completed => write!(f, "completed"),
            Self::Withdrawn => write!(f, "withdrawn"),
        }
    }
}

impl EnrollmentStatus {
    pub fn from_str_safe(s: &str) -> Self {
        match s {
            "eligible" => Self::Eligible,
            "enrolled" => Self::Enrolled,
            "active" => Self::Active,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "withdrawn" => Self::Withdrawn,
            _ => Self::Eligible,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Withdrawn)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EnrollmentTransition {
    Enroll,
    Activate,
    Pause,
    Resume,
    Complete,
    Withdraw,
}

impl fmt::Display for EnrollmentTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enroll => write!(f, "enroll"),
            Self::Activate => write!(f, "activate"),
            Self::Pause => write!(f, "pause"),
            Self::Resume => write!(f, "resume"),
            Self::Complete => write!(f, "complete"),
            Self::Withdraw => write!(f, "withdraw"),
        }
    }
}

/// A program enrollment with state machine lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramEnrollment {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub facility_id: Uuid,
    pub program_code: String,
    pub program_name: String,
    pub status: EnrollmentStatus,
    pub enrolled_by: Uuid,
    pub enrolled_at: Option<DateTime<Utc>>,
    pub activated_at: Option<DateTime<Utc>>,
    pub paused_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub withdrawn_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub version: u32,
}

impl StateMachine for ProgramEnrollment {
    type State = EnrollmentStatus;
    type Transition = EnrollmentTransition;

    fn current_state(&self) -> &EnrollmentStatus {
        &self.status
    }

    fn allowed_transitions(&self, _actor: &ActorContext) -> Vec<EnrollmentTransition> {
        match &self.status {
            EnrollmentStatus::Eligible => {
                vec![EnrollmentTransition::Enroll, EnrollmentTransition::Withdraw]
            }
            EnrollmentStatus::Enrolled => vec![
                EnrollmentTransition::Activate,
                EnrollmentTransition::Withdraw,
            ],
            EnrollmentStatus::Active => vec![
                EnrollmentTransition::Pause,
                EnrollmentTransition::Complete,
                EnrollmentTransition::Withdraw,
            ],
            EnrollmentStatus::Paused => {
                vec![EnrollmentTransition::Resume, EnrollmentTransition::Withdraw]
            }
            _ => vec![],
        }
    }

    fn apply_transition(
        &mut self,
        transition: EnrollmentTransition,
        actor: &ActorContext,
    ) -> AppResult<()> {
        self.validate_transition(&transition, actor)?;

        let now = Utc::now();
        match transition {
            EnrollmentTransition::Enroll => {
                self.status = EnrollmentStatus::Enrolled;
                self.enrolled_at = Some(now);
            }
            EnrollmentTransition::Activate => {
                self.status = EnrollmentStatus::Active;
                self.activated_at = Some(now);
            }
            EnrollmentTransition::Pause => {
                self.status = EnrollmentStatus::Paused;
                self.paused_at = Some(now);
            }
            EnrollmentTransition::Resume => {
                self.status = EnrollmentStatus::Active;
                self.activated_at = Some(now);
            }
            EnrollmentTransition::Complete => {
                self.status = EnrollmentStatus::Completed;
                self.completed_at = Some(now);
            }
            EnrollmentTransition::Withdraw => {
                self.status = EnrollmentStatus::Withdrawn;
                self.withdrawn_at = Some(now);
            }
        }
        self.version += 1;
        Ok(())
    }
}

/// Repository trait for program enrollment persistence.
pub trait ProgramEnrollmentRepo {
    fn create(&self, enrollment: &ProgramEnrollment) -> AppResult<()>;
    fn find_by_id(&self, id: Uuid) -> AppResult<Option<ProgramEnrollment>>;
    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<ProgramEnrollment>>;
    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<ProgramEnrollment>>;
    fn update(&self, enrollment: &ProgramEnrollment) -> AppResult<()>;
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
            roles: vec!["physician".to_string()],
            purpose: "clinical_care".to_string(),
            session_id: new_id(),
        }
    }

    fn new_enrollment(facility_id: Uuid, patient_id: Uuid, enrolled_by: Uuid) -> ProgramEnrollment {
        let now = Utc::now();
        ProgramEnrollment {
            id: new_id(),
            patient_id,
            facility_id,
            program_code: "hiv_art".to_string(),
            program_name: "HIV ART Program".to_string(),
            status: EnrollmentStatus::Eligible,
            enrolled_by,
            enrolled_at: None,
            activated_at: None,
            paused_at: None,
            completed_at: None,
            withdrawn_at: None,
            notes: None,
            created_at: now,
            version: 0,
        }
    }

    #[test]
    fn test_enrollment_happy_path() {
        let actor = test_actor();
        let mut enrollment = new_enrollment(actor.facility_id, new_id(), actor.user_id);

        // eligible → enrolled → active → completed
        enrollment
            .apply_transition(EnrollmentTransition::Enroll, &actor)
            .expect("enroll");
        assert_eq!(enrollment.status, EnrollmentStatus::Enrolled);
        assert!(enrollment.enrolled_at.is_some());

        enrollment
            .apply_transition(EnrollmentTransition::Activate, &actor)
            .expect("activate");
        assert_eq!(enrollment.status, EnrollmentStatus::Active);
        assert!(enrollment.activated_at.is_some());

        enrollment
            .apply_transition(EnrollmentTransition::Complete, &actor)
            .expect("complete");
        assert_eq!(enrollment.status, EnrollmentStatus::Completed);
        assert!(enrollment.completed_at.is_some());
        assert_eq!(enrollment.version, 3);
    }

    #[test]
    fn test_enrollment_pause_and_resume() {
        let actor = test_actor();
        let mut enrollment = new_enrollment(actor.facility_id, new_id(), actor.user_id);

        enrollment
            .apply_transition(EnrollmentTransition::Enroll, &actor)
            .expect("enroll");
        enrollment
            .apply_transition(EnrollmentTransition::Activate, &actor)
            .expect("activate");
        enrollment
            .apply_transition(EnrollmentTransition::Pause, &actor)
            .expect("pause");
        assert_eq!(enrollment.status, EnrollmentStatus::Paused);
        assert!(enrollment.paused_at.is_some());

        enrollment
            .apply_transition(EnrollmentTransition::Resume, &actor)
            .expect("resume");
        assert_eq!(enrollment.status, EnrollmentStatus::Active);
    }

    #[test]
    fn test_enrollment_withdraw_from_eligible() {
        let actor = test_actor();
        let mut enrollment = new_enrollment(actor.facility_id, new_id(), actor.user_id);

        enrollment
            .apply_transition(EnrollmentTransition::Withdraw, &actor)
            .expect("withdraw");
        assert_eq!(enrollment.status, EnrollmentStatus::Withdrawn);
        assert!(enrollment.withdrawn_at.is_some());
    }

    #[test]
    fn test_enrollment_withdraw_from_active() {
        let actor = test_actor();
        let mut enrollment = new_enrollment(actor.facility_id, new_id(), actor.user_id);

        enrollment
            .apply_transition(EnrollmentTransition::Enroll, &actor)
            .expect("enroll");
        enrollment
            .apply_transition(EnrollmentTransition::Activate, &actor)
            .expect("activate");
        enrollment
            .apply_transition(EnrollmentTransition::Withdraw, &actor)
            .expect("withdraw");
        assert_eq!(enrollment.status, EnrollmentStatus::Withdrawn);
    }

    #[test]
    fn test_enrollment_withdraw_from_paused() {
        let actor = test_actor();
        let mut enrollment = new_enrollment(actor.facility_id, new_id(), actor.user_id);

        enrollment
            .apply_transition(EnrollmentTransition::Enroll, &actor)
            .expect("enroll");
        enrollment
            .apply_transition(EnrollmentTransition::Activate, &actor)
            .expect("activate");
        enrollment
            .apply_transition(EnrollmentTransition::Pause, &actor)
            .expect("pause");
        enrollment
            .apply_transition(EnrollmentTransition::Withdraw, &actor)
            .expect("withdraw");
        assert_eq!(enrollment.status, EnrollmentStatus::Withdrawn);
    }

    #[test]
    fn test_enrollment_no_transitions_from_completed() {
        let actor = test_actor();
        let mut enrollment = new_enrollment(actor.facility_id, new_id(), actor.user_id);

        enrollment
            .apply_transition(EnrollmentTransition::Enroll, &actor)
            .expect("enroll");
        enrollment
            .apply_transition(EnrollmentTransition::Activate, &actor)
            .expect("activate");
        enrollment
            .apply_transition(EnrollmentTransition::Complete, &actor)
            .expect("complete");
        assert!(enrollment.allowed_transitions(&actor).is_empty());
    }

    #[test]
    fn test_enrollment_no_transitions_from_withdrawn() {
        let actor = test_actor();
        let mut enrollment = new_enrollment(actor.facility_id, new_id(), actor.user_id);

        enrollment
            .apply_transition(EnrollmentTransition::Withdraw, &actor)
            .expect("withdraw");
        assert!(enrollment.allowed_transitions(&actor).is_empty());
    }

    #[test]
    fn test_enrollment_invalid_transition_rejected() {
        let actor = test_actor();
        let mut enrollment = new_enrollment(actor.facility_id, new_id(), actor.user_id);

        // Cannot activate from eligible (must enroll first)
        let result = enrollment.apply_transition(EnrollmentTransition::Activate, &actor);
        assert!(result.is_err());

        // Cannot complete from eligible
        let result = enrollment.apply_transition(EnrollmentTransition::Complete, &actor);
        assert!(result.is_err());
    }
}
