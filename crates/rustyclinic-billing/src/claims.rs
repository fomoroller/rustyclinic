//! Claim case aggregate with state machine.
//!
//! ```text
//! CLAIM CASE STATE MACHINE:
//!
//!   draft → validated → batched → submitted → acknowledged → adjudicated → paid
//!                                     │             │              │
//!                                     └─────────────┴──────────────┴──▶ rejected → reopened → validated (retry)
//!
//!   void from any non-terminal state
//! ```

use chrono::{DateTime, Utc};
use rustyclinic_core::error::AppResult;
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

use crate::tariff::ClaimItem;

/// Claim case status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimStatus {
    Draft,
    Validated,
    Batched,
    Submitted,
    Acknowledged,
    Adjudicated,
    Paid,
    Rejected,
    Voided,
    Reopened,
}

impl fmt::Display for ClaimStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Validated => write!(f, "validated"),
            Self::Batched => write!(f, "batched"),
            Self::Submitted => write!(f, "submitted"),
            Self::Acknowledged => write!(f, "acknowledged"),
            Self::Adjudicated => write!(f, "adjudicated"),
            Self::Paid => write!(f, "paid"),
            Self::Rejected => write!(f, "rejected"),
            Self::Voided => write!(f, "voided"),
            Self::Reopened => write!(f, "reopened"),
        }
    }
}

/// Transitions for the claim case state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClaimTransition {
    Validate,
    Batch,
    Submit,
    Acknowledge,
    Adjudicate,
    Pay,
    Reject,
    Void,
    Reopen,
}

impl fmt::Display for ClaimTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validate => write!(f, "validate"),
            Self::Batch => write!(f, "batch"),
            Self::Submit => write!(f, "submit"),
            Self::Acknowledge => write!(f, "acknowledge"),
            Self::Adjudicate => write!(f, "adjudicate"),
            Self::Pay => write!(f, "pay"),
            Self::Reject => write!(f, "reject"),
            Self::Void => write!(f, "void"),
            Self::Reopen => write!(f, "reopen"),
        }
    }
}

/// A claim case — the billing aggregate for a patient encounter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimCase {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub patient_id: Uuid,
    pub encounter_id: Option<Uuid>,
    pub payer_id: String,
    pub claim_number: Option<String>,
    pub status: ClaimStatus,
    pub total_amount: f64,
    pub approved_amount: Option<f64>,
    pub items: Vec<ClaimItem>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub adjudicated_at: Option<DateTime<Utc>>,
    pub paid_at: Option<DateTime<Utc>>,
    pub rejection_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub version: u32,
}

impl StateMachine for ClaimCase {
    type State = ClaimStatus;
    type Transition = ClaimTransition;

    fn current_state(&self) -> &ClaimStatus {
        &self.status
    }

    fn allowed_transitions(&self, _actor: &ActorContext) -> Vec<ClaimTransition> {
        match &self.status {
            ClaimStatus::Draft => vec![ClaimTransition::Validate, ClaimTransition::Void],
            ClaimStatus::Validated => vec![ClaimTransition::Batch, ClaimTransition::Void],
            ClaimStatus::Batched => vec![ClaimTransition::Submit, ClaimTransition::Void],
            ClaimStatus::Submitted => vec![
                ClaimTransition::Acknowledge,
                ClaimTransition::Reject,
                ClaimTransition::Void,
            ],
            ClaimStatus::Acknowledged => vec![
                ClaimTransition::Adjudicate,
                ClaimTransition::Reject,
                ClaimTransition::Void,
            ],
            ClaimStatus::Adjudicated => vec![
                ClaimTransition::Pay,
                ClaimTransition::Reject,
                ClaimTransition::Void,
            ],
            ClaimStatus::Rejected => vec![ClaimTransition::Reopen, ClaimTransition::Void],
            ClaimStatus::Reopened => vec![ClaimTransition::Validate, ClaimTransition::Void],
            ClaimStatus::Paid => vec![],
            ClaimStatus::Voided => vec![],
        }
    }

