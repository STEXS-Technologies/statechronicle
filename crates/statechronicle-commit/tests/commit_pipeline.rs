//! Integration tests for the commit pipeline.
//!
//! Exercises the end-to-end path over real domain types: batch formation →
//! event Merkle root + state root computation → commit body assembly → Ed25519
//! signing → signature verification → persistence through in-memory fakes.
//! Also covers state-root continuity across two sequential commits (protocol
//! §14: `previous_state_root + ordered events = next_state_root`).

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::hash_bytes;

use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::{CommitId, EventId, IntentId};
use statechronicle_domain::intent::{KeyId, Operation};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_ports::commit_store::{CommitStore, CommitStoreError};
use statechronicle_ports::event_publisher::{EventPublisher, EventPublisherError};
use statechronicle_ports::event_store::{EventStore, EventStoreError};
use statechronicle_ports::state_index::{StateIndex, StateIndexError};

use statechronicle_commit::batch::CommitBatch;
use statechronicle_commit::builder::CommitBuilder;
use statechronicle_commit::error::CommitError;
use statechronicle_commit::persist::{CommitPorts, CommittedEvent, persist, projections_for};
use statechronicle_commit::roots::{
    compute_state_root, event_root, state_root_updates, verify_state_root_continuity,
};
use statechronicle_commit::sign::{sign_commit, verify_commit};

const FIXED_SEED: [u8; 32] = [42u8; 32];

fn tenant() -> TenantId {
    TenantId(String::from("stexs.game.alpha"))
}

fn executor() -> SubjectId {
    SubjectId(String::from("service:statechronicle.stexs.net"))
}

fn actor() -> SubjectId {
    SubjectId(String::from("account:stexs:player_123"))
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

fn commitment(version: u64, state: serde_json::Value) -> StateCommitment {
    StateCommitment {
        version,
        state_hash: canonicalize_and_digest(&state).unwrap(),
        state,
    }
}

/// Owner-based event (UniqueAsset style payload).
fn unique_event(id: &str, owner: &str) -> Event {
    let state = serde_json::json!({ "owner": owner, "status": "active" });
    Event::new(
        tenant(),
        EventId::new(format!("evt_{id}")).unwrap(),
        IntentId::new(format!("int_{id}")).unwrap(),
        Operation::new(String::from("asset.transfer")).unwrap(),
        ResourceId(format!("asset:{id}")),
        actor(),
        commitment(1, serde_json::json!({})),
        commitment(2, state),
        None,
        executor(),
        timestamp(),
    )
}

/// Subject-held event (FungibleBalance style payload).
fn balance_event(id: &str, subject: &str) -> Event {
    let state = serde_json::json!({
        "subject": subject,
        "balance": "100",
        "unit": "gold_minor"
    });
    Event::new(
        tenant(),
        EventId::new(format!("evt_{id}")).unwrap(),
        IntentId::new(format!("int_{id}")).unwrap(),
        Operation::new(String::from("currency.transfer")).unwrap(),
        ResourceId(format!("balance:{id}")),
        SubjectId(String::from(subject)),
        commitment(1, serde_json::json!({})),
        commitment(2, state),
        None,
        executor(),
        timestamp(),
    )
}

fn batch_from(events: &[Event]) -> CommitBatch {
    let mut batch = CommitBatch::new(CommitScope::tenant(tenant()));
    for event in events {
        batch.add_event(event.clone()).unwrap();
    }
    batch
}

fn commit_id(sequence: u64) -> Result<CommitId, CommitError> {
    CommitId::new(format!("cmt_{sequence:020}")).map_err(CommitError::from)
}

fn entries_for(events: &[Event]) -> Vec<CommittedEvent<'_>> {
    events
        .iter()
        .enumerate()
        .map(|(index, event)| CommittedEvent {
            event,
            state_type: if index % 2 == 0 {
                StateType::UniqueAsset
            } else {
                StateType::FungibleBalance
            },
        })
        .collect()
}

// ---------------------------------------------------------------------------
// In-memory fakes (HashMap/Mutex-backed).
// ---------------------------------------------------------------------------

/// A stored commit keyed by its tenant scope.
type StoredCommit = (TenantId, Signed<Commit>);

/// A stored event keyed by its tenant scope.
type StoredEvent = (TenantId, Event);

/// The index key of a projection: (tenant, resource).
type ProjectionKey = (TenantId, ResourceId);

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
struct FakeEventStore {
    inner: Arc<Mutex<Vec<StoredEvent>>>,
}

