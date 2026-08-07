//! Integration tests for the proof verification pipeline.
//!
//! Exercises the end-to-end path over real domain types: state accumulator →
//! commit signing → inclusion proof → resource state proof bundle →
//! full bundle verification (protocol §16.3, §29), plus the async
//! [`ProofService`] through in-memory fakes of the proof/state/commit/snapshot
//! ports.
//!
//! The verification claims locked here:
//! - a genuine bundle verifies against the enclosing signed commit,
//! - every §16.3 fail-closed gate rejects its specific tamper,
//! - the service loads the enclosing commit and verifies end-to-end.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::result_large_err,
    clippy::needless_pass_by_value,
    clippy::redundant_clone,
    clippy::bool_assert_comparison
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateUpdate};

use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::{ContentDigest, hash_bytes};
use statechronicle_core::signature::Signature;

use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle_domain::ids::{CommitId, EventId, SnapshotId};
use statechronicle_domain::intent::{KeyId, Operation};
use statechronicle_domain::proof::{
    NonMembershipProofBundle, ResourceStateProof, SparseMerkleProof,
};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_ports::commit_store::{CommitStore, CommitStoreError};
use statechronicle_ports::proof_index::{ProofIndex, ProofIndexError};
use statechronicle_ports::snapshot_store::{SnapshotStore, SnapshotStoreError};
use statechronicle_ports::state_index::{StateIndex, StateIndexError};

use statechronicle_commit::sign::sign_commit;

use statechronicle_proof::bundle::{
    SnapshotProof, build_snapshot_proof, build_state_proof, derive_state_key,
};
use statechronicle_proof::error::ProofError;
use statechronicle_proof::service::{ProofPorts, ProofService};
use statechronicle_proof::verify::{verify_bundle, verify_ownership, verify_proof};

const FIXED_SEED: [u8; 32] = [42u8; 32];

fn tenant() -> TenantId {
    TenantId(String::from("stexs.game.alpha"))
}

fn resource() -> ResourceId {
    ResourceId(String::from("asset:sword_001"))
}

fn owner() -> String {
    String::from("account:stexs:player_456")
}

fn subject() -> SubjectId {
    SubjectId(String::from("account:stexs:player_456"))
}

fn executor() -> SubjectId {
    SubjectId(String::from("service:statechronicle.stexs.net"))
}

fn profile() -> ProfileId {
    ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap()
}

fn timestamp() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn fixed_key() -> SigningKey {
    SigningKey::from_bytes(&FIXED_SEED)
}

fn key_id() -> KeyId {
    KeyId::new(String::from("did:key:z6Mk...#statechronicle-commit")).unwrap()
}

fn commit_id() -> CommitId {
    CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap()
}

fn event_id() -> EventId {
    EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap()
}

/// The claimed state committed at the proven leaf.
fn claimed_state() -> serde_json::Value {
    serde_json::json!({
        "owner": owner(),
        "status": "active",
        "version": 42,
    })
}

fn state_hash() -> ContentDigest {
    canonicalize_and_digest(&claimed_state()).unwrap()
}

fn state_key() -> StateKey {
    StateKey::for_resource(&tenant().0, &resource().0)
}

/// Builds a committed accumulator holding the claimed state at `state_key`,
/// returns the signed commit pinning its root, the inclusion proof, and the
/// state projection.
struct Fixture {
    signed: Signed<Commit>,
    inclusion: statechronicle_accumulator::proof::InclusionProof,
    root: statechronicle_accumulator::sparse_merkle::StateRoot,
    projection: StateProjection,
}

fn fixture() -> Fixture {
    let key = state_key();
    let mut acc = StateAccumulator::empty();
    acc.insert_batch(&[StateUpdate::new(key, *state_hash().as_bytes())])
        .unwrap();
    let root = acc.root();
    let inclusion = acc.prove_inclusion(&key).unwrap();

    let commit = Commit::new(
        CommitScope::tenant(tenant()),
        commit_id(),
        None,
        1,
        1,
        hash_bytes(b"event-root"),
        hash_bytes(b"genesis"),
        ContentDigest::new(*root.as_bytes()),
        timestamp(),
        executor(),
        profile(),
    );
    let signed = sign_commit(&commit, &fixed_key(), key_id()).unwrap();

    let projection = StateProjection {
        tenant_id: tenant(),
        resource_id: resource(),
        state_type: StateType::UniqueAsset,
        version: 42,
        last_event_id: event_id(),
        last_commit_id: commit_id(),
        state_hash: state_hash(),
        state: claimed_state(),
    };
    Fixture {
        signed,
        inclusion,
        root,
        projection,
    }
}

