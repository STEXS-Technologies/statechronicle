//! Durable ledger transaction boundary.
//!
//! `TransactionManager` is intentionally only a lifecycle primitive.  It is
//! not sufficient for a ledger mutation because it cannot enlist the intent,
//! event, commit, projection, and outbox writes.  [`LedgerStore`](crate::ledger_store::LedgerStore) is the
//! write-side contract used by a production executor: every mutation must use
//! one [`LedgerTransaction`](crate::ledger_store::LedgerTransaction) and commit exactly once.

use async_trait::async_trait;
use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::commit::Commit;
use statechronicle_domain::event::Event;
use statechronicle_domain::ids::{CommitId, IntentId};
use statechronicle_domain::intent::Intent;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;

/// A durable post-commit notification written to an outbox in the same
/// database transaction as the ledger mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRecord {
    /// Stable delivery key.  It must be unique for the commit/event pair.
    pub delivery_key: String,
    /// Tenant whose mutation generated the notification.
    pub tenant: TenantId,
    /// Commit that is safe to publish after transaction commit.
    pub commit_id: CommitId,
    /// Digest of the serialized notification body.
    pub payload_digest: ContentDigest,
    /// Canonical serialized notification body.
    pub payload: Vec<u8>,
}

/// Result of atomically reserving an idempotency key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotencyClaim {
    /// The caller owns a new reservation identified by `attempt_id`.
    NewReservation { attempt_id: String },
    /// The original mutation committed and can be replayed.
    Committed { commit_id: CommitId },
    /// Another attempt currently owns the reservation until its lease expires.
    InProgress {
        attempt_id: String,
        lease_expires_at_unix: i64,
    },
    /// The key exists with a different canonical payload digest.
    ConflictDifferentPayload,
}

/// Errors from the durable ledger boundary.
#[derive(Debug, Error)]
pub enum LedgerStoreError {
    /// The transaction was rejected by an optimistic version/head check.
    #[error("ledger transaction conflict: {0}")]
    Conflict(String),
    /// The idempotency reservation or finalization was invalid.
    #[error("ledger idempotency error: {0}")]
    Idempotency(String),
    /// The backing database or transactional engine is unavailable.
    #[error("ledger store unavailable: {0}")]
    Unavailable(String),
    /// The transaction was already completed or used after completion.
    #[error("ledger transaction is closed")]
    Closed,
    /// The adapter rejected an invariant violation.
    #[error("ledger invariant violation: {0}")]
    Invariant(String),
}

impl LedgerStoreError {
    /// Returns whether retrying the same idempotent request may make progress.
    ///
    /// Callers must still preserve the original idempotency key. Conflicts and
    /// temporary store failures are retryable; closed transactions,
    /// idempotency conflicts, and invariant violations require a new decision
    /// or operator intervention.
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Conflict(_) | Self::Unavailable(_))
    }
}

/// One write transaction that owns all durable effects of a mutation.
///
/// Implementations must execute these methods against the same underlying
/// database transaction.  Calling standalone `IntentStore`, `EventStore`, or
/// `StateIndex` methods from a mutation path defeats the guarantee.
#[async_trait]
pub trait LedgerTransaction: Send + Sync {
    /// Atomically claim or inspect an idempotency key.
    async fn claim_idempotency(
        &mut self,
        tenant: &TenantId,
        intent: &Intent,
        payload_digest: &ContentDigest,
    ) -> Result<IdempotencyClaim, LedgerStoreError>;

    /// Append immutable events after all conflict checks have passed.
    async fn append_events(&mut self, events: &[Event]) -> Result<(), LedgerStoreError>;

    /// Append the signed commit and advance the canonical scope head using
    /// the commit's declared previous-head/root values.
    async fn append_commit(&mut self, commit: &Signed<Commit>) -> Result<(), LedgerStoreError>;

    /// Append a v1 commit and stage the original intent and commit envelopes.
    ///
    /// Implementations retain all finalized effects atomically, require the
    /// existing fenced idempotency claim and canonical head/root checks, and
    /// provide matching [`crate::ledger_v1::LedgerRecoveryV1`] readback.
    /// Legacy adapters refuse this operation and roll back staged effects.
    async fn append_commit_v1(
        &mut self,
        _record: crate::ledger_v1::RetainMutationV1<'_>,
    ) -> Result<(), LedgerStoreError> {
        Err(LedgerStoreError::Invariant(String::from(
            "atomic v1 mutation retention is unsupported",
        )))
    }

    /// Append the v1 commit and stage immutable evidence for the complete group.
    ///
    /// Every ordered member must correspond to a finalized fenced claim in
    /// this transaction. The head, effects, evidence, and claims commit
    /// atomically, and each member resolves to the complete group through
    /// [`crate::ledger_v1::LedgerBatchRecoveryV1`].
    async fn append_batch_commit_v1(
        &mut self,
        _record: crate::ledger_v1::RetainBatchV1<'_>,
    ) -> Result<(), LedgerStoreError> {
        Err(LedgerStoreError::Invariant(String::from(
            "atomic v1 batch retention is unsupported",
        )))
    }

    /// Upsert a deterministic projection while enforcing monotonic version.
    async fn upsert_projection(
        &mut self,
        projection: &StateProjection,
    ) -> Result<(), LedgerStoreError>;