#[async_trait]
impl EventStore for FakeEventStore {
    async fn append_events(
        &self,
        tenant: &TenantId,
        events: &[Event],
    ) -> Result<(), EventStoreError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|err| EventStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        for event in events {
            if inner
                .iter()
                .any(|(t, e)| t == tenant && e.event_id == event.event_id)
            {
                return Err(EventStoreError::DuplicateEventId);
            }
            inner.push((tenant.clone(), event.clone()));
        }
        Ok(())
    }

    async fn events_for_resource(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
    ) -> Result<Vec<Event>, EventStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| EventStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner
            .iter()
            .filter(|(t, e)| t == tenant && e.resource_id == *resource_id)
            .map(|(_, e)| e.clone())
            .collect())
    }

    async fn event_by_id(
        &self,
        tenant: &TenantId,
        event_id: &EventId,
    ) -> Result<Option<Event>, EventStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| EventStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner
            .iter()
            .find(|(t, e)| t == tenant && e.event_id == *event_id)
            .map(|(_, e)| e.clone()))
    }
}

#[derive(Clone, Default)]
struct FakeStateIndex {
    inner: Arc<Mutex<HashMap<ProjectionKey, StateProjection>>>,
}

impl FakeStateIndex {
    /// Composition-root index adapter: applies derived projections.
    fn apply(&self, projections: Vec<StateProjection>) {
        let mut inner = self.inner.lock().unwrap();
        for projection in projections {
            inner.insert(
                (projection.tenant_id.clone(), projection.resource_id.clone()),
                projection,
            );
        }
    }
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
struct FakeEventPublisher {
    inner: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl EventPublisher for FakeEventPublisher {
    async fn publish_events(
        &self,
        _tenant: &TenantId,
        events: &[Event],
    ) -> Result<(), EventPublisherError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|err| EventPublisherError::Unavailable(format!("lock poisoned: {err}")))?;
        for event in events {
            inner.push(format!("event:{}", event.event_id.as_str()));
        }
        Ok(())
    }

