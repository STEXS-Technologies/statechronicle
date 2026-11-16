//! In-memory transactional adapter for legacy writes and retained recovery records.
#![allow(clippy::unwrap_used, clippy::panic)]
use crate::common::fixed_key;
use async_trait::async_trait;
use statechronicle::accumulator::key::StateKey;
use statechronicle::accumulator::sparse_merkle::{StateAccumulator, StateUpdate};
use statechronicle::commit::persist::SignedCommitVerifier;
use statechronicle::commit::sign::verify_commit;
use statechronicle::ports::authorization::{AuthorizationContext, AuthorizationError, Authorizer};
use statechronicle::ports::ledger_batch::{LedgerBatch, LedgerBatchLookup, LedgerBatchRecovery};
use statechronicle::ports::ledger_store::{
    IdempotencyClaim, LedgerLookup, LedgerMutation, LedgerRecovery, LedgerStore, LedgerStoreError,
    LedgerTransaction, OutboxRecord,
};
use statechronicle::{
    Commit, CommitId, ContentDigest, Event, Intent, IntentId, Signed, StateProjection, TenantId,
};
use std::collections::BTreeMap;
use std::future::pending;
use std::sync::{Arc, Mutex};

// Baseline typed resource addressing. This fixture does not model policies that
// relocate an existing subject-held row to a different subject in one event.
fn projection_key(row: &StateProjection) -> StateKey {
    statechronicle::profiles::transition::state_key_for(
        row.state_type,
        &row.tenant_id,
        row.state.subject(),
        &row.resource_id,
    )
    .unwrap()
}

