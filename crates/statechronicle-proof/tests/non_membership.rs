//! Integration tests for the non-membership proof lane (protocol §16.2, §29).
//!
//! Exercises absence proofs end-to-end over real domain types: accumulator
//! with a PRESENT resource → non-membership proof for an absent key → bundle
//! assembly → core + full-bundle verification (protocol §16.3, §29
//! "verifying absence"), plus the async [`ProofService`] through in-memory
//! port fakes.
//!
//! The absence claims locked here:
//! - a genuine absent-key bundle verifies against the enclosing signed commit,
//! - every fail-closed gate rejects its specific tamper (schema, kind, path
//!   length, key, leaf, root, tenant, commit ref, signature),
//! - a PRESENT key fails closed via the empty-leaf assertion even though the
//!   accumulator's own non-membership verifier does not assert it,
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

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{
    EMPTY_LEAF_HASH, StateAccumulator, StateRoot, StateUpdate,
};

use statechronicle_core::digest::ContentDigest;

use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle_domain::ids::CommitId;
use statechronicle_domain::intent::KeyId;
use statechronicle_domain::proof::{NonMembershipProofBundle, ResourceStateProof};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_ports::commit_store::{CanonicalHead, CommitStore, CommitStoreError};
use statechronicle_ports::proof_index::{ProofIndex, ProofIndexError};
use statechronicle_ports::snapshot_store::{SnapshotStore, SnapshotStoreError};
use statechronicle_ports::state_index::{StateIndex, StateIndexError};

use statechronicle_commit::sign::sign_commit;

use statechronicle_proof::bundle::build_non_membership_proof;
use statechronicle_proof::error::ProofError;
use statechronicle_proof::service::{ProofPorts, ProofService};
use statechronicle_proof::verify::{verify_non_membership, verify_non_membership_bundle};

const FIXED_SEED: [u8; 32] = [42u8; 32];

fn tenant() -> TenantId {
    TenantId(String::from("acme.game.alpha"))
}

