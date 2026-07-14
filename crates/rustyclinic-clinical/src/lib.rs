//! Clinical domain: encounters, observations, queue, and state machines.

pub mod admission;
pub mod lab;
pub mod pharmacy;
pub mod program;
pub mod queue;
pub mod referral;

use serde::{Deserialize, Serialize};
use std::fmt;

/// Shared priority enum used across clinical orders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Routine,
    Urgent,
    Stat,
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Routine => write!(f, "routine"),
            Self::Urgent => write!(f, "urgent"),
            Self::Stat => write!(f, "stat"),
        }
    }
}

impl Priority {
    pub fn from_str_safe(s: &str) -> Self {
        match s {
            "routine" => Self::Routine,
            "urgent" => Self::Urgent,
            "stat" => Self::Stat,
            _ => Self::Routine,
        }
    }
}
