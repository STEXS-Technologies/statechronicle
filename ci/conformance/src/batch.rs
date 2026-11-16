//! Optional single-tenant batch checks using raw independent observations.
//! These are logical transaction checks, not physical database crash tests.
use super::*;
use statechronicle::ports::ledger_batch::{LedgerBatch, LedgerBatchLookup, LedgerBatchRecovery};

/// One actual retained membership row. Include duplicates and query the
/// database directly, never reconstruct rows from a lookup result.
#[derive(Debug, Clone)]
pub struct RetainedBatch {
    pub tenant: TenantId,
    pub intent_id: IntentId,
    pub batch: LedgerBatch,
}
/// Optional raw batch inspection capability on the same isolated test backend.
#[async_trait]
pub trait LedgerBatchTestBackend: LedgerTestBackend {
    async fn inspect_batches(&self) -> Result<Vec<RetainedBatch>, String>;
}
fn tenant(batch: &LedgerBatch) -> Result<&TenantId, Violation> {
    batch
        .intents
        .first()
        .map(|intent| &intent.tenant_id)
        .ok_or_else(|| failure("fixture.batch", "empty batch"))
}
fn expected(batch: &LedgerBatch) -> Result<Snapshot, Violation> {
    let tenant = tenant(batch)?;
    let mut rows = Snapshot {
        events: batch.events.clone(),
        commits: vec![batch.commit.clone()],
        projections: batch.projections.clone(),
        outbox: batch.outbox.clone(),
        ..Default::default()
    };
    for intent in &batch.intents {
        rows.finalized.push((
            intent.tenant_id.clone(),
            intent.intent_id.clone(),
            canonicalize_and_digest(intent).map_err(operation)?,
            batch.commit.body.commit_id.clone(),
        ));
    }
    rows.heads.push((
        tenant.clone(),
        batch.commit.body.commit_id.clone(),
        batch.commit.body.next_state_root.clone(),
        batch.commit.body.sequence,
    ));
    Ok(rows)
}
fn normalized(records: &[RetainedBatch]) -> Result<Vec<Vec<u8>>, Violation> {
    let mut rows = Vec::new();
    for record in records {
        let outbox: Vec<_> = record
            .batch
            .outbox
            .iter()
            .map(|row| {
                (
                    &row.delivery_key,
                    &row.tenant,
                    &row.commit_id,
                    &row.payload_digest,
                    &row.payload,
                )
            })
            .collect();
        rows.push(
            canonicalize(&(
                &record.tenant,
                &record.intent_id,
                &record.batch.intents,
                &record.batch.commit,
                &record.batch.events,
                &record.batch.projections,
                outbox,
            ))
            .map_err(operation)?,
        );
    }
    rows.sort();
    Ok(rows)
}
async fn compare_batch<B: LedgerBatchTestBackend>(
    backend: &B,
    batch: Option<&LedgerBatch>,
    check: &'static str,
) -> Result<(), Violation> {
    compare_batches(backend, &batch.into_iter().collect::<Vec<_>>(), check).await
}
async fn compare_batches<B: LedgerBatchTestBackend>(
    backend: &B,
    batches: &[&LedgerBatch],
    check: &'static str,
) -> Result<(), Violation> {
    let mut effects = Snapshot::default();
    for batch in batches {
        let rows = expected(batch)?;
        effects.events.extend(rows.events);
        effects.commits.extend(rows.commits);
        effects.projections.extend(rows.projections);
        effects.outbox.extend(rows.outbox);
        effects.finalized.extend(rows.finalized);
        effects.heads.extend(rows.heads);
    }
    compare(backend, &effects, check).await?;
    let expected: Vec<_> = batches
        .iter()
        .flat_map(|batch| {
            batch.intents.iter().map(|intent| RetainedBatch {
                tenant: intent.tenant_id.clone(),
                intent_id: intent.intent_id.clone(),
                batch: (*batch).clone(),
            })
        })
        .collect();
    let actual = backend.inspect_batches().await.map_err(operation)?;
    require(
        check,
        normalized(&actual)? == normalized(&expected)?,
        "raw retained group membership/effects differ",
    )
}
async fn original<S: LedgerBatchRecovery>(store: &S, batch: &LedgerBatch) -> Result<(), Violation> {
    for intent in &batch.intents {
        require(
            "batch.lookup.original",
            matches!(store.lookup_batch(&intent.tenant_id, &intent.intent_id).await.map_err(operation)?, LedgerBatchLookup::Committed(record) if *record == *batch),
            "member did not resolve to the exact complete original group",
        )?;
        require(
            "batch.lookup.single",
            store
                .lookup_mutation(&intent.tenant_id, &intent.intent_id)
                .await
                .is_err(),
            "batch member was reinterpreted as a standalone mutation or absence",
        )?;
    }
    Ok(())
}
#[derive(Clone, Copy)]
enum Step {
    Claim(usize),
    Events,
    Commit,
    Projection(usize),
    Outbox(usize),
    Retain,
    Finalize(usize),
}
fn steps(batch: &LedgerBatch) -> Vec<Step> {
    let mut steps: Vec<_> = (0..batch.intents.len()).map(Step::Claim).collect();
    steps.extend([Step::Events, Step::Commit]);
    steps.extend((0..batch.projections.len()).map(Step::Projection));
    steps.extend((0..batch.outbox.len()).map(Step::Outbox));
    steps.push(Step::Retain);
    steps.extend((0..batch.intents.len()).map(Step::Finalize));
    steps
}
async fn stage<S: LedgerBatchRecovery>(
    store: &S,
    batch: &LedgerBatch,
    prefix: usize,
) -> Result<Box<dyn LedgerTransaction>, Violation> {
    let mut tx = store.begin(tenant(batch)?).await.map_err(operation)?;
    let mut attempts = Vec::new();
    for step in steps(batch).into_iter().take(prefix) {
        let result = match step {
            Step::Claim(index) => {
                let intent = &batch.intents[index];
                match tx
                    .claim_idempotency(
                        &intent.tenant_id,
                        intent,
                        &canonicalize_and_digest(intent).map_err(operation)?,
                    )
                    .await
                    .map_err(operation)?
                {
                    IdempotencyClaim::NewReservation { attempt_id } if !attempt_id.is_empty() => {
                        attempts.push(attempt_id);
                        Ok(())
                    }
                    other => {
                        return Err(failure(
                            "batch.claim.new",
                            format!("expected fresh member reservation, got {other:?}"),
                        ));
                    }
                }
            }
            Step::Events => tx.append_events(&batch.events).await,
            Step::Commit => tx.append_commit(&batch.commit).await,
            Step::Projection(index) => tx.upsert_projection(&batch.projections[index]).await,
            Step::Outbox(index) => tx.enqueue_outbox(&batch.outbox[index]).await,
            Step::Retain => tx.retain_batch(batch).await,
            Step::Finalize(index) => {
                tx.finalize_idempotency(
                    &batch.intents[index].tenant_id,
                    &batch.intents[index].intent_id,
                    &attempts[index],
                    &batch.commit.body.commit_id,
                )
                .await
            }
        };
        if let Err(error) = result {
            tx.rollback().await.map_err(operation)?;
            return Err(operation(error));
        }
    }
    Ok(tx)
}
/// Write a well-formed signed fixture directly through the adapter. Ingress and
/// signer validation remain separate authorities, as in the single-write checks.
pub async fn write<S: LedgerBatchRecovery>(
    store: &S,
    batch: &LedgerBatch,
) -> Result<(), Violation> {
    stage(store, batch, steps(batch).len())
        .await?
        .commit()
        .await
        .map_err(operation)
}
/// Every write prefix remains invisible, including retained metadata and each
/// member finalization. Commit publishes the entire group; reopen retains it.
pub async fn check_atomic_visibility<B: LedgerBatchTestBackend>(
    backend: &B,
    batch: &LedgerBatch,
) -> Result<(), Violation>
where
    B::Connection: LedgerBatchRecovery,
{
    // Inspect each prefix in a separate transaction so observations do not rely
    // on scheduling or on the adapter's own lookup/cache.
    for prefix in 0..=steps(batch).len() {
        let store = fresh(backend).await?;
        let tx = stage(&store, batch, prefix).await?;
        compare_batch(backend, None, "batch.atomic.visibility").await?;
        drop(tx);
    }
    let store = fresh(backend).await?;
    write(&store, batch).await?;
    compare_batch(backend, Some(batch), "batch.atomic.completeness").await?;
    original(&backend.connect().await.map_err(operation)?, batch).await
}
/// Explicit rollback releases every acquired member, including after another
/// member finalized privately; exact retry can then publish one complete group.
pub async fn check_rollback<B: LedgerBatchTestBackend>(
    backend: &B,
    batch: &LedgerBatch,
) -> Result<(), Violation>
where
    B::Connection: LedgerBatchRecovery,
{
    for prefix in 0..=steps(batch).len() {
        let store = fresh(backend).await?;
        stage(&store, batch, prefix)
            .await?
            .rollback()
            .await
            .map_err(operation)?;
        compare_batch(backend, None, "batch.rollback.effects").await?;
        let reopened = backend.connect().await.map_err(operation)?;
        write(&reopened, batch).await?;
        compare_batch(backend, Some(batch), "batch.rollback.retry").await?;
        original(&reopened, batch).await?;
    }
    Ok(())
}
/// Dropping each completed write prefix publishes neither effects nor a group
/// receipt. A leased unfinished reservation may remain after cancellation.
pub async fn check_cancellation<B: LedgerBatchTestBackend>(
    backend: &B,
    batch: &LedgerBatch,
) -> Result<(), Violation>
where
    B::Connection: LedgerBatchRecovery,
{
    for prefix in 0..=steps(batch).len() {
        let store = fresh(backend).await?;
        drop(stage(&store, batch, prefix).await?);
        compare_batch(backend, None, "batch.cancel.effects").await?;
        let reader = backend.connect().await.map_err(operation)?;
        for intent in &batch.intents {
            require(
                "batch.cancel.receipt",
                !matches!(
                    reader
                        .lookup_batch(&intent.tenant_id, &intent.intent_id)
                        .await
                        .map_err(operation)?,
                    LedgerBatchLookup::Committed(_)
                ),
                "cancelled stage exposed a committed group",
            )?;
        }
    }
    Ok(())
}
/// Every exact member replays the same original commit; changed payloads
/// conflict and the complete original group remains unchanged.
pub async fn check_replay<B: LedgerBatchTestBackend>(
    backend: &B,
    batch: &LedgerBatch,
) -> Result<(), Violation>
where
    B::Connection: LedgerBatchRecovery,
{
    let store = fresh(backend).await?;
    write(&store, batch).await?;
    for intent in &batch.intents {
        let mut tx = store.begin(&intent.tenant_id).await.map_err(operation)?;
        let digest = canonicalize_and_digest(intent).map_err(operation)?;
        require(
            "batch.replay.identity",
            matches!(tx.claim_idempotency(&intent.tenant_id, intent, &digest).await.map_err(operation)?, IdempotencyClaim::Committed { commit_id } if commit_id == batch.commit.body.commit_id),
            "member did not replay original commit",
        )?;
        tx.rollback().await.map_err(operation)?;
        let mut changed = intent.clone();
        let mut nonce = changed.nonce.as_bytes().to_vec();
        if let Some(first) = nonce.first_mut() {
            *first ^= 1;
        } else {
            nonce.push(1);
        }
        changed.nonce = Nonce::from_bytes(nonce).map_err(operation)?;
        let mut tx = store.begin(&intent.tenant_id).await.map_err(operation)?;
        require(
            "batch.replay.payload",
            matches!(
                tx.claim_idempotency(
                    &intent.tenant_id,
                    &changed,
                    &canonicalize_and_digest(&changed).map_err(operation)?
                )
                .await
                .map_err(operation)?,
                IdempotencyClaim::ConflictDifferentPayload
            ),
            "changed member payload did not conflict",
        )?;
        tx.rollback().await.map_err(operation)?;
    }
    compare_batch(backend, Some(batch), "batch.replay.effects").await?;
    original(&backend.connect().await.map_err(operation)?, batch).await
}
/// ACK loss is injected after a real inner commit. The advanced coordinator must
/// report unknown, and a fresh independent connection must recover the complete
/// original group and exact retry without a second publication.
pub async fn check_lost_ack<B: LedgerBatchTestBackend>(
    backend: &B,
    batch: &LedgerBatch,
    principals: &[statechronicle::ports::authorization::AuthenticatedPrincipal],
    authorizer: &dyn statechronicle::ports::authorization::Authorizer,
    verifier: &dyn statechronicle::commit::persist::SignedCommitVerifier,
) -> Result<(), Violation>
where
    B::Connection: LedgerBatchRecovery,
{
    use statechronicle::commit::durable::DurableError;
    use statechronicle::commit::durable_batch::{
        BatchMember, BatchRecoveryOutcome, DurableBatchOutcome, DurableBatchRequest,
        commit_batch_verified, recover_batch,
    };
    use statechronicle::commit::persist::CommittedEvent;
    require(
        "fixture.principals",
        principals.len() == batch.intents.len(),
        "one independent principal per member required",
    )?;
    let digests: Vec<_> = batch
        .intents
        .iter()
        .map(canonicalize_and_digest)
        .collect::<Result<_, _>>()
        .map_err(operation)?;
    let group_digest = canonicalize_and_digest(&("statechronicle.atomic-batch.v0", &batch.intents))
        .map_err(operation)?;
    let members: Vec<_> = batch
        .intents
        .iter()
        .zip(principals)
        .zip(&digests)
        .map(|((intent, principal), payload_digest)| BatchMember {
            intent,
            principal,
            payload_digest,
        })
        .collect();
    let entries: Vec<_> = batch
        .events
        .iter()
        .map(|event| CommittedEvent {
            event,
            state_type: event.after.state.state_type(),
        })
        .collect();
    let request = || DurableBatchRequest {
        members: &members,
        entries: &entries,
        commit: &batch.commit,
        projections: &batch.projections,
        outbox: &batch.outbox,
    };
    let store = fresh(backend).await?;
    let losing = crate::faults::LoseCommitAck(store);
    let error = commit_batch_verified(&losing, authorizer, verifier, request())
        .await
        .err()
        .ok_or_else(|| failure("batch.ack.unknown", "lost ACK returned success"))?;
    require(
        "batch.ack.unknown",
        matches!(error.cause(), DurableError::Unknown { .. })
            && error.identity().is_some_and(|id| {
                id.payload_digest() == &group_digest
                    && id.members().len() == members.len()
                    && id.members().iter().zip(&members).all(|(actual, member)| {
                        actual.tenant() == &member.intent.tenant_id
                            && actual.intent_id() == &member.intent.intent_id
                            && actual.payload_digest() == member.payload_digest
                    })
            }),
        "lost ACK did not retain complete unknown identity",
    )?;
    compare_batch(backend, Some(batch), "batch.ack.original").await?;
    let reopened = backend.connect().await.map_err(operation)?;
    require(
        "batch.ack.recover",
        matches!(recover_batch(&reopened, authorizer, verifier, &members).await.map_err(operation)?, BatchRecoveryOutcome::Committed(receipt) if receipt.record() == batch),
        "recovery did not return complete original group",
    )?;
    require(
        "batch.ack.replay",
        matches!(commit_batch_verified(&reopened, authorizer, verifier, request()).await.map_err(operation)?, DurableBatchOutcome::Replay(receipt) if receipt.record() == batch),
        "retry did not replay complete original group",
    )?;
    compare_batch(backend, Some(batch), "batch.ack.duplicate").await
}

