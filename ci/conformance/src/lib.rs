//! Dev-only adapter contract checks using independent connections and observations.
//! This harness performs destructive resets: supply an isolated test database.
use async_trait::async_trait;
use serde::Serialize;
use statechronicle::core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle::domain::intent::Nonce;
use statechronicle::ports::ledger_store::{
    IdempotencyClaim, LedgerLookup, LedgerMutation, LedgerRecovery, LedgerStore, LedgerStoreError,
    LedgerTransaction, OutboxRecord,
};
use statechronicle::{
    Commit, CommitId, ContentDigest, Event, IntentId, Signed, StateProjection, TenantId,
};
use std::fmt;

/// Independent raw observations. Include every row, including duplicates.
/// Inspect the database directly, not through `lookup_mutation` or cached receipts.
#[derive(Default, Clone, Debug)]
pub struct Snapshot {
    pub events: Vec<Event>,
    pub commits: Vec<Signed<Commit>>,
    pub projections: Vec<StateProjection>,
    pub outbox: Vec<OutboxRecord>,
    pub finalized: Vec<(TenantId, IntentId, ContentDigest, CommitId)>,
    /// Read the adapter's actual canonical-head index, not a max over history.
    pub heads: Vec<(TenantId, CommitId, ContentDigest, u64)>,
}
/// Adapter-specific setup and inspection; implement only in backend tests.
#[async_trait]
pub trait LedgerTestBackend: Send + Sync {
    type Connection: LedgerRecovery + 'static;
    /// Clear only the isolated test database, including reservations/history.
    async fn reset(&self) -> Result<(), String>;
    /// Open a fresh independent connection to the same database each time.
    async fn connect(&self) -> Result<Self::Connection, String>;
    /// Observe committed data using an independent reader connection.
    async fn inspect(&self) -> Result<Snapshot, String>;
}
/// A failed assertion, distinct from adapter operation/setup errors.
#[derive(Debug)]
pub struct Violation {
    pub check: &'static str,
    pub detail: String,
}
impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.check, self.detail)
    }
}
impl std::error::Error for Violation {}
fn failure(check: &'static str, detail: impl fmt::Display) -> Violation {
    Violation {
        check,
        detail: detail.to_string(),
    }
}
fn operation(error: impl fmt::Display) -> Violation {
    failure("adapter.operation", error)
}
fn require(check: &'static str, value: bool, detail: &str) -> Result<(), Violation> {
    if value {
        Ok(())
    } else {
        Err(failure(check, detail))
    }
}
fn rows<T: Serialize>(items: &[T]) -> Result<Vec<Vec<u8>>, Violation> {
    let mut encoded = items
        .iter()
        .map(canonicalize)
        .collect::<Result<Vec<_>, _>>()
        .map_err(operation)?;
    encoded.sort();
    Ok(encoded)
}
impl Snapshot {
    fn normalized(&self) -> Result<Vec<Vec<Vec<u8>>>, Violation> {
        let outbox: Vec<_> = self
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
        Ok(vec![
            rows(&self.events)?,
            rows(&self.commits)?,
            rows(&self.projections)?,
            rows(&outbox)?,
            rows(&self.finalized)?,
            rows(&self.heads)?,
        ])
    }
    /// Expected rows for independent genesis mutations in distinct tenants.
    fn from_mutations(mutations: &[&LedgerMutation]) -> Self {
        let mut snapshot = Self::default();
        for mutation in mutations {
            snapshot.events.extend(mutation.events.clone());
            snapshot.commits.push(mutation.commit.clone());
            snapshot.projections.extend(mutation.projections.clone());
            snapshot.outbox.extend(mutation.outbox.clone());
            snapshot.finalized.push((
                mutation.intent.tenant_id.clone(),
                mutation.intent.intent_id.clone(),
                mutation.payload_digest.clone(),
                mutation.commit.body.commit_id.clone(),
            ));
            snapshot.heads.push((
                mutation.intent.tenant_id.clone(),
                mutation.commit.body.commit_id.clone(),
                mutation.commit.body.next_state_root.clone(),
                mutation.commit.body.sequence,
            ));
        }
        snapshot
    }
}
async fn compare<B: LedgerTestBackend>(
    backend: &B,
    expected: &Snapshot,
    check: &'static str,
) -> Result<(), Violation> {
    let actual = backend.inspect().await.map_err(operation)?;
    require(
        check,
        actual.normalized()? == expected.normalized()?,
        "independently observed effects differ",
    )
}
async fn fresh<B: LedgerTestBackend>(backend: &B) -> Result<B::Connection, Violation> {
    backend.reset().await.map_err(operation)?;
    compare(backend, &Snapshot::default(), "setup.empty").await?;
    backend.connect().await.map_err(operation)
}
async fn lookup<S: LedgerRecovery>(
    store: &S,
    mutation: &LedgerMutation,
) -> Result<LedgerLookup, Violation> {
    store
        .lookup_mutation(&mutation.intent.tenant_id, &mutation.intent.intent_id)
        .await
        .map_err(operation)
}
async fn original<S: LedgerRecovery>(
    store: &S,
    mutation: &LedgerMutation,
) -> Result<(), Violation> {
    require(
        "lookup.original",
        matches!(lookup(store, mutation).await?, LedgerLookup::Committed(record) if *record == *mutation),
        "lookup did not retain the exact original mutation",
    )
}
async fn claim(
    tx: &mut dyn LedgerTransaction,
    mutation: &LedgerMutation,
) -> Result<String, LedgerStoreError> {
    match tx
        .claim_idempotency(
            &mutation.intent.tenant_id,
            &mutation.intent,
            &mutation.payload_digest,
        )
        .await?
    {
        IdempotencyClaim::NewReservation { attempt_id } if !attempt_id.is_empty() => Ok(attempt_id),
        other => Err(LedgerStoreError::Idempotency(format!(
            "expected fresh reservation, got {other:?}"
        ))),
    }
}
fn step_count(mutation: &LedgerMutation) -> usize {
    3 + mutation.projections.len() + mutation.outbox.len()
}
async fn step(
    tx: &mut dyn LedgerTransaction,
    mutation: &LedgerMutation,
    attempt: &str,
    index: usize,
) -> Result<(), LedgerStoreError> {
    if index == 0 {
        return tx.append_events(&mutation.events).await;
    }
    if index == 1 {
        return tx.append_commit(&mutation.commit).await;
    }
    if let Some(projection) = index
        .checked_sub(2)
        .and_then(|i| mutation.projections.get(i))
    {
        return tx.upsert_projection(projection).await;
    }
    if let Some(outbox) = index
        .checked_sub(2 + mutation.projections.len())
        .and_then(|i| mutation.outbox.get(i))
    {
        return tx.enqueue_outbox(outbox).await;
    }
    tx.finalize_idempotency(
        &mutation.intent.tenant_id,
        &mutation.intent.intent_id,
        attempt,
        &mutation.commit.body.commit_id,
    )
    .await
}
/// Write one well-formed fixture directly through the port. Cryptographic/ingress
/// validation is a separate authority; this tests the adapter transaction itself.
pub async fn write<S: LedgerRecovery>(
    store: &S,
    mutation: &LedgerMutation,
) -> Result<(), LedgerStoreError> {
    let mut tx = store.begin(&mutation.intent.tenant_id).await?;
    let staged = async {
        let attempt = claim(&mut *tx, mutation).await?;
        for index in 0..step_count(mutation) {
            step(&mut *tx, mutation, &attempt, index).await?;
        }
        Ok::<_, LedgerStoreError>(())
    }
    .await;
    if let Err(error) = staged {
        tx.rollback().await?;
        return Err(error);
    }
    tx.commit().await
}
/// Staged events, commit, projections, outbox and finalization are never visible
/// as committed effects or receipts; successful commit publishes all together.
pub async fn check_atomic_visibility<B: LedgerTestBackend>(
    backend: &B,
    mutation: &LedgerMutation,
) -> Result<(), Violation> {
    let writer = fresh(backend).await?;
    let reader = backend.connect().await.map_err(operation)?;
    let mut tx = writer
        .begin(&mutation.intent.tenant_id)
        .await
        .map_err(operation)?;
    let attempt = claim(&mut *tx, mutation).await.map_err(operation)?;
    for index in 0..step_count(mutation) {
        step(&mut *tx, mutation, &attempt, index)
            .await
            .map_err(operation)?;
        compare(backend, &Snapshot::default(), "atomic.visibility").await?;
        require(
            "atomic.receipt",
            !matches!(lookup(&reader, mutation).await?, LedgerLookup::Committed(_)),
            "staged mutation exposed as committed",
        )?;
    }
    tx.commit().await.map_err(operation)?;
    compare(
        backend,
        &Snapshot::from_mutations(&[mutation]),
        "atomic.completeness",
    )
    .await?;
    original(&reader, mutation).await?;
    original(&backend.connect().await.map_err(operation)?, mutation).await
}
/// Roll back after every individual write boundary, then reclaim the same key.
pub async fn check_rollback<B: LedgerTestBackend>(
    backend: &B,
    mutation: &LedgerMutation,
) -> Result<(), Violation> {
    for prefix in 0..=step_count(mutation) {
        let writer = fresh(backend).await?;
        let mut tx = writer
            .begin(&mutation.intent.tenant_id)
            .await
            .map_err(operation)?;
        let attempt = claim(&mut *tx, mutation).await.map_err(operation)?;
        for index in 0..prefix {
            step(&mut *tx, mutation, &attempt, index)
                .await
                .map_err(operation)?;
        }
        tx.rollback().await.map_err(operation)?;
        compare(backend, &Snapshot::default(), "rollback.effects").await?;
        let reader = backend.connect().await.map_err(operation)?;
        require(
            "rollback.claim",
            matches!(lookup(&reader, mutation).await?, LedgerLookup::NotFound),
            "rolled-back reservation survived",
        )?;
        let mut retry = reader
            .begin(&mutation.intent.tenant_id)
            .await
            .map_err(operation)?;
        claim(&mut *retry, mutation).await.map_err(operation)?;
        retry.rollback().await.map_err(operation)?;
    }
    Ok(())
}
/// Exact duplicate must identify the original commit; conflicting payload must
/// refuse. Both checks use a new connection and preserve every original effect.
pub async fn check_replay<B: LedgerTestBackend>(
    backend: &B,
    mutation: &LedgerMutation,
) -> Result<(), Violation> {
    let store = fresh(backend).await?;
    write(&store, mutation).await.map_err(operation)?;
    let reader = backend.connect().await.map_err(operation)?;
    let mut retry = reader
        .begin(&mutation.intent.tenant_id)
        .await
        .map_err(operation)?;
    let result = retry
        .claim_idempotency(
            &mutation.intent.tenant_id,
            &mutation.intent,
            &mutation.payload_digest,
        )
        .await
        .map_err(operation)?;
    require(
        "replay.identity",
        result
            == IdempotencyClaim::Committed {
                commit_id: mutation.commit.body.commit_id.clone(),
            },
        "exact retry did not resolve the original commit",
    )?;
    retry.rollback().await.map_err(operation)?;
    let mut changed = mutation.intent.clone();
    changed.nonce = Nonce::from_bytes(vec![211, 17]).map_err(operation)?;
    let digest = canonicalize_and_digest(&changed).map_err(operation)?;
    require(
        "setup.conflict",
        digest != mutation.payload_digest,
        "conflict fixture has unchanged digest",
    )?;
    let mut conflict = reader
        .begin(&mutation.intent.tenant_id)
        .await
        .map_err(operation)?;
    let result = conflict
        .claim_idempotency(&mutation.intent.tenant_id, &changed, &digest)
        .await
        .map_err(operation)?;
    require(
        "replay.conflict",
        result == IdempotencyClaim::ConflictDifferentPayload,
        "different payload accepted under original key",
    )?;
    conflict.rollback().await.map_err(operation)?;
    compare(
        backend,
        &Snapshot::from_mutations(&[mutation]),
        "replay.effects",
    )
    .await?;
    original(&reader, mutation).await
}
/// Coincident textual intent/event/commit ids in two tenants remain independent.
pub async fn check_tenant_isolation<B: LedgerTestBackend>(
    backend: &B,
    first: &LedgerMutation,
    other: &LedgerMutation,
) -> Result<(), Violation> {
    require(
        "setup.tenants",
        first.intent.tenant_id != other.intent.tenant_id
            && first.intent.intent_id == other.intent.intent_id
            && first.commit.body.commit_id == other.commit.body.commit_id,
        "requires distinct tenants with coincident ids",
    )?;
    let store = fresh(backend).await?;
    write(&store, first).await.map_err(operation)?;
    write(&backend.connect().await.map_err(operation)?, other)
        .await
        .map_err(|error| failure("tenant.claim", error))?;
    compare(
        backend,
        &Snapshot::from_mutations(&[first, other]),
        "tenant.effects",
    )
    .await?;
    original(&store, first).await?;
    original(&store, other).await
}
/// Competing distinct identities prepared against one genesis head: exactly one
/// may commit, and every row must belong to that winner. Run with 2, 8 and 32.
pub async fn check_head_contention<B: LedgerTestBackend>(
    backend: &B,
    mutations: &[LedgerMutation],
) -> Result<(), Violation> {
    require(
        "setup.contenders",
        mutations.len() >= 2,
        "at least two contenders required",
    )?;
    let first = mutations
        .first()
        .ok_or_else(|| failure("setup.contenders", "empty contenders"))?;
    let mut identities = std::collections::BTreeSet::new();
    let mut commits = std::collections::BTreeSet::new();
    for mutation in mutations {
        require(
            "setup.contenders",
            mutation.intent.tenant_id == first.intent.tenant_id
                && mutation.commit.body.scope == first.commit.body.scope
                && mutation.commit.body.sequence == 1
                && mutation.commit.body.parent_commit_id.is_none()
                && mutation.commit.body.previous_state_root
                    == first.commit.body.previous_state_root
                && identities.insert(mutation.intent.intent_id.clone())
                && commits.insert(mutation.commit.body.commit_id.clone()),
            "requires distinct identities/commits against one tenant genesis head",
        )?;
    }
    let store = fresh(backend).await?;
    let mut jobs = tokio::task::JoinSet::new();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(mutations.len()));
    for (index, mutation) in mutations.iter().cloned().enumerate() {
        let connection = backend.connect().await.map_err(operation)?;
        let barrier = barrier.clone();
        jobs.spawn(async move {
            barrier.wait().await;
            (index, write(&connection, &mutation).await)
        });
    }
    let mut winners = Vec::new();
    while let Some(result) = jobs.join_next().await {
        let (index, result) = result.map_err(operation)?;
        match result {
            Ok(()) => winners.push(index),
            Err(LedgerStoreError::Conflict(_)) => {}
            Err(error) => return Err(operation(error)),
        }
    }
    require(
        "head.single_winner",
        winners.len() == 1,
        "canonical-head race did not yield exactly one winner",
    )?;
    let winner = &mutations[winners[0]];
    compare(
        backend,
        &Snapshot::from_mutations(&[winner]),
        "head.effects",
    )
    .await?;
    original(&store, winner).await
}

