//! Admission / transfer / discharge aggregate with state machine.
//!
//! ```text
//! ADMISSION STATE MACHINE:
//!
//!   planned → admitted → transferred → discharged
//!                │                        ▲
//!                └────────────────────────-┘
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

use rustyclinic_core::error::AppResult;
use rustyclinic_core::state_machine::StateMachine;
use rustyclinic_core::types::ActorContext;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdmissionStatus {
    Planned,
    Admitted,
    Transferred,
    Discharged,
}

impl fmt::Display for AdmissionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Planned => write!(f, "planned"),
            Self::Admitted => write!(f, "admitted"),
            Self::Transferred => write!(f, "transferred"),
            Self::Discharged => write!(f, "discharged"),
        }
    }
}

impl AdmissionStatus {
    pub fn from_str_safe(s: &str) -> Self {
        match s {
            "planned" => Self::Planned,
            "admitted" => Self::Admitted,
            "transferred" => Self::Transferred,
            "discharged" => Self::Discharged,
            _ => Self::Planned,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Discharged)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AdmissionTransition {
    Admit,
    Transfer,
    Discharge,
}

impl fmt::Display for AdmissionTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admit => write!(f, "admit"),
            Self::Transfer => write!(f, "transfer"),
            Self::Discharge => write!(f, "discharge"),
        }
    }
}

/// An admission record with state machine lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Admission {
    pub id: Uuid,
    pub encounter_id: Uuid,
    pub patient_id: Uuid,
    pub facility_id: Uuid,
    pub status: AdmissionStatus,
    pub ward: String,
    pub bed: Option<String>,
    pub admitted_by: Uuid,
    pub admitted_at: Option<DateTime<Utc>>,
    pub transferred_to_ward: Option<String>,
    pub transferred_at: Option<DateTime<Utc>>,
    pub discharged_at: Option<DateTime<Utc>>,
    pub discharged_by: Option<Uuid>,
    pub discharge_reason: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub version: u32,
}

impl StateMachine for Admission {
    type State = AdmissionStatus;
    type Transition = AdmissionTransition;

    fn current_state(&self) -> &AdmissionStatus {
        &self.status
    }

    fn allowed_transitions(&self, _actor: &ActorContext) -> Vec<AdmissionTransition> {
        match &self.status {
            AdmissionStatus::Planned => vec![AdmissionTransition::Admit],
            AdmissionStatus::Admitted => vec![
                AdmissionTransition::Transfer,
                AdmissionTransition::Discharge,
            ],
            AdmissionStatus::Transferred => vec![
                AdmissionTransition::Transfer,
                AdmissionTransition::Discharge,
            ],
            AdmissionStatus::Discharged => vec![],
        }
    }

    fn apply_transition(
        &mut self,
        transition: AdmissionTransition,
        actor: &ActorContext,
    ) -> AppResult<()> {
        self.validate_transition(&transition, actor)?;

        let now = Utc::now();
        match transition {
            AdmissionTransition::Admit => {
                self.status = AdmissionStatus::Admitted;
                self.admitted_at = Some(now);
            }
            AdmissionTransition::Transfer => {
                self.status = AdmissionStatus::Transferred;
                self.transferred_at = Some(now);
            }
            AdmissionTransition::Discharge => {
                self.status = AdmissionStatus::Discharged;
                self.discharged_at = Some(now);
                self.discharged_by = Some(actor.user_id);
            }
        }
        self.version += 1;
        Ok(())
    }
}

/// Repository trait for admission persistence.
pub trait AdmissionRepo {
    fn create(&self, admission: &Admission) -> AppResult<()>;
    fn find_by_id(&self, id: Uuid) -> AppResult<Option<Admission>>;
    fn find_by_patient(&self, patient_id: Uuid) -> AppResult<Vec<Admission>>;
    fn find_by_encounter(&self, encounter_id: Uuid) -> AppResult<Vec<Admission>>;
    fn find_active_by_facility(&self, facility_id: Uuid) -> AppResult<Vec<Admission>>;
    fn update(&self, admission: &Admission) -> AppResult<()>;
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

