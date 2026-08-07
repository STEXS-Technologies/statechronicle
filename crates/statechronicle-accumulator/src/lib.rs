//! State-root accumulator for StateChronicle (ADR-005).
//!
//! Maintains the current state root over a fixed-256-bit-depth sparse Merkle
//! tree baseline, one per tenant, enabling compact inclusion and
//! non-membership proofs for state (§16.2). Node encoding:
//!
//! ```text
//! internal(h)  = H(0x10 || left(h-1) || right(h-1))
//! leaf(key)    = H(0x11 || key || state_digest)
//! EMPTY_LEAF   = H(0x11 || [0;32] || [0;32])
//! default[h]   = H(0x10 || default[h-1] || default[h-1]),  default[0] = EMPTY_LEAF
//! ```
//!
//! Logical-isolation composition over sorted `(TenantId, StateRoot)` pairs is
//! provided by [`checkpoint::CheckpointRoot`] (leaves `H(0x12 || ...)`,
//! internals `H(0x13 || ...)`, odd-duplication).
//!
//! The crate stays pure and in-memory; persistence is the ports crate's job.

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

/// Sparse Merkle tree state root and key model.
pub mod sparse_merkle;

/// Domain-separated state key derivation.
pub mod key;

/// Level-tagged inclusion and non-membership proofs.
pub mod proof;

/// Logical-isolation checkpoint root over tenant roots.
pub mod checkpoint;

/// Accumulator error type.
pub mod error;
