//! Verified atomic batches with complete original-group recovery.
//!
//! This advanced boundary authenticates commit signer trust and authorizes each
//! member; it does not authenticate ingress signatures or run domain reduction.
//! Prefer the protected facade that binds an owned prepared batch before signing.

use crate::{
    durable::{DurableError, MutationIdentity, read_error, transaction_error},
    error::CommitError,
    persist::{
        CommittedEvent, SignedCommitVerifier, TransactionFailure, TransactionPhase, abort,
        authorize_intent, projections_for, rollback_only, validate_durable_intent,
        validate_effects_with_binding, validate_event_binding, validate_tenant_commit,
    },
};
use statechronicle_core::{
    canonicalize::canonicalize,
    digest::{ContentDigest, hash_bytes},
    limits::{MAX_EVENT_BATCH_BYTES, MAX_EVENT_BYTES, MAX_EVENTS_PER_COMMIT, check_size},
};
use statechronicle_domain::{
    commit::Commit, event::Event, ids::CommitId, intent::Intent, signed::Signed,
    state::StateProjection,
};
use statechronicle_ports::{
    authorization::{AuthenticatedPrincipal, Authorizer},
    ledger_batch::{LedgerBatch, LedgerBatchLookup, LedgerBatchRecovery},
    ledger_store::{IdempotencyClaim, LedgerStoreError, OutboxRecord},
};
use std::collections::BTreeSet;

/// Ordered immutable identities of every original member. The digest is domain
/// separated over the complete ordered intent bodies, not merely intent IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchIdentity {
    members: Vec<MutationIdentity>,
    payload_digest: ContentDigest,
}
impl BatchIdentity {
    /// Complete ordered original retry identities. Never replace a subset after
    /// an unknown acknowledgement.
    pub fn members(&self) -> &[MutationIdentity] {
        &self.members
    }
    /// Domain-separated digest of the entire ordered request set.
    pub const fn payload_digest(&self) -> &ContentDigest {
        &self.payload_digest
    }
}

/// Refusal or uncertainty, retaining the complete group identity once validated.
/// `source` preserves the existing durable error classifications; an `Unknown`
/// source always has a complete group identity, not just its anchor member.
#[derive(Debug, thiserror::Error)]
#[error("durable batch: {source}")]
pub struct DurableBatchError {
    identity: Option<BatchIdentity>,
    #[source]
    source: DurableError,
}
impl DurableBatchError {
    /// Complete validated group identity, if preflight reached that stage.
    pub const fn identity(&self) -> Option<&BatchIdentity> {
        self.identity.as_ref()
    }
    /// Typed refusal or uncertainty; any contained single identity is only member context.
    pub const fn cause(&self) -> &DurableError {
        &self.source
    }
}
impl From<DurableError> for DurableBatchError {
    fn from(source: DurableError) -> Self {
        Self {
            identity: None,
            source,
        }
    }
}
fn with_identity(identity: &BatchIdentity, source: DurableError) -> DurableBatchError {
    DurableBatchError {
        identity: Some(identity.clone()),
        source,
    }
}

/// Independently authenticated request member at this advanced boundary.
pub struct BatchMember<'request> {
    /// Principal bound to this intent's actor and tenant.
    pub principal: &'request AuthenticatedPrincipal,
    /// Complete original intent body.
    pub intent: &'request Intent,
    /// Canonical body digest, independently recomputed before any transaction.
    pub payload_digest: &'request ContentDigest,
}

/// Complete advanced write request for one signed, single-tenant batch.
pub struct DurableBatchRequest<'request> {
    /// Ordered original requests; each must contribute at least one event.
    pub members: &'request [BatchMember<'request>],
    /// Complete signed event set, in commit order.
    pub entries: &'request [CommittedEvent<'request>],
    /// One signed tenant commit binding every event.
    pub commit: &'request Signed<Commit>,
    /// Complete deterministic projection set, required even when empty lists
    /// were permitted by legacy single-mutation adapters.
    pub projections: &'request [StateProjection],
    /// Complete original notification set.
    pub outbox: &'request [OutboxRecord],
}

