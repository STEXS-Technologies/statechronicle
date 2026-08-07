//! Proof service (protocol §16, reference service `get_*_proof` /
//! `verify_proof` operations).
//!
//! [`ProofService`] is the async composition layer over the proof lane's
//! driven ports: it serves portable state, ownership, and inclusion proofs
//! through the [`ProofIndex`] port, loads the enclosing signed commit through
//! the [`CommitStore`] port for bundle verification, fetches snapshot
//! payloads through the [`SnapshotStore`] port, and exposes current-state
//! projections through the [`StateIndex`] port. All verification is delegated
//! to the pure [`crate::verify`] functions; the service itself holds no
//! verification logic.

use ed25519_dalek::VerifyingKey;
use statechronicle_accumulator::key::StateKey;
use statechronicle_domain::ids::{CommitId, EventId, SnapshotId};
use statechronicle_domain::proof::{
    NonMembershipProofBundle, ResourceStateProof, SparseMerkleProof,
};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_ports::commit_store::CommitStore;
use statechronicle_ports::proof_index::ProofIndex;
use statechronicle_ports::snapshot_store::SnapshotStore;
use statechronicle_ports::state_index::StateIndex;

use crate::bundle::{SnapshotProof, build_snapshot_proof, derive_state_key};
use crate::error::ProofError;
use crate::verify::{verify_bundle, verify_non_membership_bundle};

/// Backend-agnostic store set used by [`ProofService`].
///
/// Holds the driven ports as `Send + Sync` trait objects so the service
/// composes in any runtime. `ProofPorts` deliberately exposes only read-side
/// storage: proofs are served from committed state, never written by this
/// lane.
pub struct ProofPorts {
    /// Proof index serving state, ownership, and inclusion proofs.
    pub proof_index: Box<dyn ProofIndex>,
    /// Read-only current-state index (§27).
    pub state_index: Box<dyn StateIndex>,
    /// Append-only signed commit store.
    pub commit_store: Box<dyn CommitStore>,
    /// Snapshot store holding opaque snapshot payloads.
    pub snapshot_store: Box<dyn SnapshotStore>,
}

/// Async proof service over the proof lane ports.
///
/// Mirrors the reference service operations `get_state_proof`,
/// `get_ownership_proof`, `get_inclusion_proof`, `get_snapshot`, and
/// `verify_proof` (protocol §28).
pub struct ProofService {
    ports: ProofPorts,
}

impl ProofService {
    /// Constructs a proof service from its ports.
    pub const fn new(ports: ProofPorts) -> Self {
        Self { ports }
    }

    /// Returns the service's port set.
    ///
    /// Exposed for composition roots and integration tests that need to
    /// reuse or inspect a specific store (for example to seed a snapshot
    /// payload).
    pub const fn ports(&self) -> &ProofPorts {
        &self.ports
    }

    /// Returns the current-state projection of a resource.
    ///
    /// # Errors
    ///
    /// Returns [`ProofError::ProofIndex`] when the state index cannot be
    /// reached and [`ProofError::NotFound`] when no projection is stored.
    pub async fn get_state(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
    ) -> Result<Option<StateProjection>, ProofError> {
        self.ports
            .state_index
            .get_state(tenant, resource_id)
            .await
            .map_err(|err| ProofError::ProofIndex(err.to_string()))
    }

    /// Returns a state proof for a resource at an optional commit.
    ///
    /// `at` of `None` means the latest committed state.
    ///
    /// # Errors
    ///
    /// Returns [`ProofError::ProofIndex`] when the proof index cannot serve
    /// the claim and [`ProofError::NotFound`] when no proof can be produced.
    pub async fn get_state_proof(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
        at: Option<&CommitId>,
    ) -> Result<Option<ResourceStateProof>, ProofError> {
        self.ports
            .proof_index
            .get_state_proof(tenant, resource_id, at)
            .await
            .map_err(|err| ProofError::ProofIndex(err.to_string()))
    }

    /// Returns an ownership proof for a subject over a resource at an
    /// optional commit.
    ///
    /// # Errors
    ///
    /// Returns [`ProofError::ProofIndex`] when the proof index cannot serve
    /// the claim and [`ProofError::NotFound`] when no proof can be produced.
    pub async fn get_ownership_proof(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
        subject: &SubjectId,
        at: Option<&CommitId>,
    ) -> Result<Option<ResourceStateProof>, ProofError> {
        self.ports
            .proof_index
            .get_ownership_proof(tenant, resource_id, subject, at)
            .await
            .map_err(|err| ProofError::ProofIndex(err.to_string()))
    }

    /// Returns a sparse Merkle inclusion proof of an event in a commit.
    ///
    /// # Errors
    ///
    /// Returns [`ProofError::ProofIndex`] when the proof index cannot serve
    /// the claim and [`ProofError::NotFound`] when no proof can be produced.
    pub async fn get_inclusion_proof(
        &self,
        tenant: &TenantId,
        event_id: &EventId,
        commit_id: &CommitId,
    ) -> Result<Option<SparseMerkleProof>, ProofError> {
        self.ports
            .proof_index
            .get_inclusion_proof(tenant, event_id, commit_id)
            .await
            .map_err(|err| ProofError::ProofIndex(err.to_string()))
    }