/// Distinct complete groups race against one tenant genesis head. Exactly one
/// whole group may publish; every raw effect and retained member belongs to it.
pub async fn check_head_contention<B: LedgerBatchTestBackend>(
    backend: &B,
    batches: &[LedgerBatch],
) -> Result<(), Violation>
where
    B::Connection: LedgerBatchRecovery,
{
    require(
        "fixture.contenders",
        batches.len() >= 2,
        "at least two groups required",
    )?;
    let first = &batches[0];
    let scope = tenant(first)?;
    let mut members = std::collections::BTreeSet::new();
    let mut commits = std::collections::BTreeSet::new();
    for batch in batches {
        require(
            "fixture.contenders",
            tenant(batch)? == scope
                && batch.commit.body.scope == first.commit.body.scope
                && batch.commit.body.sequence == 1
                && batch.commit.body.parent_commit_id.is_none()
                && batch.commit.body.previous_state_root == first.commit.body.previous_state_root
                && commits.insert(batch.commit.body.commit_id.clone())
                && batch.intents.iter().all(|intent| {
                    intent.tenant_id == *scope && members.insert(intent.intent_id.clone())
                }),
            "distinct groups/commits against one tenant genesis head required",
        )?;
    }
    let reader = fresh(backend).await?;
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(batches.len()));
    let mut jobs = tokio::task::JoinSet::new();
    for (index, batch) in batches.iter().cloned().enumerate() {
        let store = backend.connect().await.map_err(operation)?;
        let barrier = barrier.clone();
        jobs.spawn(async move {
            barrier.wait().await;
            let tx = stage(&store, &batch, steps(&batch).len()).await?;
            Ok::<_, Violation>((index, tx.commit().await))
        });
    }
    let mut winners = Vec::new();
    while let Some(job) = jobs.join_next().await {
        let (index, result) = job.map_err(operation)??;
        match result {
            Ok(()) => winners.push(index),
            Err(LedgerStoreError::Conflict(_)) => {}
            Err(error) => return Err(operation(error)),
        }
    }
    require(
        "batch.head.single_winner",
        winners.len() == 1,
        "head race did not publish exactly one complete group",
    )?;
    let winner = &batches[winners[0]];
    compare_batch(backend, Some(winner), "batch.head.effects").await?;
    original(&reader, winner).await
}