fn operation() -> Operation {
    Operation::new(String::from("asset.transfer")).unwrap()
}

fn build_proof(fixture: &Fixture) -> ResourceStateProof {
    build_state_proof(
        &fixture.projection,
        &fixture.signed,
        &fixture.inclusion,
        &operation(),
        None,
        state_key(),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// Pure verification pipeline (protocol §16.3).
// ---------------------------------------------------------------------------

#[test]
fn genuine_bundle_verifies_end_to_end() {
    let fixture = fixture();
    let proof = build_proof(&fixture);

    assert_eq!(proof.schema, "statechronicle.proof.resource_state.v0");
    assert_eq!(
        proof.state_inclusion_proof.kind,
        statechronicle_domain::proof::SPARSE_MERKLE_V0
    );
    assert_eq!(proof.commit.commit_id, fixture.signed.body.commit_id);
    assert_eq!(proof.commit.state_root, fixture.signed.body.next_state_root);
    assert_eq!(proof.latest_event.operation.as_str(), "asset.transfer");

    let key = derive_state_key(&proof).unwrap();
    assert_eq!(key, state_key());

    assert!(verify_proof(&proof, &fixture.root, &key).is_ok());
    assert!(verify_bundle(&proof, &fixture.signed, &fixed_key().verifying_key(), &key).is_ok());
    assert!(verify_ownership(&proof, &owner()).is_ok());
}

#[test]
fn ownership_check_rejects_wrong_subject() {
    let fixture = fixture();
    let proof = build_proof(&fixture);
    assert!(matches!(
        verify_ownership(&proof, "account:stexs:player_789"),
        Err(ProofError::SubjectMismatch { .. })
    ));
}

#[test]
fn claimed_state_tamper_is_rejected() {
    let fixture = fixture();
    let mut proof = build_proof(&fixture);
    proof.claimed_state = serde_json::json!({
        "owner": "account:stexs:player_789",
        "status": "active",
        "version": 42,
    });
    let key = derive_state_key(&proof).unwrap();
    assert!(matches!(
        verify_bundle(&proof, &fixture.signed, &fixed_key().verifying_key(), &key),
        Err(ProofError::ClaimedStateMismatch)
    ));
}

#[test]
fn leaf_hash_tamper_is_rejected() {
    let fixture = fixture();
    let mut proof = build_proof(&fixture);
    proof.state_inclusion_proof.leaf_hash = hash_bytes(b"not-the-leaf");
    let key = derive_state_key(&proof).unwrap();
    assert!(matches!(
        verify_bundle(&proof, &fixture.signed, &fixed_key().verifying_key(), &key),
        Err(ProofError::InclusionMismatch)
    ));
}

#[test]
fn commit_reference_tamper_is_rejected() {
    let fixture = fixture();
    let mut proof = build_proof(&fixture);
    proof.commit.sequence = proof.commit.sequence.wrapping_add(1);
    let key = derive_state_key(&proof).unwrap();
    assert!(matches!(
        verify_bundle(&proof, &fixture.signed, &fixed_key().verifying_key(), &key),
        Err(ProofError::CommitRefMismatch { .. })
    ));
}

#[test]
fn signature_tamper_is_rejected() {
    let fixture = fixture();
    let proof = build_proof(&fixture);

    // The bundle pins a signature over the original commit body; a tampered
    // stored commit body (in a field the commit reference does not cover, so
    // the ref gate passes and the signature gate must reject) fails strict
    // verification under the same key.
    let mut tampered_commit = fixture.signed.clone();
    tampered_commit.body.created_at = tampered_commit
        .body
        .created_at
        .checked_add_signed(chrono::Duration::seconds(1))
        .unwrap();
    let key = derive_state_key(&proof).unwrap();
    assert!(matches!(
        verify_bundle(&proof, &tampered_commit, &fixed_key().verifying_key(), &key),
        Err(ProofError::CommitSignature(_))
    ));

    // And a wrong verifying key must fail too.
    let other = SigningKey::from_bytes(&[7u8; 32]);
    assert!(verify_bundle(&proof, &fixture.signed, &other.verifying_key(), &key).is_err());
}

#[test]
fn tenant_scope_mismatch_is_rejected() {
    let fixture = fixture();
    let mut proof = build_proof(&fixture);
    proof.tenant_id = TenantId(String::from("stexs.game.beta"));
    let key = derive_state_key(&proof).unwrap();
    assert!(matches!(
        verify_bundle(&proof, &fixture.signed, &fixed_key().verifying_key(), &key),
        Err(ProofError::TenantMismatch { .. })
    ));
}

#[test]
fn wrong_root_is_rejected() {
    let fixture = fixture();
    let proof = build_proof(&fixture);
    let key = derive_state_key(&proof).unwrap();
    let wrong_root = statechronicle_accumulator::sparse_merkle::StateRoot::new([0x5au8; 32]);
    assert!(matches!(
        verify_proof(&proof, &wrong_root, &key),
        Err(ProofError::InclusionMismatch)
    ));
}

// ---------------------------------------------------------------------------
// Async ProofService over in-memory port fakes.
// ---------------------------------------------------------------------------

/// A stored commit keyed by its tenant scope.
type StoredCommit = (TenantId, Signed<Commit>);

#[derive(Clone, Default)]
struct FakeCommitStore {
    inner: Arc<Mutex<Vec<StoredCommit>>>,
}

#[async_trait]
impl CommitStore for FakeCommitStore {
    async fn put_commit(
        &self,
        tenant: &TenantId,
        commit: &Signed<Commit>,
    ) -> Result<(), CommitStoreError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|err| CommitStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        if inner
            .iter()
            .any(|(t, c)| t == tenant && c.body.commit_id == commit.body.commit_id)
        {
            return Err(CommitStoreError::Duplicate);
        }
        inner.push((tenant.clone(), commit.clone()));
        Ok(())
    }

    async fn commit_by_id(
        &self,
        tenant: &TenantId,
        commit_id: &CommitId,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| CommitStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner
            .iter()
            .find(|(t, c)| t == tenant && c.body.commit_id == *commit_id)
            .map(|(_, c)| c.clone()))
    }

    async fn commit_by_sequence(
        &self,
        tenant: &TenantId,
        sequence: u64,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| CommitStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner
            .iter()
            .find(|(t, c)| t == tenant && c.body.sequence == sequence)
            .map(|(_, c)| c.clone()))
    }
}

