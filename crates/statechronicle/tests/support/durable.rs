//! In-memory transactional adapter used to exercise the unchanged ledger port.
#![allow(clippy::unwrap_used, clippy::panic)]
use crate::common::fixed_key;
use async_trait::async_trait;
use statechronicle::accumulator::key::StateKey;
use statechronicle::accumulator::sparse_merkle::{StateAccumulator, StateUpdate};
use statechronicle::commit::persist::SignedCommitVerifier;
use statechronicle::commit::sign::verify_commit;
use statechronicle::ports::authorization::{AuthorizationContext, AuthorizationError, Authorizer};
use statechronicle::ports::ledger_store::{
    IdempotencyClaim, LedgerStore, LedgerStoreError, LedgerTransaction, OutboxRecord,
};
use statechronicle::{
    Commit, CommitId, ContentDigest, Event, Intent, IntentId, Signed, StateProjection, TenantId,
};
use std::collections::{BTreeMap, BTreeSet};
use std::future::pending;
use std::sync::{Arc, Mutex};

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
    Commit,
    Rollback,
}
#[derive(Default, Clone)]
pub struct Stored {
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
    pub lose_commit_response: bool,
    pub yield_each_step: bool,
    pub calls: Arc<Mutex<Vec<Step>>>,
    pub reservations: Arc<Mutex<BTreeSet<(String, String)>>>,
}
struct Transaction {
    ledger: MemoryLedger,
    staged: Stored,
    identity: Option<((String, String), ContentDigest)>,
}
impl MemoryLedger {
    async fn checkpoint(&self, step: Step) -> Result<(), LedgerStoreError> {
        self.calls.lock().unwrap().push(step);
        if self.yield_each_step {
            tokio::task::yield_now().await;
        }
        if self.pause == Some(step) {
            pending::<()>().await;
        }
        if self.faults.contains(&step) {
            return Err(LedgerStoreError::Unavailable(format!("injected {step:?}")));
        }
        Ok(())
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        if let Some((key, _)) = &self.identity {
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
            identity: None,
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
        if !self.ledger.reservations.lock().unwrap().insert(key.clone()) {
            return Ok(IdempotencyClaim::InProgress {
                attempt_id: String::from("attempt"),
                lease_expires_at_unix: i64::MAX,
            });
        }
        self.identity = Some((key, digest.clone()));
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
        _tenant: &TenantId,
        _intent_id: &IntentId,
        _attempt: &str,
        commit: &CommitId,
    ) -> Result<(), LedgerStoreError> {
        self.ledger.checkpoint(Step::Finalize).await?;
        let (key, digest) = self.identity.as_ref().unwrap();
        self.staged
            .identities
            .insert(key.clone(), (digest.clone(), commit.clone()));
        self.staged.finalized = true;
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
                let prior = projections.iter().find(|projection| {
                    projection.tenant_id == event.tenant_id
                        && projection.resource_id == event.resource_id
                });
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
                projections.retain(|row| {
                    row.tenant_id != projection.tenant_id
                        || row.resource_id != projection.resource_id
                });
                projections.push(projection.clone());
            }
            let updates: Vec<StateUpdate> = projections
                .iter()
                .filter(|row| Some(&row.tenant_id) == prepared.body.scope.tenant_id.as_ref())
                .map(|row| {
                    StateUpdate::new(
                        StateKey::for_resource(&row.tenant_id.0, &row.resource_id.0),
                        *row.state_hash.as_bytes(),
                    )
                })
                .collect();
            let mut accumulator = StateAccumulator::empty();
            accumulator.insert_batch(&updates).unwrap();
            if *accumulator.root().as_bytes() != *prepared.body.next_state_root.as_bytes() {
                return Err(LedgerStoreError::Invariant(String::from(
                    "prepared state root mismatch",
                )));
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
