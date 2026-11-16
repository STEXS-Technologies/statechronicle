//! Independent bounded reference adapter; deliberately broken variants exercise the oracles.
use async_trait::async_trait;
use statechronicle::accumulator::sparse_merkle::StateAccumulator;
use statechronicle::commit::{
    batch::CommitBatch,
    builder::CommitBuilder,
    persist::{CommittedEvent, projections_for},
    sign::{sign_commit, verify_commit},
};
use statechronicle::core::{canonicalize::canonicalize_and_digest, digest::hash_bytes};
use statechronicle::domain::{
    commit::CommitScope,
    event::StateCommitment,
    intent::{KeyId, Nonce},
    resource_state::{ResourceState, UniqueAssetState},
    status::Status,
};
use statechronicle::ports::ledger_batch::{LedgerBatch, LedgerBatchLookup, LedgerBatchRecovery};
use statechronicle::ports::ledger_store::{
    IdempotencyClaim, LedgerLookup, LedgerMutation, LedgerRecovery, LedgerStore, LedgerStoreError,
    LedgerTransaction, OutboxRecord,
};
use statechronicle::{
    Commit, CommitId, ContentDigest, Event, EventId, Intent, IntentId, Operation, ProfileId,
    ResourceId, Signed, StateProjection, StateType, SubjectId, TenantId,
};
use statechronicle_conformance::batch::{LedgerBatchTestBackend, RetainedBatch};
use statechronicle_conformance::{
    LedgerTestBackend, Snapshot, check_atomic_visibility, check_head_contention, check_replay,
    check_rollback, check_tenant_isolation,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

type Identity = (String, String);
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Correct,
    EagerEvents,
    DuplicateReservation,
    IgnoreHead,
    ForgetOutbox,
    GlobalIdentity,
    CorruptLookup,
    IgnoreAttempt,
    DiscardCommit,
    EagerRetention,
    ForgetMember,
    SplitGroupLookup,
    ForgetFinalization,
    FabricateSingleton,
    FilterBatchAsSingle,
    IgnoreBatchIdentity,
}
#[derive(Default)]
struct Database {
    visible: Snapshot,
    records: BTreeMap<Identity, LedgerMutation>,
    batches: BTreeMap<Identity, LedgerBatch>,
    leases: BTreeMap<Identity, (ContentDigest, String)>,
    attempt: u64,
}
#[derive(Clone)]
struct Backend {
    db: Arc<Mutex<Database>>,
    mode: Mode,
}
#[derive(Clone)]
struct Connection(Backend);
struct Transaction {
    connection: Connection,
    tenant: TenantId,
    reservations: Vec<(Identity, ContentDigest, String, Intent)>,
    batch: Option<LedgerBatch>,
    staged: Snapshot,
    finalized: bool,
}
impl Backend {
    fn new(mode: Mode) -> Self {
        Self {
            db: Arc::default(),
            mode,
        }
    }
    fn key(&self, tenant: &TenantId, intent: &IntentId) -> Identity {
        (
            (if self.mode == Mode::GlobalIdentity {
                "global"
            } else {
                &tenant.0
            })
            .to_owned(),
            intent.as_str().to_owned(),
        )
    }
}
#[async_trait]
impl LedgerTestBackend for Backend {
    type Connection = Connection;
    async fn reset(&self) -> Result<(), String> {
        *self.db.lock().unwrap() = Database::default();
        Ok(())
    }
    async fn connect(&self) -> Result<Connection, String> {
        Ok(Connection(self.clone()))
    }
    async fn inspect(&self) -> Result<Snapshot, String> {
        Ok(self.db.lock().unwrap().visible.clone())
    }
}
#[async_trait]
impl LedgerBatchTestBackend for Backend {
    async fn inspect_batches(&self) -> Result<Vec<RetainedBatch>, String> {
        Ok(self
            .db
            .lock()
            .unwrap()
            .batches
            .iter()
            .map(|((tenant, intent), batch)| RetainedBatch {
                tenant: TenantId(tenant.clone()),
                intent_id: IntentId::new(intent.clone()).unwrap(),
                batch: batch.clone(),
            })
            .collect())
    }
}
#[async_trait]
impl LedgerStore for Connection {
    async fn begin(
        &self,
        tenant: &TenantId,
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        Ok(Box::new(Transaction {
            connection: self.clone(),
            tenant: tenant.clone(),
            reservations: Vec::new(),
            batch: None,
            staged: Snapshot::default(),
            finalized: false,
        }))
    }
    async fn begin_multi(
        &self,
        _: &[TenantId],
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        Err(LedgerStoreError::Invariant(
            "reference supports one tenant transaction".into(),
        ))
    }
}
#[async_trait]
impl LedgerRecovery for Connection {
    async fn lookup_mutation(
        &self,
        tenant: &TenantId,
        intent: &IntentId,
    ) -> Result<LedgerLookup, LedgerStoreError> {
        let db = self.0.db.lock().unwrap();
        let key = self.0.key(tenant, intent);
        if let Some(batch) = db.batches.get(&key) {
            if self.0.mode == Mode::FilterBatchAsSingle {
                let intent = batch
                    .intents
                    .iter()
                    .find(|member| member.intent_id == *intent)
                    .unwrap()
                    .clone();
                return Ok(LedgerLookup::Committed(Box::new(LedgerMutation {
                    payload_digest: canonicalize_and_digest(&intent).unwrap(),
                    intent,
                    commit: batch.commit.clone(),
                    events: batch.events.clone(),
                    projections: batch.projections.clone(),
                    outbox: batch.outbox.clone(),
                })));
            }
            return Err(invariant("batch member is not a standalone mutation"));
        }
        if let Some(record) = db.records.get(&key) {
            let mut record = record.clone();
            if self.0.mode == Mode::CorruptLookup {
                record.outbox.clear();
            }
            return Ok(LedgerLookup::Committed(Box::new(record)));
        }
        if let Some((payload_digest, attempt_id)) = db.leases.get(&key) {
            return Ok(LedgerLookup::InProgress {
                payload_digest: payload_digest.clone(),
                attempt_id: attempt_id.clone(),
                lease_expires_at_unix: i64::MAX,
            });
        }
        Ok(LedgerLookup::NotFound)
    }
}
#[async_trait]
impl LedgerBatchRecovery for Connection {
    async fn lookup_batch(
        &self,
        tenant: &TenantId,
        intent: &IntentId,
    ) -> Result<LedgerBatchLookup, LedgerStoreError> {
        let db = self.0.db.lock().unwrap();
        let key = self.0.key(tenant, intent);
        if let Some(batch) = db.batches.get(&key) {
            let mut batch = batch.clone();
            if self.0.mode == Mode::SplitGroupLookup {
                batch.intents.retain(|member| member.intent_id == *intent);
                batch.events.retain(|event| event.intent_id == *intent);
            }
            return Ok(LedgerBatchLookup::Committed(Box::new(batch)));
        }
        if let Some(record) = db.records.get(&key) {
            if self.0.mode == Mode::FabricateSingleton {
                return Ok(LedgerBatchLookup::Committed(Box::new(LedgerBatch {
                    intents: vec![record.intent.clone()],
                    commit: record.commit.clone(),
                    events: record.events.clone(),
                    projections: record.projections.clone(),
                    outbox: record.outbox.clone(),
                })));
            }
            return Err(invariant("standalone mutation is not a batch"));
        }
        if db
            .visible
            .finalized
            .iter()
            .any(|(scope, id, ..)| scope == tenant && id == intent)
        {
            return Err(invariant("finalized member lacks original batch"));
        }
        if let Some((payload_digest, attempt_id)) = db.leases.get(&key) {
            return Ok(LedgerBatchLookup::InProgress {
                payload_digest: payload_digest.clone(),
                attempt_id: attempt_id.clone(),
                lease_expires_at_unix: i64::MAX,
            });
        }
        Ok(LedgerBatchLookup::NotFound)
    }
}
fn invariant(detail: &str) -> LedgerStoreError {
    LedgerStoreError::Invariant(detail.into())
}
impl Transaction {
    fn tenant(&self, tenant: &TenantId) -> Result<(), LedgerStoreError> {
        if tenant == &self.tenant {
            Ok(())
        } else {
            Err(invariant("transaction scope mismatch"))
        }
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        for (key, digest, attempt, _) in &self.reservations {
            let mut db = self.connection.0.db.lock().unwrap();
            if db.leases.get(key) == Some(&(digest.clone(), attempt.clone())) {
                db.leases.remove(key);
            }
        }
    }
}
#[async_trait]
impl LedgerTransaction for Transaction {
    async fn claim_idempotency(
        &mut self,
        tenant: &TenantId,
        intent: &Intent,
        digest: &ContentDigest,
    ) -> Result<IdempotencyClaim, LedgerStoreError> {
        tokio::task::yield_now().await;
        self.tenant(tenant)?;
        self.tenant(&intent.tenant_id)?;
        let key = self.connection.0.key(tenant, &intent.intent_id);
        if self
            .reservations
            .iter()
            .any(|(reserved, ..)| reserved == &key)
        {
            return Err(invariant("member already reserved"));
        }

        let mut db = self.connection.0.db.lock().unwrap();
        if self.connection.0.mode != Mode::DuplicateReservation {
            if let Some(batch) = db
                .batches
                .get(&key)
                .filter(|_| self.connection.0.mode != Mode::IgnoreBatchIdentity)
            {
                let original = batch
                    .intents
                    .iter()
                    .find(|member| member.intent_id == intent.intent_id)
                    .ok_or_else(|| invariant("missing retained member"))?;
                return Ok(if canonicalize_and_digest(original).unwrap() == *digest {
                    IdempotencyClaim::Committed {
                        commit_id: batch.commit.body.commit_id.clone(),
                    }
                } else {
                    IdempotencyClaim::ConflictDifferentPayload
                });
            }

            if let Some(record) = db.records.get(&key) {
                return Ok(if record.payload_digest == *digest {
                    IdempotencyClaim::Committed {
                        commit_id: record.commit.body.commit_id.clone(),
                    }
                } else {
                    IdempotencyClaim::ConflictDifferentPayload
                });
            }
        }
        if let Some((original, attempt)) = db.leases.get(&key) {
            return Ok(if original == digest {
                IdempotencyClaim::InProgress {
                    attempt_id: attempt.clone(),
                    lease_expires_at_unix: i64::MAX,
                }
            } else {
                IdempotencyClaim::ConflictDifferentPayload
            });
        }
        db.attempt += 1;
        let attempt = format!("attempt-{}", db.attempt);
        db.leases
            .insert(key.clone(), (digest.clone(), attempt.clone()));
        self.reservations
            .push((key, digest.clone(), attempt.clone(), intent.clone()));
        Ok(IdempotencyClaim::NewReservation {
            attempt_id: attempt,
        })
    }
    async fn append_events(&mut self, events: &[Event]) -> Result<(), LedgerStoreError> {
        tokio::task::yield_now().await;
        for event in events {
            self.tenant(&event.tenant_id)?;
        }
        self.staged.events.extend_from_slice(events);
        if self.connection.0.mode == Mode::EagerEvents {
            self.connection
                .0
                .db
                .lock()
                .unwrap()
                .visible
                .events
                .extend_from_slice(events);
        }
        Ok(())
    }
    async fn append_commit(&mut self, commit: &Signed<Commit>) -> Result<(), LedgerStoreError> {
        tokio::task::yield_now().await;
        if commit.body.scope.tenant_id.as_ref() != Some(&self.tenant) {
            return Err(invariant("commit scope mismatch"));
        }
        if !self.staged.commits.is_empty() {
            return Err(invariant("only one commit per reference transaction"));
        }
        self.staged.commits.push(commit.clone());
        Ok(())
    }
    async fn upsert_projection(
        &mut self,
        projection: &StateProjection,
    ) -> Result<(), LedgerStoreError> {
        tokio::task::yield_now().await;
        self.tenant(&projection.tenant_id)?;
        self.staged.projections.push(projection.clone());
        Ok(())
    }
    async fn enqueue_outbox(&mut self, record: &OutboxRecord) -> Result<(), LedgerStoreError> {
        tokio::task::yield_now().await;
        self.tenant(&record.tenant)?;
        self.staged.outbox.push(record.clone());
        Ok(())
    }
    async fn finalize_idempotency(
        &mut self,
        tenant: &TenantId,
        intent_id: &IntentId,
        attempt: &str,
        commit: &CommitId,
    ) -> Result<(), LedgerStoreError> {
        tokio::task::yield_now().await;
        self.tenant(tenant)?;
        let Some((key, digest, token, intent)) = self
            .reservations
            .iter()
            .find(|(key, ..)| key == &self.connection.0.key(tenant, intent_id))
        else {
            return Err(invariant("no reservation"));
        };
        if key != &self.connection.0.key(tenant, intent_id)
            || (token != attempt && self.connection.0.mode != Mode::IgnoreAttempt)
            || intent.intent_id != *intent_id
            || self.staged.commits.first().map(|row| &row.body.commit_id) != Some(commit)
        {
            return Err(invariant("finalization identity mismatch"));
        }
        self.staged.finalized.push((
            tenant.clone(),
            intent_id.clone(),
            digest.clone(),
            commit.clone(),
        ));
        self.finalized = true;
        Ok(())
    }
    async fn retain_batch(&mut self, batch: &LedgerBatch) -> Result<(), LedgerStoreError> {
        self.tenant(
            batch
                .commit
                .body
                .scope
                .tenant_id
                .as_ref()
                .ok_or_else(|| invariant("missing tenant"))?,
        )?;
        if self.batch.is_some() {
            return Err(invariant("batch already retained"));
        }
        self.batch = Some(batch.clone());
        if self.connection.0.mode == Mode::EagerRetention {
            let mut db = self.connection.0.db.lock().unwrap();
            for intent in &batch.intents {
                db.batches.insert(
                    self.connection.0.key(&intent.tenant_id, &intent.intent_id),
                    batch.clone(),
                );
            }
        }
        Ok(())
    }
    async fn commit(mut self: Box<Self>) -> Result<(), LedgerStoreError> {
        tokio::task::yield_now().await;
        let Some((key, digest, token, intent)) = self.reservations.first() else {
            return Err(invariant("no reservation"));
        };
        let Some(commit) = self.staged.commits.first().cloned() else {
            return Err(invariant("no commit"));
        };
        if !self.finalized || self.staged.events.is_empty() || self.staged.projections.is_empty() {
            return Err(invariant("incomplete mutation"));
        }
        let mut db = self.connection.0.db.lock().unwrap();
        if db.leases.get(key) != Some(&(digest.clone(), token.clone())) {
            return Err(invariant("reservation fenced"));
        }
        if self.reservations.len() != self.staged.finalized.len()
            || self
                .reservations
                .iter()
                .any(|(key, digest, token, intent)| {
                    db.leases.get(key) != Some(&(digest.clone(), token.clone()))
                        || !self.staged.finalized.contains(&(
                            intent.tenant_id.clone(),
                            intent.intent_id.clone(),
                            digest.clone(),
                            commit.body.commit_id.clone(),
                        ))
                })
        {
            return Err(invariant("not every fenced member finalized"));
        }
        if let Some(batch) = &self.batch {
            let original: BTreeMap<_, _> = batch
                .intents
                .iter()
                .map(|intent| (&intent.intent_id, intent))
                .collect();
            let reserved: BTreeMap<_, _> = self
                .reservations
                .iter()
                .map(|(_, _, _, intent)| (&intent.intent_id, intent))
                .collect();
            if original != reserved
                || batch.intents.len() != reserved.len()
                || batch.commit != commit
                || batch.events != self.staged.events
                || batch.projections != self.staged.projections
                || batch.outbox != self.staged.outbox
            {
                return Err(invariant(
                    "batch retention differs from complete staged mutation",
                ));
            }
        } else if self.reservations.len() != 1 {
            return Err(invariant("multiple members without group retention"));
        }
        let head = db
            .visible
            .heads
            .iter()
            .find(|(tenant, ..)| tenant == &self.tenant);
        let sequence = head.map_or(1, |(_, _, _, sequence)| sequence + 1);
        let parent = head.map(|(_, commit, ..)| commit);
        let root = head.map_or_else(
            || ContentDigest::new(*StateAccumulator::empty().root().as_bytes()),
            |(_, _, root, _)| root.clone(),
        );
        if self.connection.0.mode != Mode::IgnoreHead
            && (commit.body.sequence != sequence
                || commit.body.parent_commit_id.as_ref() != parent
                || commit.body.previous_state_root != root)
        {
            return Err(LedgerStoreError::Conflict("stale canonical head".into()));
        }
        if self.connection.0.mode == Mode::DiscardCommit {
            return Ok(());
        }
        if self.connection.0.mode == Mode::ForgetOutbox {
            self.staged.outbox.clear();
        }
        if self.connection.0.mode == Mode::ForgetFinalization {
            self.staged.finalized.pop();
        }
        if let Some(batch) = &self.batch {
            let count = if self.connection.0.mode == Mode::ForgetMember {
                batch.intents.len().saturating_sub(1)
            } else {
                batch.intents.len()
            };
            for intent in batch.intents.iter().take(count) {
                db.batches.insert(
                    self.connection.0.key(&intent.tenant_id, &intent.intent_id),
                    batch.clone(),
                );
            }
        } else {
            let record = LedgerMutation {
                intent: intent.clone(),
                payload_digest: digest.clone(),
                commit: commit.clone(),
                events: self.staged.events.clone(),
                projections: self.staged.projections.clone(),
                outbox: self.staged.outbox.clone(),
            };
            db.records.insert(key.clone(), record);
        }
        if self.connection.0.mode != Mode::EagerEvents {
            db.visible.events.append(&mut self.staged.events);
        }
        db.visible.commits.append(&mut self.staged.commits);
        db.visible.projections.append(&mut self.staged.projections);
        db.visible.outbox.append(&mut self.staged.outbox);
        db.visible.finalized.append(&mut self.staged.finalized);
        db.visible
            .heads
            .retain(|(tenant, ..)| tenant != &self.tenant);
        db.visible.heads.push((
            self.tenant.clone(),
            commit.body.commit_id,
            commit.body.next_state_root,
            commit.body.sequence,
        ));
        drop(db);
        Ok(())
    }
    async fn rollback(self: Box<Self>) -> Result<(), LedgerStoreError> {
        tokio::task::yield_now().await;
        Ok(())
    }
}

