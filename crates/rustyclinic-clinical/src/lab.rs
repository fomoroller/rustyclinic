//! Lab order aggregate with state machine.
//!
//! ```text
//! LAB WORKFLOW STATE MACHINE:
//!
//!   ordered → sample_pending → collected → received → in_process → resulted → verified
//!                                                                      │
//!                                                                      └──▶ amended
//!   Any pre-result state ──▶ cancelled
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
pub enum LabStatus {
    Ordered,
    SamplePending,
    Collected,
    Received,
    InProcess,
    Resulted,
    Verified,
    Amended,
    Cancelled,
}

impl fmt::Display for LabStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ordered => write!(f, "ordered"),
            Self::SamplePending => write!(f, "sample_pending"),
            Self::Collected => write!(f, "collected"),
            Self::Received => write!(f, "received"),
            Self::InProcess => write!(f, "in_process"),
            Self::Resulted => write!(f, "resulted"),
            Self::Verified => write!(f, "verified"),
            Self::Amended => write!(f, "amended"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl LabStatus {
    /// Parse from string (database round-trip).
    pub fn from_str_safe(s: &str) -> Self {
        match s {
            "ordered" => Self::Ordered,
            "sample_pending" => Self::SamplePending,
            "collected" => Self::Collected,
            "received" => Self::Received,
            "in_process" => Self::InProcess,
            "resulted" => Self::Resulted,
            "verified" => Self::Verified,
            "amended" => Self::Amended,
            "cancelled" => Self::Cancelled,
            _ => Self::Ordered,
        }
    }

    /// Whether this is a terminal state (no further transitions).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Verified | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LabTransition {
    RequestSample,
    CollectSample,
    ReceiveAtLab,
    BeginProcessing,
    EnterResults,
    Verify,
    Amend,
    Cancel,
}

impl fmt::Display for LabTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestSample => write!(f, "request_sample"),
            Self::CollectSample => write!(f, "collect_sample"),
            Self::ReceiveAtLab => write!(f, "receive_at_lab"),
            Self::BeginProcessing => write!(f, "begin_processing"),
            Self::EnterResults => write!(f, "enter_results"),
            Self::Verify => write!(f, "verify"),
            Self::Amend => write!(f, "amend"),
            Self::Cancel => write!(f, "cancel"),
        }
    }
}

/// A lab order with full state machine lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabOrder {
    pub id: Uuid,
    pub encounter_id: Uuid,
    pub patient_id: Uuid,
    pub facility_id: Uuid,
    pub status: LabStatus,
    pub priority: Priority,
    pub ordered_by: Uuid,
    pub tests: Vec<LabTest>,
    pub specimen_type: Option<String>,
    pub collected_at: Option<DateTime<Utc>>,
    pub collected_by: Option<Uuid>,
    pub resulted_at: Option<DateTime<Utc>>,
    pub resulted_by: Option<Uuid>,
    pub verified_at: Option<DateTime<Utc>>,
    pub verified_by: Option<Uuid>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub version: u32,
}

impl StateMachine for LabOrder {
    type State = LabStatus;
    type Transition = LabTransition;

    fn current_state(&self) -> &LabStatus {
        &self.status
    }

    fn allowed_transitions(&self, _actor: &ActorContext) -> Vec<LabTransition> {
        match &self.status {
            LabStatus::Ordered => vec![LabTransition::RequestSample, LabTransition::Cancel],
            LabStatus::SamplePending => vec![LabTransition::CollectSample, LabTransition::Cancel],
            LabStatus::Collected => vec![LabTransition::ReceiveAtLab, LabTransition::Cancel],
            LabStatus::Received => vec![LabTransition::BeginProcessing, LabTransition::Cancel],
            LabStatus::InProcess => vec![LabTransition::EnterResults, LabTransition::Cancel],
            LabStatus::Resulted => vec![LabTransition::Verify],
            LabStatus::Verified => vec![LabTransition::Amend],
            LabStatus::Amended => vec![LabTransition::Verify],
            LabStatus::Cancelled => vec![],
        }
    }

