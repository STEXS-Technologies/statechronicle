//! StateChronicle: a pure-logic, verifiable resource-state ledger protocol
//! engine.
//!
//! This is the **umbrella crate**: the single dependency consumers add to use
//! the whole protocol surface. It re-exports the nine underlying protocol
//! crates under collision-safe namespaces and surfaces the most-used types
//! directly at the top level.
//!
//! ```rust
//! use statechronicle::domain::intent::{Intent, Operation};
//! use statechronicle::core::amount::Amount;
//! use statechronicle::domain::signed::Signed;
//! ```
//!
//! The [`ports`] module holds the ten trait boundaries consumers implement to
//! wire their own storage, authority, and transport backends (intent store,
//! event store, commit store, state index, proof index, snapshot store, tenant
//! store, TrustGrant evaluator, transaction manager, event publisher). Those
//! traits are wired into the engine at the consumer's composition root. This
//! crate ships no storage, HTTP, or authority implementation.

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

/// Sparse-Merkle state accumulator and state roots.
pub use statechronicle_accumulator as accumulator;
/// Commit formation, deterministic ordering, and signing.
pub use statechronicle_commit as commit;
/// Protocol primitives: amounts, digests, signatures, limits, canonicalization.
pub use statechronicle_core as core;
/// Canonical domain objects: tenants, resources, intents, events, commits,
/// proofs, and state projections.
pub use statechronicle_domain as domain;
/// The §18.1 execution pipeline that runs validated intents through the port
/// gates and emits events.
pub use statechronicle_executor as executor;
/// Intent parsing and validation into `ValidatedIntent`.
pub use statechronicle_intent as intent;
/// The ten backend-agnostic port traits consumers implement.
pub use statechronicle_ports as ports;
/// Baseline resource profiles and their rule sets.
pub use statechronicle_profiles as profiles;
/// Proof serving and verification (state, ownership, inclusion, non-membership).
pub use statechronicle_proof as proof;

// ---- Curated top-level facade ----
//
// The most-used protocol types, re-exported directly. Everything here resolves
// to the exact canonical module path in the corresponding crate; consumers can
// use either the flat names or the namespaced forms interchangeably.

/// Exact fixed-point amount. See [`core::amount::Amount`].
pub use statechronicle_core::amount::{Amount, MAX_MANTISSA_DIGITS, MAX_SCALE};
/// SHA-256 content digest in canonical `sha256:<hex>` form. See [`core::digest::ContentDigest`].
pub use statechronicle_core::digest::ContentDigest;
/// An Ed25519 signature over canonicalized content. See [`core::signature::Signature`].
pub use statechronicle_core::signature::Signature;

/// Authority proofs and TrustGrant evaluation outcomes. See [`domain::authority`].
pub use statechronicle_domain::authority::{
    AggregationPolicy, AuthorityProof, EvaluationResult, TrustGrantOutcome,
};
/// A signed batch of events. See [`domain::commit::Commit`].
pub use statechronicle_domain::commit::{Commit, CommitScope, ProfileId, ScopeKind};
/// A validated, append-only transition. See [`domain::event::Event`].
pub use statechronicle_domain::event::{Event, StateCommitment};
/// Prefixed newtype identifiers. See [`domain::ids`].
pub use statechronicle_domain::ids::{CommitId, EventId, IntentId, SnapshotId, StateId};
/// A requested state transition. See [`domain::intent::Intent`].
pub use statechronicle_domain::intent::{
    Intent, KeyId, Nonce, Operation, SignatureAlg, SignatureBlock,
};
/// Identifies a resource within a tenant namespace. See [`domain::resource::ResourceId`].
pub use statechronicle_domain::resource::ResourceId;
/// The ADR-004 signed envelope (`body` + detached `signature`). See [`domain::signed::Signed`].
pub use statechronicle_domain::signed::Signed;
/// A derived projection of a resource's current state. See [`domain::state::StateProjection`].
pub use statechronicle_domain::state::StateProjection;
/// A resource state type. See [`domain::state_type::StateType`].
pub use statechronicle_domain::state_type::StateType;
/// Identifies a user, account, service, or authority. See [`domain::subject::SubjectId`].
pub use statechronicle_domain::subject::SubjectId;
/// Identifies an isolated tenant scope. See [`domain::tenant::TenantId`].
pub use statechronicle_domain::tenant::TenantId;

/// The execution engine. See [`executor::pipeline::Executor`].
pub use statechronicle_executor::pipeline::Executor;
/// The executor's injected port bundle. See [`executor::pipeline::Ports`].
pub use statechronicle_executor::pipeline::Ports;
/// A validated intent with its idempotency key. See [`intent::validated::ValidatedIntent`].
pub use statechronicle_intent::validated::{IdempotencyKey, ValidatedIntent};

/// An opaque SMT state key. See [`accumulator::key::StateKey`].
pub use statechronicle_accumulator::key::StateKey;
/// Sparse-Merkle state accumulator. See [`accumulator::sparse_merkle::StateAccumulator`].
pub use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateRoot, StateUpdate};

/// Baseline profile registry. See [`profiles::registry::ProfileRegistry`].
pub use statechronicle_profiles::registry::ProfileRegistry;
/// The gate every state transition passes through. See [`profiles::registry::ProfileRules`].
pub use statechronicle_profiles::registry::ProfileRules;

/// The proof service's read-side port set. See [`proof::service::ProofPorts`].
pub use statechronicle_proof::service::ProofPorts;
/// Async proof service. See [`proof::service::ProofService`].
pub use statechronicle_proof::service::ProofService;