fn fixture(tenant: &str, suffix: usize) -> LedgerMutation {
    let tenant = TenantId(tenant.into());
    let intent_id = IntentId::new(format!("int_fixture_{suffix}")).unwrap();
    let resource = ResourceId(format!("asset:{suffix}"));
    let actor = SubjectId("account:alice".into());
    let executor = SubjectId("service:fixture".into());
    let now = chrono::DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let intent = Intent::builder()
        .tenant(tenant.clone())
        .intent_id(intent_id.clone())
        .operation(Operation::from_static("asset.mint"))
        .actor(actor.clone())
        .resource(resource.clone())
        .state_type(StateType::UniqueAsset)
        .expected_version(0)
        .inputs([("to_owner".into(), serde_json::json!("account:alice"))].into())
        .created_at(now)
        .nonce(Nonce::from_bytes(vec![1]).unwrap())
        .build()
        .unwrap();
    let commitment = |version, owner: &str, status| {
        let state = ResourceState::UniqueAsset(UniqueAssetState {
            owner: SubjectId(owner.into()),
            status: Status::from_static(status),
            trade_id: None,
        });
        StateCommitment {
            version,
            state_hash: canonicalize_and_digest(&state).unwrap(),
            state,
        }
    };
    let event = Event::new(
        tenant.clone(),
        EventId::new(format!("evt_fixture_{suffix}")).unwrap(),
        intent_id,
        intent.operation.clone(),
        resource,
        actor,
        commitment(0, "", "unborn"),
        commitment(1, "account:alice", "active"),
        None,
        executor.clone(),
        now,
    );
    let scope = CommitScope::tenant(tenant.clone());
    let mut batch = CommitBatch::new(scope.clone());
    batch.add_event(event.clone()).unwrap();
    let body = CommitBuilder::builder()
        .scope(scope)
        .sequence(1)
        .executor(executor)
        .profile(ProfileId::new("statechronicle.profile.resource.v0".into()).unwrap())
        .created_at(now)
        .build(
            &batch,
            ContentDigest::new(*StateAccumulator::empty().root().as_bytes()),
            &[],
            || Ok(CommitId::new(format!("cmt_fixture_{suffix}")).unwrap()),
        )
        .unwrap();
    let signing = ed25519_dalek::SigningKey::from_bytes(&[77; 32]);
    let commit = sign_commit(
        &body,
        &signing,
        KeyId::new("test:conformance-key".into()).unwrap(),
    )
    .unwrap();
    verify_commit(&commit, &signing.verifying_key()).unwrap();
    let projections = projections_for(
        &commit.body.commit_id,
        &[CommittedEvent {
            event: &event,
            state_type: StateType::UniqueAsset,
        }],
    );
    let outbox = (0..2)
        .map(|index| {
            let payload = vec![index, 7];
            OutboxRecord {
                delivery_key: format!("notify-{suffix}-{index}"),
                tenant: tenant.clone(),
                commit_id: commit.body.commit_id.clone(),
                payload_digest: hash_bytes(&payload),
                payload,
            }
        })
        .collect();
    LedgerMutation {
        payload_digest: canonicalize_and_digest(&intent).unwrap(),
        intent,
        commit,
        events: vec![event],
        projections,
        outbox,
    }
}

