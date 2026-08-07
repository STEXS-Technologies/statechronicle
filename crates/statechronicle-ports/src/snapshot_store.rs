//! Port trait for the snapshot store (protocol §15).
//!
//! Holds optional compact checkpoints of resource state. The domain crate has
//! no snapshot struct yet, so the port exchanges opaque payload bytes; a
//! richer snapshot type belongs to a future domain lane (§27 logical stores).

use async_trait::async_trait;
use statechronicle_domain::ids::SnapshotId;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;

/// Errors produced by the snapshot store port.
#[derive(Debug, Error)]
pub enum SnapshotStoreError {
    /// Stored snapshot bytes failed an integrity check.
    #[error("stored snapshot is corrupt: {0}")]
    Corrupt(String),
    /// The backing store could not be reached or resolved.
    #[error("snapshot store unavailable: {0}")]
    Unavailable(String),
}

/// Backend-agnostic snapshot store port (no implementations in this crate).
///
/// The production adapter lives in the consuming platform's composition root;
/// StateChronicle v0 uses an in-memory fake. Async via `#[async_trait]`
/// (boxed futures keep the port dyn-compatible).
///
/// `#[async_trait]` is used rather than `trait_variant::make(Send)`: the latter
/// desugars `async fn` to `-> impl Future + Send`, which is not object-safe, so
/// adapters could not be held behind `&dyn SnapshotStore`.
#[async_trait]
pub trait SnapshotStore: Sync + Send {
    /// Stores a snapshot checkpoint as opaque payload bytes.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotStoreError::Corrupt`] when the stored bytes fail an
    /// integrity check, and [`SnapshotStoreError::Unavailable`] when the
    /// backing store cannot be reached.
    async fn put_snapshot(
        &self,
        tenant: &TenantId,
        snapshot_id: &SnapshotId,
        payload: Vec<u8>,
    ) -> Result<(), SnapshotStoreError>;

    /// Fetches a snapshot checkpoint by id within a tenant scope.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotStoreError::Corrupt`] when the stored bytes fail an
    /// integrity check, and [`SnapshotStoreError::Unavailable`] when the
    /// backing store cannot be reached.
    async fn get_snapshot(
        &self,
        tenant: &TenantId,
        snapshot_id: &SnapshotId,
    ) -> Result<Option<Vec<u8>>, SnapshotStoreError>;
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            SnapshotStoreError::Corrupt(String::from("checksum")).to_string(),
            "stored snapshot is corrupt: checksum"
        );
        assert_eq!(
            SnapshotStoreError::Unavailable(String::from("db down")).to_string(),
            "snapshot store unavailable: db down"
        );
    }
}