pub mod faults;

/// Unknown acknowledgement must resolve through a new connection to the exact
/// original record; exact retry must replay without appending another effect.
/// The supplied authorizer and signer verifier are independent fixture policy.
pub async fn check_lost_ack<B: LedgerTestBackend>(
    backend: &B,
    mutation: &LedgerMutation,
    principal: &statechronicle::ports::authorization::AuthenticatedPrincipal,
    authorizer: &dyn statechronicle::ports::authorization::Authorizer,
    verifier: &dyn statechronicle::commit::persist::SignedCommitVerifier,
) -> Result<(), Violation> {
    use statechronicle::commit::{
        durable::{DurableError, DurableOutcome, RecoveryOutcome, commit_verified, recover},
        persist::{CommittedEvent, DurableCommitRequest},
    };
    let store = fresh(backend).await?;
    let entries: Vec<_> = mutation
        .events
        .iter()
        .map(|event| CommittedEvent {
            event,
            state_type: event.after.state.state_type(),
        })
        .collect();
    let request = || DurableCommitRequest {
        principal,
        intent: &mutation.intent,
        payload_digest: &mutation.payload_digest,
        entries: &entries,
        commit: &mutation.commit,
        projections: &mutation.projections,
        outbox: &mutation.outbox,
    };
    let result = commit_verified(
        &faults::LoseCommitAck(store),
        authorizer,
        verifier,
        request(),
    )
    .await;
    require(
        "ack.unknown",
        matches!(result, Err(DurableError::Unknown { .. })),
        "lost acknowledgement did not produce Unknown",
    )?;
    let reader = backend.connect().await.map_err(operation)?;
    let recovered = recover(&reader, authorizer, verifier, principal, &mutation.intent)
        .await
        .map_err(operation)?;
    require(
        "ack.original",
        matches!(recovered, RecoveryOutcome::Committed(record) if record.record() == mutation),
        "recovery lost original effects",
    )?;
    let replay = commit_verified(&reader, authorizer, verifier, request())
        .await
        .map_err(operation)?;
    require(
        "ack.replay",
        matches!(replay, DurableOutcome::Replay(record) if record.record() == mutation),
        "exact retry did not return the original receipt",
    )?;
    compare(
        backend,
        &Snapshot::from_mutations(&[mutation]),
        "ack.effects",
    )
    .await
}