    /// Returns a non-membership proof bundle for an absent state key at an
    /// optional commit.
    ///
    /// `at` of `None` means the latest committed state. Returns `Ok(None)`
    /// when the key is present (so no absence proof exists).
    ///
    /// # Errors
    ///
    /// Returns [`ProofError::ProofIndex`] when the proof index cannot serve
    /// the claim and [`ProofError::NotFound`] when no non-membership proof can
    /// be produced.
    pub async fn get_non_membership_proof(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
        key: StateKey,
        at: Option<&CommitId>,
    ) -> Result<Option<NonMembershipProofBundle>, ProofError> {
        self.ports
            .proof_index
            .get_non_membership_proof(tenant, resource_id, key, at)
            .await
            .map_err(|err| ProofError::ProofIndex(err.to_string()))
    }

    /// Verifies a non-membership proof bundle end-to-end.
    ///
    /// Derives the proven `StateKey` from the bundle's `claimed_key` (the 32
    /// raw key bytes as a digest), loads the enclosing signed commit through
    /// the [`CommitStore`] port, and runs
    /// [`verify_non_membership_bundle`] under the supplied verifying key.
    /// Because the key is derived from the bundle's own `claimed_key`, the
    /// caller MUST independently assert `bundle.claimed_key` against its own
    /// `StateKey` (or use the pure `verify_non_membership(bundle, root, key)`
    /// with an independent key), since the service has no independent key with
    /// which to detect a forged or swapped `claimed_key`.
    ///
    /// # Errors
    ///
    /// Returns [`ProofError::Store`] when the commit store cannot be
    /// reached, [`ProofError::NotFound`] when the enclosing commit is not
    /// stored, and every [`ProofError`] variant of
    /// [`verify_non_membership_bundle`].
    pub async fn verify_non_membership(
        &self,
        bundle: &NonMembershipProofBundle,
        verifying_key: &VerifyingKey,
    ) -> Result<(), ProofError> {
        let key = StateKey::new(*bundle.claimed_key.as_bytes());
        let Some(signed) = self
            .ports
            .commit_store
            .commit_by_id(&bundle.tenant_id, &bundle.commit.commit_id)
            .await
            .map_err(|err| ProofError::Store(err.to_string()))?
        else {
            return Err(ProofError::NotFound);
        };
        verify_non_membership_bundle(bundle, &signed, verifying_key, &key)
    }

    /// Returns a snapshot proof binding a stored snapshot payload to a state
    /// proof.
    ///
    /// The snapshot payload is fetched through the [`SnapshotStore`] port and
    /// its canonical digest is bound to `state_proof` (protocol §15, §29).
    ///
    /// # Errors
    ///
    /// Returns [`ProofError::ProofIndex`] when the snapshot store cannot be
    /// reached and [`ProofError::NotFound`] when no snapshot is stored.
    pub async fn get_snapshot_proof(
        &self,
        tenant: &TenantId,
        snapshot_id: &SnapshotId,
        state_proof: ResourceStateProof,
    ) -> Result<Option<SnapshotProof>, ProofError> {
        let Some(payload) = self
            .ports
            .snapshot_store
            .get_snapshot(tenant, snapshot_id)
            .await
            .map_err(|err| ProofError::ProofIndex(err.to_string()))?
        else {
            return Ok(None);
        };
        Ok(Some(build_snapshot_proof(
            snapshot_id.clone(),
            &payload,
            state_proof,
        )))
    }

    /// Verifies a proof bundle end-to-end.
    ///
    /// Loads the enclosing signed commit through the [`CommitStore`] port and
    /// runs the full [`verify_bundle`] pipeline (schema, tenant scope, commit
    /// reference, commit signature, sparse Merkle inclusion, claimed-state
    /// hash) under the supplied verifying key and state key.
    ///
    /// # Errors
    ///
    /// Returns [`ProofError::Store`] when the commit store cannot be
    /// reached, [`ProofError::NotFound`] when the enclosing commit is not
    /// stored, and every [`ProofError`] variant of [`verify_bundle`].
    pub async fn verify(
        &self,
        proof: &ResourceStateProof,
        verifying_key: &VerifyingKey,
    ) -> Result<(), ProofError> {
        let key = derive_state_key(proof)?;
        let Some(signed) = self
            .ports
            .commit_store
            .commit_by_id(&proof.tenant_id, &proof.commit.commit_id)
            .await
            .map_err(|err| ProofError::Store(err.to_string()))?
        else {
            return Err(ProofError::NotFound);
        };
        verify_bundle(proof, &signed, verifying_key, &key)
    }

    /// Verifies a proof bundle end-to-end with an explicit state key.
    ///
    /// Like [`Self::verify`], but the caller supplies the proven leaf's state
    /// key directly (for subject-held proofs the key includes the subject,
    /// which the bundle's claimed state may not expose).
    ///
    /// # Errors
    ///
    /// See [`Self::verify`].
    pub async fn verify_with_key(
        &self,
        proof: &ResourceStateProof,
        verifying_key: &VerifyingKey,
        key: &StateKey,
    ) -> Result<(), ProofError> {
        let Some(signed) = self
            .ports
            .commit_store
            .commit_by_id(&proof.tenant_id, &proof.commit.commit_id)
            .await
            .map_err(|err| ProofError::Store(err.to_string()))?
        else {
            return Err(ProofError::NotFound);
        };
        verify_bundle(proof, &signed, verifying_key, key)
    }
}
