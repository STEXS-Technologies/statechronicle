//! Conformance tests for the `statechronicle-ports` trait surfaces.
//!
//! The ports crate declares traits only. There are no implementations, so
//! these tests prove the *shape* of every port:
//!
//! - every trait is dyn-compatible (`&dyn Port` coercion compiles),
//! - every `#[make(Send)]` trait object is `Send`, and its methods yield
//!   `Send` futures,
//! - every port's methods are callable through an async runtime and return
//!   the expected error / `None` from a minimal in-memory dummy.

#![allow(clippy::panic, clippy::unwrap_used)]

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use statechronicle_accumulator::key::StateKey;
use statechronicle_core::digest::{ContentDigest, hash_bytes};
use statechronicle_core::signature::Signature;
use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle_domain::event::Event;
use statechronicle_domain::ids::{CommitId, EventId, IntentId, SnapshotId};
use statechronicle_domain::intent::{
    Intent, KeyId, Nonce, Operation, SignatureAlg, SignatureBlock,
};
use statechronicle_domain::proof::{
    NonMembershipProofBundle, ResourceStateProof, SparseMerkleProof,
};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_ports::commit_store::{CommitStore, CommitStoreError};
use statechronicle_ports::event_publisher::{EventPublisher, EventPublisherError};
use statechronicle_ports::event_store::{EventStore, EventStoreError};
use statechronicle_ports::intent_store::{IntentStore, IntentStoreError};
use statechronicle_ports::proof_index::{ProofIndex, ProofIndexError};
use statechronicle_ports::snapshot_store::{SnapshotStore, SnapshotStoreError};
use statechronicle_ports::state_index::{StateIndex, StateIndexError};
use statechronicle_ports::tenant_store::{TenantStore, TenantStoreError};
use statechronicle_ports::transaction_manager::{
    TransactionHandle, TransactionManager, TransactionManagerError,
};

// ---------------------------------------------------------------------------
// Sample values shared by the dummy calls.
// ---------------------------------------------------------------------------

fn tenant() -> TenantId {
    TenantId(String::from("acme.game.alpha"))
}

fn resource() -> ResourceId {
    ResourceId(String::from("asset:sword_001"))
}

fn subject() -> SubjectId {
    SubjectId(String::from("account:example:player_123"))
}

fn intent_id() -> IntentId {
    IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap()
}

fn event_id() -> EventId {
    EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap()
}

fn commit_id() -> CommitId {
    CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap()
}

fn snapshot_id() -> SnapshotId {
    SnapshotId::new(String::from("snp_01JZ8X9P4DC6YC4K1YZEJX45E2")).unwrap()
}

fn timestamp() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn sample_intent() -> Intent {
    Intent::builder()
        .tenant(tenant())
        .intent_id(intent_id())
        .operation(Operation::new(String::from("asset.transfer")).unwrap())
        .actor(subject())
        .resource(resource())
        .expected_version(41)
        .inputs(BTreeMap::new())
        .created_at(timestamp())
        .nonce(Nonce::from_bytes(vec![1, 2, 3, 4]).unwrap())
        .build()
        .unwrap()
}

fn sample_signed_commit() -> Signed<Commit> {
    let commit = Commit::new(
        CommitScope::tenant(tenant()),
        commit_id(),
        None,
        1,
        1,
        hash_bytes(b"event-root"),
        hash_bytes(b"previous-root"),
        hash_bytes(b"next-root"),
        timestamp(),
        subject(),
        ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
    );
    let signature = SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
        sig: Signature::from_bytes([0u8; 64]),
    };
    Signed::new(commit, signature)
}

// ---------------------------------------------------------------------------
// Minimal in-memory dummies proving the generated method shapes are usable.
// ---------------------------------------------------------------------------

struct DummyIntentStore;

#[async_trait]
impl IntentStore for DummyIntentStore {
    async fn put_intent(
        &self,
        _tenant: &TenantId,
        _intent: &Intent,
    ) -> Result<(), IntentStoreError> {
        Err(IntentStoreError::Duplicate)
    }

    async fn get_intent(
        &self,
        _tenant: &TenantId,
        _intent_id: &IntentId,
    ) -> Result<Option<Intent>, IntentStoreError> {
        Ok(None)
    }
}