/// Coincident member, event, resource and commit IDs in distinct tenant scopes
/// must neither leak through lookup nor conflict in the claim namespace.
pub async fn check_tenant_isolation<B: LedgerBatchTestBackend>(
    backend: &B,
    left: &LedgerBatch,
    right: &LedgerBatch,
) -> Result<(), Violation>
where
    B::Connection: LedgerBatchRecovery,
{
    require(
        "fixture.tenants",
        tenant(left)? != tenant(right)?
            && left.commit.body.commit_id == right.commit.body.commit_id
            && left.intents.len() >= 2
            && left
                .intents
                .iter()
                .map(|intent| (&intent.intent_id, &intent.resource_id))
                .eq(right
                    .intents
                    .iter()
                    .map(|intent| (&intent.intent_id, &intent.resource_id)))
            && left
                .events
                .iter()
                .map(|event| (&event.event_id, &event.resource_id))
                .eq(right
                    .events
                    .iter()
                    .map(|event| (&event.event_id, &event.resource_id))),
        "distinct tenants with coincident commit/member/event/resource IDs required",
    )?;
    let writer = fresh(backend).await?;
    write(&writer, left).await?;
    let reader = backend.connect().await.map_err(operation)?;
    for intent in &right.intents {
        require(
            "batch.tenant.lookup",
            matches!(
                reader
                    .lookup_batch(&intent.tenant_id, &intent.intent_id)
                    .await
                    .map_err(operation)?,
                LedgerBatchLookup::NotFound
            ),
            "another tenant's group leaked through lookup",
        )?;
    }
    write(&backend.connect().await.map_err(operation)?, right).await?;
    compare_batches(backend, &[left, right], "batch.tenant.effects").await?;
    original(&reader, left).await?;
    original(&reader, right).await
}