#[derive(Clone, Default)]
struct FakeProofIndex {
    state_proof: Arc<Mutex<Option<ResourceStateProof>>>,
    ownership_proof: Arc<Mutex<Option<ResourceStateProof>>>,
    inclusion_proof: Arc<Mutex<Option<SparseMerkleProof>>>,
    non_membership_proof: Arc<Mutex<Option<NonMembershipProofBundle>>>,
}

impl FakeProofIndex {
    fn with_state_proof(proof: ResourceStateProof) -> Self {
        // Ownership proofs reuse the same envelope in this lane, so the fake
        // serves the bundle from both accessors.
        Self {
            state_proof: Arc::new(Mutex::new(Some(proof.clone()))),
            ownership_proof: Arc::new(Mutex::new(Some(proof))),
            ..Self::default()
        }
    }
}

#[async_trait]
impl ProofIndex for FakeProofIndex {
    async fn get_state_proof(
        &self,
        _tenant: &TenantId,
        _resource_id: &ResourceId,
        _at: Option<&CommitId>,
    ) -> Result<Option<ResourceStateProof>, ProofIndexError> {
        let inner = self
            .state_proof
            .lock()
            .map_err(|err| ProofIndexError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner.clone())
    }

    async fn get_ownership_proof(
        &self,
        _tenant: &TenantId,
        _resource_id: &ResourceId,
        _subject: &SubjectId,
        _at: Option<&CommitId>,
    ) -> Result<Option<ResourceStateProof>, ProofIndexError> {
        let inner = self
            .ownership_proof
            .lock()
            .map_err(|err| ProofIndexError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner.clone())
    }

    async fn get_inclusion_proof(
        &self,
        _tenant: &TenantId,
        _event_id: &EventId,
        _commit_id: &CommitId,
    ) -> Result<Option<SparseMerkleProof>, ProofIndexError> {
        let inner = self
            .inclusion_proof
            .lock()
            .map_err(|err| ProofIndexError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner.clone())
    }