    fn apply_transition(
        &mut self,
        transition: LabTransition,
        actor: &ActorContext,
    ) -> AppResult<()> {
        self.validate_transition(&transition, actor)?;

        let now = Utc::now();
        match transition {
            LabTransition::RequestSample => {
                self.status = LabStatus::SamplePending;
            }
            LabTransition::CollectSample => {
                self.status = LabStatus::Collected;
                self.collected_at = Some(now);
                self.collected_by = Some(actor.user_id);
            }
            LabTransition::ReceiveAtLab => {
                self.status = LabStatus::Received;
            }
            LabTransition::BeginProcessing => {
                self.status = LabStatus::InProcess;
            }
            LabTransition::EnterResults => {
                self.status = LabStatus::Resulted;
                self.resulted_at = Some(now);
                self.resulted_by = Some(actor.user_id);
            }
            LabTransition::Verify => {
                self.status = LabStatus::Verified;
                self.verified_at = Some(now);
                self.verified_by = Some(actor.user_id);
            }
            LabTransition::Amend => {
                self.status = LabStatus::Amended;
            }
            LabTransition::Cancel => {
                self.status = LabStatus::Cancelled;
            }
        }
        self.version += 1;
        Ok(())
    }
}

/// An individual lab test within a lab order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabTest {
    pub test_code: String,
    pub test_name: String,
    pub result: Option<String>,
    pub result_value: Option<f64>,
    pub unit: Option<String>,
    pub reference_range: Option<String>,
    pub is_abnormal: bool,
    pub resulted_at: Option<DateTime<Utc>>,
    pub resulted_by: Option<Uuid>,
}

/// Common lab tests available in the system.
pub const COMMON_TESTS: &[(&str, &str)] = &[
    ("malaria_rdt", "Malaria RDT"),
    ("cbc", "Complete Blood Count"),
    ("urinalysis", "Urinalysis"),
    ("hiv_rapid", "HIV Rapid Test"),
    ("blood_glucose", "Blood Glucose"),
    ("hemoglobin", "Hemoglobin"),
    ("urine_pregnancy", "Urine Pregnancy Test"),
    ("stool_exam", "Stool Examination"),
];

/// Repository trait for lab order persistence.
pub trait LabOrderRepo {
    fn create(&self, order: &LabOrder) -> AppResult<()>;
    fn find_by_id(&self, id: Uuid) -> AppResult<Option<LabOrder>>;
    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<LabOrder>>;
    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<LabOrder>>;
    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<LabOrder>>;
    fn update(&self, order: &LabOrder) -> AppResult<()>;
}