fn resource() -> ResourceId {
    ResourceId(String::from("asset:sword_001"))
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

fn present_key() -> StateKey {
    StateKey::for_resource(&tenant().0, &resource().0)
}

const fn absent_key() -> StateKey {
    StateKey::new([0xabu8; 32])
}
/// Builds a committed accumulator holding the PRESENT resource, returns the
/// signed commit pinning its root, the root, and a genuine non-membership
/// proof for the absent key plus the assembled bundle.
struct Fixture {
    signed: Signed<Commit>,
    root: StateRoot,
    bundle: NonMembershipProofBundle,
}

fn fixture() -> Fixture {
    fixture_at(absent_key(), resource())
}

fn fixture_at(absent: StateKey, absent_resource: ResourceId) -> Fixture {
    let present = present_key();
    let mut acc = StateAccumulator::empty();
    acc.insert_batch(&[StateUpdate::new(present, [0xa1u8; 32])])
        .unwrap();
    let root = acc.root();
    let non_membership = acc.prove_non_membership(&absent).unwrap();

    let commit = Commit::new(
        CommitScope::tenant(tenant()),
        commit_id(),
        None,
        1,
        1,
        ContentDigest::new([0x01u8; 32]),
        ContentDigest::new([0x02u8; 32]),
        ContentDigest::new(*root.as_bytes()),
        timestamp(),
        executor(),
        profile(),
    );
    let signed = sign_commit(&commit, &fixed_key(), key_id()).unwrap();

    let bundle = build_non_membership_proof(
        &tenant(),
        &absent_resource,
        &absent,
        &signed,
        &non_membership,
    )
    .unwrap();

    Fixture {
        signed,
        root,
        bundle,
    }
}

// ---------------------------------------------------------------------------
// Pure verification pipeline (protocol §16.3, §29 "verifying absence").
// ---------------------------------------------------------------------------

#[test]
fn genuine_absent_key_bundle_verifies_end_to_end() {
    let fixture = fixture();

    assert_eq!(
        fixture.bundle.schema,
        "statechronicle.proof.non_membership.v0"
    );
    assert_eq!(fixture.bundle.tenant_id, tenant());
    assert_eq!(fixture.bundle.resource_id, resource());
    assert_eq!(
        fixture.bundle.claimed_key.as_bytes(),
        absent_key().as_bytes()
    );
    assert_eq!(
        fixture.bundle.commit.commit_id,
        fixture.signed.body.commit_id
    );
    assert_eq!(
        fixture.bundle.commit.state_root,
        fixture.signed.body.next_state_root
    );
    assert_eq!(
        fixture.bundle.state_non_membership_proof.kind,
        statechronicle_domain::proof::SPARSE_MERKLE_V0
    );
    assert_eq!(
        fixture
            .bundle
            .state_non_membership_proof
            .leaf_hash
            .as_bytes(),
        &EMPTY_LEAF_HASH
    );

    assert!(verify_non_membership(&fixture.bundle, &fixture.root, &absent_key()).is_ok());
    assert!(
        verify_non_membership_bundle(
            &fixture.bundle,
            &fixture.signed,
            &fixed_key().verifying_key(),
            &absent_key()
        )
        .is_ok()
    );
}

#[test]
fn tampered_path_entry_is_rejected() {
    let fixture = fixture();
    let mut bundle = fixture.bundle.clone();
    bundle.state_non_membership_proof.path[0] = ContentDigest::new([0x5au8; 32]);
    assert!(matches!(
        verify_non_membership(&bundle, &fixture.root, &absent_key()),
        Err(ProofError::InclusionMismatch)
    ));
}

#[test]
fn wrong_root_is_rejected() {
    let fixture = fixture();
    let wrong_root = StateRoot::new([0x5au8; 32]);
    assert!(matches!(
        verify_non_membership(&fixture.bundle, &wrong_root, &absent_key()),
        Err(ProofError::InclusionMismatch)
    ));
}

#[test]
fn wrong_schema_is_rejected() {
    let fixture = fixture();
    let mut bundle = fixture.bundle.clone();
    bundle.schema = String::from("statechronicle.proof.non_membership.v9");
    assert!(matches!(
        verify_non_membership(&bundle, &fixture.root, &absent_key()),
        Err(ProofError::UnsupportedSchema(_))
    ));
}

#[test]
fn wrong_kind_is_rejected() {
    let fixture = fixture();
    let mut bundle = fixture.bundle.clone();
    bundle.state_non_membership_proof.kind = String::from("jellyfish_v1");
    assert!(matches!(
        verify_non_membership(&bundle, &fixture.root, &absent_key()),
        Err(ProofError::UnsupportedKind(_))
    ));
}

#[test]
fn wrong_path_length_is_rejected() {
    let fixture = fixture();
    let mut bundle = fixture.bundle.clone();
    bundle.state_non_membership_proof.path.truncate(2);
    assert!(matches!(
        verify_non_membership(&bundle, &fixture.root, &absent_key()),
        Err(ProofError::InvalidPathLength {
            expected: 256,
            actual: 2
        })
    ));
}

#[test]
fn present_key_fails_closed_via_empty_leaf_assertion() {
    // The accumulator's `verify_non_membership` does NOT assert the empty
    // leaf, so an inclusion proof of a PRESENT key smuggled in as a
    // "non-membership" bundle (leaf != EMPTY_LEAF_HASH) must be rejected by
    // the bundle verifier's load-bearing empty-leaf gate.
    let present = present_key();
    let mut acc = StateAccumulator::empty();
    acc.insert_batch(&[StateUpdate::new(present, [0xa1u8; 32])])
        .unwrap();
    let root = acc.root();
    let inclusion = acc.prove_inclusion(&present).unwrap();
    let sparse = statechronicle_proof::inclusion::sparse_proof_from_inclusion(&inclusion);
    let signed = signed_for_root(root);

    let bundle = NonMembershipProofBundle::new(
        tenant(),
        resource(),
        ContentDigest::new(*present.as_bytes()),
        statechronicle_domain::proof::CommitRef {
            commit_id: signed.body.commit_id.clone(),
            sequence: signed.body.sequence,
            state_root: signed.body.next_state_root.clone(),
            signature: signed.signature.clone(),
        },
        sparse,
    );
    assert_ne!(
        bundle.state_non_membership_proof.leaf_hash.as_bytes(),
        &EMPTY_LEAF_HASH
    );
    assert!(matches!(
        verify_non_membership(&bundle, &root, &present),
        Err(ProofError::NonMembershipLeafMismatch)
    ));
}

#[test]
fn wrong_claimed_key_vs_supplied_key_is_rejected() {
    let fixture = fixture();
    let other = StateKey::new([0xbbu8; 32]);
    assert!(matches!(
        verify_non_membership(&fixture.bundle, &fixture.root, &other),
        Err(ProofError::KeyMismatch { .. })
    ));
}

#[test]
fn tenant_mismatch_is_rejected() {
    let fixture = fixture();
    let mut bundle = fixture.bundle.clone();
    bundle.tenant_id = TenantId(String::from("acme.game.beta"));
    assert!(matches!(
        verify_non_membership_bundle(
            &bundle,
            &fixture.signed,
            &fixed_key().verifying_key(),
            &absent_key()
        ),
        Err(ProofError::TenantMismatch { .. })
    ));
}

#[test]
fn commit_ref_sequence_tamper_is_rejected() {
    let fixture = fixture();
    let mut bundle = fixture.bundle.clone();
    bundle.commit.sequence = bundle.commit.sequence.wrapping_add(1);
    assert!(matches!(
        verify_non_membership_bundle(
            &bundle,
            &fixture.signed,
            &fixed_key().verifying_key(),
            &absent_key()
        ),
        Err(ProofError::CommitRefMismatch { .. })
    ));
}

#[test]
fn stored_commit_body_tamper_is_rejected() {
    let fixture = fixture();
    let mut tampered = fixture.signed.clone();
    tampered.body.created_at = tampered
        .body
        .created_at
        .checked_add_signed(chrono::Duration::seconds(1))
        .unwrap();
    assert!(matches!(
        verify_non_membership_bundle(
            &fixture.bundle,
            &tampered,
            &fixed_key().verifying_key(),
            &absent_key()
        ),
        Err(ProofError::CommitSignature(_))
    ));
}

// ---------------------------------------------------------------------------
// Async ProofService over in-memory port fakes.
// ---------------------------------------------------------------------------

fn signed_for_root(root: StateRoot) -> Signed<Commit> {
    let commit = Commit::new(
        CommitScope::tenant(tenant()),
        commit_id(),
        None,
        1,
        1,
        ContentDigest::new([0x01u8; 32]),
        ContentDigest::new([0x02u8; 32]),
        ContentDigest::new(*root.as_bytes()),
        timestamp(),
        executor(),
        profile(),
    );
    sign_commit(&commit, &fixed_key(), key_id()).unwrap()
}

/// A stored commit keyed by its tenant scope.
type StoredCommit = (TenantId, Signed<Commit>);

#[derive(Clone, Default)]
struct FakeCommitStore {
    inner: Arc<Mutex<Vec<StoredCommit>>>,
    head: Arc<Mutex<Option<CanonicalHead>>>,
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

    async fn canonical_head(
        &self,
        requested: &TenantId,
    ) -> Result<Option<CanonicalHead>, CommitStoreError> {
        assert_eq!(requested, &tenant());
        Ok(self.head.lock().unwrap().clone())
    }
}

#[derive(Clone, Default)]
struct FakeProofIndex {
    non_membership_proof: Arc<Mutex<Option<NonMembershipProofBundle>>>,
}

impl FakeProofIndex {
    fn with_non_membership_proof(bundle: NonMembershipProofBundle) -> Self {
        Self {
            non_membership_proof: Arc::new(Mutex::new(Some(bundle))),
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
        Ok(None)
    }

    async fn get_ownership_proof(
        &self,
        _tenant: &TenantId,
        _resource_id: &ResourceId,
        _subject: &SubjectId,
        _at: Option<&CommitId>,
    ) -> Result<Option<ResourceStateProof>, ProofIndexError> {
        Ok(None)
    }

    async fn get_inclusion_proof(
        &self,
        _tenant: &TenantId,
        _event_id: &statechronicle_domain::ids::EventId,
        _commit_id: &CommitId,
    ) -> Result<Option<statechronicle_domain::proof::SparseMerkleProof>, ProofIndexError> {
        Ok(None)
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
    inner: Arc<Mutex<std::collections::HashMap<(TenantId, ResourceId), StateProjection>>>,
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

#[derive(Clone, Default)]
struct FakeSnapshotStore;

#[async_trait]
impl SnapshotStore for FakeSnapshotStore {
    async fn put_snapshot(
        &self,
        _tenant: &TenantId,
        _snapshot_id: &statechronicle_domain::ids::SnapshotId,
        _payload: Vec<u8>,
    ) -> Result<(), SnapshotStoreError> {
        Ok(())
    }

    async fn get_snapshot(
        &self,
        _tenant: &TenantId,
        _snapshot_id: &statechronicle_domain::ids::SnapshotId,
    ) -> Result<Option<Vec<u8>>, SnapshotStoreError> {
        Ok(None)
    }
}

async fn service(bundle: NonMembershipProofBundle, fixture: &Fixture) -> ProofService {
    let commit_store = FakeCommitStore::default();
    commit_store
        .put_commit(&tenant(), &fixture.signed)
        .await
        .unwrap();
    ProofService::new(ProofPorts {
        proof_index: Box::new(FakeProofIndex::with_non_membership_proof(bundle)),
        state_index: Box::new(FakeStateIndex::default()),
        commit_store: Box::new(commit_store),
        snapshot_store: Box::new(FakeSnapshotStore),
    })
}

#[tokio::test]
async fn service_serves_and_verifies_non_membership() {
    let fixture = fixture();
    let service = service(fixture.bundle.clone(), &fixture).await;

    let served = service
        .get_non_membership_proof(&tenant(), &resource(), absent_key(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(served, fixture.bundle);

    service
        .verify_non_membership(&served, &fixed_key().verifying_key())
        .await
        .unwrap();
}

#[tokio::test]
async fn service_rejects_missing_commit() {
    let fixture = fixture();
    let commit_store = FakeCommitStore::default();
    let service = ProofService::new(ProofPorts {
        proof_index: Box::new(FakeProofIndex::with_non_membership_proof(
            fixture.bundle.clone(),
        )),
        state_index: Box::new(FakeStateIndex::default()),
        commit_store: Box::new(commit_store),
        snapshot_store: Box::new(FakeSnapshotStore),
    });
    assert!(matches!(
        service
            .verify_non_membership(&fixture.bundle, &fixed_key().verifying_key())
            .await,
        Err(ProofError::NotFound)
    ));
}

// ---------------------------------------------------------------------------
// Serde wire roundtrip of the bundle.
// ---------------------------------------------------------------------------

#[test]
fn bundle_roundtrips_through_json_and_bcs() {
    let fixture = fixture();

    let json = serde_json::to_string(&fixture.bundle).unwrap();
    let decoded: NonMembershipProofBundle = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, fixture.bundle);

    let first = bcs::to_bytes(&fixture.bundle).unwrap();
    let second = bcs::to_bytes(&fixture.bundle).unwrap();
    assert_eq!(first, second);
    assert!(!first.is_empty());

    let decoded_bcs: NonMembershipProofBundle = bcs::from_bytes(&first).unwrap();
    assert_eq!(decoded_bcs, fixture.bundle);
}

/// Characterizes the compatibility API before adding an expected-claim API.
/// Success proves the supplied absent key, not the caller's intended resource.
#[tokio::test]
async fn legacy_absence_service_does_not_bind_independent_key_or_current_head() {
    let fixture = fixture();
    let service = service(fixture.bundle.clone(), &fixture).await;
    assert!(
        service
            .ports()
            .commit_store
            .canonical_head(&tenant())
            .await
            .unwrap()
            .is_none()
    );
    service
        .verify_non_membership(&fixture.bundle, &fixed_key().verifying_key())
        .await
        .unwrap();
    assert!(matches!(
        verify_non_membership_bundle(
            &fixture.bundle,
            &fixture.signed,
            &fixed_key().verifying_key(),
            &present_key()
        ),
        Err(ProofError::KeyMismatch { .. })
    ));
}

fn claim_service(store: FakeCommitStore) -> ProofService {
    ProofService::new(ProofPorts {
        proof_index: Box::new(FakeProofIndex::default()),
        state_index: Box::new(FakeStateIndex::default()),
        commit_store: Box::new(store),
        snapshot_store: Box::new(FakeSnapshotStore),
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
async fn independent_absence_claim_rejects_swapped_keys_metadata_and_kind() {
    use statechronicle_proof::claim::{ClaimProof, ExpectedClaim, VerificationPoint};
    let missing = ResourceId(String::from("asset:missing"));
    let expected = ExpectedClaim::absence(tenant(), missing.clone());
    let fixture = fixture_at(expected.key(), missing);
    let store = FakeCommitStore::default();
    store.put_commit(&tenant(), &fixture.signed).await.unwrap();
    let service = claim_service(store.clone());
    let trusted = fixed_key().verifying_key();
    let historical = service
        .verify_historical(
            ClaimProof::Absence(&fixture.bundle),
            &expected,
            &commit_id(),
            &trusted,
        )
        .await
        .unwrap();
    assert_eq!(historical.point(), VerificationPoint::Historical);
    assert_eq!(historical.expectation(), &expected);
    assert!(historical.expectation().is_absence());
    assert_eq!(historical.commit(), &fixture.bundle.commit);
    assert!(historical.state().is_none());
    assert!(matches!(
        service
            .verify_current(ClaimProof::Absence(&fixture.bundle), &expected, &trusted)
            .await,
        Err(ProofError::CanonicalHeadUnavailable)
    ));
    *store.head.lock().unwrap() = Some(head(&fixture.signed));
    let current = service
        .verify_current(ClaimProof::Absence(&fixture.bundle), &expected, &trusted)
        .await
        .unwrap();
    assert_eq!(current.point(), VerificationPoint::Current);
    assert!(matches!(
        service
            .verify_current(
                ClaimProof::Absence(&fixture.bundle),
                &expected,
                &SigningKey::from_bytes(&[43; 32]).verifying_key()
            )
            .await,
        Err(ProofError::CommitSignature(_))
    ));
    let mut swapped = fixture_at(
        StateKey::for_resource(&tenant().0, "asset:other"),
        ResourceId(String::from("asset:other")),
    )
    .bundle;
    // Both absences genuinely verify under the same signed root. Neither
    // swapping the bundle nor relabeling its unsigned resource can prove A.
    assert!(matches!(
        service
            .verify_current(ClaimProof::Absence(&swapped), &expected, &trusted)
            .await,
        Err(ProofError::ResourceMismatch { .. })
    ));
    swapped.resource_id = expected.resource().clone();
    assert!(matches!(
        service
            .verify_current(ClaimProof::Absence(&swapped), &expected, &trusted)
            .await,
        Err(ProofError::KeyMismatch { .. })
    ));
    swapped = fixture.bundle.clone();
    swapped.tenant_id = TenantId(String::from("acme.game.beta"));
    assert!(matches!(
        service
            .verify_current(ClaimProof::Absence(&swapped), &expected, &trusted)
            .await,
        Err(ProofError::TenantMismatch { .. })
    ));
    let presence = ExpectedClaim::state(tenant(), expected.resource().clone());
    assert!(matches!(
        service
            .verify_current(ClaimProof::Absence(&fixture.bundle), &presence, &trusted)
            .await,
        Err(ProofError::InvalidState(_))
    ));
}

#[tokio::test]
async fn historical_absence_survives_creation_but_cannot_prove_current_absence() {
    use statechronicle_proof::claim::{ClaimProof, ExpectedClaim};
    let resource = ResourceId(String::from("asset:created_later"));
    let expected = ExpectedClaim::absence(tenant(), resource.clone());
    let fixture = fixture_at(expected.key(), resource);
    let store = FakeCommitStore::default();
    store.put_commit(&tenant(), &fixture.signed).await.unwrap();
    let mut created = StateAccumulator::empty();
    created
        .insert_batch(&[
            StateUpdate::new(present_key(), [0xa1; 32]),
            StateUpdate::new(expected.key(), [0xb2; 32]),
        ])
        .unwrap();
    let next_commit = Commit::new(
        CommitScope::tenant(tenant()),
        CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF6")).unwrap(),
        Some(commit_id()),
        2,
        1,
        ContentDigest::new([3; 32]),
        fixture.signed.body.next_state_root.clone(),
        ContentDigest::new(*created.root().as_bytes()),
        timestamp(),
        executor(),
        profile(),
    );
    let signed = sign_commit(&next_commit, &fixed_key(), key_id()).unwrap();
    store.put_commit(&tenant(), &signed).await.unwrap();
    *store.head.lock().unwrap() = Some(head(&signed));
    let service = claim_service(store);
    assert!(matches!(
        service
            .verify_current(
                ClaimProof::Absence(&fixture.bundle),
                &expected,
                &fixed_key().verifying_key()
            )
            .await,
        Err(ProofError::NonCanonicalCommit { .. })
    ));
    service
        .verify_historical(
            ClaimProof::Absence(&fixture.bundle),
            &expected,
            &commit_id(),
            &fixed_key().verifying_key(),
        )
        .await
        .unwrap();
    // Replacing the commit reference cannot carry an old empty path into the
    // new root where this address is present.
    let mut forged = fixture.bundle;
    forged.commit = statechronicle_proof::bundle::commit_ref(&signed).unwrap();
    assert!(
        service
            .verify_current(
                ClaimProof::Absence(&forged),
                &expected,
                &fixed_key().verifying_key()
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn independent_subject_absence_binds_the_requested_holder() {
    use statechronicle_proof::claim::{ClaimProof, ExpectedClaim};
    let holder = SubjectId(String::from("account:holder"));
    let expected = ExpectedClaim::subject_absence(tenant(), resource(), holder.clone());
    let fixture = fixture_at(expected.key(), resource());
    let store = FakeCommitStore::default();
    store.put_commit(&tenant(), &fixture.signed).await.unwrap();
    *store.head.lock().unwrap() = Some(head(&fixture.signed));
    let service = claim_service(store);
    assert_eq!(expected.subject(), Some(&holder));
    service
        .verify_current(
            ClaimProof::Absence(&fixture.bundle),
            &expected,
            &fixed_key().verifying_key(),
        )
        .await
        .unwrap();
    let other = ExpectedClaim::subject_absence(
        tenant(),
        resource(),
        SubjectId(String::from("account:other")),
    );
    assert!(matches!(
        service
            .verify_current(
                ClaimProof::Absence(&fixture.bundle),
                &other,
                &fixed_key().verifying_key()
            )
            .await,
        Err(ProofError::KeyMismatch { .. })
    ));
}