/// Finalization cannot use a different reservation token. Rollback must leave
/// the original identity retryable, without any committed effect.
pub async fn check_attempt_fencing<B: LedgerTestBackend>(
    backend: &B,
    mutation: &LedgerMutation,
) -> Result<(), Violation> {
    let store = fresh(backend).await?;
    let mut tx = store
        .begin(&mutation.intent.tenant_id)
        .await
        .map_err(operation)?;
    let attempt = claim(&mut *tx, mutation).await.map_err(operation)?;
    for index in 0..step_count(mutation) - 1 {
        step(&mut *tx, mutation, &attempt, index)
            .await
            .map_err(operation)?;
    }
    let result = tx
        .finalize_idempotency(
            &mutation.intent.tenant_id,
            &mutation.intent.intent_id,
            &(attempt + "-wrong"),
            &mutation.commit.body.commit_id,
        )
        .await;
    require(
        "fencing.attempt",
        result.is_err(),
        "wrong reservation token finalized",
    )?;
    tx.rollback().await.map_err(operation)?;
    compare(backend, &Snapshot::default(), "fencing.effects").await?;
    require(
        "fencing.claim",
        matches!(lookup(&store, mutation).await?, LedgerLookup::NotFound),
        "failed finalization left a durable claim",
    )
}

