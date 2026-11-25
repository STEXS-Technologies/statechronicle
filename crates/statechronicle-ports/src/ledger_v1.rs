//! Optional versioned mutation retention on the authoritative ledger.

use async_trait::async_trait;
use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::{
    commit::Commit, event::Event, ids::IntentId, state::StateProjection, tenant::TenantId,
};

use crate::ledger_store::{LedgerStore, LedgerStoreError, OutboxRecord};

/// Original v1 mutation evidence retained in the same transaction as its effects.
///
/// Readback values remain untrusted until the commit crate verifies the
/// envelope bindings, signatures, canonical history, and complete effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerMutationV1 {
    /// Canonical signed actor-intent envelope as received.
    pub intent_record: Vec<u8>,
    /// Digest of the versioned canonical intent body.
    pub payload_digest: ContentDigest,
    /// Canonical signed commit envelope as appended to the chain.
    pub commit_record: Vec<u8>,
    /// Complete original ordered events.
    pub events: Vec<Event>,
    /// Complete original projection writes.
    pub projections: Vec<StateProjection>,
    /// Complete original outbox writes, including any delivered rows.
    pub outbox: Vec<OutboxRecord>,
}

/// Commit and original intent evidence to retain with one fenced transaction.
pub struct RetainMutationV1<'record> {
    /// Validated commit body used to advance the canonical scope head.
    pub commit: &'record Commit,
    /// Intent identity used for the already reserved idempotency claim.
    pub intent_id: &'record IntentId,
    /// Original signed actor-intent envelope.
    pub intent_record: &'record [u8],
    /// Original signed commit envelope.
    pub commit_record: &'record [u8],
}

/// Authoritative single-identity lookup across a shared format namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerLookupV1 {
    /// No matching row at this read's linearization point.
    NotFound,
    /// An unfinished reservation for this intent identity.
    InProgress {
        /// Digest bound to the reservation.
        payload_digest: ContentDigest,
        /// Adapter attempt identity.
        attempt_id: String,
        /// Lease expiry alone does not establish rollback.
        lease_expires_at_unix: i64,
    },
    /// Finalized evidence in the v1 wire format.
    Committed(Box<LedgerMutationV1>),
    /// The row exists in a different format or belongs to a retained group.
    OtherRecord,
}

/// Extends the authoritative store with immutable exact-byte v1 recovery.
///
/// The lookup observes the same transaction boundary as the write and the
/// same tenant/intent namespace as legacy claims. Reads are linearizable and
/// expose complete original effects after finalization only. Another format
/// or a group member cannot be presented as an absent or standalone v1 record.
#[async_trait]
pub trait LedgerRecoveryV1: LedgerStore {
    /// Returns original evidence without starting a reservation.
    async fn lookup_mutation_v1(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
    ) -> Result<LedgerLookupV1, LedgerStoreError>;
}
