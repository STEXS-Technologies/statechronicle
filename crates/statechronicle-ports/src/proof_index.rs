//! Port trait for the proof index (protocol §16).
//!
//! Serves portable state, ownership, and inclusion proofs over committed
//! state roots, so verifiers never need to replay full history. Each proof
//! pins the enclosing signed commit (§27 logical stores).

use async_trait::async_trait;
use statechronicle_accumulator::key::StateKey;
use statechronicle_domain::ids::{CommitId, EventId};
use statechronicle_domain::proof::{
    NonMembershipProofBundle, ResourceStateProof, SparseMerkleProof,
};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;

/// Errors produced by the proof index port.
#[derive(Debug, Error)]
pub enum ProofIndexError {
    /// No proof could be produced for the requested claim.
    #[error("no proof available for the requested claim")]
    NotFound,
    /// The backing index could not be reached or resolved.
    #[error("proof index unavailable: {0}")]
    Unavailable(String),
}

/// Backend-agnostic proof index port (no implementations in this crate).
///
/// The production adapter lives in the consuming platform's composition root;
/// StateChronicle v0 uses an in-memory fake. Async via `#[async_trait]`
/// (boxed futures keep the port dyn-compatible).
///
/// `#[async_trait]` is used rather than `trait_variant::make(Send)`: the latter
/// desugars `async fn` to `-> impl Future + Send`, which is not object-safe, so
/// adapters could not be held behind `&dyn ProofIndex`.
#[async_trait]
pub trait ProofIndex: Sync + Send {
    /// Returns a state proof for a resource at an optional commit.
    ///
    /// `at` of `None` means the latest committed state.
    ///
    /// # Errors
    ///
    /// Returns [`ProofIndexError::NotFound`] when no proof can be produced for
    /// the claimed state, and [`ProofIndexError::Unavailable`] when the
    /// backing index cannot be reached.
    async fn get_state_proof(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
        at: Option<&CommitId>,
    ) -> Result<Option<ResourceStateProof>, ProofIndexError>;

    /// Returns an ownership proof for a subject over a resource at an
    /// optional commit.
    ///
    /// `at` of `None` means the latest committed state.
    ///
    /// # Errors
    ///
    /// Returns [`ProofIndexError::NotFound`] when no ownership proof can be
    /// produced, and [`ProofIndexError::Unavailable`] when the backing index
    /// cannot be reached.
    async fn get_ownership_proof(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
        subject: &SubjectId,
        at: Option<&CommitId>,
    ) -> Result<Option<ResourceStateProof>, ProofIndexError>;

    /// Returns a sparse Merkle inclusion proof of an event in a commit.
    ///
    /// # Errors
    ///
    /// Returns [`ProofIndexError::NotFound`] when no inclusion proof can be
    /// produced, and [`ProofIndexError::Unavailable`] when the backing index
    /// cannot be reached.
    async fn get_inclusion_proof(
        &self,
        tenant: &TenantId,
        event_id: &EventId,
        commit_id: &CommitId,
    ) -> Result<Option<SparseMerkleProof>, ProofIndexError>;

    /// Returns a non-membership proof bundle for an absent state key at an
    /// optional commit.
    ///
    /// `at` of `None` means the latest committed state. Returns `Ok(None)`
    /// when the key is *present* (so no absence proof exists) and
    /// [`ProofIndexError::NotFound`] only when the claim itself cannot be
    /// served.
    ///
    /// # Errors
    ///
    /// Returns [`ProofIndexError::NotFound`] when no non-membership proof can
    /// be produced for the claim, and [`ProofIndexError::Unavailable`] when
    /// the backing index cannot be reached.
    async fn get_non_membership_proof(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
        key: StateKey,
        at: Option<&CommitId>,
    ) -> Result<Option<NonMembershipProofBundle>, ProofIndexError>;
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            ProofIndexError::NotFound.to_string(),
            "no proof available for the requested claim"
        );
        assert_eq!(
            ProofIndexError::Unavailable(String::from("db down")).to_string(),
            "proof index unavailable: db down"
        );
    }
}