async fn committed_claim<S: LedgerBatchRecovery>(
    store: &S,
    intent: &statechronicle::Intent,
    commit: &CommitId,
) -> Result<(), Violation> {
    let mut tx = store.begin(&intent.tenant_id).await.map_err(operation)?;
    let result = tx
        .claim_idempotency(
            &intent.tenant_id,
            intent,
            &canonicalize_and_digest(intent).map_err(operation)?,
        )
        .await
        .map_err(operation)?;
    tx.rollback().await.map_err(operation)?;
    require(
        "batch.namespace.claim",
        matches!(result, IdempotencyClaim::Committed { commit_id } if commit_id == *commit),
        "single/batch lanes do not share the original committed identity",
    )
}
/// A standalone mutation and a batch contain the exact same intent body. Both
/// claim directions must resolve the original commit, while cross-kind lookup
/// must refuse rather than fabricating a new group or a standalone subset.
pub async fn check_single_namespace<B: LedgerBatchTestBackend>(
    backend: &B,
    single: &LedgerMutation,
    batch: &LedgerBatch,
) -> Result<(), Violation>
where
    B::Connection: LedgerBatchRecovery,
{
    require(
        "fixture.namespace",
        batch.intents.len() >= 2
            && batch.intents.contains(&single.intent)
            && single.commit.body.commit_id != batch.commit.body.commit_id,
        "single intent must exactly match a member of a different multi-member commit",
    )?;
    let store = fresh(backend).await?;
    super::write(&store, single).await.map_err(operation)?;
    let reader = backend.connect().await.map_err(operation)?;
    committed_claim(&reader, &single.intent, &single.commit.body.commit_id).await?;
    require(
        "batch.namespace.singleton",
        matches!(
            reader
                .lookup_batch(&single.intent.tenant_id, &single.intent.intent_id)
                .await,
            Err(LedgerStoreError::Invariant(_))
        ),
        "standalone mutation was reinterpreted as a group or absence",
    )?;
    super::original(&reader, single).await?;
    compare(
        backend,
        &Snapshot::from_mutations(&[single]),
        "batch.namespace.single_effects",
    )
    .await?;
    require(
        "batch.namespace.single_metadata",
        backend
            .inspect_batches()
            .await
            .map_err(operation)?
            .is_empty(),
        "standalone mutation manufactured retained batch membership",
    )?;

    let store = fresh(backend).await?;
    write(&store, batch).await?;
    let reader = backend.connect().await.map_err(operation)?;
    for intent in &batch.intents {
        committed_claim(&reader, intent, &batch.commit.body.commit_id).await?;
        require(
            "batch.namespace.subset",
            matches!(
                reader
                    .lookup_mutation(&intent.tenant_id, &intent.intent_id)
                    .await,
                Err(LedgerStoreError::Invariant(_))
            ),
            "group member was reinterpreted as a standalone mutation or absence",
        )?;
    }
    original(&reader, batch).await?;
    compare_batch(backend, Some(batch), "batch.namespace.group_effects").await
}

