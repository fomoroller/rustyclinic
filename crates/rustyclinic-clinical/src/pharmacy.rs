//! Medication dispense aggregate with state machine.
//!
//! ```text
//! MEDICATION DISPENSE STATE MACHINE:
//!
//!   draft → prepared → dispensed
//!                │
//!                ├──▶ partial (can transition back to prepared)
//!                └──▶ returned
//!
//!   draft or prepared ──▶ voided
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
pub enum DispenseStatus {
    Draft,
    Prepared,
    Dispensed,
    Partial,
    Returned,
    Voided,
}

impl fmt::Display for DispenseStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Prepared => write!(f, "prepared"),
            Self::Dispensed => write!(f, "dispensed"),
            Self::Partial => write!(f, "partial"),
            Self::Returned => write!(f, "returned"),
            Self::Voided => write!(f, "voided"),
        }
    }
}

impl DispenseStatus {
    /// Parse from string (database round-trip).
    pub fn from_str_safe(s: &str) -> Self {
        match s {
            "draft" => Self::Draft,
            "prepared" => Self::Prepared,
            "dispensed" => Self::Dispensed,
            "partial" => Self::Partial,
            "returned" => Self::Returned,
            "voided" => Self::Voided,
            _ => Self::Draft,
        }
    }

    /// Whether this is a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Dispensed | Self::Returned | Self::Voided)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DispenseTransition {
    Prepare,
    Dispense,
    PartialDispense,
    Return,
    Void,
}

impl fmt::Display for DispenseTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prepare => write!(f, "prepare"),
            Self::Dispense => write!(f, "dispense"),
            Self::PartialDispense => write!(f, "partial_dispense"),
            Self::Return => write!(f, "return"),
            Self::Void => write!(f, "void"),
        }
    }
}

/// A medication dispense order with full state machine lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MedicationDispense {
    pub id: Uuid,
    pub encounter_id: Uuid,
    pub patient_id: Uuid,
    pub facility_id: Uuid,
    pub status: DispenseStatus,
    pub priority: Priority,
    pub prescribed_by: Uuid,
    pub dispensed_by: Option<Uuid>,
    pub items: Vec<DispenseItem>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub prepared_at: Option<DateTime<Utc>>,
    pub dispensed_at: Option<DateTime<Utc>>,
    pub version: u32,
}

impl StateMachine for MedicationDispense {
    type State = DispenseStatus;
    type Transition = DispenseTransition;

    fn current_state(&self) -> &DispenseStatus {
        &self.status
    }

    fn allowed_transitions(&self, _actor: &ActorContext) -> Vec<DispenseTransition> {
        match &self.status {
            DispenseStatus::Draft => vec![DispenseTransition::Prepare, DispenseTransition::Void],
            DispenseStatus::Prepared => vec![
                DispenseTransition::Dispense,
                DispenseTransition::PartialDispense,
                DispenseTransition::Void,
            ],
            DispenseStatus::Partial => {
                vec![DispenseTransition::Prepare, DispenseTransition::Dispense]
            }
            DispenseStatus::Dispensed => vec![DispenseTransition::Return],
            _ => vec![],
        }
    }

    fn apply_transition(
        &mut self,
        transition: DispenseTransition,
        actor: &ActorContext,
    ) -> AppResult<()> {
        self.validate_transition(&transition, actor)?;

        let now = Utc::now();
        match transition {
            DispenseTransition::Prepare => {
                self.status = DispenseStatus::Prepared;
                self.prepared_at = Some(now);
            }
            DispenseTransition::Dispense => {
                self.status = DispenseStatus::Dispensed;
                self.dispensed_at = Some(now);
                self.dispensed_by = Some(actor.user_id);
            }
            DispenseTransition::PartialDispense => {
                self.status = DispenseStatus::Partial;
                self.dispensed_by = Some(actor.user_id);
            }
            DispenseTransition::Return => {
                self.status = DispenseStatus::Returned;
            }
            DispenseTransition::Void => {
                self.status = DispenseStatus::Voided;
            }
        }
        self.version += 1;
        Ok(())
    }
}

/// A single medication line item in a dispense.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispenseItem {
    pub medication_name: String,
    pub medication_system: Option<String>,
    pub medication_code: Option<String>,
    pub medication_display: Option<String>,
    pub dosage: String,
    pub frequency: String,
    pub duration: String,
    pub quantity: u32,
    pub dispensed_quantity: Option<u32>,
    pub substituted: bool,
    pub substitution_reason: Option<String>,
}

/// Repository trait for medication dispense persistence.
pub trait MedicationDispenseRepo {
    fn create(&self, dispense: &MedicationDispense) -> AppResult<()>;
    fn find_by_id(&self, id: Uuid) -> AppResult<Option<MedicationDispense>>;
    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<MedicationDispense>>;
    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<MedicationDispense>>;
    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<MedicationDispense>>;
    fn update(&self, dispense: &MedicationDispense) -> AppResult<()>;
}

