//! Commit formation for StateChronicle.
//!
//! Groups ordered events into durable signed commits: deterministic ordering,
//! event/state roots, Ed25519 commit signatures, tenant checkpoint commits,
//! and fork/failure semantics.

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

/// Commit formation.
pub mod batch;

/// Deterministic ordering.
pub mod ordering;

/// Event Merkle root and state root computation.
pub mod roots;

/// Commit body assembly.
pub mod builder;

/// Ed25519 commit signing.
pub mod sign;
/// Versioned tenant commit and global checkpoint signing.
pub mod sign_v1;

/// Commit persistence.
pub mod persist;

/// Verified durable outcomes and authoritative recovery.
pub mod durable;
/// Exact-byte durable write and recovery for versioned mutations.
pub mod durable_v1;
mod durable_write_v1;

/// Verified atomic batch outcomes and complete original-group recovery.
pub mod durable_batch;

/// Tenant checkpoint commits.
pub mod checkpoint;

/// Fork and failure semantics (protocol §31).
pub mod fork;

/// Commit formation error type.
pub mod error;
