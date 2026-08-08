//! Core domain types for StateChronicle.
//!
//! This crate holds the canonical protocol objects: tenant and resource
//! identity, subjects, state types, intents, events, commits, proofs, and
//! state projections. It is infrastructure-agnostic and contains no
//! transport or persistence logic.

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

/// Tenant identity and isolation scope.
pub mod tenant;

/// Resource identity.
pub mod resource;

/// Subject identity for actors and services.
pub mod subject;

/// Resource state types.
pub mod state_type;

/// Intents: requested state transitions.
pub mod intent;

/// Events: validated, append-only transitions.
pub mod event;

/// Commits: signed batches of events.
pub mod commit;

/// Proofs of state and inclusion.
pub mod proof;

/// Authority proofs and TrustGrant evaluation outcomes.
pub mod authority;

/// ADR-004 signed envelope for intents, commits, and snapshots.
pub mod signed;

/// State projections over event history.
pub mod state;

/// Prefixed newtype identifiers (`stc_`, `int_`, `evt_`, `cmt_`, `snp_`).
pub mod ids;

/// Profile status names (validated, registry-open).
pub mod status;

/// Domain error type.
pub mod error;