#[tokio::test]
async fn reference_passes_independent_transaction_contract() {
    let backend = Backend::new(Mode::Correct);
    let first = fixture("acme.game.alpha", 0);
    check_atomic_visibility(&backend, &first).await.unwrap();
    check_rollback(&backend, &first).await.unwrap();
    statechronicle_conformance::check_cancellation(&backend, &first)
        .await
        .unwrap();
    check_replay(&backend, &first).await.unwrap();
    check_tenant_isolation(&backend, &first, &fixture("acme.game.beta", 0))
        .await
        .unwrap();
    for clients in [2, 8, 32] {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            statechronicle_conformance::check_idempotency_contention(&backend, &first, clients),
        )
        .await
        .unwrap()
        .unwrap();
        let contenders: Vec<_> = (0..clients)
            .map(|index| fixture("acme.game.alpha", index))
            .collect();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            check_head_contention(&backend, &contenders),
        )
        .await
        .unwrap()
        .unwrap();
    }
}
#[tokio::test]
async fn negative_controls_fail_the_intended_oracles() {
    let first = fixture("acme.game.alpha", 0);
    for (mode, check) in [
        (Mode::EagerEvents, "atomic.visibility"),
        (Mode::ForgetOutbox, "atomic.completeness"),
        (Mode::CorruptLookup, "lookup.original"),
    ] {
        let error = check_atomic_visibility(&Backend::new(mode), &first)
            .await
            .unwrap_err();
        assert_eq!(error.check, check, "{mode:?}: {error}");
    }
    let error = check_replay(&Backend::new(Mode::DuplicateReservation), &first)
        .await
        .unwrap_err();
    assert_eq!(error.check, "replay.identity");
    let contenders: Vec<_> = (0..8)
        .map(|index| fixture("acme.game.alpha", index))
        .collect();
    let error = check_head_contention(&Backend::new(Mode::IgnoreHead), &contenders)
        .await
        .unwrap_err();
    assert_eq!(error.check, "head.single_winner");
    let error = check_tenant_isolation(
        &Backend::new(Mode::GlobalIdentity),
        &first,
        &fixture("acme.game.beta", 0),
    )
    .await
    .unwrap_err();
    assert_eq!(error.check, "tenant.claim");
}

