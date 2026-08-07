//! Proof bundles and verification for StateChronicle.
//!
//! Assembles portable proof bundles (inclusion, state, ownership, authority,
//! snapshot) and implements the deterministic verification algorithm of
//! protocol §29, independent of any transport or persistence layer. The
//! sparse Merkle path checks reuse the accumulator's own path verifier and
//! commit signatures are checked with the core crate's Ed25519 strict
//! verifier over BCS canonical commit bytes.
//!
//! # Architecture
//!
//! - [`bundle`] — assembling builders that produce the domain
//!   [`ResourceStateProof`] envelope (protocol §16.2).
//! - [`verify`] — pure verifiers: [`verify::verify_proof`],
//!   [`verify::verify_inclusion`],
//!   [`verify::verify_commit_signature_with_key`],
//!   [`verify::verify_ownership`], and [`verify::verify_bundle`].
//! - [`service`] — [`service::ProofService`] / [`service::ProofPorts`], the
//!   async composition layer over the proof/state/commit/snapshot ports.
//!
//! [`ResourceStateProof`]: statechronicle_domain::proof::ResourceStateProof

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

/// Proof bundle assembly (protocol §16.2).
pub mod bundle;

/// Inclusion proofs.
pub mod inclusion;

/// State proofs.
pub mod state;

/// Ownership proofs.
pub mod ownership;

/// Verification algorithm (protocol §29).
pub mod verify;

/// Proof assembly and verification error type.
pub mod error;

/// Async proof service over the proof lane ports.
pub mod service;