/// For an adapter that declares multi-tenant transactions unsupported, refusal
/// must occur before any effect. Supporting adapters need a separate atomic
/// multi-tenant scenario rather than opting into this rejection check.
pub async fn check_unsupported_multi<B: LedgerTestBackend>(
    backend: &B,
    tenants: &[TenantId],
) -> Result<(), Violation> {
    require(
        "setup.multi",
        tenants.len() >= 2 && tenants.windows(2).any(|pair| pair[0] != pair[1]),
        "distinct tenants required",
    )?;
    let store = fresh(backend).await?;
    match store.begin_multi(tenants).await {
        Err(LedgerStoreError::Invariant(_)) => {}
        Err(error) => return Err(operation(error)),
        Ok(tx) => {
            tx.rollback().await.map_err(operation)?;
            return Err(failure(
                "multi.scope",
                "unsupported multi-tenant boundary accepted",
            ));
        }
    }
    compare(backend, &Snapshot::default(), "multi.effects").await
}

/// Concurrent exact identities may be pending or replayed, but only one writer
/// owns a new reservation and commits. Once it finishes, every new reader must
/// resolve the complete original mutation and no effect may be duplicated.
pub async fn check_idempotency_contention<B: LedgerTestBackend>(
    backend: &B,
    mutation: &LedgerMutation,
    clients: usize,
) -> Result<(), Violation> {
    require(
        "setup.clients",
        clients >= 2,
        "at least two clients required",
    )?;
    let store = fresh(backend).await?;
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(clients));
    let mut jobs = tokio::task::JoinSet::new();
    for _ in 0..clients {
        let connection = backend.connect().await.map_err(operation)?;
        let mutation = mutation.clone();
        let barrier = barrier.clone();
        jobs.spawn(async move {
            barrier.wait().await;
            let mut tx = connection.begin(&mutation.intent.tenant_id).await?;
            let result = tx
                .claim_idempotency(
                    &mutation.intent.tenant_id,
                    &mutation.intent,
                    &mutation.payload_digest,
                )
                .await?;
            match result {
                IdempotencyClaim::NewReservation { attempt_id } => {
                    for index in 0..step_count(&mutation) {
                        if let Err(error) = step(&mut *tx, &mutation, &attempt_id, index).await {
                            tx.rollback().await?;
                            return Err(error);
                        }
                    }
                    tx.commit().await?;
                    Ok(true)
                }
                IdempotencyClaim::Committed { commit_id }
                    if commit_id == mutation.commit.body.commit_id =>
                {
                    tx.rollback().await?;
                    Ok(false)
                }
                IdempotencyClaim::InProgress { .. } => {
                    tx.rollback().await?;
                    Ok(false)
                }
                _ => {
                    tx.rollback().await?;
                    Err(LedgerStoreError::Idempotency(
                        "unexpected concurrent identity result".into(),
                    ))
                }
            }
        });
    }
    let mut winners = 0;
    while let Some(result) = jobs.join_next().await {
        winners += usize::from(result.map_err(operation)?.map_err(operation)?);
    }
    require(
        "identity.single_winner",
        winners == 1,
        "exact identity produced multiple or no committed writers",
    )?;
    compare(
        backend,
        &Snapshot::from_mutations(&[mutation]),
        "identity.effects",
    )
    .await?;
    original(&store, mutation).await?;
    original(&backend.connect().await.map_err(operation)?, mutation).await
}

