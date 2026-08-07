//! Port trait for the current-state index (protocol §9).
//!
//! Serves the derived current state projection of resources. A projection is
//! a deterministic function of the append-only event history — the index is
//! never the source of truth (§27 logical stores).

use async_trait::async_trait;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;

/// Errors produced by the state index port.
#[derive(Debug, Error)]
pub enum StateIndexError {
    /// The index holds no consistent projection for the requested key.
    #[error("no consistent index projection available: {0}")]
    Inconsistent(String),
    /// The backing index could not be reached or resolved.
    #[error("state index unavailable: {0}")]
    Unavailable(String),
}

/// Backend-agnostic current-state index port (no implementations in this
/// crate).
///
/// The production adapter lives in the consuming platform's composition root;
/// StateChronicle v0 uses an in-memory fake. Async via `#[async_trait]`
/// (boxed futures keep the port dyn-compatible).
///
/// `#[async_trait]` is used rather than `trait_variant::make(Send)`: the latter
/// desugars `async fn` to `-> impl Future + Send`, which is not object-safe, so
/// adapters could not be held behind `&dyn StateIndex`.
#[async_trait]
pub trait StateIndex: Sync + Send {
    /// Returns the current state projection of a resource.
    ///
    /// # Errors
    ///
    /// Returns [`StateIndexError::Inconsistent`] when the index holds no
    /// consistent projection for the key, and
    /// [`StateIndexError::Unavailable`] when the backing index cannot be
    /// reached.
    async fn get_state(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError>;

    /// Returns the current state projection of a resource for a specific
    /// subject (owner/controller view).
    ///
    /// # Errors
    ///
    /// Returns [`StateIndexError::Inconsistent`] when the index holds no
    /// consistent projection for the key, and
    /// [`StateIndexError::Unavailable`] when the backing index cannot be
    /// reached.
    async fn get_subject_state(
        &self,
        tenant: &TenantId,
        subject: &SubjectId,
        resource_id: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError>;
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            StateIndexError::Inconsistent(String::from("stale")).to_string(),
            "no consistent index projection available: stale"
        );
        assert_eq!(
            StateIndexError::Unavailable(String::from("db down")).to_string(),
            "state index unavailable: db down"
        );
    }
}