struct FixturePolicy;
#[async_trait]
impl statechronicle::ports::authorization::Authorizer for FixturePolicy {
    async fn authorize(
        &self,
        context: statechronicle::ports::authorization::AuthorizationContext<'_>,
    ) -> Result<(), statechronicle::ports::authorization::AuthorizationError> {
        if context.principal.subject == SubjectId("account:alice".into())
            && context.principal.tenant == TenantId("acme.game.alpha".into())
            && context.tenant == &context.principal.tenant
            && context.intent.actor == context.principal.subject
            && context.intent.operation.as_str() == "asset.mint"
        {
            Ok(())
        } else {
            Err(statechronicle::ports::authorization::AuthorizationError::Denied)
        }
    }
}
struct FixtureVerifier;
impl statechronicle::commit::persist::SignedCommitVerifier for FixtureVerifier {
    fn verify(&self, commit: &Signed<Commit>) -> Result<(), String> {
        if commit.signature.key_id.as_str() != "test:conformance-key"
            || commit.body.executor != SubjectId("service:fixture".into())
        {
            return Err("untrusted fixture signer".into());
        }
        verify_commit(
            commit,
            &ed25519_dalek::SigningKey::from_bytes(&[77; 32]).verifying_key(),
        )
        .map_err(|error| error.to_string())
    }
}
#[tokio::test]
async fn lost_ack_recovers_and_replays_original_without_duplicate_effects() {
    let mutation = fixture("acme.game.alpha", 0);
    let principal = statechronicle::ports::authorization::AuthenticatedPrincipal {
        subject: SubjectId("account:alice".into()),
        tenant: TenantId("acme.game.alpha".into()),
        credential_id: "test-session".into(),
    };
    statechronicle_conformance::check_lost_ack(
        &Backend::new(Mode::Correct),
        &mutation,
        &principal,
        &FixturePolicy,
        &FixtureVerifier,
    )
    .await
    .unwrap();
    let error = statechronicle_conformance::check_lost_ack(
        &Backend::new(Mode::DiscardCommit),
        &mutation,
        &principal,
        &FixturePolicy,
        &FixtureVerifier,
    )
    .await
    .unwrap_err();
    assert_eq!(error.check, "ack.original");
}
#[tokio::test]
async fn token_fencing_and_unsupported_multi_tenant_scope_are_enforced() {
    let mutation = fixture("acme.game.alpha", 0);
    statechronicle_conformance::check_attempt_fencing(&Backend::new(Mode::Correct), &mutation)
        .await
        .unwrap();
    let error = statechronicle_conformance::check_attempt_fencing(
        &Backend::new(Mode::IgnoreAttempt),
        &mutation,
    )
    .await
    .unwrap_err();
    assert_eq!(error.check, "fencing.attempt");
    statechronicle_conformance::check_unsupported_multi(
        &Backend::new(Mode::Correct),
        &[
            TenantId("acme.game.alpha".into()),
            TenantId("acme.game.beta".into()),
        ],
    )
    .await
    .unwrap();
}

