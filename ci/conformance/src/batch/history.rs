//! Growing histories that share single and batch authority in the same tenants.
use super::*;

/// One complete committed transaction, never a standalone slice of a batch.
#[derive(Clone)]
pub enum HistoryEntry {
    Single(Box<LedgerMutation>),
    Batch(Box<LedgerBatch>),
}
impl HistoryEntry {
    fn intents(&self) -> &[Intent] {
        match self {
            Self::Single(mutation) => std::slice::from_ref(&mutation.intent),
            Self::Batch(batch) => &batch.intents,
        }
    }
    fn commit(&self) -> &Signed<Commit> {
        match self {
            Self::Single(mutation) => &mutation.commit,
            Self::Batch(batch) => &batch.commit,
        }
    }
    fn prefixes(&self) -> usize {
        match self {
            Self::Single(mutation) => step_count(mutation),
            Self::Batch(batch) => steps(batch).len(),
        }
    }
    async fn stage<S: LedgerBatchRecovery>(
        &self,
        store: &S,
        prefix: usize,
    ) -> Result<Box<dyn LedgerTransaction>, Violation> {
        match self {
            Self::Batch(batch) => super::stage(store, batch, prefix).await,
            Self::Single(mutation) => {
                let mut tx = store
                    .begin(&mutation.intent.tenant_id)
                    .await
                    .map_err(operation)?;
                let attempt = claim(&mut *tx, mutation).await.map_err(operation)?;
                for index in 0..prefix {
                    if let Err(error) = step(&mut *tx, mutation, &attempt, index).await {
                        tx.rollback().await.map_err(operation)?;
                        return Err(operation(error));
                    }
                }
                Ok(tx)
            }
        }
    }
    async fn retained<S: LedgerBatchRecovery>(&self, store: &S) -> Result<(), Violation> {
        match self {
            Self::Single(mutation) => {
                crate::original(store, mutation).await?;
                crate::retry_identity(store, mutation).await?;
                require(
                    "mixed.single_namespace",
                    matches!(
                        store
                            .lookup_batch(&mutation.intent.tenant_id, &mutation.intent.intent_id)
                            .await,
                        Err(LedgerStoreError::Invariant(_))
                    ),
                    "standalone history was reinterpreted as a group or absence",
                )
            }
            Self::Batch(batch) => {
                super::original(store, batch).await?;
                retry_members(store, batch).await
            }
        }
    }
}
async fn membership<B: LedgerBatchTestBackend>(
    backend: &B,
    entries: &[HistoryEntry],
) -> Result<(), Violation> {
    let batches: Vec<_> = entries
        .iter()
        .filter_map(|entry| match entry {
            HistoryEntry::Batch(batch) => Some(batch.as_ref()),
            HistoryEntry::Single(_) => None,
        })
        .collect();
    compare_membership(backend, &batches, "mixed.membership").await
}