    /// Insert an outbox row that becomes publishable only after commit.
    async fn enqueue_outbox(&mut self, record: &OutboxRecord) -> Result<(), LedgerStoreError>;

    /// Mark the reserved idempotency key as committed in this same transaction.
    async fn finalize_idempotency(
        &mut self,
        tenant: &TenantId,
        intent_id: &IntentId,
        attempt_id: &str,
        commit_id: &CommitId,
    ) -> Result<(), LedgerStoreError>;

    /// Retain a complete original batch in THIS transaction, binding every
    /// member claim to this group and its one signed commit. Commit must fail
    /// unless all matching fenced claims are finalized and the staged events,
    /// commit, projections and outbox exactly match this record. Retention must
    /// remain immutable and accessible by every member after later writes.
    ///
    /// Old adapters remain source-compatible and refuse this optional operation
    /// by default. No effects may escape rollback on that refusal. Implementing
    /// this method requires the `LedgerBatchRecovery` contract on the store;
    /// accepting it without complete atomic metadata is not conformant.
    async fn retain_batch(
        &mut self,
        _batch: &crate::ledger_batch::LedgerBatch,
    ) -> Result<(), LedgerStoreError> {
        Err(LedgerStoreError::Invariant(String::from(
            "atomic batch retention is unsupported",
        )))
    }

    /// Commit every staged effect atomically.
    async fn commit(self: Box<Self>) -> Result<(), LedgerStoreError>;

    /// Roll back every staged effect.  Rollback must be safe to call after any
    /// pre-commit error and must not leave a successful idempotency claim.
    async fn rollback(self: Box<Self>) -> Result<(), LedgerStoreError>;
}

/// Production write-side ledger store.
#[async_trait]
pub trait LedgerStore: Send + Sync {
    /// Begin a transaction for one tenant scope.
    async fn begin(
        &self,
        tenant: &TenantId,
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError>;

    /// Begin one transaction covering all affected tenants.  Implementations
    /// must reject this call unless all scopes share one atomic datastore.
    async fn begin_multi(
        &self,
        tenants: &[TenantId],
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError>;
}

/// Immutable evidence retained with a finalized idempotency row in the same
/// atomic ledger transaction. These are untrusted adapter values until checked
/// by the commit crate's recovery API. Projections and outbox are the ORIGINAL
/// mutation's effects, not today's mutable projection or delivery queue state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerMutation {
    /// Complete original intent, binding identity and canonical payload.
    pub intent: Intent,
    /// Canonical original intent digest stored with its idempotency claim.
    pub payload_digest: ContentDigest,
    /// Original signed commit in this tenant's canonical append-only history.
    pub commit: Signed<Commit>,
    /// Complete ordered event set for this commit.
    pub events: Vec<Event>,
    /// Complete deterministic projections originally applied by the mutation.
    pub projections: Vec<StateProjection>,
    /// Complete notification rows originally enqueued, including delivered rows.
    pub outbox: Vec<OutboxRecord>,
}

/// One consistent authoritative lookup by tenant and original intent id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerLookup {
    /// No row is visible at this read's linearization point. This does not
    /// prove an outstanding transaction cannot subsequently commit.
    NotFound,
    /// An unfinished reservation, never a committed receipt.
    InProgress {
        /// Digest bound to the active reservation.
        payload_digest: ContentDigest,
        /// Adapter attempt identity.
        attempt_id: String,
        /// Reservation lease; expiration alone does not prove rollback.
        lease_expires_at_unix: i64,
    },
    /// Finalized original record from the atomic ledger boundary.
    Committed(Box<LedgerMutation>),
}

/// Read/recovery extension for the SAME authoritative store and transaction
/// boundary as [`LedgerStore`]. Adding this trait leaves old adapters compatible.
///
/// The lookup must be linearizable, tenant-scoped and atomically observe the
/// reservation together with all original mutation effects. It must never
/// manufacture a committed record from an independent intent marker, return a
/// staged/partial write, or accept a commit absent from canonical history.
/// Final records are immutable across later commits, projection rebuilds and
/// outbox delivery. A deployment must retain these effects or reconstruct them
/// from immutable history and atomically finalized metadata, including outbox
/// completeness. Signature/root verification cannot prove adapter atomicity or
/// physical persistence; adapter conformance and database qualification remain
/// required. Bounds on stored/decoded records must match protocol write limits.
#[async_trait]
pub trait LedgerRecovery: LedgerStore {
    /// Resolve original identity without creating a reservation or dispatching
    /// effects. Failure or an incomplete view must never be reported as absent.
    async fn lookup_mutation(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
    ) -> Result<LedgerLookup, LedgerStoreError>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn retry_classification_is_fail_closed() {
        assert!(LedgerStoreError::Conflict(String::new()).is_retryable());
        assert!(LedgerStoreError::Unavailable(String::new()).is_retryable());
        assert!(!LedgerStoreError::Idempotency(String::new()).is_retryable());
        assert!(!LedgerStoreError::Invariant(String::new()).is_retryable());
        assert!(!LedgerStoreError::Closed.is_retryable());
    }
}
