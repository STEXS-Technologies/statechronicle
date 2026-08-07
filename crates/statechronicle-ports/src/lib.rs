//! Backend-agnostic port traits for StateChronicle.
//!
//! Following the trustgrant-ports convention, this crate declares port traits
//! only. There are no implementations inside. Driven adapters implement these
//! traits and are wired at the consumer's composition root.

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

/// Port trait for the intent store (deduplication and idempotency).
pub mod intent_store;

/// Port trait for the append-only event store.
pub mod event_store;

/// Port trait for the commit store.
pub mod commit_store;

/// Port trait for the current-state index.
pub mod state_index;

/// Port trait for the proof index.
pub mod proof_index;

/// Port trait for the snapshot store.
pub mod snapshot_store;

/// Port trait for tenant scope resolution.
pub mod tenant_store;

/// Port trait for TrustGrant authority evaluation.
pub mod trustgrant_evaluator;

/// Port trait for atomic multi-store transactions.
pub mod transaction_manager;

/// Port trait for event publication.
pub mod event_publisher;