    fn apply_transition(
        &mut self,
        transition: ClaimTransition,
        actor: &ActorContext,
    ) -> AppResult<()> {
        self.validate_transition(&transition, actor)?;

        let now = Utc::now();
        match transition {
            ClaimTransition::Validate => {
                self.status = ClaimStatus::Validated;
            }
            ClaimTransition::Batch => {
                self.status = ClaimStatus::Batched;
            }
            ClaimTransition::Submit => {
                self.status = ClaimStatus::Submitted;
                self.submitted_at = Some(now);
            }
            ClaimTransition::Acknowledge => {
                self.status = ClaimStatus::Acknowledged;
            }
            ClaimTransition::Adjudicate => {
                self.status = ClaimStatus::Adjudicated;
                self.adjudicated_at = Some(now);
            }
            ClaimTransition::Pay => {
                self.status = ClaimStatus::Paid;
                self.paid_at = Some(now);
            }
            ClaimTransition::Reject => {
                self.status = ClaimStatus::Rejected;
            }
            ClaimTransition::Void => {
                self.status = ClaimStatus::Voided;
            }
            ClaimTransition::Reopen => {
                self.status = ClaimStatus::Reopened;
            }
        }
        self.version += 1;
        Ok(())
    }
}

/// Repository trait for claim case persistence.
pub trait ClaimCaseRepo {
    fn create(&self, claim: &ClaimCase) -> AppResult<()>;
    fn find_by_id(&self, id: Uuid) -> AppResult<Option<ClaimCase>>;
    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<ClaimCase>>;
    fn find_by_facility_and_status(
        &self,
        facility_id: Uuid,
        status: &ClaimStatus,
    ) -> AppResult<Vec<ClaimCase>>;
    fn update(&self, claim: &ClaimCase) -> AppResult<()>;
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
            roles: vec!["billing_clerk".to_string()],
            purpose: "billing".to_string(),
            session_id: new_id(),
        }
    }

    fn new_claim(facility_id: Uuid, patient_id: Uuid) -> ClaimCase {
        let now = Utc::now();
        ClaimCase {
            id: new_id(),
            facility_id,
            patient_id,
            encounter_id: Some(new_id()),
            payer_id: "RSSB".to_string(),
            claim_number: None,
            status: ClaimStatus::Draft,
            total_amount: 15000.0,
            approved_amount: None,
            items: vec![
                ClaimItem {
                    service_code: "CONSULT-001".to_string(),
                    service_name: "General Consultation".to_string(),
                    quantity: 1,
                    unit_price: 5000.0,
                    total: 5000.0,
                    approved_amount: None,
                },
                ClaimItem {
                    service_code: "LAB-MALARIA".to_string(),
                    service_name: "Malaria RDT".to_string(),
                    quantity: 1,
                    unit_price: 3000.0,
                    total: 3000.0,
                    approved_amount: None,
                },
                ClaimItem {
                    service_code: "PHARM-ACT".to_string(),
                    service_name: "Artemether-Lumefantrine".to_string(),
                    quantity: 1,
                    unit_price: 7000.0,
                    total: 7000.0,
                    approved_amount: None,
                },
            ],
            submitted_at: None,
            adjudicated_at: None,
            paid_at: None,
            rejection_reason: None,
            created_at: now,
            version: 0,
        }
    }

    #[test]
    fn test_happy_path_draft_to_paid() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        // draft → validated → batched → submitted → acknowledged → adjudicated → paid
        claim
            .apply_transition(ClaimTransition::Validate, &actor)
            .expect("validate");
        assert_eq!(claim.status, ClaimStatus::Validated);

        claim
            .apply_transition(ClaimTransition::Batch, &actor)
            .expect("batch");
        assert_eq!(claim.status, ClaimStatus::Batched);

        claim
            .apply_transition(ClaimTransition::Submit, &actor)
            .expect("submit");
        assert_eq!(claim.status, ClaimStatus::Submitted);
        assert!(claim.submitted_at.is_some());

        claim
            .apply_transition(ClaimTransition::Acknowledge, &actor)
            .expect("acknowledge");
        assert_eq!(claim.status, ClaimStatus::Acknowledged);

        claim
            .apply_transition(ClaimTransition::Adjudicate, &actor)
            .expect("adjudicate");
        assert_eq!(claim.status, ClaimStatus::Adjudicated);
        assert!(claim.adjudicated_at.is_some());

        claim
            .apply_transition(ClaimTransition::Pay, &actor)
            .expect("pay");
        assert_eq!(claim.status, ClaimStatus::Paid);
        assert!(claim.paid_at.is_some());

        assert_eq!(claim.version, 6);
    }

    #[test]
    fn test_rejection_and_retry() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        // draft → validated → batched → submitted → rejected → reopened → validated
        claim
            .apply_transition(ClaimTransition::Validate, &actor)
            .expect("validate");
        claim
            .apply_transition(ClaimTransition::Batch, &actor)
            .expect("batch");
        claim
            .apply_transition(ClaimTransition::Submit, &actor)
            .expect("submit");

        claim
            .apply_transition(ClaimTransition::Reject, &actor)
            .expect("reject");
        assert_eq!(claim.status, ClaimStatus::Rejected);

        claim
            .apply_transition(ClaimTransition::Reopen, &actor)
            .expect("reopen");
        assert_eq!(claim.status, ClaimStatus::Reopened);

        claim
            .apply_transition(ClaimTransition::Validate, &actor)
            .expect("re-validate");
        assert_eq!(claim.status, ClaimStatus::Validated);
    }

    #[test]
    fn test_void_from_draft() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        claim
            .apply_transition(ClaimTransition::Void, &actor)
            .expect("void");
        assert_eq!(claim.status, ClaimStatus::Voided);
        assert!(claim.allowed_transitions(&actor).is_empty());
    }

    #[test]
    fn test_void_from_submitted() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        claim
            .apply_transition(ClaimTransition::Validate, &actor)
            .expect("validate");
        claim
            .apply_transition(ClaimTransition::Batch, &actor)
            .expect("batch");
        claim
            .apply_transition(ClaimTransition::Submit, &actor)
            .expect("submit");
        claim
            .apply_transition(ClaimTransition::Void, &actor)
            .expect("void from submitted");
        assert_eq!(claim.status, ClaimStatus::Voided);
    }

    #[test]
    fn test_void_from_acknowledged() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        claim
            .apply_transition(ClaimTransition::Validate, &actor)
            .expect("validate");
        claim
            .apply_transition(ClaimTransition::Batch, &actor)
            .expect("batch");
        claim
            .apply_transition(ClaimTransition::Submit, &actor)
            .expect("submit");
        claim
            .apply_transition(ClaimTransition::Acknowledge, &actor)
            .expect("acknowledge");
        claim
            .apply_transition(ClaimTransition::Void, &actor)
            .expect("void from acknowledged");
        assert_eq!(claim.status, ClaimStatus::Voided);
    }

    #[test]
    fn test_void_from_adjudicated() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        claim
            .apply_transition(ClaimTransition::Validate, &actor)
            .expect("validate");
        claim
            .apply_transition(ClaimTransition::Batch, &actor)
            .expect("batch");
        claim
            .apply_transition(ClaimTransition::Submit, &actor)
            .expect("submit");
        claim
            .apply_transition(ClaimTransition::Acknowledge, &actor)
            .expect("acknowledge");
        claim
            .apply_transition(ClaimTransition::Adjudicate, &actor)
            .expect("adjudicate");
        claim
            .apply_transition(ClaimTransition::Void, &actor)
            .expect("void from adjudicated");
        assert_eq!(claim.status, ClaimStatus::Voided);
    }

    #[test]
    fn test_void_from_rejected() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        claim
            .apply_transition(ClaimTransition::Validate, &actor)
            .expect("validate");
        claim
            .apply_transition(ClaimTransition::Batch, &actor)
            .expect("batch");
        claim
            .apply_transition(ClaimTransition::Submit, &actor)
            .expect("submit");
        claim
            .apply_transition(ClaimTransition::Reject, &actor)
            .expect("reject");
        claim
            .apply_transition(ClaimTransition::Void, &actor)
            .expect("void from rejected");
        assert_eq!(claim.status, ClaimStatus::Voided);
    }

    #[test]
    fn test_invalid_transition_draft_to_submit() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        // Cannot submit directly from draft — must validate and batch first
        let result = claim.apply_transition(ClaimTransition::Submit, &actor);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_transition_paid_is_terminal() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        claim
            .apply_transition(ClaimTransition::Validate, &actor)
            .expect("validate");
        claim
            .apply_transition(ClaimTransition::Batch, &actor)
            .expect("batch");
        claim
            .apply_transition(ClaimTransition::Submit, &actor)
            .expect("submit");
        claim
            .apply_transition(ClaimTransition::Acknowledge, &actor)
            .expect("acknowledge");
        claim
            .apply_transition(ClaimTransition::Adjudicate, &actor)
            .expect("adjudicate");
        claim
            .apply_transition(ClaimTransition::Pay, &actor)
            .expect("pay");

        assert!(claim.allowed_transitions(&actor).is_empty());

        let result = claim.apply_transition(ClaimTransition::Void, &actor);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_transition_voided_is_terminal() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        claim
            .apply_transition(ClaimTransition::Void, &actor)
            .expect("void");

        let result = claim.apply_transition(ClaimTransition::Validate, &actor);
        assert!(result.is_err());
    }

    #[test]
    fn test_cannot_pay_from_submitted() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        claim
            .apply_transition(ClaimTransition::Validate, &actor)
            .expect("validate");
        claim
            .apply_transition(ClaimTransition::Batch, &actor)
            .expect("batch");
        claim
            .apply_transition(ClaimTransition::Submit, &actor)
            .expect("submit");

        // Cannot skip acknowledge and adjudicate
        let result = claim.apply_transition(ClaimTransition::Pay, &actor);
        assert!(result.is_err());
    }

    #[test]
    fn test_cannot_reopen_from_non_rejected() {
        let actor = test_actor();
        let mut claim = new_claim(actor.facility_id, new_id());

        claim
            .apply_transition(ClaimTransition::Validate, &actor)
            .expect("validate");

        // Reopen only allowed from Rejected
        let result = claim.apply_transition(ClaimTransition::Reopen, &actor);
        assert!(result.is_err());
    }

    #[test]
    fn test_full_integration_encounter_to_paid() {
        let actor = test_actor();
        let patient_id = new_id();
        let encounter_id = new_id();

        // Simulate: encounter created, claim built from tariff lookup
        let mut claim = ClaimCase {
            id: new_id(),
            facility_id: actor.facility_id,
            patient_id,
            encounter_id: Some(encounter_id),
            payer_id: "RSSB".to_string(),
            claim_number: None,
            status: ClaimStatus::Draft,
            total_amount: 8000.0,
            approved_amount: None,
            items: vec![ClaimItem {
                service_code: "CONSULT-001".to_string(),
                service_name: "General Consultation".to_string(),
                quantity: 1,
                unit_price: 8000.0,
                total: 8000.0,
                approved_amount: None,
            }],
            submitted_at: None,
            adjudicated_at: None,
            paid_at: None,
            rejection_reason: None,
            created_at: Utc::now(),
            version: 0,
        };

        // Full lifecycle
        claim
            .apply_transition(ClaimTransition::Validate, &actor)
            .expect("validate");
        claim
            .apply_transition(ClaimTransition::Batch, &actor)
            .expect("batch");
        claim
            .apply_transition(ClaimTransition::Submit, &actor)
            .expect("submit");
        claim
            .apply_transition(ClaimTransition::Acknowledge, &actor)
            .expect("acknowledge");

        // Simulate adjudication with approved amount
        claim.approved_amount = Some(8000.0);
        for item in &mut claim.items {
            item.approved_amount = Some(item.total);
        }
        claim
            .apply_transition(ClaimTransition::Adjudicate, &actor)
            .expect("adjudicate");
        claim
            .apply_transition(ClaimTransition::Pay, &actor)
            .expect("pay");

        assert_eq!(claim.status, ClaimStatus::Paid);
        assert!(claim.submitted_at.is_some());
        assert!(claim.adjudicated_at.is_some());
        assert!(claim.paid_at.is_some());
        assert_eq!(claim.approved_amount, Some(8000.0));
        assert_eq!(claim.version, 6);
    }
}
