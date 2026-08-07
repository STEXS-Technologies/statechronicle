//! Port traits for atomic multi-store transactions (protocol §18.3).
//!
//! The executor stages writes across the logical stores inside a
//! [`TransactionHandle`], then commits them atomically or rolls back on
//! failure. Handles consume themselves on completion: commit or rollback is
//! called exactly once.
//!
//! Both [`TransactionManager`] and [`TransactionHandle`] use
//! `#[async_trait]`. `trait_variant::make(Send)` was rejected here because its
//! `async fn` desugar to `-> impl Future + Send`, which is not object-safe —
//! the very property `begin` relies on when it returns
//! `Box<dyn TransactionHandle + Send>`. `async_trait` boxes the futures and
//! keeps both traits object-safe.

use async_trait::async_trait;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;

/// Errors produced by the transaction manager port.
#[derive(Debug, Error)]
pub enum TransactionManagerError {
    /// The coordinator aborted the transaction (e.g. concurrency conflict).
    #[error("transaction aborted by coordinator")]
    Aborted,
    /// The commit phase failed.
    #[error("transaction commit failed: {0}")]
    CommitFailed(String),
    /// The rollback phase failed.
    #[error("transaction rollback failed: {0}")]
    RollbackFailed(String),
    /// The coordinator could not be reached or resolved.
    #[error("transaction coordinator unavailable: {0}")]
    Unavailable(String),
}

/// Backend-agnostic transaction manager port (no implementations in this
/// crate).
///
/// The production adapter lives in the consuming platform's composition root;
/// StateChronicle v0 uses an in-memory fake. Async via `#[async_trait]`
/// (boxed futures keep the port dyn-compatible).
#[async_trait]
pub trait TransactionManager: Sync + Send {
    /// Begins a transaction scoped to a tenant.
    ///
    /// # Errors
    ///
    /// Returns [`TransactionManagerError::Unavailable`] when the coordinator
    /// cannot be reached, and [`TransactionManagerError::Aborted`] when the
    /// coordinator aborts the new transaction.
    async fn begin(
        &self,
        tenant: &TenantId,
    ) -> Result<Box<dyn TransactionHandle + Send>, TransactionManagerError>;

    /// Begins a multi-tenant transaction spanning every affected tenant
    /// (protocol §8.2, §18.3).
    ///
    /// Maps to a multi-tenant / 2PC coordinator in production; in StateChronicle
    /// v0 the fake records it symbolically. The caller passes the *sorted*
    /// affected-tenant set so the coordinator observes a deterministic scope.
    ///
    /// # Errors
    ///
    /// Returns [`TransactionManagerError::Unavailable`] when the coordinator
    /// cannot be reached, and [`TransactionManagerError::Aborted`] when the
    /// coordinator aborts the new transaction.
    async fn begin_multi(
        &self,
        tenants: &[TenantId],
    ) -> Result<Box<dyn TransactionHandle + Send>, TransactionManagerError>;
}

/// Backend-agnostic transaction handle (no implementations in this crate).
///
/// Consumes itself on completion — a handle can be committed or rolled back
/// exactly once. Uses `#[async_trait]` rather than `trait_variant::make`
/// because the consuming `self: Box<Self>` receivers need boxed futures to
/// stay dyn-compatible.
#[async_trait]
pub trait TransactionHandle: Sync + Send {
    /// Commits the transaction, releasing its staged writes.
    ///
    /// # Errors
    ///
    /// Returns [`TransactionManagerError::CommitFailed`] when the commit phase
    /// fails, and [`TransactionManagerError::Aborted`] when the coordinator
    /// aborts the transaction.
    async fn commit(self: Box<Self>) -> Result<(), TransactionManagerError>;

    /// Rolls back the transaction, discarding its staged writes.
    ///
    /// # Errors
    ///
    /// Returns [`TransactionManagerError::RollbackFailed`] when the rollback
    /// phase fails, and [`TransactionManagerError::Aborted`] when the
    /// coordinator aborts the transaction.
    async fn rollback(self: Box<Self>) -> Result<(), TransactionManagerError>;
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            TransactionManagerError::Aborted.to_string(),
            "transaction aborted by coordinator"
        );
        assert_eq!(
            TransactionManagerError::CommitFailed(String::from("durable")).to_string(),
            "transaction commit failed: durable"
        );
        assert_eq!(
            TransactionManagerError::RollbackFailed(String::from("release")).to_string(),
            "transaction rollback failed: release"
        );
        assert_eq!(
            TransactionManagerError::Unavailable(String::from("coordinator down")).to_string(),
            "transaction coordinator unavailable: coordinator down"
        );
    }
}
