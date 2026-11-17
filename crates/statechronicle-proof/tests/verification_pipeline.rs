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
use statechronicle_domain::trade::{
    TRADE_PROOF_SCHEMA, TradeProof, TradeProofLeg, TradeStatus, TradeSummary,
};

use statechronicle_ports::commit_store::{CanonicalHead, CommitStore, CommitStoreError};
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
    TenantId(String::from("acme.game.alpha"))
}

fn resource() -> ResourceId {
    ResourceId(String::from("asset:sword_001"))
}

fn owner() -> String {
    String::from("account:example:player_456")
}

fn subject() -> SubjectId {
    SubjectId(String::from("account:example:player_456"))
}

fn executor() -> SubjectId {
    SubjectId(String::from("service:statechronicle.example.net"))
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
fn claimed_state() -> statechronicle_domain::resource_state::ResourceState {
    statechronicle_domain::resource_state::ResourceState::from_legacy_json(
        StateType::UniqueAsset,
        serde_json::json!({
            "owner": owner(),
            "status": "active",
        }),
    )
    .unwrap()
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
    fixture_for(tenant(), &fixed_key())
}

fn fixture_for(scope: TenantId, signing_key: &SigningKey) -> Fixture {
    fixture_state(scope, signing_key, claimed_state())
}

fn fixture_state(
    scope: TenantId,
    signing_key: &SigningKey,
    state: statechronicle_domain::resource_state::ResourceState,
) -> Fixture {
    let key = match state.subject() {
        Some(subject) => StateKey::for_subject_held(&scope.0, &resource().0, &subject.0),
        None => StateKey::for_resource(&scope.0, &resource().0),
    };
    let digest = canonicalize_and_digest(&state).unwrap();
    let mut acc = StateAccumulator::empty();
    acc.insert_batch(&[StateUpdate::new(key, *digest.as_bytes())])
        .unwrap();
    let root = acc.root();
    let inclusion = acc.prove_inclusion(&key).unwrap();

    let commit = Commit::new(
        CommitScope::tenant(scope.clone()),
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
    let signed = sign_commit(&commit, signing_key, key_id()).unwrap();

    let projection = StateProjection {
        tenant_id: scope,
        resource_id: resource(),
        state_type: state.state_type(),
        version: 42,
        last_event_id: event_id(),
        last_commit_id: commit_id(),
        state_hash: digest,
        state,
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
    let key = fixture.projection.state.subject().map_or_else(
        || {
            StateKey::for_resource(
                &fixture.projection.tenant_id.0,
                &fixture.projection.resource_id.0,
            )
        },
        |subject| {
            StateKey::for_subject_held(
                &fixture.projection.tenant_id.0,
                &fixture.projection.resource_id.0,
                &subject.0,
            )
        },
    );
    build_state_proof(
        &fixture.projection,
        &fixture.signed,
        &fixture.inclusion,
        &operation(),
        None,
        key,
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
        verify_ownership(&proof, "account:example:player_789"),
        Err(ProofError::SubjectMismatch { .. })
    ));
}

#[test]
fn claimed_state_tamper_is_rejected() {
    let fixture = fixture();
    let mut proof = build_proof(&fixture);
    proof.claimed_state = statechronicle_domain::resource_state::ResourceState::from_legacy_json(
        StateType::UniqueAsset,
        serde_json::json!({
            "owner": "account:example:player_789",
            "status": "active",
        }),
    )
    .unwrap();
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
    proof.tenant_id = TenantId(String::from("acme.game.beta"));
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
    heads: Arc<Mutex<HashMap<String, CanonicalHead>>>,
    advance_on_lookup: Arc<Mutex<Option<CanonicalHead>>>,
    head_reads: Arc<std::sync::atomic::AtomicUsize>,
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
        let result = inner
            .iter()
            .find(|(t, c)| t == tenant && c.body.commit_id == *commit_id)
            .map(|(_, c)| c.clone());
        if let Some(head) = self.advance_on_lookup.lock().unwrap().take() {
            self.heads.lock().unwrap().insert(tenant.0.clone(), head);
        }
        Ok(result)
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

    async fn canonical_head(
        &self,
        tenant: &TenantId,
    ) -> Result<Option<CanonicalHead>, CommitStoreError> {
        self.head_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(self.heads.lock().unwrap().get(&tenant.0).cloned())
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
async fn service_verify_trade_loads_leg_commits_and_resolves_keys() {
    let fixture = fixture();
    let state_proof = build_proof(&fixture);

    // A single-leg trade proof over tenant alpha, pinning the fixture's commit.
    let trade_proof = TradeProof {
        schema: String::from(TRADE_PROOF_SCHEMA),
        trade_id: String::from("trade_001"),
        summary: TradeSummary {
            trade_id: String::from("trade_001"),
            status: TradeStatus::Settled,
            sides: Vec::new(),
            value_legs: Vec::new(),
        },
        legs: vec![TradeProofLeg {
            tenant: tenant(),
            commit: state_proof.commit.clone(),
            state_proofs: vec![state_proof],
        }],
    };

    // Fail closed when a leg's settle commit is not stored: the service must
    // surface NotFound rather than proceeding with an unverifiable leg.
    let empty_store = ProofService::new(ProofPorts {
        proof_index: Box::new(FakeProofIndex::default()),
        state_index: Box::new(FakeStateIndex::default()),
        commit_store: Box::new(FakeCommitStore::default()),
        snapshot_store: Box::new(FakeSnapshotStore::default()),
    });
    assert!(matches!(
        empty_store
            .verify_trade(&trade_proof, &|_tenant: &TenantId| None)
            .await,
        Err(ProofError::NotFound)
    ));

    // Fail closed when the leg's commit is stored but no verifying key resolves
    // for its tenant: KeyNotFound, before any cryptographic work.
    let commit_store = FakeCommitStore::default();
    commit_store
        .put_commit(&tenant(), &fixture.signed)
        .await
        .unwrap();
    let store_with_commit = ProofService::new(ProofPorts {
        proof_index: Box::new(FakeProofIndex::default()),
        state_index: Box::new(FakeStateIndex::default()),
        commit_store: Box::new(commit_store.clone()),
        snapshot_store: Box::new(FakeSnapshotStore::default()),
    });
    assert!(matches!(
        store_with_commit
            .verify_trade(&trade_proof, &|_tenant: &TenantId| None)
            .await,
        Err(ProofError::KeyNotFound(_))
    ));

    // With a resolving key the service proceeds into the pure verifier; the
    // summary's empty side set makes it fail closed on the trade shape check
    // (TradeMissingSide) rather than passing, proving the delegation path runs.
    let store_with_key = ProofService::new(ProofPorts {
        proof_index: Box::new(FakeProofIndex::default()),
        state_index: Box::new(FakeStateIndex::default()),
        commit_store: Box::new(commit_store),
        snapshot_store: Box::new(FakeSnapshotStore::default()),
    });
    assert!(matches!(
        store_with_key
            .verify_trade(&trade_proof, &|_tenant: &TenantId| Some(
                fixed_key().verifying_key()
            ))
            .await,
        Err(ProofError::TradeMissingSide(_))
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

fn claim_service(store: FakeCommitStore) -> ProofService {
    ProofService::new(ProofPorts {
        proof_index: Box::new(FakeProofIndex::default()),
        state_index: Box::new(FakeStateIndex::default()),
        commit_store: Box::new(store),
        snapshot_store: Box::new(FakeSnapshotStore::default()),
    })
}

fn head(signed: &Signed<Commit>) -> CanonicalHead {
    CanonicalHead {
        commit_id: signed.body.commit_id.clone(),
        sequence: signed.body.sequence,
        state_root: signed.body.next_state_root.clone(),
    }
}

#[tokio::test]
async fn expected_claims_bind_address_owner_signer_kind_and_commit() {
    use statechronicle_proof::claim::{ClaimProof, ExpectedClaim, VerificationPoint};
    let fixture = fixture();
    let proof = build_proof(&fixture);
    let store = FakeCommitStore::default();
    store.put_commit(&tenant(), &fixture.signed).await.unwrap();
    let service = claim_service(store.clone());
    let expected = ExpectedClaim::ownership(tenant(), resource(), SubjectId(owner()));
    assert_eq!(expected.key(), state_key());
    assert_eq!(expected.owner(), Some(&SubjectId(owner())));
    assert_eq!(expected.resource(), &resource());
    assert!(!expected.is_absence());
    let trusted = fixed_key().verifying_key();
    let historical = service
        .verify_historical(ClaimProof::State(&proof), &expected, &commit_id(), &trusted)
        .await
        .unwrap();
    assert_eq!(historical.point(), VerificationPoint::Historical);
    assert_eq!(historical.expectation(), &expected);
    assert_eq!(historical.commit(), &proof.commit);
    assert_eq!(
        store.head_reads.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert!(matches!(
        service
            .verify_current(ClaimProof::State(&proof), &expected, &trusted)
            .await,
        Err(ProofError::CanonicalHeadUnavailable)
    ));
    store
        .heads
        .lock()
        .unwrap()
        .insert(tenant().0, head(&fixture.signed));
    let current = service
        .verify_current(ClaimProof::State(&proof), &expected, &trusted)
        .await
        .unwrap();
    assert_eq!(current.point(), VerificationPoint::Current);
    assert_eq!(current.commit(), historical.commit());
    let wrong_owner = ExpectedClaim::ownership(
        tenant(),
        resource(),
        SubjectId(String::from("account:mallory")),
    );
    assert!(matches!(
        service
            .verify_current(ClaimProof::State(&proof), &wrong_owner, &trusted)
            .await,
        Err(ProofError::SubjectMismatch { .. })
    ));
    assert!(matches!(
        service
            .verify_current(
                ClaimProof::State(&proof),
                &expected,
                &SigningKey::from_bytes(&[43; 32]).verifying_key()
            )
            .await,
        Err(ProofError::CommitSignature(_))
    ));
    assert!(matches!(
        service
            .verify_current(
                ClaimProof::State(&proof),
                &ExpectedClaim::absence(tenant(), resource()),
                &trusted
            )
            .await,
        Err(ProofError::InvalidState(_))
    ));
    let other_commit = CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF6")).unwrap();
    assert!(matches!(
        service
            .verify_historical(
                ClaimProof::State(&proof),
                &expected,
                &other_commit,
                &trusted
            )
            .await,
        Err(ProofError::CommitRefMismatch { .. })
    ));
    let mut swapped = proof.clone();
    swapped.tenant_id = TenantId(String::from("acme.game.beta"));
    assert!(matches!(
        service
            .verify_current(ClaimProof::State(&swapped), &expected, &trusted)
            .await,
        Err(ProofError::TenantMismatch { .. })
    ));
    swapped = proof.clone();
    swapped.resource_id = ResourceId(String::from("asset:other"));
    assert!(matches!(
        service
            .verify_current(ClaimProof::State(&swapped), &expected, &trusted)
            .await,
        Err(ProofError::ResourceMismatch { .. })
    ));
    // Renaming both the request and unsigned resource metadata cannot move the
    // actual signed leaf to that resource's independently derived key.
    let other = ExpectedClaim::state(tenant(), swapped.resource_id.clone());
    assert!(
        service
            .verify_current(ClaimProof::State(&swapped), &other, &trusted)
            .await
            .is_err()
    );
    swapped = proof.clone();
    swapped.commit.state_root = hash_bytes(b"forged root");
    assert!(matches!(
        service
            .verify_current(ClaimProof::State(&swapped), &expected, &trusted)
            .await,
        Err(ProofError::CommitRefMismatch { .. })
    ));
}

#[tokio::test]
async fn current_claim_rejects_head_advance_during_lookup_and_pins_success() {
    use statechronicle_proof::claim::{ClaimProof, ExpectedClaim};
    let fixture = fixture();
    let proof = build_proof(&fixture);
    let expected = ExpectedClaim::state(tenant(), resource());
    let store = FakeCommitStore::default();
    store.put_commit(&tenant(), &fixture.signed).await.unwrap();
    let old_head = head(&fixture.signed);
    store
        .heads
        .lock()
        .unwrap()
        .insert(tenant().0, old_head.clone());
    let mut advanced = old_head.clone();
    advanced.commit_id = CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF6")).unwrap();
    advanced.sequence += 1;
    *store.advance_on_lookup.lock().unwrap() = Some(advanced.clone());
    let service = claim_service(store.clone());
    assert!(matches!(
        service
            .verify_current(
                ClaimProof::State(&proof),
                &expected,
                &fixed_key().verifying_key()
            )
            .await,
        Err(ProofError::NonCanonicalCommit { .. })
    ));
    assert_eq!(
        store.head_reads.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    store.heads.lock().unwrap().insert(tenant().0, old_head);
    let verified = service
        .verify_current(
            ClaimProof::State(&proof),
            &expected,
            &fixed_key().verifying_key(),
        )
        .await
        .unwrap();
    assert_eq!(
        store.head_reads.load(std::sync::atomic::Ordering::Relaxed),
        2
    );
    // A later head advance cannot silently retarget an already returned claim.
    store.heads.lock().unwrap().insert(tenant().0, advanced);
    assert_eq!(verified.commit(), &proof.commit);
    assert!(matches!(
        service
            .verify_current(
                ClaimProof::State(&proof),
                &expected,
                &fixed_key().verifying_key()
            )
            .await,
        Err(ProofError::NonCanonicalCommit { .. })
    ));
    service
        .verify_historical(
            ClaimProof::State(&proof),
            &expected,
            &commit_id(),
            &fixed_key().verifying_key(),
        )
        .await
        .unwrap();
    assert_eq!(
        store.head_reads.load(std::sync::atomic::Ordering::Relaxed),
        3
    );
}

#[tokio::test]
async fn expected_subject_claim_uses_the_independent_holder_key() {
    use statechronicle_proof::claim::{ClaimProof, ExpectedClaim};
    let holder = SubjectId(String::from("account:holder"));
    let state = statechronicle_domain::resource_state::ResourceState::from_legacy_json(
        StateType::FungibleBalance,
        serde_json::json!({"subject": holder.0, "unit": "gold_minor", "balance": "10"}),
    )
    .unwrap();
    let fixture = fixture_state(tenant(), &fixed_key(), state);
    let proof = build_proof(&fixture);
    let store = FakeCommitStore::default();
    store.put_commit(&tenant(), &fixture.signed).await.unwrap();
    store
        .heads
        .lock()
        .unwrap()
        .insert(tenant().0, head(&fixture.signed));
    let service = claim_service(store);
    let expected = ExpectedClaim::subject_state(tenant(), resource(), holder.clone());
    assert_eq!(expected.subject(), Some(&holder));
    assert_eq!(
        expected.key(),
        StateKey::for_subject_held(&tenant().0, &resource().0, &holder.0)
    );
    service
        .verify_current(
            ClaimProof::State(&proof),
            &expected,
            &fixed_key().verifying_key(),
        )
        .await
        .unwrap();
    let other = ExpectedClaim::subject_state(
        tenant(),
        resource(),
        SubjectId(String::from("account:other")),
    );
    assert!(matches!(
        service
            .verify_current(
                ClaimProof::State(&proof),
                &other,
                &fixed_key().verifying_key()
            )
            .await,
        Err(ProofError::KeyMismatch { .. })
    ));
    assert!(matches!(
        service
            .verify_current(
                ClaimProof::State(&proof),
                &ExpectedClaim::state(tenant(), resource()),
                &fixed_key().verifying_key()
            )
            .await,
        Err(ProofError::KeyMismatch { .. })
    ));
}

#[tokio::test]
async fn verified_claim_never_promotes_unverified_reference_metadata() {
    use statechronicle_proof::claim::{ClaimProof, ExpectedClaim};
    let fixture = fixture();
    let mut proof = build_proof(&fixture);
    let store = FakeCommitStore::default();
    store.put_commit(&tenant(), &fixture.signed).await.unwrap();
    store
        .heads
        .lock()
        .unwrap()
        .insert(tenant().0, head(&fixture.signed));
    let service = claim_service(store);
    let expected = ExpectedClaim::state(tenant(), resource());
    let verified = service
        .verify_current(
            ClaimProof::State(&proof),
            &expected,
            &fixed_key().verifying_key(),
        )
        .await
        .unwrap();
    let authentic = proof.claimed_state.clone();
    proof.claimed_state = statechronicle_domain::resource_state::ResourceState::from_legacy_json(
        StateType::UniqueAsset,
        serde_json::json!({"owner":"account:mallory", "status":"active"}),
    )
    .unwrap();
    assert_eq!(verified.state(), Some(&authentic));
    proof.claimed_state = authentic;
    proof.commit.signature.sig = Signature::from_bytes([0; 64]);
    // The compatibility helper verifies the stored signature; the redundant
    // signature in the supplied reference was never part of its guarantee.
    service
        .verify_with_key(&proof, &fixed_key().verifying_key(), &state_key())
        .await
        .unwrap();
    assert!(matches!(
        service
            .verify_current(
                ClaimProof::State(&proof),
                &expected,
                &fixed_key().verifying_key()
            )
            .await,
        Err(ProofError::CommitSignature(_))
    ));
    assert!(matches!(
        service
            .verify_historical(
                ClaimProof::State(&proof),
                &expected,
                &commit_id(),
                &fixed_key().verifying_key()
            )
            .await,
        Err(ProofError::CommitSignature(_))
    ));

    let mut malformed = fixture.signed.body.clone();
    malformed.scope.kind = statechronicle_domain::commit::ScopeKind::GlobalCheckpoint;
    let signed = sign_commit(&malformed, &fixed_key(), key_id()).unwrap();
    let mut malformed_proof = build_proof(&fixture);
    malformed_proof.commit.signature = signed.signature.clone();
    let malformed_store = FakeCommitStore::default();
    malformed_store
        .put_commit(&tenant(), &signed)
        .await
        .unwrap();
    let malformed_service = claim_service(malformed_store);
    assert!(matches!(
        malformed_service
            .verify_historical(
                ClaimProof::State(&malformed_proof),
                &expected,
                &commit_id(),
                &fixed_key().verifying_key()
            )
            .await,
        Err(ProofError::CommitScope(_))
    ));
}

/// A genuine state proof must not establish a different tenant's trade leg.
#[tokio::test]
async fn trade_verifier_rejects_state_proof_under_a_different_leg_tenant() {
    use statechronicle_domain::trade::TradeSide;
    use std::collections::BTreeMap;
    let fixture = fixture();
    let mut state_proof = build_proof(&fixture);
    // Operation metadata is not included in the signed commit/state root.
    state_proof.latest_event.operation = Operation::from_static("trade.settle");
    let other_tenant = TenantId(String::from("acme.game.beta"));
    let mut trade = TradeProof {
        schema: String::from(TRADE_PROOF_SCHEMA),
        trade_id: String::from("trade_audit"),
        summary: TradeSummary {
            trade_id: String::from("trade_audit"),
            status: TradeStatus::Settled,
            sides: vec![TradeSide {
                tenant: other_tenant.clone(),
                settle_assets: vec![resource()],
                from_owner: String::from("unverified_previous_owner"),
                to_owner: owner(),
                settle_commit: state_proof.commit.clone(),
                settle_commits_by_asset: BTreeMap::from([(resource(), state_proof.commit.clone())]),
                settle_event_ids: vec![state_proof.latest_event.event_id.clone()],
            }],
            value_legs: Vec::new(),
        },
        legs: vec![TradeProofLeg {
            tenant: other_tenant,
            commit: state_proof.commit.clone(),
            state_proofs: vec![state_proof],
        }],
    };
    let commits = BTreeMap::from([(
        fixture.signed.body.commit_id.0.clone(),
        (fixture.signed.clone(), fixed_key().verifying_key()),
    )]);
    assert!(matches!(
        statechronicle_proof::trade::verify_trade_proof(&trade, &commits),
        Err(ProofError::TenantMismatch { .. })
    ));
    // A genuine alpha leg primes the service's commit-id-only cache. The beta
    // leg reuses it without loading a beta commit or resolving a beta signer.
    let mut alpha_side = trade.summary.sides[0].clone();
    alpha_side.tenant = tenant();
    trade.summary.sides.insert(0, alpha_side);
    let mut alpha_leg = trade.legs[0].clone();
    alpha_leg.tenant = tenant();
    trade.legs.insert(0, alpha_leg);
    let service = service(trade.legs[0].state_proofs[0].clone(), &fixture).await;
    let resolved = Mutex::new(Vec::new());
    let result = service
        .verify_trade(&trade, &|requested| {
            resolved.lock().unwrap().push(requested.clone());
            (requested == &tenant()).then(|| fixed_key().verifying_key())
        })
        .await;
    assert!(matches!(result, Err(ProofError::TenantMismatch { .. })));
    assert_eq!(*resolved.lock().unwrap(), vec![tenant()]);
}

/// Tenant-local commit identifiers can coincide while roots and signers differ.
#[tokio::test]
async fn trade_service_resolves_coincident_commit_ids_in_each_tenant() {
    use statechronicle_domain::trade::TradeSide;
    use statechronicle_proof::trade::{verify_trade_proof, verify_trade_proof_scoped};
    use std::collections::BTreeMap;

    let alpha = fixture();
    let beta_tenant = TenantId(String::from("acme.game.beta"));
    let beta_key = SigningKey::from_bytes(&[43; 32]);
    let beta = fixture_for(beta_tenant.clone(), &beta_key);
    assert_eq!(alpha.signed.body.commit_id, beta.signed.body.commit_id);
    assert_ne!(
        alpha.signed.body.next_state_root,
        beta.signed.body.next_state_root
    );

    let mut trade = TradeProof {
        schema: String::from(TRADE_PROOF_SCHEMA),
        trade_id: String::from("trade_scoped"),
        summary: TradeSummary {
            trade_id: String::from("trade_scoped"),
            status: TradeStatus::Settled,
            sides: Vec::new(),
            value_legs: Vec::new(),
        },
        legs: Vec::new(),
    };
    let commits = FakeCommitStore::default();
    let mut scoped = BTreeMap::new();
    for (fixture, signer) in [(&alpha, fixed_key()), (&beta, beta_key.clone())] {
        let mut proof = build_proof(fixture);
        // This test establishes tenant-bound signed state only. Operation
        // metadata is unsigned; it is not evidence of actual settlement.
        proof.latest_event.operation = Operation::from_static("trade.settle");
        let scope = proof.tenant_id.clone();
        trade.summary.sides.push(TradeSide {
            tenant: scope.clone(),
            settle_assets: vec![resource()],
            from_owner: String::from("unverified_previous_owner"),
            to_owner: owner(),
            settle_commit: proof.commit.clone(),
            settle_commits_by_asset: BTreeMap::from([(resource(), proof.commit.clone())]),
            settle_event_ids: vec![proof.latest_event.event_id.clone()],
        });
        trade.legs.push(TradeProofLeg {
            tenant: scope.clone(),
            commit: proof.commit.clone(),
            state_proofs: vec![proof],
        });
        commits.put_commit(&scope, &fixture.signed).await.unwrap();
        scoped.insert(
            (scope.0, fixture.signed.body.commit_id.0.clone()),
            (fixture.signed.clone(), signer.verifying_key()),
        );
    }
    verify_trade_proof_scoped(&trade, &scoped).unwrap();
    let legacy = BTreeMap::from([(
        alpha.signed.body.commit_id.0.clone(),
        (alpha.signed.clone(), fixed_key().verifying_key()),
    )]);
    assert!(matches!(
        verify_trade_proof(&trade, &legacy),
        Err(ProofError::TenantMismatch { .. })
    ));
    let beta_scope = (beta_tenant.0.clone(), beta.signed.body.commit_id.0.clone());
    let mut swapped = scoped.clone();
    swapped.get_mut(&beta_scope).unwrap().1 = fixed_key().verifying_key();
    assert!(matches!(
        verify_trade_proof_scoped(&trade, &swapped),
        Err(ProofError::CommitSignature(_))
    ));
    swapped.insert(
        beta_scope.clone(),
        (alpha.signed.clone(), fixed_key().verifying_key()),
    );
    assert!(matches!(
        verify_trade_proof_scoped(&trade, &swapped),
        Err(ProofError::TenantMismatch { .. })
    ));
    swapped.remove(&beta_scope);
    assert!(matches!(
        verify_trade_proof_scoped(&trade, &swapped),
        Err(ProofError::KeyNotFound(_))
    ));

    let service = ProofService::new(ProofPorts {
        proof_index: Box::new(FakeProofIndex::default()),
        state_index: Box::new(FakeStateIndex::default()),
        commit_store: Box::new(commits),
        snapshot_store: Box::new(FakeSnapshotStore::default()),
    });
    let resolved = Mutex::new(Vec::new());
    service
        .verify_trade(&trade, &|requested| {
            resolved.lock().unwrap().push(requested.clone());
            if requested == &tenant() {
                Some(fixed_key().verifying_key())
            } else if requested == &beta_tenant {
                Some(beta_key.verifying_key())
            } else {
                None
            }
        })
        .await
        .unwrap();
    assert_eq!(
        *resolved.lock().unwrap(),
        vec![tenant(), beta_tenant.clone()]
    );
    assert!(matches!(
        service
            .verify_trade(&trade, &|requested| {
                (requested == &tenant()).then(|| fixed_key().verifying_key())
            })
            .await,
        Err(ProofError::KeyNotFound(_))
    ));
}

#[tokio::test]
async fn commit_reader_bridge_matches_compatibility_claim_checks() {
    use statechronicle_proof::claim::{ClaimProof, ClaimVerifier, ExpectedClaim};
    let fixture = fixture();
    let proof = build_proof(&fixture);
    let valid = ExpectedClaim::ownership(tenant(), resource(), SubjectId(owner()));
    let expectations = [
        valid.clone(),
        ExpectedClaim::ownership(
            tenant(),
            resource(),
            SubjectId(String::from("account:other")),
        ),
        ExpectedClaim::state(tenant(), ResourceId(String::from("asset:other"))),
        ExpectedClaim::absence(tenant(), resource()),
    ];
    for head_mode in ["matching", "missing", "changed", "advance_during_lookup"] {
        for expected in &expectations {
            let store = FakeCommitStore::default();
            store.put_commit(&tenant(), &fixture.signed).await.unwrap();
            let mut canonical = head(&fixture.signed);
            if head_mode != "missing" {
                store
                    .heads
                    .lock()
                    .unwrap()
                    .insert(tenant().0, canonical.clone());
            }
            if head_mode == "changed" {
                canonical.sequence = canonical.sequence.saturating_add(1);
                store
                    .heads
                    .lock()
                    .unwrap()
                    .insert(tenant().0, canonical.clone());
            }
            if head_mode == "advance_during_lookup" {
                canonical.sequence = canonical.sequence.saturating_add(1);
                *store.advance_on_lookup.lock().unwrap() = Some(canonical.clone());
            }
            let service = claim_service(store.clone());
            let reader = ClaimVerifier::new(Box::new(store.clone()));
            let key = fixed_key().verifying_key();
            let old = service
                .verify_current(ClaimProof::State(&proof), expected, &key)
                .await
                .map_err(|error| error.to_string());
            let old_reads = store
                .head_reads
                .swap(0, std::sync::atomic::Ordering::Relaxed);
            // Re-arm the same head race for the second independent invocation.
            if head_mode == "advance_during_lookup" {
                store
                    .heads
                    .lock()
                    .unwrap()
                    .insert(tenant().0, head(&fixture.signed));
                *store.advance_on_lookup.lock().unwrap() = Some(canonical);
            }
            let new = reader
                .verify_current(ClaimProof::State(&proof), expected, &key)
                .await
                .map_err(|error| error.to_string());
            assert_eq!(new, old, "{head_mode}");
            assert_eq!(
                store
                    .head_reads
                    .swap(0, std::sync::atomic::Ordering::Relaxed),
                old_reads,
                "{head_mode}"
            );
            let historical_old = service
                .verify_historical(ClaimProof::State(&proof), expected, &commit_id(), &key)
                .await
                .map_err(|error| error.to_string());
            let historical_new = reader
                .verify_historical(ClaimProof::State(&proof), expected, &commit_id(), &key)
                .await
                .map_err(|error| error.to_string());
            assert_eq!(historical_new, historical_old);
            assert_eq!(
                store.head_reads.load(std::sync::atomic::Ordering::Relaxed),
                0
            );
        }
    }
}
