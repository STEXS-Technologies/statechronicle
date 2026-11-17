//! Optional immutable recovery evidence for one atomic, single-tenant batch.
//!
//! This extends the same ledger transaction boundary; it is not a second
//! datastore or a coordinator of independently committed single mutations.

use async_trait::async_trait;
use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::{
    commit::Commit, event::Event, ids::IntentId, intent::Intent, signed::Signed,
    state::StateProjection, tenant::TenantId,
};

use crate::ledger_store::{LedgerRecovery, LedgerStoreError, OutboxRecord};

/// Complete original batch metadata and effects, retained immutably in the
/// transaction that finalizes every member's idempotency claim.
/// These adapter values are untrusted until verified by the commit boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerBatch {
    /// Exact ordered original intent bodies. Order is part of batch identity.
    pub intents: Vec<Intent>,
    /// One signed tenant commit covering every event of every member.
    pub commit: Signed<Commit>,
    /// Complete event set in signed commit order.
    pub events: Vec<Event>,
    /// Original deterministic projections, retained after later updates.
    pub projections: Vec<StateProjection>,
    /// Complete original outbox rows, retained after delivery.
    pub outbox: Vec<OutboxRecord>,
}

/// One linearizable lookup by a member's tenant and intent id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerBatchLookup {
    /// No visible member at this read's linearization point. An outstanding
    /// transaction may still commit; absence does not authorize a new identity.
    NotFound,
    /// A member reservation, never evidence that the complete batch committed.
    InProgress {
        /// Canonical payload digest of the queried member, not the group.
        payload_digest: ContentDigest,
        /// Active fenced adapter attempt.
        attempt_id: String,
        /// Lease timestamp; expiry alone does not prove rollback.
        lease_expires_at_unix: i64,
    },
    /// Complete original group containing this member, never a synthesized
    /// union of independently committed mutations.
    Committed(Box<LedgerBatch>),
}

/// Recovery capability for the SAME store as [`LedgerRecovery`]. All members
/// must resolve to the same immutable complete record. Implementations must
/// not reinterpret a standalone committed mutation as a singleton batch or
/// combine unrelated receipts. A finalized single mutation queried here must
/// return an invariant error, never `NotFound`.
///
/// Batch member claims share the ordinary tenant/intent idempotency namespace.
/// Conversely, `lookup_mutation` must refuse a batch member when it cannot
/// represent the complete original group; it must not manufacture a single
/// receipt by filtering that group's effects. Atomicity, history membership,
/// linearizability and physical durability still require adapter qualification.
#[async_trait]
pub trait LedgerBatchRecovery: LedgerRecovery {
    /// Resolve the complete original group containing this exact member.
    async fn lookup_batch(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
    ) -> Result<LedgerBatchLookup, LedgerStoreError>;
}
