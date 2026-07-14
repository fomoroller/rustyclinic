//! Shared state machine framework.
//!
//! All state-machine-driven workflows (queue, claims, lab, pharmacy, etc.)
//! implement this trait. Transition validation, permission checks, and audit
//! logging are handled by the framework.

use crate::error::{AppError, AppResult};
use crate::types::ActorContext;
use std::fmt;

/// A state machine for a domain workflow.
///
/// Each workflow declares its states, transitions, and permission rules
/// by implementing this trait.
pub trait StateMachine: Sized {
    /// The state type (an enum of possible states).
    type State: Clone + PartialEq + fmt::Display;

    /// The transition type (an enum of possible actions).
    type Transition: Clone + fmt::Display;

    /// Return the current state.
    fn current_state(&self) -> &Self::State;

    /// Return which transitions are valid from the current state
    /// for the given actor.
    fn allowed_transitions(&self, actor: &ActorContext) -> Vec<Self::Transition>;

    /// Attempt to apply a transition. Returns error if the transition
    /// is not valid from the current state or the actor lacks permission.
    fn apply_transition(
        &mut self,
        transition: Self::Transition,
        actor: &ActorContext,
    ) -> AppResult<()>;

    /// Validate that a transition is allowed without applying it.
    fn validate_transition(
        &self,
        transition: &Self::Transition,
        actor: &ActorContext,
    ) -> AppResult<()> {
        let allowed = self.allowed_transitions(actor);
        if allowed
            .iter()
            .any(|t| format!("{t}") == format!("{transition}"))
        {
            Ok(())
        } else {
            Err(AppError::InvalidTransition {
                from: self.current_state().to_string(),
                to: transition.to_string(),
            })
        }
    }
}
