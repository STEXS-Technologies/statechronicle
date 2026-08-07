//! Port trait for the commit store (protocol §13).
//!
//! Persists signed commit batches append-only, keyed by tenant scope, and
//! supports lookups by commit id and by monotonic sequence number. Duplicate
//! commit ids are rejected fail-closed (§27 logical stores).

use async_trait::async_trait;
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
