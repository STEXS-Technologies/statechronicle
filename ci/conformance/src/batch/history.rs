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
        self.stage_scheduled(store, prefix, None).await
    }
    async fn stage_scheduled<S: LedgerBatchRecovery>(
        &self,
        store: &S,
        prefix: usize,
        mut schedule: Option<u64>,
    ) -> Result<Box<dyn LedgerTransaction>, Violation> {
        match self {
            Self::Batch(batch) => super::stage_scheduled(store, batch, prefix, schedule).await,
            Self::Single(mutation) => {
                let mut tx = store
                    .begin(&mutation.intent.tenant_id)
                    .await
                    .map_err(operation)?;
                let attempt = claim(&mut *tx, mutation).await.map_err(operation)?;
                scheduled_yield(&mut schedule).await;
                for index in 0..prefix {
                    if let Err(error) = step(&mut *tx, mutation, &attempt, index).await {
                        tx.rollback().await.map_err(operation)?;
                        return Err(operation(error));
                    }
                    scheduled_yield(&mut schedule).await;
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
    validate_chain(entries)?;
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

fn validate_chain(entries: &[HistoryEntry]) -> Result<(), Violation> {
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
    Ok(())
}

// Fixed, portable scheduling PRNG. Zero is a valid reproducible seed.
fn advance(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *seed
}
pub(super) async fn scheduled_yield(schedule: &mut Option<u64>) {
    if let Some(seed) = schedule {
        for _ in 0..(advance(seed) >> 62) {
            tokio::task::yield_now().await;
        }
    }
}
fn scoped(snapshot: &Snapshot, tenant: &TenantId) -> Snapshot {
    Snapshot {
        events: snapshot
            .events
            .iter()
            .filter(|x| &x.tenant_id == tenant)
            .cloned()
            .collect(),
        commits: snapshot
            .commits
            .iter()
            .filter(|x| x.body.scope.tenant_id.as_ref() == Some(tenant))
            .cloned()
            .collect(),
        projections: snapshot
            .projections
            .iter()
            .filter(|x| &x.tenant_id == tenant)
            .cloned()
            .collect(),
        outbox: snapshot
            .outbox
            .iter()
            .filter(|x| &x.tenant == tenant)
            .cloned()
            .collect(),
        finalized: snapshot
            .finalized
            .iter()
            .filter(|x| &x.0 == tenant)
            .cloned()
            .collect(),
        heads: snapshot
            .heads
            .iter()
            .filter(|x| &x.0 == tenant)
            .cloned()
            .collect(),
    }
}
fn counts(snapshot: &Snapshot) -> [usize; 6] {
    [
        snapshot.events.len(),
        snapshot.commits.len(),
        snapshot.projections.len(),
        snapshot.outbox.len(),
        snapshot.finalized.len(),
        snapshot.heads.len(),
    ]
}
// Every independently observed tenant is wholly before or wholly after this wave.
// Different tenants may linearize independently; no global atomicity is assumed.
async fn observe_wave<B: LedgerBatchTestBackend>(
    backend: &B,
    tenants: &std::collections::BTreeMap<String, TenantId>,
    before: &Snapshot,
    after: &Snapshot,
) -> Result<(), Violation> {
    let actual = backend.inspect().await.map_err(operation)?;
    let mut accounted = [0usize; 6];
    for tenant in tenants.values() {
        let partition = scoped(&actual, tenant);
        for (total, count) in accounted.iter_mut().zip(counts(&partition)) {
            *total += count;
        }
        let rows = partition.normalized()?;
        require(
            "mixed.concurrent.visibility",
            rows == scoped(before, tenant).normalized()?
                || rows == scoped(after, tenant).normalized()?,
            "reader observed a partial tenant transaction",
        )?;
    }
    require(
        "mixed.concurrent.scope",
        accounted == counts(&actual),
        "reader observed rows outside the supplied tenant histories",
    )
}

/// Check seeded concurrent waves of single/batch transactions with retained history.
///
/// Each wave contains one next transaction per participating tenant. Supply an
/// independent complete snapshot after every wave, linked signed chains, both
/// transaction kinds, and at least one wave with multiple tenants. Fixtures are
/// validated before destructive reset. Independent connections stage concurrently
/// with seeded yields at each completed write boundary. Each staged writer is
/// inspected and released independently, allowing serialized storage engines.
/// Staged effects remain private; readers may see either complete tenant state,
/// and the final wave snapshot, membership and every old identity must remain exact.
///
/// Seed controls cooperative yields, not the operating system or database schedule.
/// This supplements same-tenant head/idempotency races; it is not process/crash or
/// power-loss qualification. Use a dedicated backend and an outer timeout.
pub async fn check_concurrent<B: LedgerBatchTestBackend>(
    backend: &B,
    waves: &[Vec<HistoryEntry>],
    expected: &[Snapshot],
    mut seed: u64,
) -> Result<(), Violation>
where
    B::Connection: LedgerBatchRecovery,
{
    let entries: Vec<_> = waves.iter().flatten().cloned().collect();
    require(
        "setup.mixed_concurrent",
        !waves.is_empty()
            && waves.len() == expected.len()
            && waves.iter().all(|wave| !wave.is_empty())
            && waves.iter().any(|wave| wave.len() > 1)
            && entries.iter().any(|e| matches!(e, HistoryEntry::Single(_)))
            && entries.iter().any(|e| matches!(e, HistoryEntry::Batch(_))),
        "requires nonempty waves, both transaction kinds, concurrency and one snapshot per wave",
    )?;
    validate_chain(&entries)?;
    let mut tenants = std::collections::BTreeMap::new();
    for wave in waves {
        let mut scopes = std::collections::BTreeSet::new();
        for entry in wave {
            let tenant = entry.intents()[0].tenant_id.clone();
            require(
                "setup.mixed_concurrent_scope",
                scopes.insert(tenant.0.clone()),
                "a wave contains competing entries in the same tenant; use the head-contention check",
            )?;
            tenants.insert(tenant.0.clone(), tenant);
        }
    }
    let _ = fresh(backend).await?;
    membership(backend, &[]).await?;
    let mut committed = Vec::new();
    let empty = Snapshot::default();
    for (index, wave) in waves.iter().enumerate() {
        let before = index.checked_sub(1).map_or(&empty, |i| &expected[i]);
        let mut staging = tokio::task::JoinSet::new();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(wave.len()));
        for entry in wave {
            let entry = entry.clone();
            let writer = backend.connect().await.map_err(operation)?;
            let schedule = advance(&mut seed);
            let barrier = barrier.clone();
            staging.spawn(async move {
                barrier.wait().await;
                let tx = entry
                    .stage_scheduled(&writer, entry.prefixes(), Some(schedule))
                    .await?;
                Ok::<_, Violation>((entry, tx))
            });
        }
        let mut jobs = tokio::task::JoinSet::new();
        // Inspect and release each staged writer independently. Waiting for all
        // writers to stage before releasing any would deadlock serial writers.
        while !staging.is_empty() || !jobs.is_empty() {
            tokio::select! {
                result = staging.join_next(), if !staging.is_empty() => {
                    let (entry, tx) = result.ok_or_else(|| operation("missing staging task"))?.map_err(operation)??;
                    let tenant = &entry.intents()[0].tenant_id;
                    let actual = backend.inspect().await.map_err(operation)?;
                    require("mixed.concurrent.staged_effects", scoped(&actual, tenant).normalized()? == scoped(before, tenant).normalized()?, "staged tenant effects became visible")?;
                    let reader = backend.connect().await.map_err(operation)?;
                    for intent in entry.intents() {
                        require("mixed.concurrent.staged_receipt",
                            !matches!(reader.lookup_mutation(&intent.tenant_id, &intent.intent_id).await.map_err(operation)?, LedgerLookup::Committed(_))
                            && !matches!(reader.lookup_batch(&intent.tenant_id, &intent.intent_id).await.map_err(operation)?, LedgerBatchLookup::Committed(_)),
                            "fully staged transaction issued a committed receipt")?;
                    }
                    let rows = backend.inspect_batches().await.map_err(operation)?;
                    require("mixed.concurrent.staged_membership", !rows.iter().any(|row| &row.tenant == tenant && row.batch.commit.body.commit_id == entry.commit().body.commit_id), "uncommitted group membership became visible")?;
                    let mut schedule = Some(advance(&mut seed));
                    jobs.spawn(async move {
                        scheduled_yield(&mut schedule).await;
                        tx.commit().await.map_err(operation)
                    });
                }
                result = jobs.join_next(), if !jobs.is_empty() => {
                    result.ok_or_else(|| operation("missing commit task"))?.map_err(operation)??;
                    observe_wave(backend, &tenants, before, &expected[index]).await?;
                }
            }
        }
        committed.extend(wave.iter().cloned());
        compare(backend, &expected[index], "mixed.concurrent.effects").await?;
        membership(backend, &committed).await?;
        for original in &committed {
            original
                .retained(&backend.connect().await.map_err(operation)?)
                .await?;
            compare(backend, &expected[index], "mixed.concurrent.retry_effects").await?;
            membership(backend, &committed).await?;
        }
    }
    Ok(())
}