/// Verified complete original group; private construction and borrowed access
/// prevent synthesizing receipts for independently committed subsets.
///
/// ```compile_fail
/// use statechronicle_commit::durable_batch::CommittedBatch;
/// let forged = CommittedBatch { identity: todo!(), record: todo!() };
/// ```
#[derive(Debug)]
pub struct CommittedBatch {
    identity: BatchIdentity,
    record: Box<LedgerBatch>,
}
impl CommittedBatch {
    /// Exact original group identity.
    pub const fn identity(&self) -> &BatchIdentity {
        &self.identity
    }
    /// Complete original immutable effects, never today's mutable projections.
    pub const fn record(&self) -> &LedgerBatch {
        &self.record
    }
}

/// Confirmed original atomic group, never a successful subset.
#[derive(Debug)]
pub enum DurableBatchOutcome {
    /// A new group committed and its complete original evidence was checked.
    Committed(CommittedBatch),
    /// The exact original group was already committed and checked.
    Replay(CommittedBatch),
}
/// Point-in-time recovery result; no variant creates a new reservation.
#[derive(Debug)]
pub enum BatchRecoveryOutcome {
    /// No visible anchor member; an outstanding transaction may still commit.
    NotFound(BatchIdentity),
    /// A matching member is reserved; this is not a committed group.
    InProgress {
        /// Complete original group identity to retain.
        identity: BatchIdentity,
        /// Adapter's fenced attempt identifier.
        attempt_id: String,
        /// Lease expiry does not establish rollback.
        lease_expires_at_unix: i64,
    },
    /// Complete original group, checked without reducing or signing again.
    Committed(CommittedBatch),
}