/// Every asynchronous boundary exposed by the durable mutation path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Begin,
    Claim,
    Events,
    CommitRow,
    Projection,
    Outbox,
    Finalize,
    RetainBatch,
    Commit,
    Rollback,
    Lookup,
}
#[derive(Default, Clone)]
pub struct Stored {
    pub mutations: BTreeMap<(String, String), LedgerMutation>,
    pub batches: BTreeMap<(String, String), LedgerBatch>,
    pub events: Vec<Event>,
    pub commits: Vec<Signed<Commit>>,
    pub projections: Vec<StateProjection>,
    pub finalized: bool,
    pub outbox: Vec<OutboxRecord>,
    pub identities: BTreeMap<(String, String), (ContentDigest, CommitId)>,
}
#[derive(Default, Clone)]
pub struct MemoryLedger {
    pub data: Arc<Mutex<Stored>>,
    pub fail_projection: bool,
    pub faults: Vec<Step>,
    pub pause: Option<Step>,
    pub fail_at: Option<(Step, usize)>,
    pub pause_at: Option<(Step, usize)>,
    pub lose_commit_response: bool,
    pub yield_each_step: bool,
    pub calls: Arc<Mutex<Vec<Step>>>,
    pub reservations: Arc<Mutex<BTreeMap<(String, String), ContentDigest>>>,
}
struct Transaction {
    ledger: MemoryLedger,
    staged: Stored,
    claims: Vec<((String, String), ContentDigest, Intent)>,
    batch: Option<LedgerBatch>,
}
impl MemoryLedger {
    async fn checkpoint(&self, step: Step) -> Result<(), LedgerStoreError> {
        self.calls.lock().unwrap().push(step);
        if self.yield_each_step {
            tokio::task::yield_now().await;
        }
        let occurrence = self
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|called| **called == step)
            .count();
        if self.pause == Some(step) || self.pause_at == Some((step, occurrence)) {
            pending::<()>().await;
        }
        if self.faults.contains(&step) || self.fail_at == Some((step, occurrence)) {
            return Err(LedgerStoreError::Unavailable(format!("injected {step:?}")));
        }
        Ok(())
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        for (key, _, _) in &self.claims {
            self.ledger.reservations.lock().unwrap().remove(key);
        }
    }
}
#[async_trait]
impl LedgerStore for MemoryLedger {
    async fn begin(
        &self,
        _tenant: &TenantId,
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        self.checkpoint(Step::Begin).await?;
        Ok(Box::new(Transaction {
            ledger: self.clone(),
            staged: Stored::default(),
            claims: Vec::new(),
            batch: None,
        }))
    }
    async fn begin_multi(
        &self,
        _tenants: &[TenantId],
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        Err(LedgerStoreError::Invariant(String::from(
            "single tenant fixture",
        )))
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
        self.ledger.checkpoint(Step::Claim).await?;
        let key = (tenant.0.clone(), intent.intent_id.as_str().to_owned());
        if let Some((original, commit_id)) = self.ledger.data.lock().unwrap().identities.get(&key) {
            return Ok(if original == digest {
                IdempotencyClaim::Committed {
                    commit_id: commit_id.clone(),
                }
            } else {
                IdempotencyClaim::ConflictDifferentPayload
            });
        }
        let mut reservations = self.ledger.reservations.lock().unwrap();
        if let Some(existing) = reservations.get(&key) {
            return Ok(if existing == digest {
                IdempotencyClaim::InProgress {
                    attempt_id: String::from("attempt"),
                    lease_expires_at_unix: i64::MAX,
                }
            } else {
                IdempotencyClaim::ConflictDifferentPayload
            });
        }
        reservations.insert(key.clone(), digest.clone());
        drop(reservations);
        self.claims.push((key, digest.clone(), intent.clone()));
        Ok(IdempotencyClaim::NewReservation {
            attempt_id: String::from("attempt"),
        })
    }
    async fn append_events(&mut self, events: &[Event]) -> Result<(), LedgerStoreError> {
        self.ledger.checkpoint(Step::Events).await?;
        self.staged.events.extend_from_slice(events);
        Ok(())
    }
    async fn append_commit(&mut self, commit: &Signed<Commit>) -> Result<(), LedgerStoreError> {
        self.ledger.checkpoint(Step::CommitRow).await?;
        self.staged.commits.push(commit.clone());
        Ok(())
    }
    async fn upsert_projection(
        &mut self,
        projection: &StateProjection,
    ) -> Result<(), LedgerStoreError> {
        self.ledger.checkpoint(Step::Projection).await?;
        if self.ledger.fail_projection {
            return Err(LedgerStoreError::Unavailable(String::from(
                "injected failure",
            )));
        }
        self.staged.projections.push(projection.clone());
        Ok(())
    }
    async fn enqueue_outbox(&mut self, record: &OutboxRecord) -> Result<(), LedgerStoreError> {
        self.ledger.checkpoint(Step::Outbox).await?;
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
        self.ledger.checkpoint(Step::Finalize).await?;
        let (key, digest, _) = self
            .claims
            .iter()
            .find(|(key, _, _)| key == &(tenant.0.clone(), intent_id.as_str().to_owned()))
            .ok_or_else(|| LedgerStoreError::Invariant(String::from("missing member claim")))?;
        if attempt != "attempt" {
            return Err(LedgerStoreError::Idempotency(String::from("wrong attempt")));
        }
        self.staged
            .identities
            .insert(key.clone(), (digest.clone(), commit.clone()));
        self.staged.finalized = true;
        Ok(())
    }
    async fn retain_batch(&mut self, batch: &LedgerBatch) -> Result<(), LedgerStoreError> {
        self.ledger.checkpoint(Step::RetainBatch).await?;
        if self.batch.is_some() {
            return Err(LedgerStoreError::Invariant(String::from(
                "batch already retained",
            )));
        }
        self.batch = Some(batch.clone());
        Ok(())
    }
    async fn commit(mut self: Box<Self>) -> Result<(), LedgerStoreError> {
        self.ledger.checkpoint(Step::Commit).await?;
        {
            let mut data = self.ledger.data.lock().unwrap();
            let prepared = self.staged.commits.first().unwrap();
            let head = data
                .commits
                .iter()
                .rev()
                .find(|commit| commit.body.scope == prepared.body.scope);
            let expected_sequence = head.map_or(1, |commit| commit.body.sequence.saturating_add(1));
            let expected_parent = head.map(|commit| &commit.body.commit_id);
            let previous_root = head.map_or_else(
                || ContentDigest::new(*StateAccumulator::empty().root().as_bytes()),
                |commit| commit.body.next_state_root.clone(),
            );
            if prepared.body.sequence != expected_sequence
                || prepared.body.parent_commit_id.as_ref() != expected_parent
                || prepared.body.previous_state_root != previous_root
            {
                return Err(LedgerStoreError::Conflict(String::from(
                    "canonical head changed",
                )));
            }
            let mut projections = data.projections.clone();
            for event in &self.staged.events {
                let target = statechronicle::commit::roots::state_key_for(
                    event,
                    event.after.state.state_type(),
                )
                .unwrap();
                let prior = projections
                    .iter()
                    .find(|projection| projection_key(projection) == target);
                if event.before.version != prior.map_or(0, |projection| projection.version)
                    || prior
                        .is_some_and(|projection| projection.state_hash != event.before.state_hash)
                {
                    return Err(LedgerStoreError::Conflict(String::from(
                        "projected state changed",
                    )));
                }
                let projection = self
                    .staged
                    .projections
                    .iter()
                    .find(|projection| projection.last_event_id == event.event_id)
                    .unwrap();
                projections.retain(|row| projection_key(row) != target);
                projections.push(projection.clone());
            }
            let updates: Vec<StateUpdate> = projections
                .iter()
                .filter(|row| Some(&row.tenant_id) == prepared.body.scope.tenant_id.as_ref())
                .map(|row| StateUpdate::new(projection_key(row), *row.state_hash.as_bytes()))
                .collect();
            let mut accumulator = StateAccumulator::empty();
            accumulator.insert_batch(&updates).unwrap();
            if *accumulator.root().as_bytes() != *prepared.body.next_state_root.as_bytes() {
                return Err(LedgerStoreError::Invariant(String::from(
                    "prepared state root mismatch",
                )));
            }
            if self.claims.len() != self.staged.identities.len()
                || self.claims.iter().any(|(key, digest, _)| {
                    self.staged.identities.get(key)
                        != Some(&(digest.clone(), prepared.body.commit_id.clone()))
                })
            {
                return Err(LedgerStoreError::Invariant(String::from(
                    "not every member finalized",
                )));
            }
            if let Some(batch) = &self.batch {
                if batch.intents
                    != self
                        .claims
                        .iter()
                        .map(|(_, _, intent)| intent.clone())
                        .collect::<Vec<_>>()
                {
                    // Claims are acquired in sorted order; group identity keeps
                    // caller order. Compare membership independently below.
                    let mut left: Vec<_> = batch
                        .intents
                        .iter()
                        .map(|intent| (&intent.intent_id, intent))
                        .collect();
                    let mut right: Vec<_> = self
                        .claims
                        .iter()
                        .map(|(_, _, intent)| (&intent.intent_id, intent))
                        .collect();
                    left.sort_by_key(|(id, _)| *id);
                    right.sort_by_key(|(id, _)| *id);
                    if left != right {
                        return Err(LedgerStoreError::Invariant(String::from(
                            "incomplete batch membership",
                        )));
                    }
                }
                if batch.commit != *prepared
                    || batch.events != self.staged.events
                    || batch.projections != self.staged.projections
                    || batch.outbox != self.staged.outbox
                {
                    return Err(LedgerStoreError::Invariant(String::from(
                        "batch metadata differs from staged effects",
                    )));
                }
                for (key, _, _) in &self.claims {
                    data.batches.insert(key.clone(), batch.clone());
                }
            } else {
                if self.claims.len() != 1 {
                    return Err(LedgerStoreError::Invariant(String::from(
                        "multiple claims without batch retention",
                    )));
                }
                let (identity, digest, intent) = self.claims.first().unwrap();
                let record = LedgerMutation {
                    intent: intent.clone(),
                    payload_digest: digest.clone(),
                    commit: prepared.clone(),
                    events: self.staged.events.clone(),
                    projections: self.staged.projections.clone(),
                    outbox: self.staged.outbox.clone(),
                };
                data.mutations.insert(identity.clone(), record);
            }
            data.events.append(&mut self.staged.events);
            data.commits.append(&mut self.staged.commits);
            data.projections = projections;
            data.outbox.append(&mut self.staged.outbox);
            data.identities.append(&mut self.staged.identities);
            data.finalized = self.staged.finalized;
        }
        if self.ledger.lose_commit_response {
            return Err(LedgerStoreError::Unavailable(String::from(
                "commit response lost",
            )));
        }
        Ok(())
    }
    async fn rollback(self: Box<Self>) -> Result<(), LedgerStoreError> {
        self.ledger.checkpoint(Step::Rollback).await?;
        Ok(())
    }
}
#[async_trait]
impl LedgerRecovery for MemoryLedger {
    async fn lookup_mutation(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
    ) -> Result<LedgerLookup, LedgerStoreError> {
        self.checkpoint(Step::Lookup).await?;
        let identity = (tenant.0.clone(), intent_id.as_str().to_owned());
        let data = self.data.lock().unwrap();
        if let Some(record) = data.mutations.get(&identity) {
            if data.identities.get(&identity)
                != Some(&(
                    record.payload_digest.clone(),
                    record.commit.body.commit_id.clone(),
                ))
                || !data.commits.contains(&record.commit)
            {
                return Err(LedgerStoreError::Invariant(String::from(
                    "retained record is not bound to finalized canonical history",
                )));
            }
            return Ok(LedgerLookup::Committed(Box::new(record.clone())));
        }
        if data.identities.contains_key(&identity) {
            return Err(LedgerStoreError::Invariant(String::from(
                "finalized identity is missing its original record",
            )));
        }
        let reservations = self.reservations.lock().unwrap();
        if let Some(payload_digest) = reservations.get(&identity) {
            return Ok(LedgerLookup::InProgress {
                payload_digest: payload_digest.clone(),
                attempt_id: String::from("attempt"),
                lease_expires_at_unix: i64::MAX,
            });
        }
        Ok(LedgerLookup::NotFound)
    }
}

