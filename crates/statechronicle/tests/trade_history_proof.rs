//! Integration test: the trade read-side vertical slice (Phase 2).
//!
//! Runs a full 2-tenant asset-for-gold trade through the REAL cross-crate
//! executor, commits each tenant leg, ingests the batches through the trade
//! service, and asserts:
//!
//! (a) `get_history` returns the ordered lock/settle/value-pair events with the
//!     committing commit refs;
//! (b) `get_proof` assembles a trade proof whose alpha leg carries a genuine,
//!     verifiable state proof of the settled asset;
//! (c) `verify_trade_proof` succeeds on the genuine proof and fails closed on a
//!     tampered proof.
//!
//! The port fakes mirror the in-memory style used elsewhere in this crate.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::type_complexity
)]

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;

use statechronicle::accumulator::key::StateKey;
use statechronicle::accumulator::sparse_merkle::{StateAccumulator, StateRoot};
use statechronicle::commit::roots::state_root_updates;
use statechronicle::commit::sign::sign_commit;
use statechronicle::core::digest::{ContentDigest, hash_bytes};
use statechronicle::domain::authority::AuthorityProof;
use statechronicle::domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle::domain::event::Event;
use statechronicle::domain::ids::{CommitId, EventId, IntentId};
use statechronicle::domain::intent::{Intent, Nonce, Operation};
use statechronicle::domain::proof::ResourceStateProof;
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::signed::Signed;
use statechronicle::domain::state::StateProjection;
use statechronicle::domain::state_type::StateType;
use statechronicle::domain::subject::SubjectId;
use statechronicle::domain::tenant::TenantId;
use statechronicle::domain::trade::TradeStatus;
use statechronicle::executor::atomicity::{SettleLeg, TradeManifest, ValueLeg};
use statechronicle::index::build::IngestBatch;
use statechronicle::index::error::TradeServiceError;
use statechronicle::index::service::{TradePorts, TradeService};
use statechronicle::ports::commit_store::{CommitStore, CommitStoreError};
use statechronicle::ports::event_store::{EventStore, EventStoreError};
use statechronicle::ports::proof_index::{ProofIndex, ProofIndexError};
use statechronicle::ports::trade_index::{TradeIndex, TradeIndexError};
use statechronicle::proof::bundle::build_state_proof;
use statechronicle::proof::error::ProofError;
use statechronicle::proof::trade::verify_trade_proof;

use common::{Harness, beta, executor_subject, fixed_key, fixed_timestamp_placeholder, key_id};

const ALICE: &str = "account:example:player_123"; // seller
const BOB: &str = "account:example:player_456"; // buyer
const ASSET: &str = "asset:relic_001";
const WALLET: &str = "wallet:gold";
const TRADE: &str = "trade_001";
const PRICE: u64 = 100;

// ---------------------------------------------------------------------------
// In-memory port fakes (mirror the style in common/mod.rs).
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct FakeTradeIndex {
    inner: Arc<Mutex<BTreeMap<String, statechronicle::domain::trade::TradeRecord>>>,
}

#[async_trait]
impl TradeIndex for FakeTradeIndex {
    async fn put_trade(
        &self,
        trade: &statechronicle::domain::trade::TradeRecord,
    ) -> Result<(), TradeIndexError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|err| TradeIndexError::Unavailable(format!("lock poisoned: {err}")))?;
        inner.insert(trade.trade_id.clone(), trade.clone());
        Ok(())
    }

    async fn get_trade(
        &self,
        trade_id: &str,
    ) -> Result<Option<statechronicle::domain::trade::TradeRecord>, TradeIndexError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| TradeIndexError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner.get(trade_id).cloned())
    }
}

#[derive(Clone, Default)]
struct FakeEventStore {
    inner: Arc<Mutex<BTreeMap<(String, String), Event>>>,
}

impl FakeEventStore {
    fn put(&self, event: Event) {
        self.inner
            .lock()
            .unwrap()
            .insert((event.tenant_id.0.clone(), event.event_id.0.clone()), event);
    }
}

#[async_trait]
impl EventStore for FakeEventStore {
    async fn append_events(
        &self,
        _tenant: &TenantId,
        _events: &[Event],
    ) -> Result<(), EventStoreError> {
        Ok(())
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
        tenant: &TenantId,
        event_id: &EventId,
    ) -> Result<Option<Event>, EventStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| EventStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner.get(&(tenant.0.clone(), event_id.0.clone())).cloned())
    }
}

#[derive(Clone, Default)]
struct FakeCommitStore {
    inner: Arc<Mutex<BTreeMap<(String, String), Signed<Commit>>>>,
}

impl FakeCommitStore {
    fn put(&self, tenant: &TenantId, commit: Signed<Commit>) {
        self.inner
            .lock()
            .unwrap()
            .insert((tenant.0.clone(), commit.body.commit_id.0.clone()), commit);
    }
}

#[async_trait]
impl CommitStore for FakeCommitStore {
    async fn put_commit(
        &self,
        tenant: &TenantId,
        commit: &Signed<Commit>,
    ) -> Result<(), CommitStoreError> {
        self.put(tenant, commit.clone());
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
        Ok(inner.get(&(tenant.0.clone(), commit_id.0.clone())).cloned())
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
            .find(|((t, _), c)| t == &tenant.0 && c.body.sequence == sequence)
            .map(|(_, c)| c.clone()))
    }
}

#[derive(Clone, Default)]
struct FakeProofIndex {
    state_proofs: Arc<Mutex<BTreeMap<(String, String), ResourceStateProof>>>,
}

impl FakeProofIndex {
    fn put(&self, tenant: &TenantId, resource: &ResourceId, proof: ResourceStateProof) {
        self.state_proofs
            .lock()
            .unwrap()
            .insert((tenant.0.clone(), resource.0.clone()), proof);
    }
}

#[async_trait]
impl ProofIndex for FakeProofIndex {
    async fn get_state_proof(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
        _at: Option<&CommitId>,
    ) -> Result<Option<ResourceStateProof>, ProofIndexError> {
        let inner = self
            .state_proofs
            .lock()
            .map_err(|err| ProofIndexError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner
            .get(&(tenant.0.clone(), resource_id.0.clone()))
            .cloned())
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
    ) -> Result<Option<statechronicle::domain::proof::SparseMerkleProof>, ProofIndexError> {
        Ok(None)
    }

    async fn get_non_membership_proof(
        &self,
        _tenant: &TenantId,
        _resource_id: &ResourceId,
        _key: StateKey,
        _at: Option<&CommitId>,
    ) -> Result<Option<statechronicle::domain::proof::NonMembershipProofBundle>, ProofIndexError>
    {
        Ok(None)
    }
}

