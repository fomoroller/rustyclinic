//! Form engine for RustyClinic.
//!
//! Provides configurable clinical forms with JSON-based expression trees
//! for skip logic, computed fields, and validation rules.
//! The engine compiles expressions to flat instruction arrays and evaluates
//! them on a pre-allocated stack for minimal runtime allocation.

pub mod compiler;
pub mod dag;
pub mod definition;
pub mod engine;
pub mod evaluator;
pub mod expression;

#[cfg(test)]
mod tests;
