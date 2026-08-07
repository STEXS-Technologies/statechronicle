//! The execution pipeline for StateChronicle.
//!
//! Runs validated intents through the protocol's ordered checks (§18.1),
//! deterministic transition rules, fail-closed conflict checks (§18.2), and
//! multi-resource atomicity (§18.3), emitting events only when every check
//! passes.

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

/// The validation pipeline (protocol §18.1).
pub mod pipeline;

/// Deterministic after-state rules.
pub mod transition;

/// Fail-closed conflict rules (protocol §18.2).
pub mod conflict;

/// Multi-resource atomic transactions (protocol §18.3).
pub mod atomicity;

/// Execution pipeline error type.
pub mod error;