/// Dropping a pre-commit transaction after each completed stage must not expose
/// effects or a committed receipt. An unfinished lease may remain InProgress;
/// this observation never grants permission to retry under a different identity.
pub async fn check_cancellation<B: LedgerTestBackend>(
    backend: &B,
    mutation: &LedgerMutation,
) -> Result<(), Violation> {
    for prefix in 0..=step_count(mutation) {
        let store = fresh(backend).await?;
        let mut tx = store
            .begin(&mutation.intent.tenant_id)
            .await
            .map_err(operation)?;
        let attempt = claim(&mut *tx, mutation).await.map_err(operation)?;
        for index in 0..prefix {
            step(&mut *tx, mutation, &attempt, index)
                .await
                .map_err(operation)?;
        }
        drop(tx);
        compare(backend, &Snapshot::default(), "cancellation.effects").await?;
        let reader = backend.connect().await.map_err(operation)?;
        require(
            "cancellation.receipt",
            !matches!(lookup(&reader, mutation).await?, LedgerLookup::Committed(_)),
            "cancelled pre-commit attempt returned a committed record",
        )?;
    }
    Ok(())
}

/// Optional complete-group adapter contract checks.
pub mod batch;

/// Optional test control for adapters that support expiring reservations.
/// Advance the adapter's injected clock past this reservation's lease and run
/// its normal expiry machinery. Do not delete or replace the reservation here:
/// the next ordinary claim must perform the real fenced takeover.
#[async_trait]
pub trait LedgerLeaseTestBackend: LedgerTestBackend {
    async fn expire_reservation(&self, tenant: &TenantId, intent: &IntentId) -> Result<(), String>;
}

