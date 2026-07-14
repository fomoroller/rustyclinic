//! Referral aggregate with state machine.
//!
//! ```text
//! REFERRAL STATE MACHINE:
//!
//!   drafted → sent → received → accepted → completed
//!                        │
//!                        ├──▶ declined
//!                        └──▶ cancelled
//!
//!   drafted or sent ──▶ cancelled
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

use rustyclinic_core::error::AppResult;
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;

use crate::Priority;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferralStatus {
    Drafted,
    Sent,
    Received,
    Accepted,
    Completed,
    Declined,
    Cancelled,
}

impl fmt::Display for ReferralStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Drafted => write!(f, "drafted"),
            Self::Sent => write!(f, "sent"),
            Self::Received => write!(f, "received"),
            Self::Accepted => write!(f, "accepted"),
            Self::Completed => write!(f, "completed"),
            Self::Declined => write!(f, "declined"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl ReferralStatus {
    pub fn from_str_safe(s: &str) -> Self {
        match s {
            "drafted" => Self::Drafted,
            "sent" => Self::Sent,
            "received" => Self::Received,
            "accepted" => Self::Accepted,
            "completed" => Self::Completed,
            "declined" => Self::Declined,
            "cancelled" => Self::Cancelled,
            _ => Self::Drafted,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Declined | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReferralTransition {
    Send,
    Receive,
    Accept,
    Complete,
    Decline,
    Cancel,
}

impl fmt::Display for ReferralTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Send => write!(f, "send"),
            Self::Receive => write!(f, "receive"),
            Self::Accept => write!(f, "accept"),
            Self::Complete => write!(f, "complete"),
            Self::Decline => write!(f, "decline"),
            Self::Cancel => write!(f, "cancel"),
        }
    }
}

/// A referral with state machine lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Referral {
    pub id: Uuid,
    pub encounter_id: Uuid,
    pub patient_id: Uuid,
    pub facility_id: Uuid,
    pub status: ReferralStatus,
    pub priority: Priority,
    pub referred_by: Uuid,
    pub referred_to_facility: Option<String>,
    pub referred_to_department: Option<String>,
    pub reason: String,
    pub clinical_summary: Option<String>,
    pub sent_at: Option<DateTime<Utc>>,
    pub received_at: Option<DateTime<Utc>>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub version: u32,
}

impl StateMachine for Referral {
    type State = ReferralStatus;
    type Transition = ReferralTransition;

    fn current_state(&self) -> &ReferralStatus {
        &self.status
    }

    fn allowed_transitions(&self, _actor: &ActorContext) -> Vec<ReferralTransition> {
        match &self.status {
            ReferralStatus::Drafted => vec![ReferralTransition::Send, ReferralTransition::Cancel],
            ReferralStatus::Sent => vec![ReferralTransition::Receive, ReferralTransition::Cancel],
            ReferralStatus::Received => {
                vec![ReferralTransition::Accept, ReferralTransition::Decline]
            }
            ReferralStatus::Accepted => vec![ReferralTransition::Complete],
            _ => vec![],
        }
    }

    fn apply_transition(
        &mut self,
        transition: ReferralTransition,
        actor: &ActorContext,
    ) -> AppResult<()> {
        self.validate_transition(&transition, actor)?;

        let now = Utc::now();
        match transition {
            ReferralTransition::Send => {
                self.status = ReferralStatus::Sent;
                self.sent_at = Some(now);
            }
            ReferralTransition::Receive => {
                self.status = ReferralStatus::Received;
                self.received_at = Some(now);
            }
            ReferralTransition::Accept => {
                self.status = ReferralStatus::Accepted;
                self.accepted_at = Some(now);
            }
            ReferralTransition::Complete => {
                self.status = ReferralStatus::Completed;
                self.completed_at = Some(now);
            }
            ReferralTransition::Decline => {
                self.status = ReferralStatus::Declined;
            }
            ReferralTransition::Cancel => {
                self.status = ReferralStatus::Cancelled;
            }
        }
        self.version += 1;
        Ok(())
    }
}

/// Repository trait for referral persistence.
pub trait ReferralRepo {
    fn create(&self, referral: &Referral) -> AppResult<()>;
    fn find_by_id(&self, id: Uuid) -> AppResult<Option<Referral>>;
    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Referral>>;
    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<Referral>>;
    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<Referral>>;
    fn update(&self, referral: &Referral) -> AppResult<()>;
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