/// Repository trait for dispense items within a dispense.
pub trait DispenseItemRepo {
    fn create_items(&self, dispense_id: Uuid, items: &[DispenseItem]) -> AppResult<()>;
    fn find_by_dispense(&self, dispense_id: Uuid) -> AppResult<Vec<DispenseItem>>;
    fn update_dispensed(
        &self,
        dispense_id: Uuid,
        medication_name: &str,
        dispensed_quantity: u32,
        substituted: bool,
        substitution_reason: Option<&str>,
    ) -> AppResult<()>;
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
            roles: vec!["pharmacist".to_string()],
            purpose: "clinical_care".to_string(),
            session_id: new_id(),
        }
    }

    fn new_dispense(
        facility_id: Uuid,
        patient_id: Uuid,
        prescribed_by: Uuid,
    ) -> MedicationDispense {
        let now = Utc::now();
        MedicationDispense {
            id: new_id(),
            encounter_id: new_id(),
            patient_id,
            facility_id,
            status: DispenseStatus::Draft,
            priority: Priority::Routine,
            prescribed_by,
            dispensed_by: None,
            items: vec![DispenseItem {
                medication_name: "Amoxicillin".to_string(),
                medication_system: None,
                medication_code: None,
                medication_display: None,
                dosage: "500mg".to_string(),
                frequency: "3x daily".to_string(),
                duration: "7 days".to_string(),
                quantity: 21,
                dispensed_quantity: None,
                substituted: false,
                substitution_reason: None,
            }],
            notes: None,
            created_at: now,
            prepared_at: None,
            dispensed_at: None,
            version: 0,
        }
    }

    #[test]
    fn test_dispense_happy_path() {
        let actor = test_actor();
        let mut dispense = new_dispense(actor.facility_id, new_id(), new_id());

        // draft → prepared → dispensed
        dispense
            .apply_transition(DispenseTransition::Prepare, &actor)
            .expect("prepare");
        assert_eq!(dispense.status, DispenseStatus::Prepared);
        assert!(dispense.prepared_at.is_some());

        dispense
            .apply_transition(DispenseTransition::Dispense, &actor)
            .expect("dispense");
        assert_eq!(dispense.status, DispenseStatus::Dispensed);
        assert!(dispense.dispensed_at.is_some());
        assert_eq!(dispense.dispensed_by, Some(actor.user_id));
        assert_eq!(dispense.version, 2);
    }

    #[test]
    fn test_dispense_partial() {
        let actor = test_actor();
        let mut dispense = new_dispense(actor.facility_id, new_id(), new_id());

        dispense
            .apply_transition(DispenseTransition::Prepare, &actor)
            .expect("prepare");
        dispense
            .apply_transition(DispenseTransition::PartialDispense, &actor)
            .expect("partial");
        assert_eq!(dispense.status, DispenseStatus::Partial);

        // Can prepare again (restock) then fully dispense
        dispense
            .apply_transition(DispenseTransition::Prepare, &actor)
            .expect("re-prepare");
        assert_eq!(dispense.status, DispenseStatus::Prepared);

        dispense
            .apply_transition(DispenseTransition::Dispense, &actor)
            .expect("dispense");
        assert_eq!(dispense.status, DispenseStatus::Dispensed);
    }

    #[test]
    fn test_dispense_return() {
        let actor = test_actor();
        let mut dispense = new_dispense(actor.facility_id, new_id(), new_id());

        dispense
            .apply_transition(DispenseTransition::Prepare, &actor)
            .expect("prepare");
        dispense
            .apply_transition(DispenseTransition::Dispense, &actor)
            .expect("dispense");
        dispense
            .apply_transition(DispenseTransition::Return, &actor)
            .expect("return");
        assert_eq!(dispense.status, DispenseStatus::Returned);
    }

    #[test]
    fn test_dispense_void_from_draft() {
        let actor = test_actor();
        let mut dispense = new_dispense(actor.facility_id, new_id(), new_id());

        dispense
            .apply_transition(DispenseTransition::Void, &actor)
            .expect("void");
        assert_eq!(dispense.status, DispenseStatus::Voided);
    }

    #[test]
    fn test_dispense_void_from_prepared() {
        let actor = test_actor();
        let mut dispense = new_dispense(actor.facility_id, new_id(), new_id());

        dispense
            .apply_transition(DispenseTransition::Prepare, &actor)
            .expect("prepare");
        dispense
            .apply_transition(DispenseTransition::Void, &actor)
            .expect("void");
        assert_eq!(dispense.status, DispenseStatus::Voided);
    }

    #[test]
    fn test_dispense_cannot_void_after_dispensed() {
        let actor = test_actor();
        let mut dispense = new_dispense(actor.facility_id, new_id(), new_id());

        dispense
            .apply_transition(DispenseTransition::Prepare, &actor)
            .expect("prepare");
        dispense
            .apply_transition(DispenseTransition::Dispense, &actor)
            .expect("dispense");

        let result = dispense.apply_transition(DispenseTransition::Void, &actor);
        assert!(result.is_err());
    }

    #[test]
    fn test_dispense_no_transitions_from_voided() {
        let actor = test_actor();
        let mut dispense = new_dispense(actor.facility_id, new_id(), new_id());

        dispense
            .apply_transition(DispenseTransition::Void, &actor)
            .expect("void");
        assert!(dispense.allowed_transitions(&actor).is_empty());
    }

    #[test]
    fn test_dispense_no_transitions_from_returned() {
        let actor = test_actor();
        let mut dispense = new_dispense(actor.facility_id, new_id(), new_id());

        dispense
            .apply_transition(DispenseTransition::Prepare, &actor)
            .expect("prepare");
        dispense
            .apply_transition(DispenseTransition::Dispense, &actor)
            .expect("dispense");
        dispense
            .apply_transition(DispenseTransition::Return, &actor)
            .expect("return");
        assert!(dispense.allowed_transitions(&actor).is_empty());
    }

    #[test]
    fn test_dispense_invalid_transition_rejected() {
        let actor = test_actor();
        let mut dispense = new_dispense(actor.facility_id, new_id(), new_id());

        // Cannot dispense from draft (must prepare first)
        let result = dispense.apply_transition(DispenseTransition::Dispense, &actor);
        assert!(result.is_err());
    }
}