/// Destructively check a bounded mixed history against independent raw snapshots.
///
/// Supply valid signed fixtures and an expected snapshot after every transaction.
/// At least three transactions and both kinds are required. All member identities
/// must be distinct within each tenant and each tenant must have one consecutive
/// chain across both kinds. Fixture shape/chain errors refuse before reset.
/// Every staged prefix is rolled back over retained prior history. Each completed
/// transaction then rechecks every original receipt, exact/conflicting identity,
/// namespace and raw membership/effect set through independent connections.
///
/// This is logical conformance, not randomized concurrency, signer/ingress
/// qualification or process/power-loss durability. Wrap in a backend timeout.
pub async fn check<B: LedgerBatchTestBackend>(
    backend: &B,
    entries: &[HistoryEntry],
    expected: &[Snapshot],
) -> Result<(), Violation>
where
    B::Connection: LedgerBatchRecovery,
{
    require(
        "setup.mixed",
        entries.len() >= 3
            && entries.len() == expected.len()
            && entries
                .iter()
                .any(|entry| matches!(entry, HistoryEntry::Single(_)))
            && entries
                .iter()
                .any(|entry| matches!(entry, HistoryEntry::Batch(_))),
        "requires both kinds and one independent snapshot per transaction",
    )?;
    let mut tips = std::collections::BTreeMap::<&str, &Commit>::new();
    let mut identities = std::collections::BTreeSet::new();
    let mut commits = std::collections::BTreeSet::new();
    let genesis = ContentDigest::new(*statechronicle::StateAccumulator::empty().root().as_bytes());
    for entry in entries {
        let body = &entry.commit().body;
        let tenant = entry
            .intents()
            .first()
            .map(|intent| &intent.tenant_id)
            .ok_or_else(|| failure("setup.mixed_chain", "empty group"))?;
        let linked = tips.get(tenant.0.as_str()).map_or_else(
            || {
                body.sequence == 1
                    && body.parent_commit_id.is_none()
                    && body.previous_state_root == genesis
            },
            |old| {
                body.parent_commit_id.as_ref() == Some(&old.commit_id)
                    && old.sequence.checked_add(1) == Some(body.sequence)
                    && body.previous_state_root == old.next_state_root
            },
        );
        require(
            "setup.mixed_chain",
            linked
                && statechronicle::commit::persist::validate_tenant_commit(body).ok()
                    == Some(tenant)
                && commits.insert((tenant.0.as_str(), &body.commit_id))
                && entry.intents().iter().all(|intent| {
                    &intent.tenant_id == tenant
                        && identities.insert((tenant.0.as_str(), &intent.intent_id))
                }),
            "requires one consecutive chain and distinct scoped identities across both kinds",
        )?;
        tips.insert(tenant.0.as_str(), body);
    }
    let _ = fresh(backend).await?;
    membership(backend, &[]).await?;
    let empty = Snapshot::default();
    for (position, entry) in entries.iter().enumerate() {
        let before = position.checked_sub(1).map_or(&empty, |i| &expected[i]);
        for prefix in 0..=entry.prefixes() {
            let writer = backend.connect().await.map_err(operation)?;
            let tx = entry.stage(&writer, prefix).await?;
            compare(backend, before, "mixed.staged_effects").await?;
            membership(backend, &entries[..position]).await?;
            let reader = backend.connect().await.map_err(operation)?;
            for intent in entry.intents() {
                require(
                    "mixed.staged_receipt",
                    !matches!(
                        reader
                            .lookup_batch(&intent.tenant_id, &intent.intent_id)
                            .await
                            .map_err(operation)?,
                        LedgerBatchLookup::Committed(_)
                    ) && !matches!(
                        reader
                            .lookup_mutation(&intent.tenant_id, &intent.intent_id)
                            .await
                            .map_err(operation)?,
                        LedgerLookup::Committed(_)
                    ),
                    "staged prefix issued a committed receipt",
                )?;
            }
            tx.rollback().await.map_err(operation)?;
            compare(backend, before, "mixed.rollback_effects").await?;
            membership(backend, &entries[..position]).await?;
            for intent in entry.intents() {
                require(
                    "mixed.rollback_identity",
                    matches!(
                        reader
                            .lookup_mutation(&intent.tenant_id, &intent.intent_id)
                            .await
                            .map_err(operation)?,
                        LedgerLookup::NotFound
                    ) && matches!(
                        reader
                            .lookup_batch(&intent.tenant_id, &intent.intent_id)
                            .await
                            .map_err(operation)?,
                        LedgerBatchLookup::NotFound
                    ),
                    "rolled-back identity survived in either namespace",
                )?;
            }
        }
        let writer = backend.connect().await.map_err(operation)?;
        entry
            .stage(&writer, entry.prefixes())
            .await?
            .commit()
            .await
            .map_err(operation)?;
        compare(backend, &expected[position], "mixed.effects").await?;
        membership(backend, &entries[..=position]).await?;
        for original in &entries[..=position] {
            original
                .retained(&backend.connect().await.map_err(operation)?)
                .await?;
            compare(backend, &expected[position], "mixed.retry_effects").await?;
            membership(backend, &entries[..=position]).await?;
        }
    }
    Ok(())
}