    fn new_admission(facility_id: Uuid, patient_id: Uuid, admitted_by: Uuid) -> Admission {
        let now = Utc::now();
        Admission {
            id: new_id(),
            encounter_id: new_id(),
            patient_id,
            facility_id,
            status: AdmissionStatus::Planned,
            ward: "General".to_string(),
            bed: Some("A-1".to_string()),
            admitted_by,
            admitted_at: None,
            transferred_to_ward: None,
            transferred_at: None,
            discharged_at: None,
            discharged_by: None,
            discharge_reason: None,
            notes: None,
            created_at: now,
            version: 0,
        }
    }

    #[test]
    fn test_admission_happy_path() {
        let actor = test_actor();
        let mut admission = new_admission(actor.facility_id, new_id(), actor.user_id);

        // planned → admitted → transferred → discharged
        admission
            .apply_transition(AdmissionTransition::Admit, &actor)
            .expect("admit");
        assert_eq!(admission.status, AdmissionStatus::Admitted);
        assert!(admission.admitted_at.is_some());

        admission
            .apply_transition(AdmissionTransition::Transfer, &actor)
            .expect("transfer");
        assert_eq!(admission.status, AdmissionStatus::Transferred);
        assert!(admission.transferred_at.is_some());

        admission
            .apply_transition(AdmissionTransition::Discharge, &actor)
            .expect("discharge");
        assert_eq!(admission.status, AdmissionStatus::Discharged);
        assert!(admission.discharged_at.is_some());
        assert_eq!(admission.discharged_by, Some(actor.user_id));
        assert_eq!(admission.version, 3);
    }

    #[test]
    fn test_admission_direct_discharge() {
        let actor = test_actor();
        let mut admission = new_admission(actor.facility_id, new_id(), actor.user_id);

        admission
            .apply_transition(AdmissionTransition::Admit, &actor)
            .expect("admit");
        admission
            .apply_transition(AdmissionTransition::Discharge, &actor)
            .expect("discharge");
        assert_eq!(admission.status, AdmissionStatus::Discharged);
    }

    #[test]
    fn test_admission_multiple_transfers() {
        let actor = test_actor();
        let mut admission = new_admission(actor.facility_id, new_id(), actor.user_id);

        admission
            .apply_transition(AdmissionTransition::Admit, &actor)
            .expect("admit");
        admission
            .apply_transition(AdmissionTransition::Transfer, &actor)
            .expect("transfer 1");
        admission
            .apply_transition(AdmissionTransition::Transfer, &actor)
            .expect("transfer 2");
        admission
            .apply_transition(AdmissionTransition::Discharge, &actor)
            .expect("discharge");
        assert_eq!(admission.status, AdmissionStatus::Discharged);
    }

    #[test]
    fn test_admission_no_transitions_from_discharged() {
        let actor = test_actor();
        let mut admission = new_admission(actor.facility_id, new_id(), actor.user_id);

        admission
            .apply_transition(AdmissionTransition::Admit, &actor)
            .expect("admit");
        admission
            .apply_transition(AdmissionTransition::Discharge, &actor)
            .expect("discharge");
        assert!(admission.allowed_transitions(&actor).is_empty());
    }

    #[test]
    fn test_admission_cannot_transfer_from_planned() {
        let actor = test_actor();
        let mut admission = new_admission(actor.facility_id, new_id(), actor.user_id);

        let result = admission.apply_transition(AdmissionTransition::Transfer, &actor);
        assert!(result.is_err());
    }

    #[test]
    fn test_admission_cannot_discharge_from_planned() {
        let actor = test_actor();
        let mut admission = new_admission(actor.facility_id, new_id(), actor.user_id);

        let result = admission.apply_transition(AdmissionTransition::Discharge, &actor);
        assert!(result.is_err());
    }
}