    async fn get_non_membership_proof(
        &self,
        _tenant: &TenantId,
        _resource_id: &ResourceId,
        _key: StateKey,
        _at: Option<&CommitId>,
    ) -> Result<Option<NonMembershipProofBundle>, ProofIndexError> {
        let inner = self
            .non_membership_proof
            .lock()
            .map_err(|err| ProofIndexError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner.clone())
    }
}

#[derive(Clone, Default)]
struct FakeStateIndex {
    inner: Arc<Mutex<HashMap<(TenantId, ResourceId), StateProjection>>>,
}

#[async_trait]
impl StateIndex for FakeStateIndex {
    async fn get_state(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| StateIndexError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner.get(&(tenant.clone(), resource_id.clone())).cloned())
    }

    async fn get_subject_state(
        &self,
        tenant: &TenantId,
        _subject: &SubjectId,
        resource_id: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        self.get_state(tenant, resource_id).await
    }
}

/// A stored snapshot payload keyed by its tenant scope.
type StoredSnapshot = (TenantId, SnapshotId, Vec<u8>);

#[derive(Clone, Default)]
struct FakeSnapshotStore {
    inner: Arc<Mutex<Vec<StoredSnapshot>>>,
}

#[async_trait]
impl SnapshotStore for FakeSnapshotStore {
    async fn put_snapshot(
        &self,
        tenant: &TenantId,
        snapshot_id: &SnapshotId,
        payload: Vec<u8>,
    ) -> Result<(), SnapshotStoreError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|err| SnapshotStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        inner.push((tenant.clone(), snapshot_id.clone(), payload));
        Ok(())
    }

    async fn get_snapshot(
        &self,
        tenant: &TenantId,
        snapshot_id: &SnapshotId,
    ) -> Result<Option<Vec<u8>>, SnapshotStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| SnapshotStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner
            .iter()
            .find(|(t, s, _)| t == tenant && s == snapshot_id)
            .map(|(_, _, payload)| payload.clone()))
    }
}

async fn service(proof: ResourceStateProof, fixture: &Fixture) -> ProofService {
    let commit_store = FakeCommitStore::default();
    commit_store
        .put_commit(&tenant(), &fixture.signed)
        .await
        .unwrap();
    let proof_index = FakeProofIndex::with_state_proof(proof);
    let state_index = FakeStateIndex::default();
    state_index
        .inner
        .lock()
        .unwrap()
        .insert((tenant(), resource()), fixture.projection.clone());
    let snapshot_store = FakeSnapshotStore::default();

    ProofService::new(ProofPorts {
        proof_index: Box::new(proof_index),
        state_index: Box::new(state_index),
        commit_store: Box::new(commit_store),
        snapshot_store: Box::new(snapshot_store),
    })
}