#[async_trait]
impl LedgerBatchRecovery for MemoryLedger {
    async fn lookup_batch(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
    ) -> Result<LedgerBatchLookup, LedgerStoreError> {
        self.checkpoint(Step::Lookup).await?;
        let key = (tenant.0.clone(), intent_id.as_str().to_owned());
        let data = self.data.lock().unwrap();
        if let Some(batch) = data.batches.get(&key) {
            if !data.commits.contains(&batch.commit)
                || batch.intents.iter().any(|intent| {
                    let digest =
                        statechronicle::core::canonicalize::canonicalize_and_digest(intent)
                            .unwrap();
                    data.identities
                        .get(&(tenant.0.clone(), intent.intent_id.as_str().to_owned()))
                        != Some(&(digest, batch.commit.body.commit_id.clone()))
                })
            {
                return Err(LedgerStoreError::Invariant(String::from(
                    "incomplete finalized batch",
                )));
            }
            return Ok(LedgerBatchLookup::Committed(Box::new(batch.clone())));
        }
        if data.identities.contains_key(&key) {
            return Err(LedgerStoreError::Invariant(String::from(
                "finalized member has no original batch",
            )));
        }
        if let Some(digest) = self.reservations.lock().unwrap().get(&key) {
            return Ok(LedgerBatchLookup::InProgress {
                payload_digest: digest.clone(),
                attempt_id: String::from("attempt"),
                lease_expires_at_unix: i64::MAX,
            });
        }
        Ok(LedgerBatchLookup::NotFound)
    }
}

pub struct Allow;
#[async_trait]
impl Authorizer for Allow {
    async fn authorize(
        &self,
        _context: AuthorizationContext<'_>,
    ) -> Result<(), AuthorizationError> {
        Ok(())
    }
}
pub struct Verify;
impl SignedCommitVerifier for Verify {
    fn verify(&self, commit: &Signed<Commit>) -> Result<(), String> {
        verify_commit(commit, &fixed_key().verifying_key()).map_err(|error| error.to_string())
    }
}