fn batch_fixture(suffix: usize) -> LedgerBatch {
    batch_fixture_at("acme.game.alpha", suffix)
}
fn batch_fixture_at(tenant: &str, suffix: usize) -> LedgerBatch {
    let left = fixture(tenant, suffix * 2);
    let right = fixture(tenant, suffix * 2 + 1);
    let events: Vec<_> = left.events.iter().chain(&right.events).cloned().collect();
    let mut group = CommitBatch::new(left.commit.body.scope.clone());
    for event in &events {
        group.add_event(event.clone()).unwrap();
    }
    let body = CommitBuilder::builder()
        .scope(left.commit.body.scope.clone())
        .sequence(1)
        .executor(left.commit.body.executor.clone())
        .profile(left.commit.body.profile.clone())
        .created_at(left.commit.body.created_at)
        .build(
            &group,
            left.commit.body.previous_state_root.clone(),
            &[],
            || Ok(CommitId::new(format!("cmt_batch_{suffix}")).unwrap()),
        )
        .unwrap();
    let commit = sign_commit(
        &body,
        &ed25519_dalek::SigningKey::from_bytes(&[77; 32]),
        KeyId::new("test:conformance-key".into()).unwrap(),
    )
    .unwrap();
    let entries: Vec<_> = events
        .iter()
        .map(|event| CommittedEvent {
            event,
            state_type: StateType::UniqueAsset,
        })
        .collect();
    let projections = projections_for(&commit.body.commit_id, &entries);
    let mut outbox: Vec<_> = left.outbox.into_iter().chain(right.outbox).collect();
    for row in &mut outbox {
        row.commit_id = commit.body.commit_id.clone();
    }
    LedgerBatch {
        intents: vec![left.intent, right.intent],
        commit,
        events,
        projections,
        outbox,
    }
}
#[tokio::test]
async fn reference_passes_optional_batch_contract() {
    use statechronicle_conformance::batch;
    let backend = Backend::new(Mode::Correct);
    let group = batch_fixture(0);
    batch::check_atomic_visibility(&backend, &group)
        .await
        .unwrap();
    batch::check_rollback(&backend, &group).await.unwrap();
    batch::check_cancellation(&backend, &group).await.unwrap();
    batch::check_replay(&backend, &group).await.unwrap();
    let mut nonce_collision_fixture = group.clone();
    for intent in &mut nonce_collision_fixture.intents {
        intent.nonce = Nonce::from_bytes(vec![0x97, 0x23]).unwrap();
    }
    batch::check_replay(&backend, &nonce_collision_fixture)
        .await
        .unwrap();
    let principals: Vec<_> = group
        .intents
        .iter()
        .map(
            |intent| statechronicle::ports::authorization::AuthenticatedPrincipal {
                tenant: intent.tenant_id.clone(),
                subject: intent.actor.clone(),
                credential_id: "test-session".into(),
            },
        )
        .collect();
    batch::check_lost_ack(
        &backend,
        &group,
        &principals,
        &FixturePolicy,
        &FixtureVerifier,
    )
    .await
    .unwrap();
    for clients in [2, 8, 32] {
        let contenders: Vec<_> = (0..clients).map(batch_fixture).collect();
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            batch::check_head_contention(&backend, &contenders),
        )
        .await
        .unwrap()
        .unwrap();
    }
}
#[tokio::test]
async fn independently_broken_batch_adapters_fail_the_intended_oracles() {
    use statechronicle_conformance::batch;
    let group = batch_fixture(0);
    for (mode, check) in [
        (Mode::EagerEvents, "batch.atomic.visibility"),
        (Mode::EagerRetention, "batch.atomic.visibility"),
        (Mode::ForgetOutbox, "batch.atomic.completeness"),
        (Mode::ForgetMember, "batch.atomic.completeness"),
        (Mode::ForgetFinalization, "batch.atomic.completeness"),
        (Mode::SplitGroupLookup, "batch.lookup.original"),
    ] {
        let error = batch::check_atomic_visibility(&Backend::new(mode), &group)
            .await
            .unwrap_err();
        assert_eq!(error.check, check, "{mode:?}: {error}");
    }
    let error = batch::check_replay(&Backend::new(Mode::DuplicateReservation), &group)
        .await
        .unwrap_err();
    assert_eq!(error.check, "batch.replay.identity");
    let error = batch::check_head_contention(
        &Backend::new(Mode::IgnoreHead),
        &[batch_fixture(0), batch_fixture(1)],
    )
    .await
    .unwrap_err();
    assert_eq!(error.check, "batch.head.single_winner");
    let principals: Vec<_> = group
        .intents
        .iter()
        .map(
            |intent| statechronicle::ports::authorization::AuthenticatedPrincipal {
                tenant: intent.tenant_id.clone(),
                subject: intent.actor.clone(),
                credential_id: "test-session".into(),
            },
        )
        .collect();
    let error = batch::check_lost_ack(
        &Backend::new(Mode::DiscardCommit),
        &group,
        &principals,
        &FixturePolicy,
        &FixtureVerifier,
    )
    .await
    .unwrap_err();
    assert_eq!(error.check, "batch.ack.original");
}

