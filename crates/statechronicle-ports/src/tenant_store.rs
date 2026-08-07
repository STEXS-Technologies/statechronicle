//! Port trait for tenant scope resolution (protocol §8).
//!
//! Resolves tenant existence and the tenant checkpoint root digest that pins
//! the tenant-scoped history (protocol §8, §13.4).

use async_trait::async_trait;
use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;

/// Errors produced by the tenant store port.
#[derive(Debug, Error)]
pub enum TenantStoreError {
    /// The tenant is known but no checkpoint root is recorded yet.
    #[error("tenant checkpoint root not found")]
    NotFound,
    /// The backing store could not be reached or resolved.
    #[error("tenant store unavailable: {0}")]
    Unavailable(String),
}

/// Backend-agnostic tenant store port (no implementations in this crate).
///
/// The production adapter lives in the consuming platform's composition root;
/// StateChronicle v0 uses an in-memory fake. Async via `#[async_trait]`
/// (boxed futures keep the port dyn-compatible).
///
/// `#[async_trait]` is used rather than `trait_variant::make(Send)`: the latter
/// desugars `async fn` to `-> impl Future + Send`, which is not object-safe, so
/// adapters could not be held behind `&dyn TenantStore`.
#[async_trait]
pub trait TenantStore: Sync + Send {
    /// Reports whether a tenant scope exists.
    ///
    /// # Errors
    ///
    /// Returns [`TenantStoreError::Unavailable`] when the backing store cannot
    /// be reached.
    async fn tenant_exists(&self, tenant: &TenantId) -> Result<bool, TenantStoreError>;

    /// Returns the tenant checkpoint root digest, if a checkpoint has been
    /// recorded.
    ///
    /// # Errors
    ///
    /// Returns [`TenantStoreError::Unavailable`] when the backing store cannot
    /// be reached.
    async fn get_tenant_root(
        &self,
        tenant: &TenantId,
    ) -> Result<Option<ContentDigest>, TenantStoreError>;
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            TenantStoreError::NotFound.to_string(),
            "tenant checkpoint root not found"
        );
        assert_eq!(
            TenantStoreError::Unavailable(String::from("db down")).to_string(),
            "tenant store unavailable: db down"
        );
    }
}