/// Repository trait for lab test results within an order.
pub trait LabTestRepo {
    fn create_tests(&self, order_id: Uuid, tests: &[LabTest]) -> AppResult<()>;
    fn find_by_order(&self, order_id: Uuid) -> AppResult<Vec<LabTest>>;
    fn update_test(&self, order_id: Uuid, test: &LabTest) -> AppResult<()>;
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
            roles: vec!["lab_tech".to_string()],
            purpose: "clinical_care".to_string(),
            session_id: new_id(),
        }
    }

    fn new_lab_order(facility_id: Uuid, patient_id: Uuid, ordered_by: Uuid) -> LabOrder {
        let now = Utc::now();
        LabOrder {
            id: new_id(),
            encounter_id: new_id(),
            patient_id,
            facility_id,
            status: LabStatus::Ordered,
            priority: Priority::Routine,
            ordered_by,
            tests: vec![LabTest {
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
            collected_at: None,
            collected_by: None,
            resulted_at: None,
            resulted_by: None,
            verified_at: None,
            verified_by: None,
            notes: None,
            created_at: now,
            version: 0,
        }
    }

    #[test]
    fn test_lab_happy_path() {
        let actor = test_actor();
        let mut order = new_lab_order(actor.facility_id, new_id(), actor.user_id);

        // ordered → sample_pending → collected → received → in_process → resulted → verified
        order
            .apply_transition(LabTransition::RequestSample, &actor)
            .expect("request_sample");
        assert_eq!(order.status, LabStatus::SamplePending);

        order
            .apply_transition(LabTransition::CollectSample, &actor)
            .expect("collect");
        assert_eq!(order.status, LabStatus::Collected);
        assert!(order.collected_at.is_some());
        assert_eq!(order.collected_by, Some(actor.user_id));

        order
            .apply_transition(LabTransition::ReceiveAtLab, &actor)
            .expect("receive");
        assert_eq!(order.status, LabStatus::Received);

        order
            .apply_transition(LabTransition::BeginProcessing, &actor)
            .expect("begin");
        assert_eq!(order.status, LabStatus::InProcess);

        order
            .apply_transition(LabTransition::EnterResults, &actor)
            .expect("results");
        assert_eq!(order.status, LabStatus::Resulted);
        assert!(order.resulted_at.is_some());

        order
            .apply_transition(LabTransition::Verify, &actor)
            .expect("verify");
        assert_eq!(order.status, LabStatus::Verified);
        assert!(order.verified_at.is_some());
        assert_eq!(order.version, 6);
    }

    #[test]
    fn test_lab_cancel_from_pre_result_states() {
        let actor = test_actor();

        // Cancel from ordered
        let mut order = new_lab_order(actor.facility_id, new_id(), actor.user_id);
        order
            .apply_transition(LabTransition::Cancel, &actor)
            .expect("cancel from ordered");
        assert_eq!(order.status, LabStatus::Cancelled);

        // Cancel from sample_pending
        let mut order = new_lab_order(actor.facility_id, new_id(), actor.user_id);
        order
            .apply_transition(LabTransition::RequestSample, &actor)
            .expect("request");
        order
            .apply_transition(LabTransition::Cancel, &actor)
            .expect("cancel from sample_pending");
        assert_eq!(order.status, LabStatus::Cancelled);

        // Cancel from in_process
        let mut order = new_lab_order(actor.facility_id, new_id(), actor.user_id);
        order
            .apply_transition(LabTransition::RequestSample, &actor)
            .expect("req");
        order
            .apply_transition(LabTransition::CollectSample, &actor)
            .expect("collect");
        order
            .apply_transition(LabTransition::ReceiveAtLab, &actor)
            .expect("receive");
        order
            .apply_transition(LabTransition::BeginProcessing, &actor)
            .expect("begin");
        order
            .apply_transition(LabTransition::Cancel, &actor)
            .expect("cancel from in_process");
        assert_eq!(order.status, LabStatus::Cancelled);
    }

    #[test]
    fn test_lab_cannot_cancel_after_resulted() {
        let actor = test_actor();
        let mut order = new_lab_order(actor.facility_id, new_id(), actor.user_id);

        order
            .apply_transition(LabTransition::RequestSample, &actor)
            .expect("req");
        order
            .apply_transition(LabTransition::CollectSample, &actor)
            .expect("collect");
        order
            .apply_transition(LabTransition::ReceiveAtLab, &actor)
            .expect("receive");
        order
            .apply_transition(LabTransition::BeginProcessing, &actor)
            .expect("begin");
        order
            .apply_transition(LabTransition::EnterResults, &actor)
            .expect("results");

        let result = order.apply_transition(LabTransition::Cancel, &actor);
        assert!(result.is_err());
    }

    #[test]
    fn test_lab_amend_after_verified() {
        let actor = test_actor();
        let mut order = new_lab_order(actor.facility_id, new_id(), actor.user_id);

        // Go through full happy path
        order
            .apply_transition(LabTransition::RequestSample, &actor)
            .expect("req");
        order
            .apply_transition(LabTransition::CollectSample, &actor)
            .expect("collect");
        order
            .apply_transition(LabTransition::ReceiveAtLab, &actor)
            .expect("receive");
        order
            .apply_transition(LabTransition::BeginProcessing, &actor)
            .expect("begin");
        order
            .apply_transition(LabTransition::EnterResults, &actor)
            .expect("results");
        order
            .apply_transition(LabTransition::Verify, &actor)
            .expect("verify");

        // Amend and re-verify
        order
            .apply_transition(LabTransition::Amend, &actor)
            .expect("amend");
        assert_eq!(order.status, LabStatus::Amended);

        order
            .apply_transition(LabTransition::Verify, &actor)
            .expect("re-verify");
        assert_eq!(order.status, LabStatus::Verified);
    }

    #[test]
    fn test_lab_no_transitions_from_cancelled() {
        let actor = test_actor();
        let mut order = new_lab_order(actor.facility_id, new_id(), actor.user_id);

        order
            .apply_transition(LabTransition::Cancel, &actor)
            .expect("cancel");
        assert!(order.allowed_transitions(&actor).is_empty());
    }

    #[test]
    fn test_lab_invalid_transition_rejected() {
        let actor = test_actor();
        let mut order = new_lab_order(actor.facility_id, new_id(), actor.user_id);

        // Cannot verify from ordered
        let result = order.apply_transition(LabTransition::Verify, &actor);
        assert!(result.is_err());

        // Cannot enter results from ordered
        let result = order.apply_transition(LabTransition::EnterResults, &actor);
        assert!(result.is_err());
    }
}
