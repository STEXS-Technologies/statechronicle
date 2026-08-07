//! Port trait for the append-only event store (protocol §12).
//!
//! Persists validated, append-only transitions for replay. The store fails
//! closed when a batch contains a duplicate event id, and serves per-resource
//! history in commit order (§27 logical stores).

use async_trait::async_trait;
use statechronicle_domain::event::Event;
use statechronicle_domain::ids::EventId;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;

/// Errors produced by the event store port.
#[derive(Debug, Error)]
pub enum EventStoreError {
    /// A batch of events contains a duplicate event id.
    #[error("batch contains a duplicate event id")]
    DuplicateEventId,
    /// The backing store could not be reached or resolved.
    #[error("event store unavailable: {0}")]
    Unavailable(String),
}

/// Backend-agnostic append-only event store port (no implementations in this
/// crate).
///
/// The production adapter lives in the consuming platform's composition root;
/// StateChronicle v0 uses an in-memory fake. Async via `#[async_trait]`
/// (boxed futures keep the port dyn-compatible).
#[async_trait]
pub trait EventStore: Sync + Send {
    /// Appends a batch of events for a tenant, fail-closed on duplicates.
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::DuplicateEventId`] when `events` contains two
    /// events with the same id, and [`EventStoreError::Unavailable`] when the
    /// backing store cannot be reached.
    async fn append_events(
        &self,
        tenant: &TenantId,
        events: &[Event],
    ) -> Result<(), EventStoreError>;

    /// Returns the ordered history of events for a resource.
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::Unavailable`] when the backing store cannot
    /// be reached.
    async fn events_for_resource(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
    ) -> Result<Vec<Event>, EventStoreError>;

    /// Fetches an event by id within a tenant scope.
    ///
    /// # Errors
    ///
    /// Returns [`EventStoreError::Unavailable`] when the backing store cannot
    /// be reached.
    async fn event_by_id(
        &self,
        tenant: &TenantId,
        event_id: &EventId,
    ) -> Result<Option<Event>, EventStoreError>;
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            EventStoreError::DuplicateEventId.to_string(),
            "batch contains a duplicate event id"
        );
        assert_eq!(
            EventStoreError::Unavailable(String::from("db down")).to_string(),
            "event store unavailable: db down"
        );
    }
}
