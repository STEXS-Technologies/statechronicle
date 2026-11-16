//! Exact legacy durable preflight/outbox errors, callback order and matched timing.
#[path = "common.rs"]
mod common;
use async_trait::async_trait;
use serde_json::{Value, json};
use statechronicle::commit::{
    persist::{CommittedEvent, DurableCommitRequest, SignedCommitVerifier, persist_durable_verified, projections_for},
    sign::verify_commit,
};
use statechronicle::core::{canonicalize::canonicalize_and_digest, digest::hash_bytes};
use statechronicle::ports::{
    authorization::{AuthenticatedPrincipal, AuthorizationContext, AuthorizationError, Authorizer},
    ledger_store::{IdempotencyClaim, LedgerStore, LedgerStoreError, LedgerTransaction, OutboxRecord},
};
use statechronicle::{Commit, CommitId, ContentDigest, Event, Intent, IntentId, ResourceId, Signed, StateProjection, StateType, TenantId};
use std::{sync::{Arc, Mutex}, hint::black_box, time::Instant};

const VARIANTS: usize = 18;
const FAULTS: usize = 12;
#[derive(Clone)]
struct Host { calls: Arc<Mutex<Vec<String>>>, faults: u16, claim: usize, trace: bool }
impl Host {
    fn call(&self, slot: usize, name: &str) -> Result<(), LedgerStoreError> {
        if self.trace { self.calls.lock().unwrap().push(name.into()); }
        if self.faults & (1 << slot) != 0 { Err(LedgerStoreError::Unavailable(format!("sentinel:{name}"))) } else { Ok(()) }
    }
}
impl SignedCommitVerifier for Host {
    fn verify(&self, commit: &Signed<Commit>) -> Result<(), String> {
        self.call(0, "verify").map_err(|e| e.to_string())?;
        verify_commit(commit, &common::fixed_key().verifying_key()).map_err(|e| e.to_string())
    }
}
#[async_trait]
impl Authorizer for Host {
    async fn authorize(&self, _: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
        self.call(1, "authorize").map_err(|e| AuthorizationError::Unavailable(e.to_string()))
    }
}
#[async_trait]
impl LedgerStore for Host {
    async fn begin(&self, _: &TenantId) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        self.call(2, "begin")?; Ok(Box::new(self.clone()))
    }
    async fn begin_multi(&self, _: &[TenantId]) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> { panic!("unexpected multi-tenant begin") }
}
#[async_trait]
impl LedgerTransaction for Host {
    async fn claim_idempotency(&mut self, _: &TenantId, _: &Intent, _: &ContentDigest) -> Result<IdempotencyClaim, LedgerStoreError> {
        self.call(3, "claim")?;
        Ok(match self.claim {
            0 => IdempotencyClaim::NewReservation { attempt_id: "attempt".into() },
            1 => IdempotencyClaim::Committed { commit_id: CommitId::new("cmt_original_replay".into()).unwrap() },
            2 => IdempotencyClaim::InProgress { attempt_id: "attempt".into(), lease_expires_at_unix: 42 },
            3 => IdempotencyClaim::ConflictDifferentPayload,
            _ => panic!("unknown claim"),
        })
    }
    async fn append_events(&mut self, _: &[Event]) -> Result<(), LedgerStoreError> { self.call(4, "events") }
    async fn append_commit(&mut self, _: &Signed<Commit>) -> Result<(), LedgerStoreError> { self.call(5, "commit-row") }
    async fn upsert_projection(&mut self, _: &StateProjection) -> Result<(), LedgerStoreError> { self.call(6, "projection") }
    async fn enqueue_outbox(&mut self, _: &OutboxRecord) -> Result<(), LedgerStoreError> { self.call(7, "outbox") }
    async fn finalize_idempotency(&mut self, _: &TenantId, _: &IntentId, _: &str, _: &CommitId) -> Result<(), LedgerStoreError> { self.call(8, "finalize") }
    async fn commit(self: Box<Self>) -> Result<(), LedgerStoreError> { self.call(9, "commit") }
    async fn rollback(self: Box<Self>) -> Result<(), LedgerStoreError> { self.call(10, "rollback") }
}
struct Fixture { intent: Intent, events: Vec<Event>, commit: Signed<Commit>, projections: Vec<StateProjection>, principal: AuthenticatedPrincipal, digest: ContentDigest }
impl Fixture {
    async fn new() -> Self {
        let h = common::Harness::new();
        let accepted = h.accept(&json!({"schema":"statechronicle.intent.v0", "tenant_id":"acme.game.alpha",
            "intent_id":"int_outbox", "operation":"asset.mint", "actor":"account:alice",
            "resource_id":"asset:outbox", "state_type":"unique_asset", "expected_version":0,
            "created_at":"2026-07-14T00:00:00Z", "nonce":"b64u:AA", "inputs":{"to_owner":"account:alice"}}), Some(h.authority()));
        let event = h.run(&accepted, StateType::UniqueAsset).await;
        let (commit, _) = h.commit_events(std::slice::from_ref(&event));
        let events = vec![event];
        let entries = [CommittedEvent { event: &events[0], state_type: StateType::UniqueAsset }];
        let projections = projections_for(&commit.body.commit_id, &entries);
        let principal = AuthenticatedPrincipal { tenant: h.tenant(), subject: accepted.intent.actor.clone(), credential_id: "fixture-session".into() };
        let digest = canonicalize_and_digest(&accepted.intent).unwrap();
        Self { intent: accepted.intent, events, commit, projections, principal, digest }
    }
    fn outbox(&self, count: usize, size: usize) -> Vec<OutboxRecord> {
        (0..count).map(|index| {
            let payload = vec![73; size];
            OutboxRecord { delivery_key: format!("delivery:{index}"), tenant: self.intent.tenant_id.clone(),
                commit_id: self.commit.body.commit_id.clone(), payload_digest: hash_bytes(&payload), payload }
        }).collect()
    }
    async fn run(&self, host: &Host, rows: &[OutboxRecord], projections: &[StateProjection]) -> String {
        let entries = [CommittedEvent { event: &self.events[0], state_type: StateType::UniqueAsset }];
        match persist_durable_verified(host, host, host, DurableCommitRequest {
            principal: &self.principal, intent: &self.intent, payload_digest: &self.digest, entries: &entries,
            commit: &self.commit, projections, outbox: rows,
        }).await {
            Ok(result) => format!("ok:{result:?}"),
            Err(error) => format!("error:{error:?}|{error}"),
        }
    }
}
async fn characterize(fixture: &Fixture) -> Value {
    let mut masks = vec![0];
    for first in 0..FAULTS { masks.push(1 << first); }
    for first in 0..FAULTS { for second in first+1..FAULTS { masks.push((1 << first) | (1 << second)); } }
    let mut result = Vec::new();
    for variant in 0..VARIANTS {
        let mut rows = fixture.outbox(2, 16);
        let mut projections = fixture.projections.clone();
        match variant {
            0 => rows.clear(),
            1 => { rows.pop(); },
            2 => {},
            3 => rows[0].delivery_key.clear(),
            4 => rows[0].delivery_key.push('\n'),
            5 => rows[0].delivery_key = "k".repeat(257),
            6 => rows[0].payload_digest = hash_bytes(b"wrong"),
            7 => { rows[0].payload = vec![1; 1024*1024+1]; rows[0].payload_digest = hash_bytes(&rows[0].payload); },
            8 => rows[1].delivery_key = rows[0].delivery_key.clone(),
            9 => rows[0].tenant = TenantId("other.game".into()),
            10 => rows[0].commit_id = CommitId::new("cmt_wrong_scope".into()).unwrap(),
            11 => { rows[0].delivery_key.clear(); rows[0].tenant = TenantId("other.game".into()); },
            12 => { rows[0].tenant = TenantId("other.game".into()); rows[1].payload_digest = hash_bytes(b"wrong"); },
            13 => { rows.push(rows[1].clone()); rows[1].delivery_key = rows[0].delivery_key.clone(); rows[2].payload_digest = hash_bytes(b"wrong"); },
            14 => { rows[0].commit_id = CommitId::new("cmt_wrong_scope".into()).unwrap(); rows[1].delivery_key = rows[0].delivery_key.clone(); },
            15 => { projections[0].resource_id = ResourceId("asset:wrong".into()); rows[0].delivery_key.clear(); },
            16 => { rows[0].payload.clear(); rows[0].payload_digest = hash_bytes(b""); },
            17 => { rows[1].tenant = TenantId("other.game".into()); rows[1].payload_digest = hash_bytes(b"wrong"); },
            _ => panic!("unknown outbox variant"),
        }
        for claim in 0..4 {
            for mask in &masks {
                let host = Host { calls: Arc::default(), faults: *mask, claim, trace: true };
                let outcome = fixture.run(&host, &rows, &projections).await;
                result.push(json!({"case":format!("outbox:{variant}:claim:{claim}:faults:{mask}"),"outcome":outcome,"calls":host.calls.lock().unwrap().clone()}));
            }
        }
    }
    json!(result)
}
async fn benchmark(fixture: &Fixture, selected: Option<&str>) -> Value {
    let mut output = Vec::new();
    for (count, size, iterations) in [(0,0,100), (1,0,100), (1,1024,100), (1,65536,50), (8,65536,10), (1,1024*1024,10)] {
        let name = format!("outbox-preflight-{count}x{size}");
        if selected.is_some_and(|s| s != name) { continue; }
        let rows = fixture.outbox(count, size);
        // Refuse at begin after complete preflight. This is neither a physical
        // commit benchmark nor a success receipt; both revisions do equal work.
        let host = Host { calls: Arc::default(), faults: 1 << 2, claim: 0, trace: false };
        for _ in 0..10 { black_box(fixture.run(&host, &rows, &fixture.projections).await); }
        let mut samples = Vec::new();
        for _ in 0..31 {
            let started = Instant::now();
            for _ in 0..iterations { black_box(fixture.run(&host, &rows, &fixture.projections).await); }
            samples.push(started.elapsed().as_nanos() as f64 / f64::from(iterations));
        }
        let mut sorted = samples.clone(); sorted.sort_by(f64::total_cmp);
        output.push(json!({"workload":name,"iterations_per_sample":iterations,"samples_ns_per_operation":samples,"median_ns":sorted[15],"p95_ns":sorted[29]}));
    }
    json!(output)
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let fixture = runtime.block_on(Fixture::new());
    let output = if args.get(1).is_some_and(|s| s == "--bench") {
        runtime.block_on(benchmark(&fixture, args.get(2).map(String::as_str)))
    } else { runtime.block_on(characterize(&fixture)) };
    println!("{}", serde_json::to_string(&output).unwrap());
}
