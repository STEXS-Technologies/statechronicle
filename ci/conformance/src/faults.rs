//! Logical fault injection outside the adapter; not a database crash simulator.
use async_trait::async_trait;
use statechronicle::ports::ledger_batch::{LedgerBatch, LedgerBatchLookup, LedgerBatchRecovery};
use statechronicle::ports::ledger_store::{
    IdempotencyClaim, LedgerLookup, LedgerRecovery, LedgerStore, LedgerStoreError,
    LedgerTransaction, OutboxRecord,
};
use statechronicle::{
    Commit, CommitId, ContentDigest, Event, Intent, IntentId, Signed, StateProjection, TenantId,
};
/// Commits successfully in the underlying store, then loses the acknowledgement.
pub struct LoseCommitAck<S>(pub S);
struct Transaction(Box<dyn LedgerTransaction>);
#[async_trait]
impl<S: LedgerStore> LedgerStore for LoseCommitAck<S> {
    async fn begin(
        &self,
        tenant: &TenantId,
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        Ok(Box::new(Transaction(self.0.begin(tenant).await?)))
    }
    async fn begin_multi(
        &self,
        tenants: &[TenantId],
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        Ok(Box::new(Transaction(self.0.begin_multi(tenants).await?)))
    }
}
#[async_trait]
impl<S: LedgerRecovery> LedgerRecovery for LoseCommitAck<S> {
    async fn lookup_mutation(
        &self,
        tenant: &TenantId,
        intent: &IntentId,
    ) -> Result<LedgerLookup, LedgerStoreError> {
        self.0.lookup_mutation(tenant, intent).await
    }
}
#[async_trait]
impl<S: LedgerBatchRecovery> LedgerBatchRecovery for LoseCommitAck<S> {
    async fn lookup_batch(
        &self,
        tenant: &TenantId,
        intent: &IntentId,
    ) -> Result<LedgerBatchLookup, LedgerStoreError> {
        self.0.lookup_batch(tenant, intent).await
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
        self.0.claim_idempotency(tenant, intent, digest).await
    }
    async fn append_events(&mut self, events: &[Event]) -> Result<(), LedgerStoreError> {
        self.0.append_events(events).await
    }
    async fn append_commit(&mut self, commit: &Signed<Commit>) -> Result<(), LedgerStoreError> {
        self.0.append_commit(commit).await
    }
    async fn upsert_projection(
        &mut self,
        projection: &StateProjection,
    ) -> Result<(), LedgerStoreError> {
        self.0.upsert_projection(projection).await
    }
    async fn enqueue_outbox(&mut self, record: &OutboxRecord) -> Result<(), LedgerStoreError> {
        self.0.enqueue_outbox(record).await
    }
    async fn finalize_idempotency(
        &mut self,
        tenant: &TenantId,
        intent: &IntentId,
        attempt: &str,
        commit: &CommitId,
    ) -> Result<(), LedgerStoreError> {
        self.0
            .finalize_idempotency(tenant, intent, attempt, commit)
            .await
    }
    async fn retain_batch(&mut self, batch: &LedgerBatch) -> Result<(), LedgerStoreError> {
        self.0.retain_batch(batch).await
    }
    async fn commit(self: Box<Self>) -> Result<(), LedgerStoreError> {
        self.0.commit().await?;
        Err(LedgerStoreError::Unavailable(
            "conformance: committed but acknowledgement lost".into(),
        ))
    }
    async fn rollback(self: Box<Self>) -> Result<(), LedgerStoreError> {
        self.0.rollback().await
    }
}
