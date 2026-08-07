//! Pure protocol primitives for StateChronicle.
//!
//! This crate is the transport- and persistence-free foundation of the
//! protocol: BCS canonical serialization (ADR-004), SHA-256 digests, Ed25519
//! signatures, size/safety limits, and the shared protocol error type.

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

/// BCS canonical serialization of protocol objects (ADR-004).
pub mod canonicalize;

/// Exact fixed-point amounts for economic arithmetic (ADR-004, no floats).
pub mod amount;

/// SHA-256 digest computation over canonicalized content.
pub mod digest;

/// Ed25519 signatures for commits and proof bundles.
pub mod signature;

/// Size and safety bounds used across the protocol.
pub mod limits;

/// The shared protocol error type and conversion rules.
pub mod error;