/// A superseded writer cannot commit, roll back or drop its successor's claim.
/// Requires two simultaneously live independent write transactions. Run only
/// for adapters that declare expiring leases and provide a deterministic clock.
pub async fn check_lease_takeover<B: LedgerLeaseTestBackend>(
    backend: &B,
    mutation: &LedgerMutation,
) -> Result<(), Violation> {
    for cleanup in 0..3 {
        let store = fresh(backend).await?;
        let mut stale = store
            .begin(&mutation.intent.tenant_id)
            .await
            .map_err(operation)?;
        let old_attempt = claim(&mut *stale, mutation).await.map_err(operation)?;
        for index in 0..step_count(mutation) {
            step(&mut *stale, mutation, &old_attempt, index)
                .await
                .map_err(operation)?;
        }
        let observer = backend.connect().await.map_err(operation)?;
        let mut pending = observer
            .begin(&mutation.intent.tenant_id)
            .await
            .map_err(operation)?;
        let observed = pending
            .claim_idempotency(
                &mutation.intent.tenant_id,
                &mutation.intent,
                &mutation.payload_digest,
            )
            .await
            .map_err(operation)?;
        require(
            "lease.live",
            matches!(observed, IdempotencyClaim::InProgress { attempt_id, .. } if attempt_id == old_attempt),
            "unexpired reservation was not retained",
        )?;
        pending.rollback().await.map_err(operation)?;
        backend
            .expire_reservation(&mutation.intent.tenant_id, &mutation.intent.intent_id)
            .await
            .map_err(operation)?;
        let successor = backend.connect().await.map_err(operation)?;
        let mut winner = successor
            .begin(&mutation.intent.tenant_id)
            .await
            .map_err(operation)?;
        let attempt = claim(&mut *winner, mutation).await.map_err(operation)?;
        require(
            "lease.token",
            attempt != old_attempt,
            "takeover reused the stale attempt token",
        )?;
        for index in 0..step_count(mutation) {
            step(&mut *winner, mutation, &attempt, index)
                .await
                .map_err(operation)?;
        }
        match cleanup {
            0 => require(
                "lease.stale_commit",
                stale.commit().await.is_err(),
                "superseded writer committed",
            )?,
            1 => {
                stale.rollback().await.map_err(operation)?;
            }
            _ => drop(stale),
        }
        compare(backend, &Snapshot::default(), "lease.private").await?;
        let reader = backend.connect().await.map_err(operation)?;
        require(
            "lease.successor",
            matches!(lookup(&reader, mutation).await?, LedgerLookup::InProgress { attempt_id, payload_digest, .. }
                if attempt_id == attempt && payload_digest == mutation.payload_digest),
            "stale cleanup lost or changed the successor reservation",
        )?;
        winner.commit().await.map_err(operation)?;
        original(&reader, mutation).await?;
        compare(
            backend,
            &Snapshot::from_mutations(&[mutation]),
            "lease.effects",
        )
        .await?;
    }
    Ok(())
}