#[tokio::test]
async fn service_serves_and_verifies_proofs() {
    let fixture = fixture();
    let proof = build_proof(&fixture);
    let service = service(proof.clone(), &fixture).await;

    // State projection passthrough.
    let projection = service
        .get_state(&tenant(), &resource())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(projection.state_hash, state_hash());

    // State proof from the proof index.
    let served = service
        .get_state_proof(&tenant(), &resource(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(served, proof);

    // Ownership proof from the proof index.
    let ownership = service
        .get_ownership_proof(&tenant(), &resource(), &subject(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ownership, proof);

    // Full end-to-end verification: the service loads the enclosing signed
    // commit through the commit-store port and runs the §16.3 pipeline.
    service
        .verify(&proof, &fixed_key().verifying_key())
        .await
        .unwrap();
    service
        .verify_with_key(&proof, &fixed_key().verifying_key(), &state_key())
        .await
        .unwrap();

    // A wrong verifying key is rejected by the service too.
    let other = SigningKey::from_bytes(&[7u8; 32]);
    assert!(
        service
            .verify(&proof, &other.verifying_key())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn service_verifies_only_when_commit_is_stored() {
    let fixture = fixture();
    let proof = build_proof(&fixture);

    // The enclosing commit is deliberately not stored.
    let commit_store = FakeCommitStore::default();

    let service = ProofService::new(ProofPorts {
        proof_index: Box::new(FakeProofIndex::with_state_proof(proof.clone())),
        state_index: Box::new(FakeStateIndex::default()),
        commit_store: Box::new(commit_store),
        snapshot_store: Box::new(FakeSnapshotStore::default()),
    });
    assert!(matches!(
        service.verify(&proof, &fixed_key().verifying_key()).await,
        Err(ProofError::NotFound)
    ));
}

#[tokio::test]
async fn service_builds_snapshot_proofs_from_stored_payloads() {
    let fixture = fixture();
    let proof = build_proof(&fixture);
    let service = service(proof.clone(), &fixture).await;

    let snapshot_id = SnapshotId::new(String::from("snp_01JZ8X9P4DC6YC4K1YZEJX45E2")).unwrap();
    let payload = b"serialized snapshot checkpoint".to_vec();

    // Nothing stored yet: no snapshot proof.
    let missing = service
        .get_snapshot_proof(&tenant(), &snapshot_id, proof.clone())
        .await
        .unwrap();
    assert!(missing.is_none());

    // Store the payload through a fake adapter then fetch the proof.
    let adapter = FakeSnapshotStore::default();
    adapter
        .put_snapshot(&tenant(), &snapshot_id, payload.clone())
        .await
        .unwrap();
    let commit_store = FakeCommitStore::default();
    commit_store
        .put_commit(&tenant(), &fixture.signed)
        .await
        .unwrap();
    let with_snapshot = ProofService::new(ProofPorts {
        proof_index: Box::new(FakeProofIndex::with_state_proof(proof.clone())),
        state_index: Box::new(FakeStateIndex::default()),
        commit_store: Box::new(commit_store),
        snapshot_store: Box::new(adapter),
    });
    let snapshot: SnapshotProof = with_snapshot
        .get_snapshot_proof(&tenant(), &snapshot_id, proof)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        snapshot.schema,
        statechronicle_proof::bundle::SNAPSHOT_PROOF_SCHEMA
    );
    assert_eq!(snapshot.payload_digest, hash_bytes(&payload));
    assert_eq!(
        snapshot,
        build_snapshot_proof(snapshot_id, &payload, snapshot.state.clone())
    );
}

#[tokio::test]
async fn service_inclusion_proof_passthrough() {
    let fixture = fixture();
    let proof = build_proof(&fixture);
    let mut index = FakeProofIndex::with_state_proof(proof);
    index.inclusion_proof = Arc::new(Mutex::new(Some(
        statechronicle_proof::bundle::build_inclusion_proof(&fixture.inclusion),
    )));
    let service = ProofService::new(ProofPorts {
        proof_index: Box::new(index),
        state_index: Box::new(FakeStateIndex::default()),
        commit_store: Box::new(FakeCommitStore::default()),
        snapshot_store: Box::new(FakeSnapshotStore::default()),
    });

    let sparse = service
        .get_inclusion_proof(&tenant(), &event_id(), &commit_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sparse.kind, statechronicle_domain::proof::SPARSE_MERKLE_V0);
    assert_eq!(
        sparse.leaf_hash,
        ContentDigest::new(fixture.inclusion.leaf_hash)
    );

    // The dense wire proof verifies through the pure verifier.
    let root = statechronicle_accumulator::sparse_merkle::StateRoot::new(
        *fixture.signed.body.next_state_root.as_bytes(),
    );
    assert!(
        statechronicle_proof::verify::verify_sparse_merkle_v0(&root, &state_key(), &sparse).is_ok()
    );
}

// ---------------------------------------------------------------------------
// Serde wire roundtrip of the bundle.
// ---------------------------------------------------------------------------

#[test]
fn bundle_roundtrips_through_json_and_bcs() {
    let fixture = fixture();
    let proof = build_proof(&fixture);

    let json = serde_json::to_string(&proof).unwrap();
    let decoded: ResourceStateProof = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, proof);

    // BCS encode-side determinism (BCS is not self-describing, ADR-004).
    let first = bcs::to_bytes(&proof).unwrap();
    let second = bcs::to_bytes(&proof).unwrap();
    assert_eq!(first, second);

    // The signature round-trips through its `b64u:` string form.
    let sig_json = serde_json::to_string(&proof.commit.signature.sig).unwrap();
    assert!(sig_json.starts_with("\"b64u:"));
    let sig_back: Signature = serde_json::from_str(&sig_json).unwrap();
    assert_eq!(sig_back, proof.commit.signature.sig);
}