struct DummyEventStore;

#[async_trait]
impl EventStore for DummyEventStore {
    async fn append_events(
        &self,
        _tenant: &TenantId,
        _events: &[Event],
    ) -> Result<(), EventStoreError> {
        Err(EventStoreError::DuplicateEventId)
    }

    async fn events_for_resource(
        &self,
        _tenant: &TenantId,
        _resource_id: &ResourceId,
    ) -> Result<Vec<Event>, EventStoreError> {
        Ok(Vec::new())
    }

    async fn event_by_id(
        &self,
        _tenant: &TenantId,
        _event_id: &EventId,
    ) -> Result<Option<Event>, EventStoreError> {
        Ok(None)
    }
}

struct DummyCommitStore;

#[async_trait]
impl CommitStore for DummyCommitStore {
    async fn put_commit(
        &self,
        _tenant: &TenantId,
        _commit: &Signed<Commit>,
    ) -> Result<(), CommitStoreError> {
        Err(CommitStoreError::Duplicate)
    }

    async fn commit_by_id(
        &self,
        _tenant: &TenantId,
        _commit_id: &CommitId,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        Ok(None)
    }

    async fn commit_by_sequence(
        &self,
        _tenant: &TenantId,
        _sequence: u64,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        Ok(None)
    }
}

struct DummyStateIndex;

#[async_trait]
impl StateIndex for DummyStateIndex {
    async fn get_state(
        &self,
        _tenant: &TenantId,
        _resource_id: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        Err(StateIndexError::Inconsistent(String::from("dummy")))
    }

    async fn get_subject_state(
        &self,
        _tenant: &TenantId,
        _subject: &SubjectId,
        _resource_id: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        Ok(None)
    }
}

struct DummyProofIndex;

#[async_trait]
impl ProofIndex for DummyProofIndex {
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
        _event_id: &EventId,
        _commit_id: &CommitId,
    ) -> Result<Option<SparseMerkleProof>, ProofIndexError> {
        Err(ProofIndexError::NotFound)
    }

    async fn get_non_membership_proof(
        &self,
        _tenant: &TenantId,
        _resource_id: &ResourceId,
        _key: StateKey,
        _at: Option<&CommitId>,
    ) -> Result<Option<NonMembershipProofBundle>, ProofIndexError> {
        Ok(None)
    }
}

struct DummySnapshotStore;

#[async_trait]
impl SnapshotStore for DummySnapshotStore {
    async fn put_snapshot(
        &self,
        _tenant: &TenantId,
        _snapshot_id: &SnapshotId,
        _payload: Vec<u8>,
    ) -> Result<(), SnapshotStoreError> {
        Err(SnapshotStoreError::Unavailable(String::from("dummy")))
    }

    async fn get_snapshot(
        &self,
        _tenant: &TenantId,
        _snapshot_id: &SnapshotId,
    ) -> Result<Option<Vec<u8>>, SnapshotStoreError> {
        Ok(None)
    }
}

struct DummyTenantStore;

#[async_trait]
impl TenantStore for DummyTenantStore {
    async fn tenant_exists(&self, _tenant: &TenantId) -> Result<bool, TenantStoreError> {
        Ok(true)
    }

    async fn get_tenant_root(
        &self,
        _tenant: &TenantId,
    ) -> Result<Option<ContentDigest>, TenantStoreError> {
        Ok(None)
    }
}

struct DummyTransactionHandle;

#[async_trait]
impl TransactionHandle for DummyTransactionHandle {
    async fn commit(self: Box<Self>) -> Result<(), TransactionManagerError> {
        Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<(), TransactionManagerError> {
        Err(TransactionManagerError::RollbackFailed(String::from(
            "dummy",
        )))
    }
}

struct DummyTransactionManager;

#[async_trait]
impl TransactionManager for DummyTransactionManager {
    async fn begin(
        &self,
        _tenant: &TenantId,
    ) -> Result<Box<dyn TransactionHandle + Send>, TransactionManagerError> {
        Ok(Box::new(DummyTransactionHandle))
    }

