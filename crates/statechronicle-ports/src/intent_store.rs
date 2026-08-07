//! Port trait for the intent store (protocol §11.2).
//!
//! Stores submitted intents for deduplication and idempotency, keyed by
//! tenant scope and intent id. Re-inserting an intent with the same id and
//! payload succeeds; inserting the same id with a different payload fails
//! closed as a duplicate (§27 logical stores).

use async_trait::async_trait;
use statechronicle_domain::ids::IntentId;
use statechronicle_domain::intent::Intent;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;

/// Errors produced by the intent store port.
#[derive(Debug, Error)]
pub enum IntentStoreError {
    /// An intent with the same id but a different payload is already stored.
    #[error("intent id already exists with a different payload")]
    Duplicate,
    /// The backing store could not be reached or resolved.
    #[error("intent store unavailable: {0}")]
    Unavailable(String),
}

/// Backend-agnostic intent store port (no implementations in this crate).
///
/// The production adapter lives in the consuming platform's composition root;
/// StateChronicle v0 uses an in-memory fake. Async via `#[async_trait]`
/// (boxed futures keep the port dyn-compatible).
#[async_trait]
pub trait IntentStore: Sync + Send {
    /// Stores an intent idempotently within a tenant scope.
    ///
    /// Re-inserting an intent with the same id and payload succeeds;
    /// inserting a different payload under an existing id fails closed.
    ///
    /// # Errors
    ///
    /// Returns [`IntentStoreError::Duplicate`] when an intent with the same id
    /// but a different payload already exists, and
    /// [`IntentStoreError::Unavailable`] when the backing store cannot be
    /// reached.
    async fn put_intent(&self, tenant: &TenantId, intent: &Intent) -> Result<(), IntentStoreError>;

    /// Fetches an intent by id within a tenant scope.
    ///
    /// # Errors
    ///
    /// Returns [`IntentStoreError::Unavailable`] when the backing store cannot
    /// be reached.
    async fn get_intent(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
    ) -> Result<Option<Intent>, IntentStoreError>;
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            IntentStoreError::Duplicate.to_string(),
            "intent id already exists with a different payload"
        );
        assert_eq!(
            IntentStoreError::Unavailable(String::from("db down")).to_string(),
            "intent store unavailable: db down"
        );
    }
}