fn invalid(message: &str) -> DurableError {
    DurableError::Rejected(CommitError::InvalidEvent(String::from(message)))
}
fn group_identity(intents: &[&Intent]) -> Result<BatchIdentity, DurableError> {
    let first = intents
        .first()
        .ok_or_else(|| invalid("empty durable batch"))?;
    check_size("batch_intents", MAX_EVENTS_PER_COMMIT, intents.len())
        .map_err(|e| DurableError::Rejected(CommitError::Core(e)))?;
    let mut seen = BTreeSet::new();
    let mut members = Vec::with_capacity(intents.len());
    for intent in intents {
        validate_durable_intent(intent).map_err(DurableError::Rejected)?;
        if intent.tenant_id != first.tenant_id || !seen.insert(&intent.intent_id) {
            return Err(invalid(
                "duplicate intent identity or mixed tenant in durable batch",
            ));
        }
        members.push(MutationIdentity::from_intent(intent).map_err(DurableError::Rejected)?);
    }
    let input = ("statechronicle.atomic-batch.v0", intents);
    let bytes = canonicalize(&input).map_err(|e| DurableError::Rejected(CommitError::Core(e)))?;
    check_size("batch_intent_bytes", MAX_EVENT_BATCH_BYTES, bytes.len())
        .map_err(|e| DurableError::Rejected(CommitError::Core(e)))?;
    let payload_digest = hash_bytes(&bytes);
    Ok(BatchIdentity {
        members,
        payload_digest,
    })
}
async fn authorize_members(
    authorizer: &dyn Authorizer,
    members: &[BatchMember<'_>],
    identity: &BatchIdentity,
) -> Result<(), DurableBatchError> {
    for (member, expected) in members.iter().zip(identity.members()) {
        if member.payload_digest != expected.payload_digest() {
            return Err(with_identity(
                identity,
                DurableError::Rejected(CommitError::PayloadDigestMismatch),
            ));
        }
        authorize_intent(
            authorizer,
            member.principal,
            member.intent,
            expected.tenant(),
        )
        .await
        .map_err(|error| with_identity(identity, DurableError::Authorization(error)))?;
    }
    Ok(())
}

fn group_effects(
    intents: &[&Intent],
    entries: &[CommittedEvent<'_>],
    commit: &Signed<Commit>,
    projections: &[StateProjection],
    outbox: &[OutboxRecord],
) -> Result<Vec<Event>, CommitError> {
    let first = intents
        .first()
        .ok_or_else(|| CommitError::InvalidEvent(String::from("empty durable batch")))?;
    check_size("events", MAX_EVENTS_PER_COMMIT, entries.len()).map_err(CommitError::Core)?;
    for entry in entries {
        let bytes = canonicalize(entry.event).map_err(CommitError::Core)?;
        check_size("event", MAX_EVENT_BYTES, bytes.len()).map_err(CommitError::Core)?;
    }

    let tenant = validate_tenant_commit(&commit.body)?;
    if *tenant != first.tenant_id {
        return Err(CommitError::MixedTenant);
    }
    let mut covered = BTreeSet::new();
    let events =
        validate_effects_with_binding(entries, commit, projections, outbox, tenant, |event| {
            let intent = intents
                .iter()
                .find(|intent| intent.intent_id == event.intent_id)
                .ok_or_else(|| {
                    CommitError::InvalidEvent(String::from("event has no batch member"))
                })?;
            validate_event_binding(event, intent, tenant)
        })?;
    for event in &events {
        covered.insert(&event.intent_id);
    }
    if covered.len() != intents.len() {
        return Err(CommitError::InvalidEvent(String::from(
            "batch member has no committed event",
        )));
    }
    if projections != projections_for(&commit.body.commit_id, entries) {
        return Err(CommitError::InvalidEvent(String::from(
            "batch requires complete deterministic projections",
        )));
    }
    Ok(events)
}

fn verify_record(
    record: Box<LedgerBatch>,
    identity: &BatchIdentity,
    intents: &[&Intent],
    verifier: &dyn SignedCommitVerifier,
    expected_commit: Option<&CommitId>,
) -> Result<CommittedBatch, DurableError> {
    let original: Vec<_> = record.intents.iter().collect();
    let original_identity =
        group_identity(&original).map_err(|e| DurableError::Corrupt(e.to_string()))?;
    let anchor = identity
        .members
        .first()
        .ok_or_else(|| DurableError::Corrupt(String::from("missing batch anchor")))?;
    if !original_identity.members.iter().any(|member| {
        member.tenant() == anchor.tenant() && member.intent_id() == anchor.intent_id()
    }) {
        return Err(DurableError::Corrupt(String::from(
            "lookup returned another batch member or tenant",
        )));
    }
    if expected_commit.is_some_and(|id| *id != record.commit.body.commit_id) {
        return Err(DurableError::Corrupt(String::from(
            "batch claim and original commit disagree",
        )));
    }
    verifier
        .verify(&record.commit)
        .map_err(DurableError::Corrupt)?;
    let entries: Vec<_> = record
        .events
        .iter()
        .map(|event| CommittedEvent {
            event,
            state_type: event.after.state.state_type(),
        })
        .collect();
    group_effects(
        &original,
        &entries,
        &record.commit,
        &record.projections,
        &record.outbox,
    )
    .map_err(|e| DurableError::Corrupt(e.to_string()))?;
    if original.as_slice() != intents || original_identity != *identity {
        return Err(DurableError::PayloadConflict);
    }
    Ok(CommittedBatch {
        identity: identity.clone(),
        record,
    })
}

async fn lookup<S: LedgerBatchRecovery + ?Sized>(
    ledger: &S,
    verifier: &dyn SignedCommitVerifier,
    intents: &[&Intent],
    identity: &BatchIdentity,
    expected_commit: Option<&CommitId>,
) -> Result<BatchRecoveryOutcome, DurableError> {
    let anchor = identity
        .members
        .first()
        .ok_or_else(|| invalid("empty durable batch"))?;
    match ledger
        .lookup_batch(anchor.tenant(), anchor.intent_id())
        .await
        .map_err(read_error)?
    {
        LedgerBatchLookup::NotFound => Ok(BatchRecoveryOutcome::NotFound(identity.clone())),
        LedgerBatchLookup::InProgress {
            payload_digest,
            attempt_id,
            lease_expires_at_unix,
        } => {
            if payload_digest != *anchor.payload_digest() {
                return Err(DurableError::PayloadConflict);
            }
            Ok(BatchRecoveryOutcome::InProgress {
                identity: identity.clone(),
                attempt_id,
                lease_expires_at_unix,
            })
        }
        LedgerBatchLookup::Committed(record) => {
            let receipt = verify_record(record, identity, intents, verifier, expected_commit)?;
            // Immutable finalized records permit separate linearizable reads:
            // every member must resolve to exactly the same complete group.
            for member in identity.members.iter().skip(1) {
                match ledger
                    .lookup_batch(member.tenant(), member.intent_id())
                    .await
                    .map_err(read_error)?
                {
                    LedgerBatchLookup::Committed(other) if *other == *receipt.record => {}
                    LedgerBatchLookup::NotFound
                    | LedgerBatchLookup::InProgress { .. }
                    | LedgerBatchLookup::Committed(_) => {
                        return Err(DurableError::Corrupt(String::from(
                            "batch members do not resolve to one complete original group",
                        )));
                    }
                }
            }
            Ok(BatchRecoveryOutcome::Committed(receipt))
        }
    }
}

/// Reauthorize every independently supplied original member and recover the
/// entire group without reduction, signing, claims or effect dispatch.
/// # Errors
/// Returns typed refusal, conflicting membership, corruption or unavailable
/// evidence. A failed read never resolves a prior unknown as safely failed.
pub async fn recover_batch<S: LedgerBatchRecovery + ?Sized>(
    ledger: &S,
    authorizer: &dyn Authorizer,
    verifier: &dyn SignedCommitVerifier,
    members: &[BatchMember<'_>],
) -> Result<BatchRecoveryOutcome, DurableBatchError> {
    let intents: Vec<_> = members.iter().map(|member| member.intent).collect();
    let identity = group_identity(&intents)?;
    authorize_members(authorizer, members, &identity).await?;
    lookup(ledger, verifier, &intents, &identity, None)
        .await
        .map_err(|e| with_identity(&identity, e))
}

enum BatchDecision {
    Committed,
    Replay(CommitId),
    Conflict,
    InProgress {
        member: MutationIdentity,
        attempt_id: String,
        lease_expires_at_unix: i64,
    },
}

async fn write<S: LedgerBatchRecovery + ?Sized>(
    ledger: &S,
    request: &DurableBatchRequest<'_>,
    events: &[Event],
    identity: &BatchIdentity,
) -> Result<BatchDecision, TransactionFailure> {
    let tenant = identity
        .members
        .first()
        .ok_or_else(|| TransactionFailure {
            source: LedgerStoreError::Invariant(String::from("empty durable batch")),
            phase: TransactionPhase::Begin,
            rollback_error: None,
        })?
        .tenant();
    let mut tx = ledger
        .begin(tenant)
        .await
        .map_err(|source| TransactionFailure {
            source,
            phase: TransactionPhase::Begin,
            rollback_error: None,
        })?;
    let mut attempts = Vec::new();
    let mut replay = None;
    let mut ordered: Vec<_> = request
        .members
        .iter()
        .zip(identity.members.iter())
        .collect();
    ordered.sort_by(|left, right| left.0.intent.intent_id.cmp(&right.0.intent.intent_id));
    for (member, member_identity) in ordered {
        let claim = match tx
            .claim_idempotency(tenant, member.intent, member.payload_digest)
            .await
        {
            Ok(claim) => claim,
            Err(error) => return Err(abort(tx, error).await),
        };
        match claim {
            IdempotencyClaim::NewReservation { attempt_id } => {
                if replay.is_some() {
                    rollback_only(tx).await?;
                    return Ok(BatchDecision::Conflict);
                }
                attempts.push((member.intent.intent_id.clone(), attempt_id));
            }
            IdempotencyClaim::Committed { commit_id } => {
                if !attempts.is_empty() || replay.as_ref().is_some_and(|id| *id != commit_id) {
                    rollback_only(tx).await?;
                    return Ok(BatchDecision::Conflict);
                }
                replay = Some(commit_id);
            }
            IdempotencyClaim::ConflictDifferentPayload => {
                rollback_only(tx).await?;
                return Ok(BatchDecision::Conflict);
            }
            IdempotencyClaim::InProgress {
                attempt_id,
                lease_expires_at_unix,
            } => {
                rollback_only(tx).await?;
                return Ok(BatchDecision::InProgress {
                    member: member_identity.clone(),
                    attempt_id,
                    lease_expires_at_unix,
                });
            }
        }
    }
    if let Some(commit_id) = replay {
        rollback_only(tx).await?;
        return Ok(BatchDecision::Replay(commit_id));
    }
    let record = LedgerBatch {
        intents: request
            .members
            .iter()
            .map(|member| member.intent.clone())
            .collect(),
        commit: request.commit.clone(),
        events: events.to_vec(),
        projections: request.projections.to_vec(),
        outbox: request.outbox.to_vec(),
    };
    if let Err(error) = tx.append_events(events).await {
        return Err(abort(tx, error).await);
    }
    if let Err(error) = tx.append_commit(request.commit).await {
        return Err(abort(tx, error).await);
    }
    for projection in request.projections {
        if let Err(error) = tx.upsert_projection(projection).await {
            return Err(abort(tx, error).await);
        }
    }
    for outbox in request.outbox {
        if let Err(error) = tx.enqueue_outbox(outbox).await {
            return Err(abort(tx, error).await);
        }
    }
    if let Err(error) = tx.retain_batch(&record).await {
        return Err(abort(tx, error).await);
    }
    for (intent_id, attempt_id) in attempts {
        if let Err(error) = tx
            .finalize_idempotency(
                tenant,
                &intent_id,
                &attempt_id,
                &request.commit.body.commit_id,
            )
            .await
        {
            return Err(abort(tx, error).await);
        }
    }
    // Retained identity is checked during preflight; no member can be omitted.
    debug_assert_eq!(record.intents.len(), identity.members.len());
    tx.commit().await.map_err(|source| TransactionFailure {
        source,
        phase: TransactionPhase::Commit,
        rollback_error: None,
    })?;
    Ok(BatchDecision::Committed)
}

/// Verify, reauthorize and write every member through ONE ledger transaction,
/// then require complete immutable readback through every member identity.
/// # Errors
/// Every final-commit error, rollback failure or uncertain finalized readback
/// is `Unknown`, retaining the entire group identity. Replay verifies the
/// original group, never the caller's newly proposed events or commit.
pub async fn commit_batch_verified<S: LedgerBatchRecovery + ?Sized>(
    ledger: &S,
    authorizer: &dyn Authorizer,
    verifier: &dyn SignedCommitVerifier,
    request: DurableBatchRequest<'_>,
) -> Result<DurableBatchOutcome, DurableBatchError> {
    let intents: Vec<_> = request.members.iter().map(|member| member.intent).collect();
    let identity = group_identity(&intents)?;
    let anchor = identity
        .members
        .first()
        .ok_or_else(|| invalid("empty durable batch"))?
        .clone();
    verifier
        .verify(request.commit)
        .map_err(|e| with_identity(&identity, DurableError::Rejected(CommitError::Store(e))))?;
    authorize_members(authorizer, request.members, &identity).await?;
    let events = group_effects(
        &intents,
        request.entries,
        request.commit,
        request.projections,
        request.outbox,
    )
    .map_err(|e| with_identity(&identity, DurableError::Rejected(e)))?;
    let decision = write(ledger, &request, &events, &identity)
        .await
        .map_err(|failure| with_identity(&identity, transaction_error(anchor.clone(), failure)))?;
    let (committed, original_id) = match decision {
        BatchDecision::Committed => (true, request.commit.body.commit_id.clone()),
        BatchDecision::Replay(id) => (false, id),
        BatchDecision::Conflict => {
            return Err(with_identity(&identity, DurableError::PayloadConflict));
        }
        BatchDecision::InProgress {
            member,
            attempt_id,
            lease_expires_at_unix,
        } => {
            return Err(with_identity(
                &identity,
                DurableError::InProgress {
                    identity: member,
                    attempt_id,
                    lease_expires_at_unix,
                },
            ));
        }
    };
    let result = lookup(ledger, verifier, &intents, &identity, Some(&original_id)).await;
    let failure = match result {
        Ok(BatchRecoveryOutcome::Committed(receipt)) => {
            if !committed
                || (receipt.record.commit == *request.commit
                    && receipt.record.events == events
                    && receipt.record.projections == request.projections
                    && receipt.record.outbox == request.outbox)
            {
                return Ok(if committed {
                    DurableBatchOutcome::Committed(receipt)
                } else {
                    DurableBatchOutcome::Replay(receipt)
                });
            }
            LedgerStoreError::Invariant(String::from(
                "batch readback differs from acknowledged effects",
            ))
        }
        Err(DurableError::PayloadConflict) if !committed => {
            return Err(with_identity(&identity, DurableError::PayloadConflict));
        }
        Err(DurableError::Unavailable(source) | DurableError::Conflict(source)) => source,
        Err(error) => LedgerStoreError::Invariant(error.to_string()),
        Ok(_) => LedgerStoreError::Invariant(String::from(
            "finalized batch missing from authoritative readback",
        )),
    };
    Err(with_identity(
        &identity,
        DurableError::Unknown {
            identity: anchor,
            source: failure,
            rollback_error: None,
        },
    ))
}