    fn new_referral(facility_id: Uuid, patient_id: Uuid, referred_by: Uuid) -> Referral {
        let now = Utc::now();
        Referral {
            id: new_id(),
            encounter_id: new_id(),
            patient_id,
            facility_id,
            status: ReferralStatus::Drafted,
            priority: Priority::Routine,
            referred_by,
            referred_to_facility: Some("District Hospital".to_string()),
            referred_to_department: Some("Surgery".to_string()),
            reason: "Requires surgical intervention".to_string(),
            clinical_summary: Some("Appendicitis suspected".to_string()),
            sent_at: None,
            received_at: None,
            accepted_at: None,
            completed_at: None,
            notes: None,
            created_at: now,
            version: 0,
        }
    }

    #[test]
    fn test_referral_happy_path() {
        let actor = test_actor();
        let mut referral = new_referral(actor.facility_id, new_id(), actor.user_id);

        // drafted → sent → received → accepted → completed
        referral
            .apply_transition(ReferralTransition::Send, &actor)
            .expect("send");
        assert_eq!(referral.status, ReferralStatus::Sent);
        assert!(referral.sent_at.is_some());

        referral
            .apply_transition(ReferralTransition::Receive, &actor)
            .expect("receive");
        assert_eq!(referral.status, ReferralStatus::Received);

        referral
            .apply_transition(ReferralTransition::Accept, &actor)
            .expect("accept");
        assert_eq!(referral.status, ReferralStatus::Accepted);

        referral
            .apply_transition(ReferralTransition::Complete, &actor)
            .expect("complete");
        assert_eq!(referral.status, ReferralStatus::Completed);
        assert_eq!(referral.version, 4);
    }

    #[test]
    fn test_referral_decline_path() {
        let actor = test_actor();
        let mut referral = new_referral(actor.facility_id, new_id(), actor.user_id);

        referral
            .apply_transition(ReferralTransition::Send, &actor)
            .expect("send");
        referral
            .apply_transition(ReferralTransition::Receive, &actor)
            .expect("receive");
        referral
            .apply_transition(ReferralTransition::Decline, &actor)
            .expect("decline");
        assert_eq!(referral.status, ReferralStatus::Declined);
    }

    #[test]
    fn test_referral_cancel_from_drafted() {
        let actor = test_actor();
        let mut referral = new_referral(actor.facility_id, new_id(), actor.user_id);

        referral
            .apply_transition(ReferralTransition::Cancel, &actor)
            .expect("cancel");
        assert_eq!(referral.status, ReferralStatus::Cancelled);
    }

    #[test]
    fn test_referral_cancel_from_sent() {
        let actor = test_actor();
        let mut referral = new_referral(actor.facility_id, new_id(), actor.user_id);

        referral
            .apply_transition(ReferralTransition::Send, &actor)
            .expect("send");
        referral
            .apply_transition(ReferralTransition::Cancel, &actor)
            .expect("cancel");
        assert_eq!(referral.status, ReferralStatus::Cancelled);
    }

    #[test]
    fn test_referral_no_transitions_from_terminal() {
        let actor = test_actor();

        // completed
        let mut referral = new_referral(actor.facility_id, new_id(), actor.user_id);
        referral
            .apply_transition(ReferralTransition::Send, &actor)
            .expect("send");
        referral
            .apply_transition(ReferralTransition::Receive, &actor)
            .expect("receive");
        referral
            .apply_transition(ReferralTransition::Accept, &actor)
            .expect("accept");
        referral
            .apply_transition(ReferralTransition::Complete, &actor)
            .expect("complete");
        assert!(referral.allowed_transitions(&actor).is_empty());

        // declined
        let mut referral = new_referral(actor.facility_id, new_id(), actor.user_id);
        referral
            .apply_transition(ReferralTransition::Send, &actor)
            .expect("send");
        referral
            .apply_transition(ReferralTransition::Receive, &actor)
            .expect("receive");
        referral
            .apply_transition(ReferralTransition::Decline, &actor)
            .expect("decline");
        assert!(referral.allowed_transitions(&actor).is_empty());

        // cancelled
        let mut referral = new_referral(actor.facility_id, new_id(), actor.user_id);
        referral
            .apply_transition(ReferralTransition::Cancel, &actor)
            .expect("cancel");
        assert!(referral.allowed_transitions(&actor).is_empty());
    }

    #[test]
    fn test_referral_invalid_transition_rejected() {
        let actor = test_actor();
        let mut referral = new_referral(actor.facility_id, new_id(), actor.user_id);

        // Cannot complete from drafted
        let result = referral.apply_transition(ReferralTransition::Complete, &actor);
        assert!(result.is_err());

        // Cannot accept from drafted
        let result = referral.apply_transition(ReferralTransition::Accept, &actor);
        assert!(result.is_err());
    }
}
