//! Billing domain: coverage, eligibility, tariffs, claims, payments, waivers.
//!
//! This crate defines the domain types and repository traits for the billing
//! and payer workflow. The `ClaimCase` aggregate implements `StateMachine`
//! for the claim lifecycle:
//!
//! ```text
//! draft → validated → batched → submitted → acknowledged → adjudicated → paid
//!                                    │             │              │
//!                                    └─────────────┴──────────────┴──▶ rejected → reopened → validated
//!
//! void from any non-terminal state
//! ```

pub mod claims;
pub mod coverage;
pub mod payment;
pub mod tariff;