#[tokio::test]
async fn batch_tenants_and_single_namespace_remain_distinct_authorities() {
    use statechronicle_conformance::batch;
    let backend = Backend::new(Mode::Correct);
    let group = batch_fixture(0);
    batch::check_tenant_isolation(&backend, &group, &batch_fixture_at("acme.game.beta", 0))
        .await
        .unwrap();
    batch::check_single_namespace(&backend, &fixture("acme.game.alpha", 0), &group)
        .await
        .unwrap();
    let principals: Vec<_> = group
        .intents
        .iter()
        .map(
            |intent| statechronicle::ports::authorization::AuthenticatedPrincipal {
                tenant: intent.tenant_id.clone(),
                subject: intent.actor.clone(),
                credential_id: "test-session".into(),
            },
        )
        .collect();
    for clients in [2, 8, 32] {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            batch::check_idempotency_contention(
                &backend,
                &group,
                &principals,
                Arc::new(FixturePolicy),
                Arc::new(FixtureVerifier),
                clients,
            ),
        )
        .await
        .unwrap()
        .unwrap();
    }
}
#[tokio::test]
async fn namespace_negative_controls_fail_before_any_false_receipt() {
    use statechronicle_conformance::batch;
    let group = batch_fixture(0);
    let error = batch::check_tenant_isolation(
        &Backend::new(Mode::GlobalIdentity),
        &group,
        &batch_fixture_at("acme.game.beta", 0),
    )
    .await
    .unwrap_err();
    assert_eq!(error.check, "batch.tenant.lookup");
    for (mode, check) in [
        (Mode::FabricateSingleton, "batch.namespace.singleton"),
        (Mode::FilterBatchAsSingle, "batch.namespace.subset"),
        (Mode::IgnoreBatchIdentity, "batch.namespace.claim"),
    ] {
        let error = batch::check_single_namespace(
            &Backend::new(mode),
            &fixture("acme.game.alpha", 0),
            &group,
        )
        .await
        .unwrap_err();
        assert_eq!(error.check, check, "{mode:?}: {error}");
    }
}
