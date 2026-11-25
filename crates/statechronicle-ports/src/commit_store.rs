//! Port trait for the commit store (protocol §13).
//!
//! Persists signed commit batches append-only, keyed by tenant scope, and
//! supports lookups by commit id and by monotonic sequence number. Duplicate
//! commit ids are rejected fail-closed (§27 logical stores).

use async_trait::async_trait;
use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::commit::Commit;
use statechronicle_domain::ids::CommitId;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;

/// Errors produced by the commit store port.
#[derive(Debug, Error)]
pub enum CommitStoreError {
    /// A commit with the same id is already stored.
    #[error("commit id already stored")]
    Duplicate,
    /// The backing store could not be reached or resolved.
    #[error("commit store unavailable: {0}")]
    Unavailable(String),
}

/// Canonical head metadata for a tenant/scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalHead {
    /// Current canonical commit identifier.
    pub commit_id: CommitId,
    /// Current canonical sequence.
    pub sequence: u64,
    /// State root declared by the canonical commit.
    pub state_root: ContentDigest,
}

/// Backend-agnostic append-only commit store port (no implementations in this
/// crate).
///
/// The production adapter lives in the consuming platform's composition root;
/// StateChronicle v0 uses an in-memory fake. Async via `#[async_trait]`
/// (boxed futures keep the port dyn-compatible).
///
/// `#[async_trait]` is used rather than `trait_variant::make(Send)`: the latter
/// desugars `async fn` to `-> impl Future + Send`, which is not object-safe, so
/// adapters could not be held behind `&dyn CommitStore`.
#[async_trait]
pub trait CommitStore: Sync + Send {
    /// Stores a signed commit append-only within a tenant scope.
    ///
    /// # Errors
    ///
    /// Returns [`CommitStoreError::Duplicate`] when a commit with the same id
    /// is already stored, and [`CommitStoreError::Unavailable`] when the
    /// backing store cannot be reached.
    async fn put_commit(
        &self,
        tenant: &TenantId,
        commit: &Signed<Commit>,
    ) -> Result<(), CommitStoreError>;

    /// Fetches a signed commit by id within a tenant scope.
    ///
    /// # Errors
    ///
    /// Returns [`CommitStoreError::Unavailable`] when the backing store cannot
    /// be reached.
    async fn commit_by_id(
        &self,
        tenant: &TenantId,
        commit_id: &CommitId,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError>;

    /// Fetches a signed commit by its monotonic sequence number.
    ///
    /// # Errors
    ///
    /// Returns [`CommitStoreError::Unavailable`] when the backing store cannot
    /// be reached.
    async fn commit_by_sequence(
        &self,
        tenant: &TenantId,
        sequence: u64,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError>;

    /// Returns the canonical head when the adapter tracks one. Adapters that
    /// expose only append-only commit lookup may return `Ok(None)`; production
    /// proof verification should require a non-`None` head implementation.
    ///
    /// Current-claim verification requires a linearizable read of the complete
    /// `(commit_id, sequence, state_root)` tuple within this tenant scope. The
    /// head must advance atomically with its immutable commit and canonical
    /// ledger effects. A stale cache or independently read tuple fields cannot
    /// implement that contract. A verifier's result is current at this read,
    /// pinned to that exact head; it cannot promise freshness after the read.
    async fn canonical_head(
        &self,
        _tenant: &TenantId,
    ) -> Result<Option<CanonicalHead>, CommitStoreError> {
        Ok(None)
    }
}

/// An object-safe commit read, returning the adapter's existing future directly.
/// This alias does not add another async wrapper or require an async-trait macro.
pub type CommitReadFuture<'read, T> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<T, CommitStoreError>> + Send + 'read>,
>;

/// Minimum read authority for verifying supplied historical or current claims.
///
/// Reads must be tenant-scoped and return immutable signed commits. A current
/// claim additionally requires the same linearizable, atomically advanced head
/// tuple described by [`CommitStore::canonical_head`]. Reads/callbacks must not
/// persist data or dispatch effects. This trait exposes no write or sequence API.
/// Existing [`CommitStore`] adapters implement it automatically, forwarding the
/// original boxed future without allocating another async state machine.
pub trait CommitReader: Send + Sync {
    /// Load an immutable signed commit under the independently expected tenant.
    fn read_commit<'read>(
        &'read self,
        tenant: &'read TenantId,
        commit: &'read CommitId,
    ) -> CommitReadFuture<'read, Option<Signed<Commit>>>;

    /// Linearizable current head, or `None` when this reader is history-only.
    /// A missing head always refuses current verification.
    fn read_canonical_head<'read>(
        &'read self,
        _tenant: &'read TenantId,
    ) -> CommitReadFuture<'read, Option<CanonicalHead>> {
        Box::pin(async { Ok(None) })
    }
}

/// Read-only access to exact v1 signed commit envelope bytes and a canonical head.
/// Implementations must scope both reads by the independently supplied tenant.
pub trait CommitReaderV1: Send + Sync {
    /// Load the exact immutable envelope bytes stored for this tenant and commit.
    fn read_commit_record_v1<'read>(
        &'read self,
        tenant: &'read TenantId,
        commit: &'read CommitId,
    ) -> CommitReadFuture<'read, Option<Vec<u8>>>;

    /// Linearizable canonical head tuple for current verification.
    fn read_canonical_head_v1<'read>(
        &'read self,
        _tenant: &'read TenantId,
    ) -> CommitReadFuture<'read, Option<CanonicalHead>> {
        Box::pin(async { Ok(None) })
    }
}

impl<T: CommitStore + ?Sized> CommitReader for T {
    fn read_commit<'read>(
        &'read self,
        tenant: &'read TenantId,
        commit: &'read CommitId,
    ) -> CommitReadFuture<'read, Option<Signed<Commit>>> {
        self.commit_by_id(tenant, commit)
    }

    fn read_canonical_head<'read>(
        &'read self,
        tenant: &'read TenantId,
    ) -> CommitReadFuture<'read, Option<CanonicalHead>> {
        self.canonical_head(tenant)
    }
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            CommitStoreError::Duplicate.to_string(),
            "commit id already stored"
        );
        assert_eq!(
            CommitStoreError::Unavailable(String::from("db down")).to_string(),
            "commit store unavailable: db down"
        );
    }
}