/// Exact concurrent groups must yield one writer, with all others pending or
/// replaying the complete original group. Independent inspection includes every
/// finalized member and retained record. Supply independent fixture policies.
pub async fn check_idempotency_contention<B: LedgerBatchTestBackend>(
    backend: &B,
    batch: &LedgerBatch,
    principals: &[statechronicle::ports::authorization::AuthenticatedPrincipal],
    authorizer: std::sync::Arc<dyn statechronicle::ports::authorization::Authorizer>,
    verifier: std::sync::Arc<dyn statechronicle::commit::persist::SignedCommitVerifier>,
    clients: usize,
) -> Result<(), Violation>
where
    B::Connection: LedgerBatchRecovery,
{
    use statechronicle::commit::durable_batch::{
        BatchMember, DurableBatchOutcome, DurableBatchRequest, commit_batch_verified,
    };
    use statechronicle::commit::{durable::DurableError, persist::CommittedEvent};
    require(
        "fixture.clients",
        clients >= 2 && principals.len() == batch.intents.len(),
        "multiple clients and one independent principal per member required",
    )?;
    let reader = fresh(backend).await?;
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(clients));
    let mut jobs = tokio::task::JoinSet::new();
    for _ in 0..clients {
        let store = backend.connect().await.map_err(operation)?;
        let batch = batch.clone();
        let principals = principals.to_vec();
        let barrier = barrier.clone();
        let authorizer = authorizer.clone();
        let verifier = verifier.clone();
        jobs.spawn(async move {
            let digests: Vec<_> = batch
                .intents
                .iter()
                .map(canonicalize_and_digest)
                .collect::<Result<_, _>>()
                .map_err(operation)?;
            let members: Vec<_> = batch
                .intents
                .iter()
                .zip(&principals)
                .zip(&digests)
                .map(|((intent, principal), payload_digest)| BatchMember {
                    intent,
                    principal,
                    payload_digest,
                })
                .collect();
            let entries: Vec<_> = batch
                .events
                .iter()
                .map(|event| CommittedEvent {
                    event,
                    state_type: event.after.state.state_type(),
                })
                .collect();
            barrier.wait().await;
            let result = commit_batch_verified(
                &store,
                &*authorizer,
                &*verifier,
                DurableBatchRequest {
                    members: &members,
                    entries: &entries,
                    commit: &batch.commit,
                    projections: &batch.projections,
                    outbox: &batch.outbox,
                },
            )
            .await;
            match result {
                Ok(DurableBatchOutcome::Committed(receipt)) => {
                    require(
                        "batch.identity.original",
                        receipt.record() == &batch,
                        "writer returned a different original group",
                    )?;
                    Ok(true)
                }
                Ok(DurableBatchOutcome::Replay(receipt)) => {
                    require(
                        "batch.identity.original",
                        receipt.record() == &batch,
                        "replay returned a different original group",
                    )?;
                    Ok(false)
                }
                Err(error) if matches!(error.cause(), DurableError::InProgress { .. }) => {
                    require(
                        "batch.identity.pending",
                        error.identity().is_some_and(|identity| {
                            identity.members().len() == members.len()
                                && identity.members().iter().zip(&members).all(
                                    |(actual, expected)| {
                                        actual.tenant() == &expected.intent.tenant_id
                                            && actual.intent_id() == &expected.intent.intent_id
                                            && actual.payload_digest() == expected.payload_digest
                                    },
                                )
                        }),
                        "pending response lost or changed group membership",
                    )?;
                    Ok(false)
                }
                Err(error) => Err(operation(error)),
            }
        });
    }
    let mut writers = 0;
    while let Some(job) = jobs.join_next().await {
        if job.map_err(operation)?? {
            writers += 1;
        }
    }
    require(
        "batch.identity.single_writer",
        writers == 1,
        "exact concurrent group did not yield one writer",
    )?;
    compare_batch(backend, Some(batch), "batch.identity.effects").await?;
    original(&reader, batch).await
}