    async fn publish_commit(
        &self,
        _tenant: &TenantId,
        commit: &Signed<Commit>,
    ) -> Result<(), EventPublisherError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|err| EventPublisherError::Unavailable(format!("lock poisoned: {err}")))?;
        inner.push(format!("commit:{}", commit.body.commit_id.as_str()));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn build_sign_verify_and_persist_end_to_end() {
    let events = vec![unique_event("sword", "alice"), balance_event("gold", "bob")];
    let batch = batch_from(&events);
    let genesis_root = hash_bytes(b"genesis");

    let builder = CommitBuilder::new(
        CommitScope::tenant(tenant()),
        1,
        executor(),
        profile(),
        timestamp(),
        None,
    );
    let commit = builder
        .build(&batch, genesis_root, &[], || commit_id(1))
        .unwrap();

    // Body invariants.
    assert_eq!(commit.event_count, 2);
    assert_eq!(commit.event_merkle_root, event_root(&events).unwrap());
    let updates = state_root_updates(&events).unwrap();
    assert_eq!(
        commit.next_state_root.as_bytes(),
        compute_state_root(&updates).unwrap().as_bytes()
    );

    // Sign and verify.
    let key = fixed_key();
    let signed = sign_commit(&commit, &key, key_id()).unwrap();
    assert!(verify_commit(&signed, &key.verifying_key()).is_ok());

    // A tampered body must fail verification.
    let mut tampered = signed.clone();
    tampered.body.sequence = tampered.body.sequence.wrapping_add(1);
    assert!(verify_commit(&tampered, &key.verifying_key()).is_err());

    // The signed body round-trips through BCS (ADR-004).
    let body_bytes = bcs::to_bytes(&signed.body).unwrap();
    let decoded: Commit = bcs::from_bytes(&body_bytes).unwrap();
    assert_eq!(decoded, signed.body);

    // Persist through in-memory fakes.
    let commit_store = FakeCommitStore::default();
    let event_store = FakeEventStore::default();
    let state_index = FakeStateIndex::default();
    let publisher = FakeEventPublisher::default();
    let ports = CommitPorts {
        commit_store: Box::new(commit_store.clone()),
        event_store: Box::new(event_store.clone()),
        state_index: Box::new(state_index.clone()),
        event_publisher: Some(Box::new(publisher.clone())),
    };
    let entries = entries_for(&events);
    persist(&ports, &signed, &entries).await.unwrap();

    // Commit store contents.
    let stored = commit_store
        .commit_by_id(&tenant(), &commit.commit_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.body, commit);
    let by_sequence = commit_store
        .commit_by_sequence(&tenant(), 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_sequence.body.commit_id, commit.commit_id);

    // Event store contents.
    let stored_events = event_store
        .events_for_resource(&tenant(), &events[0].resource_id)
        .await
        .unwrap();
    assert_eq!(stored_events.len(), 1);
    assert_eq!(stored_events[0].event_id, events[0].event_id);
    let by_id = event_store
        .event_by_id(&tenant(), &events[1].event_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_id, events[1]);

    // Publisher received the events and the enclosing commit.
    {
        let published = publisher.inner.lock().unwrap();
        assert!(published.iter().any(|message| message == "event:evt_sword"));
        assert!(published.iter().any(|message| message == "event:evt_gold"));
        assert!(
            published
                .iter()
                .any(|message| message == "commit:cmt_00000000000000000001")
        );
    }

    // Composition-root index projection derived from the commit.
    let projections = projections_for(&commit.commit_id, &entries);
    assert_eq!(projections.len(), 2);
    state_index.apply(projections);
    let projected = state_index
        .get_state(&tenant(), &events[0].resource_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(projected.state_type, StateType::UniqueAsset);
    assert_eq!(projected.version, events[0].after.version);
    assert_eq!(projected.last_event_id, events[0].event_id);
    assert_eq!(projected.last_commit_id, commit.commit_id);
    assert_eq!(projected.state_hash, events[0].after.state_hash);
}

#[tokio::test]
async fn state_root_chain_across_two_commits() {
    let events1 = vec![unique_event("sword", "alice"), balance_event("gold", "bob")];
    let events2 = vec![unique_event("shield", "bob")];
    let batch1 = batch_from(&events1);
    let batch2 = batch_from(&events2);
    let genesis_root = hash_bytes(b"genesis");

    let builder = CommitBuilder::new(
        CommitScope::tenant(tenant()),
        1,
        executor(),
        profile(),
        timestamp(),
        None,
    );
    let commit1 = builder
        .build(&batch1, genesis_root, &[], || commit_id(1))
        .unwrap();
    let updates1 = state_root_updates(&events1).unwrap();
    assert_eq!(
        commit1.next_state_root.as_bytes(),
        compute_state_root(&updates1).unwrap().as_bytes()
    );

    let builder2 = CommitBuilder::new(
        CommitScope::tenant(tenant()),
        2,
        executor(),
        profile(),
        timestamp(),
        Some(commit1.commit_id.clone()),
    );
    let commit2 = builder2
        .build(&batch2, commit1.next_state_root.clone(), &updates1, || {
            commit_id(2)
        })
        .unwrap();

    // The chain links: commit 2 declares commit 1's next root as its previous.
    assert_eq!(commit2.previous_state_root, commit1.next_state_root);
    assert_eq!(
        commit2.parent_commit_id.as_ref().unwrap(),
        &commit1.commit_id
    );

    // The replay equation holds: previous + events2 = next.
    let updates2 = state_root_updates(&events2).unwrap();
    assert!(
        verify_state_root_continuity(
            &commit1.next_state_root,
            &updates1,
            &updates2,
            &commit2.next_state_root,
        )
        .is_ok()
    );

    // And it fails closed when a declared root is wrong.
    let wrong = hash_bytes(b"not-the-root");
    assert!(matches!(
        verify_state_root_continuity(&wrong, &updates1, &updates2, &commit2.next_state_root,),
        Err(CommitError::StateRootMismatch { .. })
    ));
}

#[tokio::test]
async fn persist_rejects_event_root_mismatch_without_writing() {
    let events = vec![unique_event("sword", "alice"), balance_event("gold", "bob")];
    let batch = batch_from(&events);
    let commit = CommitBuilder::new(
        CommitScope::tenant(tenant()),
        1,
        executor(),
        profile(),
        timestamp(),
        None,
    )
    .build(&batch, hash_bytes(b"genesis"), &[], || commit_id(1))
    .unwrap();
    let signed = sign_commit(&commit, &fixed_key(), key_id()).unwrap();

    let commit_store = FakeCommitStore::default();
    let event_store = FakeEventStore::default();
    let ports = CommitPorts {
        commit_store: Box::new(commit_store.clone()),
        event_store: Box::new(event_store.clone()),
        state_index: Box::new(FakeStateIndex::default()),
        event_publisher: None,
    };

    // Only one of the two declared events is supplied: fail closed.
    let partial = vec![CommittedEvent {
        event: &events[0],
        state_type: StateType::UniqueAsset,
    }];
    let error = persist(&ports, &signed, &partial).await.unwrap_err();
    assert!(matches!(error, CommitError::EventRootMismatch));

    // Nothing was written.
    assert!(
        commit_store
            .commit_by_id(&tenant(), &commit.commit_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        event_store
            .event_by_id(&tenant(), &events[0].event_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn persist_rejects_global_checkpoint_commits() {
    let events = vec![unique_event("sword", "alice")];
    // A global checkpoint commit carries tenant roots, not direct events
    // (§13.4), so persistence through the direct-event path must fail closed.
    let commit = Commit::new(
        CommitScope::global_checkpoint(),
        CommitId::new(String::from("cmt_checkpoint_001")).unwrap(),
        None,
        1,
        1,
        event_root(&events).unwrap(),
        hash_bytes(b"genesis"),
        hash_bytes(b"next"),
        timestamp(),
        executor(),
        profile(),
    );
    let signed = sign_commit(&commit, &fixed_key(), key_id()).unwrap();

    let ports = CommitPorts {
        commit_store: Box::new(FakeCommitStore::default()),
        event_store: Box::new(FakeEventStore::default()),
        state_index: Box::new(FakeStateIndex::default()),
        event_publisher: None,
    };
    let entries = entries_for(&events);
    let error = persist(&ports, &signed, &entries).await.unwrap_err();
    assert!(matches!(error, CommitError::Store(_)));
}
