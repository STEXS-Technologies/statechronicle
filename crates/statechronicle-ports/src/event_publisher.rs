//! Port trait for event publication (protocol §28 API surface).
//!
//! Delivers committed events and the enclosing signed commit to consumers
//! (websocket feeds, webhook integrations). Publication happens only after a
//! commit is durable; failures fail closed so consumers never observe partial
//! state.

use async_trait::async_trait;
use statechronicle_domain::commit::Commit;
use statechronicle_domain::event::Event;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;

/// Errors produced by the event publisher port.
#[derive(Debug, Error)]
pub enum EventPublisherError {
    /// The publisher rejected the payload (e.g. out-of-order delivery).
    #[error("publisher rejected delivery: {0}")]
    Rejected(String),
    /// The publisher could not be reached or resolved.
    #[error("event publisher unavailable: {0}")]
    Unavailable(String),
}

/// Backend-agnostic event publisher port (no implementations in this crate).
///
/// The production adapter lives in the consuming platform's composition root;
/// StateChronicle v0 uses an in-memory fake. Async via `#[async_trait]`
/// (boxed futures keep the port dyn-compatible).
///
/// `#[async_trait]` is used rather than `trait_variant::make(Send)`: the latter
/// desugars `async fn` to `-> impl Future + Send`, which is not object-safe, so
/// adapters could not be held behind `&dyn EventPublisher`.
#[async_trait]
pub trait EventPublisher: Sync + Send {
    /// Publishes a batch of committed events for a tenant.
    ///
    /// # Errors
    ///
    /// Returns [`EventPublisherError::Rejected`] when the publisher rejects
    /// the payload, and [`EventPublisherError::Unavailable`] when the
    /// publisher cannot be reached.
    async fn publish_events(
        &self,
        tenant: &TenantId,
        events: &[Event],
    ) -> Result<(), EventPublisherError>;

    /// Publishes the signed commit that encloses the events.
    ///
    /// # Errors
    ///
    /// Returns [`EventPublisherError::Rejected`] when the publisher rejects
    /// the payload, and [`EventPublisherError::Unavailable`] when the
    /// publisher cannot be reached.
    async fn publish_commit(
        &self,
        tenant: &TenantId,
        commit: &Signed<Commit>,
    ) -> Result<(), EventPublisherError>;
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            EventPublisherError::Rejected(String::from("out of order")).to_string(),
            "publisher rejected delivery: out of order"
        );
        assert_eq!(
            EventPublisherError::Unavailable(String::from("broker down")).to_string(),
            "event publisher unavailable: broker down"
        );
    }
}