    async fn begin_multi(
        &self,
        _tenants: &[TenantId],
    ) -> Result<Box<dyn TransactionHandle + Send>, TransactionManagerError> {
        Ok(Box::new(DummyTransactionHandle))
    }
}

struct DummyEventPublisher;

#[async_trait]
impl EventPublisher for DummyEventPublisher {
    async fn publish_events(
        &self,
        _tenant: &TenantId,
        _events: &[Event],
    ) -> Result<(), EventPublisherError> {
        Err(EventPublisherError::Rejected(String::from("dummy")))
    }

    async fn publish_commit(
        &self,
        _tenant: &TenantId,
        _commit: &Signed<Commit>,
    ) -> Result<(), EventPublisherError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Dyn-compatibility: every port coerces to its `dyn` trait object.
// ---------------------------------------------------------------------------

#[test]
fn all_ports_are_dyn_compatible() {
    fn takes_intent_store(_: &dyn IntentStore) {}
    fn takes_event_store(_: &dyn EventStore) {}
    fn takes_commit_store(_: &dyn CommitStore) {}
    fn takes_state_index(_: &dyn StateIndex) {}
    fn takes_proof_index(_: &dyn ProofIndex) {}
    fn takes_snapshot_store(_: &dyn SnapshotStore) {}
    fn takes_tenant_store(_: &dyn TenantStore) {}
    fn takes_transaction_manager(_: &dyn TransactionManager) {}
    fn takes_transaction_handle(_: &dyn TransactionHandle) {}
    fn takes_event_publisher(_: &dyn EventPublisher) {}

    takes_intent_store(&DummyIntentStore);
    takes_event_store(&DummyEventStore);
    takes_commit_store(&DummyCommitStore);
    takes_state_index(&DummyStateIndex);
    takes_proof_index(&DummyProofIndex);
    takes_snapshot_store(&DummySnapshotStore);
    takes_tenant_store(&DummyTenantStore);
    takes_transaction_manager(&DummyTransactionManager);
    takes_transaction_handle(&DummyTransactionHandle);
    takes_event_publisher(&DummyEventPublisher);
}

// ---------------------------------------------------------------------------
// Send-ness: the `#[make(Send)]` trait objects are `Send`, and the futures
// their methods yield are `Send`.
// ---------------------------------------------------------------------------

#[test]
fn port_trait_objects_are_send() {
    fn assert_send<T: Send>() {}

    // `Box<dyn Trait + Send>` is the unit that gets moved across tasks; it is
    // `Send` exactly when the port's trait object carries the `Send` bound.
    assert_send::<Box<dyn IntentStore + Send>>();
    assert_send::<Box<dyn EventStore + Send>>();
    assert_send::<Box<dyn CommitStore + Send>>();
    assert_send::<Box<dyn StateIndex + Send>>();
    assert_send::<Box<dyn ProofIndex + Send>>();
    assert_send::<Box<dyn SnapshotStore + Send>>();
    assert_send::<Box<dyn TenantStore + Send>>();
    assert_send::<Box<dyn TransactionManager + Send>>();
    assert_send::<Box<dyn TransactionHandle + Send>>();
    assert_send::<Box<dyn EventPublisher + Send>>();
}

#[test]
fn port_method_futures_are_send() {
    fn assert_send<T: Send>(_: T) {}

    assert_send(DummyIntentStore.put_intent(&tenant(), &sample_intent()));
    assert_send(DummyEventStore.events_for_resource(&tenant(), &resource()));
    assert_send(DummyCommitStore.commit_by_id(&tenant(), &commit_id()));
    assert_send(DummyStateIndex.get_state(&tenant(), &resource()));
    assert_send(DummyProofIndex.get_state_proof(&tenant(), &resource(), None));
    assert_send(DummySnapshotStore.get_snapshot(&tenant(), &snapshot_id()));
    assert_send(DummyTenantStore.get_tenant_root(&tenant()));
    assert_send(DummyTransactionManager.begin(&tenant()));
    assert_send(DummyTransactionManager.begin_multi(&[tenant()]));
    assert_send(DummyEventPublisher.publish_commit(&tenant(), &sample_signed_commit()));
}

// ---------------------------------------------------------------------------
// Smoke calls: each port's methods are callable through an async runtime and
// return the expected error / `None`.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn intent_store_smoke() {
    let store = DummyIntentStore;
    assert!(matches!(
        store.put_intent(&tenant(), &sample_intent()).await,
        Err(IntentStoreError::Duplicate)
    ));
    assert!(matches!(
        store.get_intent(&tenant(), &intent_id()).await,
        Ok(None)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn event_store_smoke() {
    let store = DummyEventStore;
    assert!(matches!(
        store.append_events(&tenant(), &[]).await,
        Err(EventStoreError::DuplicateEventId)
    ));
    assert!(matches!(
        store.events_for_resource(&tenant(), &resource()).await,
        Ok(events) if events.is_empty()
    ));
    assert!(matches!(
        store.event_by_id(&tenant(), &event_id()).await,
        Ok(None)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn commit_store_smoke() {
    let store = DummyCommitStore;
    assert!(matches!(
        store.put_commit(&tenant(), &sample_signed_commit()).await,
        Err(CommitStoreError::Duplicate)
    ));
    assert!(matches!(
        store.commit_by_id(&tenant(), &commit_id()).await,
        Ok(None)
    ));
    assert!(matches!(
        store.commit_by_sequence(&tenant(), 1).await,
        Ok(None)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn state_index_smoke() {
    let index = DummyStateIndex;
    assert!(matches!(
        index.get_state(&tenant(), &resource()).await,
        Err(StateIndexError::Inconsistent(_))
    ));
    assert!(matches!(
        index
            .get_subject_state(&tenant(), &subject(), &resource())
            .await,
        Ok(None)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn proof_index_smoke() {
    let index = DummyProofIndex;
    assert!(matches!(
        index.get_state_proof(&tenant(), &resource(), None).await,
        Ok(None)
    ));
    assert!(matches!(
        index
            .get_ownership_proof(&tenant(), &resource(), &subject(), None)
            .await,
        Ok(None)
    ));
    assert!(matches!(
        index
            .get_inclusion_proof(&tenant(), &event_id(), &commit_id())
            .await,
        Err(ProofIndexError::NotFound)
    ));
    assert!(matches!(
        index
            .get_non_membership_proof(&tenant(), &resource(), StateKey::new([0u8; 32]), None)
            .await,
        Ok(None)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn snapshot_store_smoke() {
    let store = DummySnapshotStore;
    assert!(matches!(
        store
            .put_snapshot(&tenant(), &snapshot_id(), Vec::new())
            .await,
        Err(SnapshotStoreError::Unavailable(_))
    ));
    assert!(matches!(
        store.get_snapshot(&tenant(), &snapshot_id()).await,
        Ok(None)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn tenant_store_smoke() {
    let store = DummyTenantStore;
    assert!(matches!(store.tenant_exists(&tenant()).await, Ok(true)));
    assert!(matches!(store.get_tenant_root(&tenant()).await, Ok(None)));
}

#[tokio::test(flavor = "current_thread")]
async fn transaction_manager_smoke() {
    let manager = DummyTransactionManager;

    let handle = manager.begin(&tenant()).await.unwrap();
    assert!(handle.commit().await.is_ok());

    let rollback_handle = manager.begin(&tenant()).await.unwrap();
    assert!(matches!(
        rollback_handle.rollback().await,
        Err(TransactionManagerError::RollbackFailed(_))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn transaction_manager_begin_multi_smoke() {
    let manager = DummyTransactionManager;
    let tenants = vec![
        TenantId(String::from("acme.game.alpha")),
        TenantId(String::from("acme.game.beta")),
    ];

    let handle = manager.begin_multi(&tenants).await.unwrap();
    assert!(handle.commit().await.is_ok());

    let rollback_handle = manager.begin_multi(&tenants).await.unwrap();
    assert!(matches!(
        rollback_handle.rollback().await,
        Err(TransactionManagerError::RollbackFailed(_))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn event_publisher_smoke() {
    let publisher = DummyEventPublisher;
    assert!(matches!(
        publisher.publish_events(&tenant(), &[]).await,
        Err(EventPublisherError::Rejected(_))
    ));
    assert!(
        publisher
            .publish_commit(&tenant(), &sample_signed_commit())
            .await
            .is_ok()
    );
}
