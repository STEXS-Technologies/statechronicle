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
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

// Reference projection keys follow resource/subject addressing, independently
// of receipt lookup. Owners are values, not part of an owner-based state key.
fn same_projection_key(a: &StateProjection, b: &StateProjection) -> bool {
    a.tenant_id == b.tenant_id
        && a.resource_id == b.resource_id
        && a.state.subject() == b.state.subject()
}
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
    IgnoreSuperseded,
    ReleaseSuccessor,
    FenceFirstMemberOnly,
    ReuseExpiredToken,
    SharedExpiry,
    LieClaimToken,
    PruneOriginal,
    CurrentProjectionLookup,
    PendingOutboxLookup,
    AppendProjections,
    ResourceOnlyProjections,
    PruneBatchMemberAfterHead,
    ReplayLatestHead,
    CorruptOldSecondMemberLookup,
    PruneAfterFourthHead,
    LatestReceiptAfterFourthHead,
    RollbackForgetsHistory,
}
#[derive(Default)]
struct Database {
    visible: Snapshot,
    records: BTreeMap<Identity, LedgerMutation>,
    batches: BTreeMap<Identity, LedgerBatch>,
    leases: BTreeMap<Identity, (ContentDigest, String)>,
    attempt: u64,
    expired: BTreeSet<Identity>,
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
impl statechronicle_conformance::LedgerOutboxTestBackend for Backend {
    async fn acknowledge_outbox(&self, rows: &[OutboxRecord]) -> Result<(), String> {
        self.db.lock().unwrap().visible.outbox.retain(|row| {
            !rows
                .iter()
                .any(|old| old.tenant == row.tenant && old.delivery_key == row.delivery_key)
        });
        Ok(())
    }
}
#[async_trait]
impl statechronicle_conformance::LedgerLeaseTestBackend for Backend {
    async fn expire_reservation(&self, tenant: &TenantId, intent: &IntentId) -> Result<(), String> {
        let key = self.key(tenant, intent);
        let mut db = self.db.lock().unwrap();
        if !db.leases.contains_key(&key) {
            return Err("no live lease to expire".into());
        }
        if self.mode == Mode::SharedExpiry {
            let keys: Vec<_> = db.leases.keys().cloned().collect();
            db.expired.extend(keys);
        } else {
            db.expired.insert(key);
        }
        Ok(())
    }
}
#[async_trait]
impl statechronicle_conformance::batch::LedgerBatchMemberLeaseTestBackend for Backend {
    async fn configure_member_expiry(
        &self,
        batch: &LedgerBatch,
        member: &IntentId,
    ) -> Result<(), String> {
        if self.mode == Mode::SharedExpiry {
            return Err("shared member deadlines do not support isolated expiry".into());
        }
        // This bounded model already expires by key. It changes eligibility
        // only; normal claim code replaces the token, and all others stay live.
        if batch
            .intents
            .iter()
            .any(|intent| intent.intent_id == *member)
        {
            Ok(())
        } else {
            Err("unknown expiry fixture member".into())
        }
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
            if self.0.mode == Mode::LatestReceiptAfterFourthHead
                && let Some((_, head, _, sequence)) =
                    db.visible.heads.iter().find(|(scope, ..)| scope == tenant)
                && *sequence >= 4
            {
                record.commit = db
                    .visible
                    .commits
                    .iter()
                    .find(|signed| {
                        signed.body.commit_id == *head
                            && signed.body.scope == record.commit.body.scope
                    })
                    .unwrap()
                    .clone();
            }
            if self.0.mode == Mode::CorruptLookup {
                record.outbox.clear();
            }
            if self.0.mode == Mode::PendingOutboxLookup {
                record.outbox.retain(|old| {
                    db.visible
                        .outbox
                        .iter()
                        .any(|row| old.tenant == row.tenant && old.delivery_key == row.delivery_key)
                });
            }
            if self.0.mode == Mode::CurrentProjectionLookup {
                record.projections = db.visible.projections.clone();
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
            if self.0.mode == Mode::CurrentProjectionLookup
                || (self.0.mode == Mode::CorruptOldSecondMemberLookup
                    && batch.commit.body.sequence == 1
                    && batch
                        .intents
                        .get(1)
                        .is_some_and(|member| member.intent_id == *intent)
                    && db
                        .visible
                        .heads
                        .iter()
                        .any(|(scope, _, _, sequence)| scope == tenant && *sequence > 1))
            {
                batch.projections = db.visible.projections.clone();
            }
            if self.0.mode == Mode::PendingOutboxLookup {
                batch.outbox.retain(|old| {
                    db.visible
                        .outbox
                        .iter()
                        .any(|row| old.tenant == row.tenant && old.delivery_key == row.delivery_key)
                });
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
            if self.connection.0.mode == Mode::ReleaseSuccessor
                || db.leases.get(key) == Some(&(digest.clone(), attempt.clone()))
            {
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
                        commit_id: if self.connection.0.mode == Mode::ReplayLatestHead {
                            db.visible
                                .heads
                                .iter()
                                .find(|(tenant, ..)| tenant == &intent.tenant_id)
                                .unwrap()
                                .1
                                .clone()
                        } else {
                            batch.commit.body.commit_id.clone()
                        },
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
            if original != digest {
                return Ok(IdempotencyClaim::ConflictDifferentPayload);
            }
            if !db.expired.contains(&key) {
                return Ok(IdempotencyClaim::InProgress {
                    attempt_id: attempt.clone(),
                    lease_expires_at_unix: i64::MAX,
                });
            }
        }
        let expired_token = db.leases.get(&key).map(|(_, token)| token.clone());
        let takeover = expired_token.is_some();
        db.expired.remove(&key);
        db.attempt += 1;
        let attempt = if self.connection.0.mode == Mode::ReuseExpiredToken {
            expired_token.unwrap_or_else(|| format!("attempt-{}", db.attempt))
        } else {
            format!("attempt-{}", db.attempt)
        };
        db.leases
            .insert(key.clone(), (digest.clone(), attempt.clone()));
        self.reservations
            .push((key, digest.clone(), attempt.clone(), intent.clone()));
        Ok(IdempotencyClaim::NewReservation {
            attempt_id: if self.connection.0.mode == Mode::LieClaimToken && takeover {
                format!("{attempt}-reported")
            } else {
                attempt
            },
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
        if self.connection.0.mode != Mode::IgnoreSuperseded
            && db.leases.get(key) != Some(&(digest.clone(), token.clone()))
        {
            return Err(invariant("reservation fenced"));
        }
        if self.reservations.len() != self.staged.finalized.len()
            || self
                .reservations
                .iter()
                .any(|(key, digest, token, intent)| {
                    (!matches!(
                        self.connection.0.mode,
                        Mode::IgnoreSuperseded | Mode::FenceFirstMemberOnly
                    ) && db.leases.get(key) != Some(&(digest.clone(), token.clone())))
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
        let mut projections = db.visible.projections.clone();
        for projection in &self.staged.projections {
            if projections.iter().any(|old| {
                same_projection_key(old, projection) && old.version >= projection.version
            }) {
                return Err(LedgerStoreError::Conflict(
                    "non-monotonic projection version".into(),
                ));
            }
            if self.connection.0.mode != Mode::AppendProjections {
                projections.retain(|old| {
                    if self.connection.0.mode == Mode::ResourceOnlyProjections {
                        old.tenant_id != projection.tenant_id
                            || old.resource_id != projection.resource_id
                    } else {
                        !same_projection_key(old, projection)
                    }
                });
            }
            projections.push(projection.clone());
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
        if self.connection.0.mode == Mode::PruneAfterFourthHead && commit.body.sequence >= 4 {
            db.records.retain(|(tenant, _), _| tenant != &self.tenant.0);
        }
        if self.connection.0.mode == Mode::PruneOriginal {
            db.records.clear();
            db.batches.clear();
        }
        if self.connection.0.mode == Mode::PruneBatchMemberAfterHead && commit.body.sequence > 1 {
            let key = db
                .batches
                .iter()
                .find(|(_, batch)| batch.commit.body.sequence == 1)
                .map(|(_, batch)| {
                    self.connection
                        .0
                        .key(&batch.intents[1].tenant_id, &batch.intents[1].intent_id)
                });
            if let Some(key) = key {
                db.batches.remove(&key);
            }
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
        db.visible.projections = projections;
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
        if self.connection.0.mode == Mode::RollbackForgetsHistory && !self.reservations.is_empty() {
            self.connection.0.db.lock().unwrap().visible.events.clear();
        }
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

#[tokio::test]
async fn expired_lease_takeover_fences_stale_commit_and_cleanup() {
    let mutation = fixture("alpha", 700);
    statechronicle_conformance::check_lease_takeover(&Backend::new(Mode::Correct), &mutation)
        .await
        .unwrap();
    for (mode, check) in [
        (Mode::IgnoreSuperseded, "lease.stale_commit"),
        (Mode::ReleaseSuccessor, "lease.successor"),
    ] {
        let error =
            statechronicle_conformance::check_lease_takeover(&Backend::new(mode), &mutation)
                .await
                .unwrap_err();
        assert_eq!(error.check, check);
    }
}

#[tokio::test]
async fn batch_lease_takeover_preserves_complete_successor_group() {
    use statechronicle_conformance::batch;
    let group = batch_fixture(800);
    for mode in [
        Mode::Correct,
        Mode::SharedExpiry,
        Mode::FenceFirstMemberOnly,
    ] {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            batch::check_lease_takeover(&Backend::new(mode), &group),
        )
        .await
        .unwrap()
        .unwrap();
    }
    let unsupported = batch::check_member_lease_takeover(&Backend::new(Mode::SharedExpiry), &group)
        .await
        .unwrap_err();
    assert_eq!(unsupported.check, "adapter.operation");
    assert!(unsupported.detail.contains("shared member deadlines"));
    for (mode, check) in [
        (Mode::IgnoreSuperseded, "batch.lease.stale_commit"),
        (Mode::ReleaseSuccessor, "batch.lease.successor"),
        (Mode::ReuseExpiredToken, "batch.lease.token"),
    ] {
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            batch::check_lease_takeover(&Backend::new(mode), &group),
        )
        .await
        .unwrap()
        .unwrap_err();
        println!("{mode:?}: {error}");
        assert_eq!(error.check, check, "{mode:?}: {error}");
    }
}
#[tokio::test]
async fn batch_member_expiry_fences_nonfirst_member_and_stale_cleanup() {
    use statechronicle_conformance::batch;
    let group = batch_fixture(800);
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        batch::check_member_lease_takeover(&Backend::new(Mode::Correct), &group),
    )
    .await
    .unwrap()
    .unwrap();
    for (mode, check) in [
        (Mode::IgnoreSuperseded, "batch.lease.stale_member_commit"),
        (
            Mode::FenceFirstMemberOnly,
            "batch.lease.stale_member_commit",
        ),
        (Mode::LieClaimToken, "batch.lease.claim_token"),
        (Mode::ReleaseSuccessor, "batch.lease.successor"),
        (Mode::ReuseExpiredToken, "batch.lease.token"),
    ] {
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            batch::check_member_lease_takeover(&Backend::new(mode), &group),
        )
        .await
        .unwrap()
        .unwrap_err();
        println!("{mode:?}: {error}");
        assert_eq!(error.check, check, "{mode:?}: {error}");
        if mode == Mode::FenceFirstMemberOnly {
            assert!(error.detail.contains("Some(1)"), "{error}");
        }
    }
}

fn seal_after(mutation: &mut LedgerMutation, previous: Option<&LedgerMutation>) {
    // Keep the existing two-commit fixture contract explicit.
    if let Some(previous) = previous {
        assert_eq!(previous.commit.body.sequence, 1);
    }
    seal_after_history(mutation, &previous.into_iter().collect::<Vec<_>>());
}
fn seal_after_history(mutation: &mut LedgerMutation, history: &[&LedgerMutation]) {
    use statechronicle::commit::roots::state_root_updates;
    let previous = history.last().copied();
    let scope = CommitScope::tenant(mutation.intent.tenant_id.clone());
    let mut batch = CommitBatch::new(scope.clone());
    for event in &mutation.events {
        batch.add_event(event.clone()).unwrap();
    }
    let prior: Vec<_> = history
        .iter()
        .flat_map(|m| state_root_updates(&m.events).unwrap())
        .collect();
    let old = &mutation.commit.body;
    let body = CommitBuilder::builder()
        .scope(scope)
        .sequence(previous.map_or(1, |m| m.commit.body.sequence + 1))
        .parent(previous.map(|m| m.commit.body.commit_id.clone()))
        .executor(old.executor.clone())
        .profile(old.profile.clone())
        .created_at(old.created_at)
        .build(
            &batch,
            previous.map_or_else(
                || ContentDigest::new(*StateAccumulator::empty().root().as_bytes()),
                |m| m.commit.body.next_state_root.clone(),
            ),
            &prior,
            || Ok(old.commit_id.clone()),
        )
        .unwrap();
    let signing = ed25519_dalek::SigningKey::from_bytes(&[77; 32]);
    mutation.commit = sign_commit(
        &body,
        &signing,
        KeyId::new("test:conformance-key".into()).unwrap(),
    )
    .unwrap();
    verify_commit(&mutation.commit, &signing.verifying_key()).unwrap();
    mutation.payload_digest = canonicalize_and_digest(&mutation.intent).unwrap();
    let entries: Vec<_> = mutation
        .events
        .iter()
        .map(|event| CommittedEvent {
            event,
            state_type: event.after.state.state_type(),
        })
        .collect();
    mutation.projections = projections_for(&mutation.commit.body.commit_id, &entries);
}
fn expected_pair(
    first: &LedgerMutation,
    later: &LedgerMutation,
    projections: Vec<StateProjection>,
) -> Snapshot {
    Snapshot {
        events: first.events.iter().chain(&later.events).cloned().collect(),
        commits: vec![first.commit.clone(), later.commit.clone()],
        projections,
        outbox: first.outbox.iter().chain(&later.outbox).cloned().collect(),
        finalized: [first, later]
            .into_iter()
            .map(|m| {
                (
                    m.intent.tenant_id.clone(),
                    m.intent.intent_id.clone(),
                    m.payload_digest.clone(),
                    m.commit.body.commit_id.clone(),
                )
            })
            .collect(),
        heads: vec![(
            later.intent.tenant_id.clone(),
            later.commit.body.commit_id.clone(),
            later.commit.body.next_state_root.clone(),
            later.commit.body.sequence,
        )],
    }
}

#[tokio::test]
async fn overwritten_projection_does_not_rewrite_original_receipt() {
    use statechronicle::profiles::{registry::ProfileRules, unique_asset::UniqueAssetRules};
    use statechronicle_conformance::check_delivered_retention;
    let first = fixture("acme.game.alpha", 910);
    let mut later = fixture("acme.game.alpha", 911);
    later.intent.resource_id = first.intent.resource_id.clone();
    later.intent.operation = Operation::from_static("asset.transfer");
    later.intent.expected_version = 1;
    later.intent.inputs = [
        ("from_owner".into(), serde_json::json!("account:alice")),
        ("to_owner".into(), serde_json::json!("account:bob")),
    ]
    .into();
    UniqueAssetRules
        .check(
            &later.intent.operation,
            first.projections.first(),
            &later.intent.inputs,
        )
        .unwrap();
    let after = UniqueAssetRules
        .apply(
            first.projections.first(),
            &later.intent.operation,
            &later.intent.inputs,
        )
        .unwrap();
    let event = &mut later.events[0];
    event.resource_id = first.intent.resource_id.clone();
    event.operation = later.intent.operation.clone();
    event.before = first.events[0].after.clone();
    event.after = StateCommitment {
        version: 2,
        state_hash: canonicalize_and_digest(&after).unwrap(),
        state: after,
    };
    seal_after(&mut later, Some(&first));
    assert_ne!(first.projections, later.projections);
    let expected = expected_pair(&first, &later, later.projections.clone());
    for (mode, oracle) in [
        (Mode::Correct, None),
        (Mode::AppendProjections, Some("retention.effects")),
        (Mode::CurrentProjectionLookup, Some("lookup.original")),
        (Mode::PruneOriginal, Some("lookup.original")),
        (Mode::PendingOutboxLookup, Some("lookup.original")),
    ] {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            check_delivered_retention(&Backend::new(mode), &first, &later, &expected),
        )
        .await
        .unwrap();
        if let Some(oracle) = oracle {
            let error = result.unwrap_err();
            println!("same-key {mode:?}: {error}");
            assert_eq!(error.check, oracle);
        } else {
            result.unwrap();
        }
    }
}

#[tokio::test]
async fn non_monotonic_projection_refusal_leaves_original_commit_intact() {
    use statechronicle_conformance::write;
    let first = fixture("acme.game.alpha", 930);
    let mut stale = fixture("acme.game.alpha", 931);
    stale.intent.resource_id = first.intent.resource_id.clone();
    stale.events[0].resource_id = first.intent.resource_id.clone();
    // Deliberately invalid storage input: a new identity/head cannot replace
    // the current projection with another projection at the same version.
    seal_after(&mut stale, Some(&first));
    let backend = Backend::new(Mode::Correct);
    write(&backend.connect().await.unwrap(), &first)
        .await
        .unwrap();
    let error = write(&backend.connect().await.unwrap(), &stale)
        .await
        .unwrap_err();
    assert!(matches!(error, LedgerStoreError::Conflict(_)));
    let actual = backend.inspect().await.unwrap();
    assert_eq!(actual.events, first.events);
    assert_eq!(actual.commits, vec![first.commit.clone()]);
    assert_eq!(actual.projections, first.projections);
    assert_eq!(actual.outbox, first.outbox);
    assert_eq!(
        actual.finalized,
        vec![(
            first.intent.tenant_id.clone(),
            first.intent.intent_id.clone(),
            first.payload_digest.clone(),
            first.commit.body.commit_id.clone()
        )]
    );
    assert_eq!(
        actual.heads,
        vec![(
            first.intent.tenant_id.clone(),
            first.commit.body.commit_id.clone(),
            first.commit.body.next_state_root.clone(),
            1
        )]
    );
    let reader = backend.connect().await.unwrap();
    assert!(
        matches!(reader.lookup_mutation(&first.intent.tenant_id, &first.intent.intent_id).await.unwrap(),
        LedgerLookup::Committed(record) if *record == first)
    );
    assert!(!matches!(
        reader
            .lookup_mutation(&stale.intent.tenant_id, &stale.intent.intent_id)
            .await
            .unwrap(),
        LedgerLookup::Committed(_)
    ));
}

#[tokio::test]
async fn subject_held_projection_keys_do_not_erase_other_holders() {
    use statechronicle::profiles::{
        fungible_balance::FungibleBalanceRules, registry::ProfileRules,
    };
    use statechronicle_conformance::check_later_head_retention;
    let mut first = fixture("acme.game.alpha", 920);
    let mut later = fixture("acme.game.alpha", 921);
    for (mutation, subject) in [(&mut first, "account:alice"), (&mut later, "account:bob")] {
        mutation.intent.resource_id = ResourceId("currency:shared".into());
        mutation.intent.state_type = Some(StateType::FungibleBalance);
        mutation.intent.operation = Operation::from_static("balance.create");
        mutation.intent.inputs = [
            ("subject".into(), serde_json::json!(subject)),
            ("unit".into(), serde_json::json!("COIN")),
            ("balance".into(), serde_json::json!("10")),
        ]
        .into();
        FungibleBalanceRules
            .check(&mutation.intent.operation, None, &mutation.intent.inputs)
            .unwrap();
        let state = FungibleBalanceRules
            .apply(None, &mutation.intent.operation, &mutation.intent.inputs)
            .unwrap();
        let event = &mut mutation.events[0];
        event.resource_id = mutation.intent.resource_id.clone();
        event.operation = mutation.intent.operation.clone();
        let mut before = state.clone();
        if let ResourceState::FungibleBalance(balance) = &mut before {
            balance.balance = statechronicle::core::amount::Amount::ZERO;
        }
        event.before = StateCommitment {
            version: 0,
            state_hash: canonicalize_and_digest(&before).unwrap(),
            state: before,
        };
        event.after = StateCommitment {
            version: 1,
            state_hash: canonicalize_and_digest(&state).unwrap(),
            state,
        };
    }
    seal_after(&mut first, None);
    seal_after(&mut later, Some(&first));
    let expected = expected_pair(
        &first,
        &later,
        vec![first.projections[0].clone(), later.projections[0].clone()],
    );
    for mode in [Mode::Correct, Mode::ResourceOnlyProjections] {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            check_later_head_retention(&Backend::new(mode), &first, &later, &expected),
        )
        .await
        .unwrap();
        if mode == Mode::Correct {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            println!("holders {mode:?}: {error}");
            assert_eq!(error.check, "retention.effects");
        }
    }
}

#[tokio::test]
async fn original_records_survive_later_canonical_head() {
    use statechronicle_conformance::check_delivered_retention;
    let first = fixture("acme.game.alpha", 900);
    let mut later = fixture("acme.game.alpha", 901);
    seal_after(&mut later, Some(&first));
    let expected = expected_pair(
        &first,
        &later,
        first
            .projections
            .iter()
            .chain(&later.projections)
            .cloned()
            .collect(),
    );
    for mode in [
        Mode::Correct,
        Mode::PruneOriginal,
        Mode::CurrentProjectionLookup,
        Mode::PendingOutboxLookup,
    ] {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            check_delivered_retention(&Backend::new(mode), &first, &later, &expected),
        )
        .await
        .unwrap();
        if mode == Mode::Correct {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            println!("{mode:?}: {error}");
            assert_eq!(error.check, "lookup.original");
        }
    }
}

#[tokio::test]
async fn complete_original_batches_survive_replaced_projections_and_delivery() {
    use statechronicle::commit::roots::state_root_updates;
    use statechronicle::profiles::{registry::ProfileRules, unique_asset::UniqueAssetRules};
    use statechronicle_conformance::batch;
    let first = batch_fixture(950);
    let mut later = batch_fixture(951);
    for index in 0..later.intents.len() {
        let intent = &mut later.intents[index];
        intent.resource_id = first.intents[index].resource_id.clone();
        intent.operation = Operation::from_static("asset.transfer");
        intent.expected_version = 1;
        intent.inputs = [
            ("from_owner".into(), serde_json::json!("account:alice")),
            ("to_owner".into(), serde_json::json!("account:bob")),
        ]
        .into();
        let current = Some(&first.projections[index]);
        UniqueAssetRules
            .check(&intent.operation, current, &intent.inputs)
            .unwrap();
        let state = UniqueAssetRules
            .apply(current, &intent.operation, &intent.inputs)
            .unwrap();
        let event = &mut later.events[index];
        event.resource_id = intent.resource_id.clone();
        event.operation = intent.operation.clone();
        event.before = first.events[index].after.clone();
        event.after = StateCommitment {
            version: 2,
            state_hash: canonicalize_and_digest(&state).unwrap(),
            state,
        };
    }
    let mut group = CommitBatch::new(later.commit.body.scope.clone());
    for event in &later.events {
        group.add_event(event.clone()).unwrap();
    }
    let old = &later.commit.body;
    let body = CommitBuilder::builder()
        .scope(old.scope.clone())
        .sequence(2)
        .parent(Some(first.commit.body.commit_id.clone()))
        .executor(old.executor.clone())
        .profile(old.profile.clone())
        .created_at(old.created_at)
        .build(
            &group,
            first.commit.body.next_state_root.clone(),
            &state_root_updates(&first.events).unwrap(),
            || Ok(old.commit_id.clone()),
        )
        .unwrap();
    let signing = ed25519_dalek::SigningKey::from_bytes(&[77; 32]);
    later.commit = sign_commit(
        &body,
        &signing,
        KeyId::new("test:conformance-key".into()).unwrap(),
    )
    .unwrap();
    verify_commit(&later.commit, &signing.verifying_key()).unwrap();
    let entries: Vec<_> = later
        .events
        .iter()
        .map(|event| CommittedEvent {
            event,
            state_type: StateType::UniqueAsset,
        })
        .collect();
    later.projections = projections_for(&later.commit.body.commit_id, &entries);
    let expected = Snapshot {
        events: first.events.iter().chain(&later.events).cloned().collect(),
        commits: vec![first.commit.clone(), later.commit.clone()],
        projections: later.projections.clone(),
        outbox: first.outbox.iter().chain(&later.outbox).cloned().collect(),
        finalized: [&first, &later]
            .into_iter()
            .flat_map(|batch| {
                batch.intents.iter().map(|intent| {
                    (
                        intent.tenant_id.clone(),
                        intent.intent_id.clone(),
                        canonicalize_and_digest(intent).unwrap(),
                        batch.commit.body.commit_id.clone(),
                    )
                })
            })
            .collect(),
        heads: vec![(
            later.intents[0].tenant_id.clone(),
            later.commit.body.commit_id.clone(),
            later.commit.body.next_state_root.clone(),
            2,
        )],
    };
    for (mode, oracle) in [
        (Mode::Correct, None),
        (Mode::PruneOriginal, Some("batch.retention.membership")),
        (
            Mode::PruneBatchMemberAfterHead,
            Some("batch.retention.membership"),
        ),
        (Mode::CurrentProjectionLookup, Some("batch.lookup.original")),
        (Mode::PendingOutboxLookup, Some("batch.lookup.original")),
        (Mode::ReplayLatestHead, Some("batch.replay.identity")),
        (
            Mode::CorruptOldSecondMemberLookup,
            Some("batch.lookup.original"),
        ),
        (Mode::AppendProjections, Some("batch.retention.effects")),
    ] {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            batch::check_delivered_retention(&Backend::new(mode), &first, &later, &expected),
        )
        .await
        .unwrap();
        if let Some(oracle) = oracle {
            let error = result.unwrap_err();
            println!("batch retention {mode:?}: {error}");
            assert_eq!(error.check, oracle);
            if mode == Mode::CorruptOldSecondMemberLookup {
                assert!(error.detail.contains("member 1"));
            }
        } else {
            result.unwrap();
        }
    }
}

// Fixture history and raw expectations are constructed before opening the backend.
// No receipt/lookup/adapter key helper supplies expected rows or state.
fn mixed_history() -> (Vec<LedgerMutation>, Vec<Snapshot>) {
    use statechronicle::profiles::{
        fungible_balance::FungibleBalanceRules, registry::ProfileRules,
        unique_asset::UniqueAssetRules,
    };
    let mut mutations: Vec<LedgerMutation> = Vec::new();
    let mut expected = Vec::new();
    let mut view = Snapshot::default();
    for index in 0..32 {
        let tenant = if index % 2 == 0 {
            "acme.game.alpha"
        } else {
            "acme.game.beta"
        };
        let turn = index / 2;
        let slot = turn % 4;
        // Cross-tenant textual intent/event/commit IDs deliberately coincide.
        let mut mutation = fixture(tenant, 1200 + turn);
        let resource = if slot < 2 {
            format!("asset:history-{slot}")
        } else {
            "currency:history".into()
        };
        let subject = if slot == 2 {
            "account:alice"
        } else {
            "account:bob"
        };
        let key = (
            tenant.to_owned(),
            resource.clone(),
            (slot >= 2).then(|| subject.to_owned()),
        );
        let current = view.projections.iter().find(|row| {
            (
                row.tenant_id.0.clone(),
                row.resource_id.0.clone(),
                row.state.subject().map(|s| s.0.clone()),
            ) == key
        });
        mutation.intent.resource_id = ResourceId(resource);
        let rules: &dyn ProfileRules = if slot < 2 {
            &UniqueAssetRules
        } else {
            &FungibleBalanceRules
        };
        mutation.intent.state_type = Some(if slot < 2 {
            StateType::UniqueAsset
        } else {
            StateType::FungibleBalance
        });
        mutation.intent.expected_version = current.map_or(0, |row| row.version);
        if slot < 2 {
            mutation.intent.operation = Operation::from_static(if current.is_some() {
                "asset.transfer"
            } else {
                "asset.mint"
            });
            let to = if (turn / 4) % 2 == 0 {
                "account:alice"
            } else {
                "account:bob"
            };
            mutation.intent.inputs = [("to_owner".into(), serde_json::json!(to))].into();
            if let Some(row) = current {
                let ResourceState::UniqueAsset(state) = &row.state else {
                    panic!("asset fixture state")
                };
                mutation
                    .intent
                    .inputs
                    .insert("from_owner".into(), serde_json::json!(state.owner.0));
            }
        } else {
            mutation.intent.operation = Operation::from_static(if current.is_some() {
                "balance.credit"
            } else {
                "balance.create"
            });
            mutation.intent.inputs = if current.is_some() {
                [("amount".into(), serde_json::json!("3"))].into()
            } else {
                [
                    ("subject".into(), serde_json::json!(subject)),
                    ("unit".into(), serde_json::json!("COIN")),
                    ("balance".into(), serde_json::json!("10")),
                ]
                .into()
            };
        }
        rules
            .check(&mutation.intent.operation, current, &mutation.intent.inputs)
            .unwrap();
        let state = rules
            .apply(current, &mutation.intent.operation, &mutation.intent.inputs)
            .unwrap();
        let event = &mut mutation.events[0];
        event.resource_id = mutation.intent.resource_id.clone();
        event.operation = mutation.intent.operation.clone();
        if let Some(row) = current {
            event.before = StateCommitment {
                version: row.version,
                state_hash: row.state_hash.clone(),
                state: row.state.clone(),
            };
        } else if slot >= 2 {
            let mut unborn = state.clone();
            let ResourceState::FungibleBalance(balance) = &mut unborn else {
                panic!("balance fixture state")
            };
            balance.balance = statechronicle::core::amount::Amount::ZERO;
            event.before = StateCommitment {
                version: 0,
                state_hash: canonicalize_and_digest(&unborn).unwrap(),
                state: unborn,
            };
        }
        event.after = StateCommitment {
            version: mutation.intent.expected_version + 1,
            state_hash: canonicalize_and_digest(&state).unwrap(),
            state,
        };
        let history: Vec<_> = mutations
            .iter()
            .filter(|m| m.intent.tenant_id == mutation.intent.tenant_id)
            .collect();
        seal_after_history(&mut mutation, &history);
        view.events.extend(mutation.events.clone());
        view.commits.push(mutation.commit.clone());
        view.projections.retain(|row| {
            (
                row.tenant_id.0.clone(),
                row.resource_id.0.clone(),
                row.state.subject().map(|s| s.0.clone()),
            ) != key
        });
        view.projections.extend(mutation.projections.clone());
        view.outbox.extend(mutation.outbox.clone());
        view.finalized.push((
            mutation.intent.tenant_id.clone(),
            mutation.intent.intent_id.clone(),
            mutation.payload_digest.clone(),
            mutation.commit.body.commit_id.clone(),
        ));
        view.heads
            .retain(|(scope, ..)| scope != &mutation.intent.tenant_id);
        view.heads.push((
            mutation.intent.tenant_id.clone(),
            mutation.commit.body.commit_id.clone(),
            mutation.commit.body.next_state_root.clone(),
            mutation.commit.body.sequence,
        ));
        mutations.push(mutation);
        expected.push(view.clone());
    }
    // Reject invalid test histories independently of the storage under test.
    use statechronicle::index::rebuild::verified::{HistoryPin, HistoryWitness, verify_history};
    for tenant in ["acme.game.alpha", "acme.game.beta"] {
        let chain: Vec<_> = mutations
            .iter()
            .filter(|m| m.intent.tenant_id.0 == tenant)
            .collect();
        let witnesses: Vec<_> = chain
            .iter()
            .map(|m| HistoryWitness {
                commit: &m.commit,
                events: &m.events,
            })
            .collect();
        verify_history(
            &witnesses,
            &HistoryPin {
                tenant: TenantId(tenant.into()),
                tip_digest: canonicalize_and_digest(&chain.last().unwrap().commit.body).unwrap(),
            },
            &FixtureVerifier,
        )
        .unwrap();
    }
    assert_eq!(view.commits.len(), 32);
    assert_eq!(view.projections.len(), 8);
    assert_eq!(view.heads.len(), 2);
    (mutations, expected)
}

#[tokio::test]
async fn mixed_history_retains_originals_across_rollback_retry_and_tenants() {
    let (mutations, expected) = mixed_history();
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        statechronicle_conformance::check_history(
            &Backend::new(Mode::Correct),
            &mutations,
            &expected,
        ),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn history_detects_faults_that_pass_smaller_schedules() {
    let (mutations, expected) = mixed_history();
    for (mode, oracle) in [
        (Mode::PruneAfterFourthHead, "lookup.original"),
        (Mode::LatestReceiptAfterFourthHead, "lookup.original"),
        (Mode::RollbackForgetsHistory, "history.rollback_effects"),
    ] {
        let backend = Backend::new(mode);
        // Existing genesis rollback and two-commit retention schedules do not
        // expose these delayed/history-dependent faults.
        check_rollback(&backend, &mutations[0]).await.unwrap();
        let pair = expected_pair(
            &mutations[0],
            &mutations[2],
            vec![
                mutations[0].projections[0].clone(),
                mutations[2].projections[0].clone(),
            ],
        );
        statechronicle_conformance::check_later_head_retention(
            &backend,
            &mutations[0],
            &mutations[2],
            &pair,
        )
        .await
        .unwrap();
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            statechronicle_conformance::check_history(&backend, &mutations, &expected),
        )
        .await
        .unwrap()
        .unwrap_err();
        println!("mixed-history {mode:?}: {error}");
        assert_eq!(error.check, oracle);
    }
}

#[tokio::test]
async fn history_rejects_invalid_setup_before_destructive_reset() {
    let (mutations, expected) = mixed_history();
    let backend = Backend::new(Mode::Correct);
    let store = backend.connect().await.unwrap();
    statechronicle_conformance::write(&store, &mutations[0])
        .await
        .unwrap();
    for (input, snapshots, check) in [
        (
            mutations[..2].to_vec(),
            expected[..2].to_vec(),
            "setup.history",
        ),
        (mutations.clone(), expected[..31].to_vec(), "setup.history"),
        (
            {
                let mut broken = mutations.clone();
                broken[4].commit.body.sequence += 1;
                broken
            },
            expected.clone(),
            "setup.history_chain",
        ),
        (
            {
                let mut broken = mutations.clone();
                broken[4].intent.intent_id = broken[0].intent.intent_id.clone();
                broken
            },
            expected.clone(),
            "setup.history_chain",
        ),
    ] {
        let error = statechronicle_conformance::check_history(&backend, &input, &snapshots)
            .await
            .unwrap_err();
        assert_eq!(error.check, check);
        // A fixture mistake must not erase even the isolated test database.
        let observation = backend.inspect().await.unwrap();
        assert_eq!(observation.events, mutations[0].events);
        assert_eq!(observation.commits, vec![mutations[0].commit.clone()]);
        assert!(
            matches!(store.lookup_mutation(&mutations[0].intent.tenant_id, &mutations[0].intent.intent_id).await.unwrap(), LedgerLookup::Committed(record) if *record == mutations[0])
        );
    }
}