/// A proof index that is commit-aware: it returns a state proof only when the
/// requested commit matches the commit the proof was stored under, and records
/// every `(resource, commit)` request so a test can assert which per-asset
/// commit `get_proof` used.
#[derive(Clone, Default)]
struct CommitAwareProofIndex {
    state_proofs: Arc<Mutex<BTreeMap<(String, String, String), ResourceStateProof>>>,
    requests: Arc<Mutex<Vec<(String, String)>>>,
}

impl CommitAwareProofIndex {
    fn put(
        &self,
        tenant: &TenantId,
        resource: &ResourceId,
        commit: &CommitId,
        proof: ResourceStateProof,
    ) {
        self.state_proofs.lock().unwrap().insert(
            (tenant.0.clone(), resource.0.clone(), commit.0.clone()),
            proof,
        );
    }

    fn requests(&self) -> Vec<(String, String)> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ProofIndex for CommitAwareProofIndex {
    async fn get_state_proof(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
        at: Option<&CommitId>,
    ) -> Result<Option<ResourceStateProof>, ProofIndexError> {
        let commit = at.map(|c| c.0.clone()).unwrap_or_default();
        self.requests
            .lock()
            .unwrap()
            .push((resource_id.0.clone(), commit.clone()));
        let inner = self
            .state_proofs
            .lock()
            .map_err(|err| ProofIndexError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner
            .get(&(tenant.0.clone(), resource_id.0.clone(), commit))
            .cloned())
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
    ) -> Result<Option<statechronicle::domain::proof::SparseMerkleProof>, ProofIndexError> {
        Ok(None)
    }

    async fn get_non_membership_proof(
        &self,
        _tenant: &TenantId,
        _resource_id: &ResourceId,
        _key: StateKey,
        _at: Option<&CommitId>,
    ) -> Result<Option<statechronicle::domain::proof::NonMembershipProofBundle>, ProofIndexError>
    {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Test fixtures.
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn signed(
    harness: &Harness,
    tenant: TenantId,
    id: &str,
    op: &'static str,
    actor: &str,
    resource: &str,
    state_type: StateType,
    version: u64,
    inputs: &[(&str, serde_json::Value)],
    authority: Option<AuthorityProof>,
) -> statechronicle::intent::validated::ValidatedIntent {
    let mut b = Intent::builder()
        .tenant(tenant)
        .intent_id(IntentId::new(format!("int_{id}")).unwrap())
        .operation(Operation::from_static(op))
        .actor(SubjectId(String::from(actor)))
        .resource(ResourceId(String::from(resource)))
        .state_type(state_type)
        .expected_version(version)
        .created_at(harness.now())
        .nonce(Nonce::from_bytes(vec![0]).unwrap());
    for (k, v) in inputs {
        b = b.input(k, v.clone());
    }
    harness.sign(b.build().unwrap(), authority)
}

/// Seeds alpha (asset minted + locked into a trade) and beta (buyer wallet).
async fn seed(harness: &Harness, alpha: &TenantId, beta: &TenantId) {
    harness
        .run(
            &signed(
                harness,
                alpha.clone(),
                "thp_mint",
                "asset.mint",
                ALICE,
                ASSET,
                StateType::UniqueAsset,
                0,
                &[("to_owner", json!(ALICE))],
                None,
            ),
            StateType::UniqueAsset,
        )
        .await;
    harness
        .run(
            &signed(
                harness,
                alpha.clone(),
                "thp_lock",
                "trade.lock",
                ALICE,
                ASSET,
                StateType::UniqueAsset,
                1,
                &[("from_owner", json!(ALICE)), ("trade_id", json!(TRADE))],
                None,
            ),
            StateType::UniqueAsset,
        )
        .await;
    harness
        .run(
            &signed(
                harness,
                beta.clone(),
                "thp_wallet",
                "balance.create",
                BOB,
                WALLET,
                StateType::FungibleBalance,
                0,
                &[
                    ("subject", json!(BOB)),
                    ("unit", json!("gold_minor")),
                    ("balance", json!("1000")),
                ],
                None,
            ),
            StateType::FungibleBalance,
        )
        .await;
}

fn settle_intents(
    harness: &Harness,
    alpha: &TenantId,
    beta: &TenantId,
) -> Vec<statechronicle::intent::validated::ValidatedIntent> {
    vec![
        signed(
            harness,
            alpha.clone(),
            "thp_settle",
            "trade.settle",
            ALICE,
            ASSET,
            StateType::UniqueAsset,
            2,
            &[
                ("from_owner", json!(ALICE)),
                ("to_owner", json!(BOB)),
                ("trade_id", json!(TRADE)),
                ("value_resource", json!(WALLET)),
                ("value_amount", json!(PRICE.to_string())),
                ("value_to_subject", json!(ALICE)),
            ],
            Some(harness.authority()),
        ),
        signed(
            harness,
            beta.clone(),
            "thp_value",
            "balance.transfer",
            BOB,
            WALLET,
            StateType::FungibleBalance,
            1,
            &[
                ("to_subject", json!(ALICE)),
                ("amount", json!(PRICE.to_string())),
            ],
            None,
        ),
    ]
}

fn manifest() -> TradeManifest {
    TradeManifest {
        trade_id: String::from(TRADE),
        settle_legs: vec![SettleLeg {
            asset: ResourceId(String::from(ASSET)),
            settle_intent_id: IntentId::new(String::from("int_thp_settle")).unwrap(),
        }],
        value_legs: vec![ValueLeg {
            resource: ResourceId(String::from(WALLET)),
            amount: PRICE.to_string(),
            to_subject: SubjectId(String::from(ALICE)),
        }],
    }
}

/// Builds a genuine, verifiable state proof of the settled asset at the alpha
/// commit, using the accumulator returned by the harness's commit formation.
fn build_asset_proof(
    settle_event: &Event,
    signed_alpha: &Signed<Commit>,
    acc_alpha: &StateAccumulator,
) -> ResourceStateProof {
    let projection = StateProjection {
        tenant_id: settle_event.tenant_id.clone(),
        resource_id: settle_event.resource_id.clone(),
        state_type: StateType::UniqueAsset,
        version: settle_event.after.version,
        last_event_id: settle_event.event_id.clone(),
        last_commit_id: signed_alpha.body.commit_id.clone(),
        state_hash: settle_event.after.state_hash.clone(),
        state: settle_event.after.state.clone(),
    };
    let key = StateKey::for_resource(&settle_event.tenant_id.0, &settle_event.resource_id.0);
    let inclusion = acc_alpha.prove_inclusion(&key).unwrap();
    build_state_proof(
        &projection,
        signed_alpha,
        &inclusion,
        &Operation::from_static("trade.settle"),
        None,
        key,
    )
    .unwrap()
}

/// Commits each tenant group and returns the signed commits plus the alpha
/// accumulator (which reproduces the alpha state root).
fn commit_groups(
    harness: &Harness,
    groups: &[statechronicle::executor::atomicity::TenantEventGroup],
) -> (Signed<Commit>, Signed<Commit>, StateAccumulator) {
    let (signed_alpha, acc_alpha) = harness.commit_events(&groups[0].events);
    let (signed_beta, _acc_beta) = harness.commit_events(&groups[1].events);
    (signed_alpha, signed_beta, acc_alpha)
}

/// Forms + signs a commit over `events` with a caller-chosen commit id and a
/// state accumulator reproducing its root (so a test can mint two DISTINCT
/// commits for one tenant).
fn commit_at(events: &[Event], commit_id: &str) -> (Signed<Commit>, StateAccumulator) {
    let tenant = events
        .first()
        .expect("commit_at requires at least one event")
        .tenant_id
        .clone();
    let updates = state_root_updates(events).unwrap();
    let mut accumulator = StateAccumulator::empty();
    accumulator.insert_batch(&updates).unwrap();
    let commit = Commit::new(
        CommitScope::tenant(tenant),
        CommitId::new(String::from(commit_id)).unwrap(),
        None,
        1,
        events.len() as u64,
        hash_bytes(b"event-root"),
        ContentDigest::new(*StateRoot::empty().as_bytes()),
        ContentDigest::new(*accumulator.root().as_bytes()),
        fixed_timestamp_placeholder(),
        executor_subject(),
        ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
    );
    let signed = sign_commit(&commit, &fixed_key(), key_id()).unwrap();
    (signed, accumulator)
}

/// Wire the trade service over populated fakes.
fn wire_service(
    alpha: &TenantId,
    beta: &TenantId,
    events: &[Event],
    signed_alpha: &Signed<Commit>,
    signed_beta: &Signed<Commit>,
    asset_proof: &ResourceStateProof,
) -> (
    TradeService,
    FakeTradeIndex,
    FakeEventStore,
    FakeCommitStore,
    FakeProofIndex,
) {
    let trade_index = FakeTradeIndex::default();
    let event_store = FakeEventStore::default();
    let commit_store = FakeCommitStore::default();
    let proof_index = FakeProofIndex::default();

    for event in events {
        event_store.put(event.clone());
    }
    commit_store.put(alpha, signed_alpha.clone());
    commit_store.put(beta, signed_beta.clone());
    proof_index.put(alpha, &ResourceId(String::from(ASSET)), asset_proof.clone());

    let ports = TradePorts {
        trade_index: Box::new(trade_index.clone()),
        event_store: Box::new(event_store.clone()),
        proof_index: Box::new(proof_index.clone()),
        commit_store: Box::new(commit_store.clone()),
    };
    let service = TradeService::new(ports, |_tenant| Some(fixed_key().verifying_key()));
    (service, trade_index, event_store, commit_store, proof_index)
}

#[tokio::test]
async fn history_and_proof_for_a_settled_two_tenant_trade() {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    harness.tenant_store.register(beta.clone());
    seed(&harness, &alpha, &beta).await;

    let intents = settle_intents(&harness, &alpha, &beta);
    let groups = harness
        .executor
        .execute_cross_tenant_trade(&intents, &manifest())
        .await
        .unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].tenant, alpha);
    assert_eq!(groups[1].tenant, beta);

    let (signed_alpha, signed_beta, acc_alpha) = commit_groups(&harness, &groups);
    let settle_event = &groups[0].events[0];
    let asset_proof = build_asset_proof(settle_event, &signed_alpha, &acc_alpha);

    let mut all_events = Vec::new();
    for group in &groups {
        all_events.extend(group.events.iter().cloned());
    }
    let (service, _trade_index, _event_store, _commit_store, _proof_index) = wire_service(
        &alpha,
        &beta,
        &all_events,
        &signed_alpha,
        &signed_beta,
        &asset_proof,
    );

    // Ingest the two tenant batches (settle in alpha, value pair in beta),
    // both carrying the value-declaring settle intent.
    let settle_intent = intents[0].intent.clone();
    service
        .ingest_batch(&IngestBatch {
            events: groups[0].events.clone(),
            settle_intents: vec![settle_intent.clone()],
            commit: signed_alpha.clone(),
        })
        .await
        .unwrap();
    service
        .ingest_batch(&IngestBatch {
            events: groups[1].events.clone(),
            settle_intents: vec![settle_intent],
            commit: signed_beta.clone(),
        })
        .await
        .unwrap();

    // --- get_history ---
    let history = service
        .get_history(TRADE)
        .await
        .unwrap()
        .expect("history should be present");
    assert_eq!(history.status, TradeStatus::Settled);
    assert_eq!(history.events.len(), 3);
    // Ordered: the alpha settle first, then the beta value pair.
    assert_eq!(history.events[0].event.operation.as_str(), "trade.settle");
    assert_eq!(history.events[0].tenant_id, alpha);
    assert_eq!(
        history.events[1].event.operation.as_str(),
        "balance.transfer"
    );
    assert_eq!(
        history.events[2].event.operation.as_str(),
        "balance.transfer"
    );
    // Two distinct committing commits, in canonical order (alpha settle, beta value).
    assert_eq!(history.commits.len(), 2);
    assert_eq!(history.commits[0].0, alpha);
    assert_eq!(history.commits[0].1.commit_id, signed_alpha.body.commit_id);
    assert_eq!(history.commits[1].0, beta);
    assert_eq!(history.commits[1].1.commit_id, signed_beta.body.commit_id);

    // --- get_proof assembles (and internally verifies) ---
    let proof = service
        .get_proof(TRADE)
        .await
        .unwrap()
        .expect("proof should be present");
    assert_eq!(
        bcs::to_bytes(&proof).unwrap(),
        include_bytes!("fixtures/wire-v0.3.0/cross-tenant-trade-proof.bcs").as_slice(),
        "cross-tenant-trade-proof.bcs compatibility",
    );
    assert_eq!(
        serde_json::to_vec(&proof).unwrap(),
        include_bytes!("fixtures/wire-v0.3.0/cross-tenant-trade-proof.json").as_slice(),
        "cross-tenant-trade-proof.json compatibility",
    );
    assert_eq!(proof.trade_id, TRADE);
    assert_eq!(proof.summary.trade_id, TRADE);
    assert_eq!(proof.summary.status, TradeStatus::Settled);
    assert_eq!(proof.summary.sides.len(), 1);
    assert_eq!(proof.summary.sides[0].tenant, alpha);
    assert_eq!(proof.summary.value_legs.len(), 1);
    assert_eq!(proof.legs.len(), 1);
    assert_eq!(proof.legs[0].tenant, alpha);
    assert_eq!(proof.legs[0].state_proofs.len(), 1);

    // --- verify_trade_proof succeeds on the genuine proof ---
    let mut commits_by_id: BTreeMap<String, (Signed<Commit>, _)> = BTreeMap::new();
    commits_by_id.insert(
        signed_alpha.body.commit_id.0.clone(),
        (signed_alpha.clone(), fixed_key().verifying_key()),
    );
    assert!(verify_trade_proof(&proof, &commits_by_id).is_ok());
    assert_trade_coverage(&proof, &commits_by_id);

    // --- a tampered proof fails closed ---
    let mut tampered = proof.clone();
    tampered.summary.sides[0].to_owner = String::from("account:example:player_999");
    assert!(matches!(
        verify_trade_proof(&tampered, &commits_by_id),
        Err(ProofError::SubjectMismatch { .. })
    ));

    let mut tampered_id = proof.clone();
    tampered_id.trade_id = String::from("trade_999");
    assert!(matches!(
        verify_trade_proof(&tampered_id, &commits_by_id),
        Err(ProofError::TradeIdMismatch { .. })
    ));

    // --- missing trade ---
    assert!(
        service
            .get_history("trade_missing")
            .await
            .unwrap()
            .is_none()
    );
    assert!(service.get_proof("trade_missing").await.unwrap().is_none());
}

#[tokio::test]
async fn get_proof_fails_closed_when_settle_asset_has_no_state_proof() {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    harness.tenant_store.register(beta.clone());
    seed(&harness, &alpha, &beta).await;

    let intents = settle_intents(&harness, &alpha, &beta);
    let groups = harness
        .executor
        .execute_cross_tenant_trade(&intents, &manifest())
        .await
        .unwrap();
    let (signed_alpha, signed_beta, _acc_alpha) = commit_groups(&harness, &groups);

    let mut all_events = Vec::new();
    for group in &groups {
        all_events.extend(group.events.iter().cloned());
    }

    // Wire a service whose proof index holds NO state proof for the settle
    // asset: get_proof must fail closed rather than silently dropping it from
    // the returned proof.
    let trade_index = FakeTradeIndex::default();
    let event_store = FakeEventStore::default();
    let commit_store = FakeCommitStore::default();
    let proof_index = FakeProofIndex::default();
    for event in &all_events {
        event_store.put(event.clone());
    }
    commit_store.put(&alpha, signed_alpha.clone());
    commit_store.put(&beta, signed_beta.clone());
    // Deliberately: proof_index is left empty for the settle asset.
    let ports = TradePorts {
        trade_index: Box::new(trade_index.clone()),
        event_store: Box::new(event_store.clone()),
        proof_index: Box::new(proof_index.clone()),
        commit_store: Box::new(commit_store.clone()),
    };
    let service = TradeService::new(ports, |_tenant| Some(fixed_key().verifying_key()));

    let settle_intent = intents[0].intent.clone();
    service
        .ingest_batch(&IngestBatch {
            events: groups[0].events.clone(),
            settle_intents: vec![settle_intent.clone()],
            commit: signed_alpha.clone(),
        })
        .await
        .unwrap();
    service
        .ingest_batch(&IngestBatch {
            events: groups[1].events.clone(),
            settle_intents: vec![settle_intent],
            commit: signed_beta.clone(),
        })
        .await
        .unwrap();

    let error = service.get_proof(TRADE).await.unwrap_err();
    assert!(matches!(
        error,
        TradeServiceError::ProofIndex(message) if message.contains("no state proof stored")
    ));
}

#[tokio::test]
async fn rebuild_replays_to_the_same_index_as_incremental() {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    harness.tenant_store.register(beta.clone());
    seed(&harness, &alpha, &beta).await;

    let intents = settle_intents(&harness, &alpha, &beta);
    let groups = harness
        .executor
        .execute_cross_tenant_trade(&intents, &manifest())
        .await
        .unwrap();
    let (signed_alpha, signed_beta, acc_alpha) = commit_groups(&harness, &groups);
    let settle_event = &groups[0].events[0];
    let asset_proof = build_asset_proof(settle_event, &signed_alpha, &acc_alpha);

    let mut all_events = Vec::new();
    for group in &groups {
        all_events.extend(group.events.iter().cloned());
    }
    let (service, trade_index, _event_store, _commit_store, _proof_index) = wire_service(
        &alpha,
        &beta,
        &all_events,
        &signed_alpha,
        &signed_beta,
        &asset_proof,
    );

    let settle_intent = intents[0].intent.clone();
    let batches = vec![
        IngestBatch {
            events: groups[0].events.clone(),
            settle_intents: vec![settle_intent.clone()],
            commit: signed_alpha.clone(),
        },
        IngestBatch {
            events: groups[1].events.clone(),
            settle_intents: vec![settle_intent],
            commit: signed_beta.clone(),
        },
    ];

    service.rebuild(&batches).await.unwrap();
    let record = trade_index.get_trade(TRADE).await.unwrap().unwrap();
    assert_eq!(record.status, TradeStatus::Settled);
    assert_eq!(record.value_legs.len(), 1);
    assert_eq!(record.sides.len(), 1);
    assert_eq!(record.events.len(), 3);
}

/// F4: incremental ingestion via `TradeService::ingest_batch` (each batch
/// seeding from existing records, then applying) produces exactly the same
/// trade index as `TradeService::rebuild` from the raw batch stream.
#[tokio::test]
async fn incremental_ingest_matches_rebuild_index() {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    harness.tenant_store.register(beta.clone());
    seed(&harness, &alpha, &beta).await;

    let intents = settle_intents(&harness, &alpha, &beta);
    let groups = harness
        .executor
        .execute_cross_tenant_trade(&intents, &manifest())
        .await
        .unwrap();
    let (signed_alpha, signed_beta, acc_alpha) = commit_groups(&harness, &groups);
    let settle_event = &groups[0].events[0];
    let asset_proof = build_asset_proof(settle_event, &signed_alpha, &acc_alpha);

    let mut all_events = Vec::new();
    for group in &groups {
        all_events.extend(group.events.iter().cloned());
    }
    let settle_intent = intents[0].intent.clone();
    let batches = vec![
        IngestBatch {
            events: groups[0].events.clone(),
            settle_intents: vec![settle_intent.clone()],
            commit: signed_alpha.clone(),
        },
        IngestBatch {
            events: groups[1].events.clone(),
            settle_intents: vec![settle_intent],
            commit: signed_beta.clone(),
        },
    ];

    // Incremental path: two separate ingest_batch calls.
    let (incremental_service, incremental_index, _, _, _) = wire_service(
        &alpha,
        &beta,
        &all_events,
        &signed_alpha,
        &signed_beta,
        &asset_proof,
    );
    for batch in &batches {
        incremental_service.ingest_batch(batch).await.unwrap();
    }
    let incremental_record = incremental_index
        .get_trade(TRADE)
        .await
        .unwrap()
        .expect("incremental record should be present");

    // Rebuild path: one rebuild call from the raw stream.
    let (rebuild_service, rebuild_index, _, _, _) = wire_service(
        &alpha,
        &beta,
        &all_events,
        &signed_alpha,
        &signed_beta,
        &asset_proof,
    );
    rebuild_service.rebuild(&batches).await.unwrap();
    let rebuilt_record = rebuild_index
        .get_trade(TRADE)
        .await
        .unwrap()
        .expect("rebuilt record should be present");

    assert_eq!(incremental_record, rebuilt_record);
}

/// F3 regression: when one tenant settles two assets of one trade in two
/// different commits, `get_proof` must fetch each asset's state proof at the
/// commit that settled it (per-asset commit), not the side's stale first
/// commit.
#[tokio::test]
async fn get_proof_uses_per_asset_commit_for_two_commit_settle() {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let asset_a = String::from("asset:relic_001");
    let asset_b = String::from("asset:relic_002");

    // Mint + lock BOTH assets into the same trade.
    for (id, asset) in [("f3_ma", asset_a.as_str()), ("f3_mb", asset_b.as_str())] {
        harness
            .run(
                &signed(
                    &harness,
                    alpha.clone(),
                    id,
                    "asset.mint",
                    ALICE,
                    asset,
                    StateType::UniqueAsset,
                    0,
                    &[("to_owner", json!(ALICE))],
                    None,
                ),
                StateType::UniqueAsset,
            )
            .await;
        harness
            .run(
                &signed(
                    &harness,
                    alpha.clone(),
                    &format!("{id}_lock"),
                    "trade.lock",
                    ALICE,
                    asset,
                    StateType::UniqueAsset,
                    1,
                    &[("from_owner", json!(ALICE)), ("trade_id", json!(TRADE))],
                    None,
                ),
                StateType::UniqueAsset,
            )
            .await;
    }

    // Settle asset A (commit 1), then asset B (commit 2) via separate
    // execute_settle calls.
    let settle_a = signed(
        &harness,
        alpha.clone(),
        "f3_settle_a",
        "trade.settle",
        ALICE,
        &asset_a,
        StateType::UniqueAsset,
        2,
        &[
            ("from_owner", json!(ALICE)),
            ("to_owner", json!(BOB)),
            ("trade_id", json!(TRADE)),
        ],
        Some(harness.authority()),
    );
    let events_a = harness.executor.execute_settle(&[settle_a]).await.unwrap();
    for ev in &events_a {
        harness.index.apply(ev, StateType::UniqueAsset);
    }
    let settle_b = signed(
        &harness,
        alpha.clone(),
        "f3_settle_b",
        "trade.settle",
        ALICE,
        &asset_b,
        StateType::UniqueAsset,
        2,
        &[
            ("from_owner", json!(ALICE)),
            ("to_owner", json!(BOB)),
            ("trade_id", json!(TRADE)),
        ],
        Some(harness.authority()),
    );
    let events_b = harness.executor.execute_settle(&[settle_b]).await.unwrap();
    for ev in &events_b {
        harness.index.apply(ev, StateType::UniqueAsset);
    }

    // Two DISTINCT commits for the same tenant.
    let (signed_c1, acc1) = commit_at(&events_a, "cmt_0000000000000000f301");
    let (signed_c2, acc2) = commit_at(&events_b, "cmt_0000000000000000f302");

    // The historical-event route authenticates both independently pinned
    // assets even when their settlement commits differ. Legacy state-only
    // fixtures above deliberately do not contain real event roots; construct
    // separate genuine event-root commits for this additional assertion.
    {
        use statechronicle::core::canonicalize::canonicalize_and_digest;
        use statechronicle::proof::settlement::{
            ExpectedAssetSettlement, ExpectedSettlement, ExpectedSettlementEvent,
            SettlementWitness, verify_settlement_history,
        };
        let mut body_a = signed_c1.body.clone();
        body_a.event_merkle_root = statechronicle::commit::roots::event_root(&events_a).unwrap();
        let mut body_b = signed_c2.body.clone();
        body_b.event_merkle_root = statechronicle::commit::roots::event_root(&events_b).unwrap();
        let history_a = sign_commit(&body_a, &fixed_key(), key_id()).unwrap();
        let history_b = sign_commit(&body_b, &fixed_key(), key_id()).unwrap();
        let assets = [(&events_a[0], &body_a), (&events_b[0], &body_b)]
            .into_iter()
            .map(|(event, body)| ExpectedAssetSettlement {
                event: ExpectedSettlementEvent {
                    tenant: alpha.clone(),
                    resource: event.resource_id.clone(),
                    event_id: event.event_id.clone(),
                    intent_id: event.intent_id.clone(),
                    commit_digest: canonicalize_and_digest(body).unwrap(),
                },
                from: SubjectId(ALICE.into()),
                to: SubjectId(BOB.into()),
            })
            .collect();
        let expected = ExpectedSettlement {
            trade_id: TRADE.into(),
            assets,
            payments: vec![],
        };
        let witnesses = [
            SettlementWitness {
                commit: &history_b,
                events: &events_b,
            },
            SettlementWitness {
                commit: &history_a,
                events: &events_a,
            },
        ];
        let key = |_: &TenantId, _: &Signed<Commit>| Ok(fixed_key().verifying_key());
        assert_eq!(
            verify_settlement_history(&expected, &witnesses, key)
                .unwrap()
                .claim(),
            &expected
        );
        assert!(verify_settlement_history(&expected, &witnesses[..1], key).is_err());
    }

    // Genuine state proofs at each asset's own commit.
    let proof_a = build_asset_proof(&events_a[0], &signed_c1, &acc1);
    let proof_b = build_asset_proof(&events_b[0], &signed_c2, &acc2);

    let trade_index = FakeTradeIndex::default();
    let event_store = FakeEventStore::default();
    let commit_store = FakeCommitStore::default();
    for ev in events_a.iter().chain(events_b.iter()) {
        event_store.put(ev.clone());
    }
    commit_store.put(&alpha, signed_c1.clone());
    commit_store.put(&alpha, signed_c2.clone());
    let proof_index = CommitAwareProofIndex::default();
    proof_index.put(
        &alpha,
        &ResourceId(asset_a.clone()),
        &signed_c1.body.commit_id,
        proof_a,
    );
    proof_index.put(
        &alpha,
        &ResourceId(asset_b.clone()),
        &signed_c2.body.commit_id,
        proof_b,
    );

    let ports = TradePorts {
        trade_index: Box::new(trade_index.clone()),
        event_store: Box::new(event_store.clone()),
        proof_index: Box::new(proof_index.clone()),
        commit_store: Box::new(commit_store.clone()),
    };
    let service = TradeService::new(ports, |_tenant| Some(fixed_key().verifying_key()));

    // Ingest one batch per commit.
    service
        .ingest_batch(&IngestBatch {
            events: events_a.clone(),
            settle_intents: Vec::new(),
            commit: signed_c1.clone(),
        })
        .await
        .unwrap();
    service
        .ingest_batch(&IngestBatch {
            events: events_b.clone(),
            settle_intents: Vec::new(),
            commit: signed_c2.clone(),
        })
        .await
        .unwrap();

    let result = service.get_proof(TRADE).await;

    // The fix must have fetched asset B at its own commit (c2), never the
    // stale first commit (c1) that would have been a proof-index miss.
    let requested = proof_index.requests();
    assert!(
        requested.contains(&(asset_b.clone(), signed_c2.body.commit_id.0.clone())),
        "expected asset B to be fetched at its own commit, requests were: {requested:?}"
    );
    assert!(
        !requested.contains(&(asset_b.clone(), signed_c1.body.commit_id.0.clone())),
        "asset B must not be fetched at the stale first commit"
    );

    // The two-commit settle must produce a verifiable proof: get_proof returns
    // Ok(Some(..)) and the assembled proof verifies (each state proof resolved
    // against the commit its own commit_ref names, so both commits are loaded
    // and both proofs pass).
    let proof = result.expect("two-commit settle must produce a proof");
    let proof = proof.expect("proof must be present");
    assert_eq!(proof.legs.len(), 1, "both assets settle in the alpha leg");
    assert_eq!(proof.legs[0].state_proofs.len(), 2);
    statechronicle::proof::trade::verify_trade_proof(
        &proof,
        &BTreeMap::from([
            (
                signed_c1.body.commit_id.0.clone(),
                (signed_c1.clone(), fixed_key().verifying_key()),
            ),
            (
                signed_c2.body.commit_id.0.clone(),
                (signed_c2.clone(), fixed_key().verifying_key()),
            ),
        ]),
    )
    .expect("two-commit proof must verify");
    assert_trade_coverage(
        &proof,
        &BTreeMap::from([
            (
                signed_c1.body.commit_id.0.clone(),
                (signed_c1, fixed_key().verifying_key()),
            ),
            (
                signed_c2.body.commit_id.0.clone(),
                (signed_c2, fixed_key().verifying_key()),
            ),
        ]),
    );
}

/// Genuine signed state proofs must not be counted twice to cover missing
/// tenants/assets. This checks structural coverage, not summary authenticity.
fn assert_trade_coverage(
    proof: &statechronicle::domain::trade::TradeProof,
    commits: &BTreeMap<String, (Signed<Commit>, ed25519_dalek::VerifyingKey)>,
) {
    let mut missing_tenant = proof.clone();
    let mut second_side = missing_tenant.summary.sides[0].clone();
    second_side.tenant = beta();
    missing_tenant.summary.sides.push(second_side);
    missing_tenant.legs.push(missing_tenant.legs[0].clone());

    let mut duplicate_side = proof.clone();
    duplicate_side
        .summary
        .sides
        .push(duplicate_side.summary.sides[0].clone());
    duplicate_side.legs.push(duplicate_side.legs[0].clone());

    let mut duplicate_asset = proof.clone();
    duplicate_asset.summary.sides[0]
        .settle_assets
        .push(proof.summary.sides[0].settle_assets[0].clone());
    duplicate_asset.legs[0]
        .state_proofs
        .push(proof.legs[0].state_proofs[0].clone());
    let mut cases = vec![
        ("missing tenant", missing_tenant),
        ("duplicate side", duplicate_side),
        ("duplicate declared asset", duplicate_asset),
    ];
    if proof.legs[0].state_proofs.len() == 2 {
        let mut missing_asset = proof.clone();
        missing_asset.legs[0].state_proofs[1] = missing_asset.legs[0].state_proofs[0].clone();
        cases.push(("missing asset", missing_asset));
    }
    let scoped = commits
        .iter()
        .map(|(id, (signed, key))| {
            (
                (
                    signed.body.scope.tenant_id.as_ref().unwrap().0.clone(),
                    id.clone(),
                ),
                (signed.clone(), *key),
            )
        })
        .collect();
    use statechronicle::proof::trade::verify_trade_proof_scoped;
    verify_trade_proof_scoped(proof, &scoped).unwrap();
    let mut reordered = proof.clone();
    reordered.legs[0].state_proofs.reverse();
    verify_trade_proof_scoped(&reordered, &scoped).unwrap();
    for (name, candidate) in cases {
        assert!(
            matches!(
                verify_trade_proof(&candidate, commits),
                Err(ProofError::InvalidState(_))
            ),
            "legacy verifier: {name}"
        );
        assert!(
            matches!(
                verify_trade_proof_scoped(&candidate, &scoped),
                Err(ProofError::InvalidState(_))
            ),
            "scoped verifier: {name}"
        );
    }

    // These fields are not authenticated by state inclusion/signatures.
    // Keep this limitation executable: this API does not prove settlement.
    let mut unsigned_metadata = proof.clone();
    unsigned_metadata.trade_id = String::from("another_unverified_trade");
    unsigned_metadata.summary.trade_id = unsigned_metadata.trade_id.clone();
    unsigned_metadata.summary.sides[0].from_owner = String::from("unverified_previous_owner");
    unsigned_metadata.summary.sides[0].settle_event_ids.clear();
    unsigned_metadata.summary.sides[0]
        .settle_commits_by_asset
        .clear();
    unsigned_metadata.summary.value_legs.clear();
    unsigned_metadata.summary.sides[0].settle_commit.commit_id =
        CommitId::new(String::from("cmt_unverified_summary")).unwrap();
    unsigned_metadata.legs[0].commit.commit_id =
        CommitId::new(String::from("cmt_unverified_representative")).unwrap();
    verify_trade_proof_scoped(&unsigned_metadata, &scoped).unwrap();
}

/// Independent claims authenticate signed events, including previous owner and
/// payment deltas; unsigned summary equality is deliberately not an input.
#[tokio::test]
async fn requested_settlement_facts_require_authenticated_event_history() {
    use statechronicle::core::{amount::Amount, canonicalize::canonicalize_and_digest};
    use statechronicle::domain::resource_state::ResourceState;
    use statechronicle::proof::settlement::{
        ExpectedAssetSettlement, ExpectedSettlement, ExpectedSettlementEvent,
        ExpectedSettlementPayment, SettlementWitness, verify_settlement_history,
    };
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    harness.tenant_store.register(beta.clone());
    seed(&harness, &alpha, &beta).await;
    let groups = harness
        .executor
        .execute_cross_tenant_trade(&settle_intents(&harness, &alpha, &beta), &manifest())
        .await
        .unwrap();
    let (signed_alpha, signed_beta, _) = commit_groups(&harness, &groups);
    assert_eq!(signed_alpha.body.commit_id, signed_beta.body.commit_id);
    let pin = |event: &Event, commit: &Signed<Commit>| ExpectedSettlementEvent {
        tenant: event.tenant_id.clone(),
        resource: event.resource_id.clone(),
        event_id: event.event_id.clone(),
        intent_id: event.intent_id.clone(),
        commit_digest: canonicalize_and_digest(&commit.body).unwrap(),
    };
    let debit = groups[1]
        .events
        .iter()
        .find(|event| event.before.state.subject().unwrap().0 == BOB)
        .unwrap();
    let credit = groups[1]
        .events
        .iter()
        .find(|event| event.after.state.subject().unwrap().0 == ALICE)
        .unwrap();
    // Fixture-produced event identities/commit digests are captured before any
    // adversarial alteration. Business expectations come from the test request.
    let expected = ExpectedSettlement {
        trade_id: TRADE.into(),
        assets: vec![ExpectedAssetSettlement {
            event: pin(&groups[0].events[0], &signed_alpha),
            from: SubjectId(ALICE.into()),
            to: SubjectId(BOB.into()),
        }],
        payments: vec![ExpectedSettlementPayment {
            debit: pin(debit, &signed_beta),
            credit: pin(credit, &signed_beta),
            from: SubjectId(BOB.into()),
            to: SubjectId(ALICE.into()),
            amount: Amount::from_u64(PRICE),
            unit: "gold_minor".into(),
        }],
    };
    let witnesses = [
        SettlementWitness {
            commit: &signed_alpha,
            events: &groups[0].events,
        },
        SettlementWitness {
            commit: &signed_beta,
            events: &groups[1].events,
        },
    ];
    let trusted_key = |tenant: &TenantId, commit: &Signed<Commit>| {
        assert!(tenant == &alpha || tenant == &beta);
        if commit.signature.key_id != key_id() || commit.body.executor != executor_subject() {
            return Err(ProofError::KeyNotFound(commit.signature.key_id.0.clone()));
        }
        Ok(fixed_key().verifying_key())
    };
    let verified = verify_settlement_history(&expected, &witnesses, trusted_key).unwrap();
    assert_eq!(verified.claim(), &expected);
    let witness_wire = witnesses
        .each_ref()
        .map(|witness| (witness.commit, witness.events));
    assert_eq!(
        bcs::to_bytes(&witness_wire).unwrap(),
        include_bytes!("fixtures/wire-v0.3.0/cross-tenant-settlement-witnesses.bcs").as_slice(),
        "cross-tenant-settlement-witnesses.bcs compatibility",
    );
    assert_eq!(
        serde_json::to_vec(&witness_wire).unwrap(),
        include_bytes!("fixtures/wire-v0.3.0/cross-tenant-settlement-witnesses.json").as_slice(),
        "cross-tenant-settlement-witnesses.json compatibility",
    );
    let mut reordered = witnesses;
    reordered.reverse();
    verify_settlement_history(&expected, &reordered, trusted_key).unwrap();
    let mut candidates = Vec::new();
    macro_rules! wrong {
        ($name:expr, $alter:expr) => {{
            let mut claim = expected.clone();
            ($alter)(&mut claim);
            candidates.push(($name, claim));
        }};
    }
    wrong!("trade", |c: &mut ExpectedSettlement| c.trade_id =
        "another_trade".into());
    wrong!("from owner", |c: &mut ExpectedSettlement| c.assets[0]
        .from =
        SubjectId("mallory".into()));
    wrong!("to owner", |c: &mut ExpectedSettlement| c.assets[0].to =
        SubjectId("mallory".into()));
    wrong!("tenant", |c: &mut ExpectedSettlement| c.assets[0]
        .event
        .tenant =
        beta.clone());
    wrong!("resource", |c: &mut ExpectedSettlement| c.assets[0]
        .event
        .resource =
        ResourceId("another_asset".into()));
    wrong!("event", |c: &mut ExpectedSettlement| c.assets[0]
        .event
        .event_id =
        EventId("another_event".into()));
    wrong!("intent", |c: &mut ExpectedSettlement| c.assets[0]
        .event
        .intent_id =
        IntentId("another_intent".into()));
    wrong!("commit", |c: &mut ExpectedSettlement| c.assets[0]
        .event
        .commit_digest =
        hash_bytes(b"wrong"));
    wrong!("duplicate asset", |c: &mut ExpectedSettlement| c
        .assets
        .push(c.assets[0].clone()));
    wrong!("payment amount", |c: &mut ExpectedSettlement| c.payments
        [0]
    .amount =
        Amount::from_u64(PRICE + 1));
    wrong!("payment unit", |c: &mut ExpectedSettlement| c.payments[0]
        .unit =
        "other".into());
    wrong!("payer", |c: &mut ExpectedSettlement| c.payments[0].from =
        SubjectId("mallory".into()));
    wrong!("payee", |c: &mut ExpectedSettlement| c.payments[0].to =
        SubjectId("mallory".into()));
    wrong!("zero payment", |c: &mut ExpectedSettlement| c.payments[0]
        .amount =
        Amount::ZERO);
    wrong!("swapped payment events", |c: &mut ExpectedSettlement| {
        let p = &mut c.payments[0];
        std::mem::swap(&mut p.debit, &mut p.credit);
    });
    wrong!("duplicate payment", |c: &mut ExpectedSettlement| c
        .payments
        .push(c.payments[0].clone()));
    wrong!("empty assets", |c: &mut ExpectedSettlement| c
        .assets
        .clear());
    wrong!("empty trade", |c: &mut ExpectedSettlement| c
        .trade_id
        .clear());
    for (name, candidate) in candidates {
        assert!(
            verify_settlement_history(&candidate, &reordered, trusted_key).is_err(),
            "{name}"
        );
    }
    assert!(verify_settlement_history(&expected, &[], trusted_key).is_err());
    assert!(verify_settlement_history(&expected, &reordered[..1], trusted_key).is_err());
    assert!(
        verify_settlement_history(&expected, &reordered, |_, _| Ok(
            ed25519_dalek::SigningKey::from_bytes(&[91; 32]).verifying_key()
        ))
        .is_err()
    );
    assert!(
        verify_settlement_history(&expected, &reordered, |_, _| Err(ProofError::KeyNotFound(
            "untrusted".into()
        )))
        .is_err()
    );
    // All these inputs retain the genuine signed commit, so event substitution,
    // reordering, omission and duplication cannot be hidden by summary changes.
    let altered_batches = [
        {
            let mut batch = groups[1].events.clone();
            batch.reverse();
            batch
        },
        {
            let mut batch = groups[1].events.clone();
            batch.pop();
            batch
        },
        {
            let mut batch = groups[1].events.clone();
            batch.push(batch[0].clone());
            batch
        },
        {
            let mut batch = groups[1].events.clone();
            batch[0].actor = SubjectId("mallory".into());
            batch
        },
    ];
    for altered in altered_batches {
        let altered_events_witness = [
            SettlementWitness {
                commit: &signed_alpha,
                events: &groups[0].events,
            },
            SettlementWitness {
                commit: &signed_beta,
                events: &altered,
            },
        ];
        assert!(
            verify_settlement_history(&expected, &altered_events_witness, trusted_key).is_err()
        );
    }
    // A trusted signer cannot make a body claim authenticate a different body.
    let mut altered_commit = signed_beta.clone();
    altered_commit.body.sequence += 1;
    let changed_commit_witness = [
        SettlementWitness {
            commit: &signed_alpha,
            events: &groups[0].events,
        },
        SettlementWitness {
            commit: &altered_commit,
            events: &groups[1].events,
        },
    ];
    assert!(verify_settlement_history(&expected, &changed_commit_witness, trusted_key).is_err());
    let mut bad_signature = signed_beta.clone();
    bad_signature.signature = signed_alpha.signature.clone();
    let signature_witness = [
        SettlementWitness {
            commit: &signed_alpha,
            events: &groups[0].events,
        },
        SettlementWitness {
            commit: &bad_signature,
            events: &groups[1].events,
        },
    ];
    assert!(matches!(
        verify_settlement_history(&expected, &signature_witness, trusted_key),
        Err(ProofError::CommitSignature(_))
    ));
    let duplicates = [
        SettlementWitness {
            commit: &signed_alpha,
            events: &groups[0].events,
        },
        SettlementWitness {
            commit: &signed_alpha,
            events: &groups[0].events,
        },
        SettlementWitness {
            commit: &signed_beta,
            events: &groups[1].events,
        },
    ];
    assert!(verify_settlement_history(&expected, &duplicates, trusted_key).is_err());
    let oversized: Vec<_> = (0..257)
        .map(|_| SettlementWitness {
            commit: &signed_alpha,
            events: &groups[0].events,
        })
        .collect();
    assert!(
        verify_settlement_history(&expected, &oversized, |_, _| panic!(
            "oversized witnesses reached signer lookup"
        ))
        .is_err()
    );
    let mut oversized_claim = expected.clone();
    oversized_claim.assets = vec![expected.assets[0].clone(); 257];
    assert!(
        verify_settlement_history(&oversized_claim, &reordered, |_, _| panic!(
            "oversized expectations reached signer lookup"
        ))
        .is_err()
    );
    // Re-sign malformed transitions and independently pin each new commit:
    // structural checks remain necessary even when signature/root are genuine.
    for case in 0..4 {
        let mut malformed = groups[0].events.clone();
        match case {
            0 => {
                if let ResourceState::UniqueAsset(state) = &mut malformed[0].after.state {
                    state.trade_id = Some(TRADE.into());
                }
            }
            1 => malformed[0].after.version = malformed[0].before.version,
            2 => {
                malformed[0].before.version = 0;
                malformed[0].after.version = 1;
            }
            _ => malformed[0].before.state_hash = hash_bytes(b"forged state hash"),
        }
        malformed[0].after.state_hash = canonicalize_and_digest(&malformed[0].after.state).unwrap();
        let mut body = signed_alpha.body.clone();
        body.event_merkle_root = statechronicle::commit::roots::event_root(&malformed).unwrap();
        let malformed_commit = sign_commit(&body, &fixed_key(), key_id()).unwrap();
        let mut bad_claim = expected.clone();
        bad_claim.assets[0].event.commit_digest = canonicalize_and_digest(&body).unwrap();
        let malformed_witness = [
            SettlementWitness {
                commit: &malformed_commit,
                events: &malformed,
            },
            SettlementWitness {
                commit: &signed_beta,
                events: &groups[1].events,
            },
        ];
        assert!(
            verify_settlement_history(&bad_claim, &malformed_witness, trusted_key).is_err(),
            "signed malformed asset case {case}"
        );
    }
    // Version zero requires the exact canonical unborn recipient, not merely
    // a zero balance with a plausible existing holder and denomination.
    let mut noncanonical_credit = groups[1].events.clone();
    let recipient = noncanonical_credit
        .iter_mut()
        .find(|event| event.after.state.subject().unwrap().0 == ALICE)
        .unwrap();
    if let ResourceState::FungibleBalance(state) = &mut recipient.before.state {
        state.subject = SubjectId(ALICE.into());
        state.unit = "gold_minor".into();
    }
    recipient.before.state_hash = canonicalize_and_digest(&recipient.before.state).unwrap();
    let mut payment_body = signed_beta.body.clone();
    payment_body.event_merkle_root =
        statechronicle::commit::roots::event_root(&noncanonical_credit).unwrap();
    let payment_commit = sign_commit(&payment_body, &fixed_key(), key_id()).unwrap();
    let mut payment_claim = expected;
    let digest = canonicalize_and_digest(&payment_body).unwrap();
    payment_claim.payments[0].debit.commit_digest = digest.clone();
    payment_claim.payments[0].credit.commit_digest = digest;
    let payment_witnesses = [
        SettlementWitness {
            commit: &signed_alpha,
            events: &groups[0].events,
        },
        SettlementWitness {
            commit: &payment_commit,
            events: &noncanonical_credit,
        },
    ];
    assert!(verify_settlement_history(&payment_claim, &payment_witnesses, trusted_key).is_err());
}
