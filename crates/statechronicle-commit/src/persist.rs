//! Commit persistence (protocol §18.1 step 15).
//!
//! [`persist`] writes a signed commit and its events to the stores and
//! publishes them. Writes are validated fail-closed against the commit body
//! before anything is persisted: the supplied events must match the declared
//! `event_count` and recompute the declared `event_merkle_root`, otherwise
//! nothing is written.
//!
//! The v0 [`StateIndex`](statechronicle_ports::state_index::StateIndex) port is read-only (§27): it exposes `get_state` and
//! `get_subject_state` but no write operation. [`persist`] therefore derives
//! the current-state projections via [`projections_for`](crate::persist::projections_for) and leaves applying
//! them to the composition root's index adapter (e.g. inside its
//! `TransactionManager`), the documented integration point for projection
//! writes in this workspace.
//!
//! The `Commit` body carries only `event_count` and `event_merkle_root`
//! (protocol §13.1), not the events themselves, so [`persist`] takes the
//! committed events (with their state types) from the caller: the executor
//! that assembled the batch.

#![allow(clippy::let_underscore_must_use)]

use std::collections::BTreeSet;

use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::hash_bytes;
use statechronicle_core::limits::{
    MAX_COMMIT_BYTES, MAX_EVENT_BATCH_BYTES, MAX_EVENTS_PER_COMMIT, MAX_ID_LENGTH,
    MAX_OUTBOX_PAYLOAD_BYTES, MAX_QUOTA_KEY_BYTES, check_size,
};
use statechronicle_domain::commit::{COMMIT_SCHEMA, Commit, ProfileId, ScopeKind};
use statechronicle_domain::event::{EVENT_SCHEMA, Event};
use statechronicle_domain::ids::{CommitId, EventId, IntentId};
use statechronicle_domain::resource_state::ResourceState;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::tenant::TenantId;
use statechronicle_intent::validated::ValidatedIntent;

use statechronicle_ports::authorization::{
    AuthenticatedPrincipal, AuthorizationContext, AuthorizationError, Authorizer,
};
use statechronicle_ports::commit_store::CommitStore;
use statechronicle_ports::event_publisher::EventPublisher;
use statechronicle_ports::event_store::EventStore;
use statechronicle_ports::ledger_store::{
    IdempotencyClaim, LedgerStore, LedgerStoreError, LedgerTransaction, OutboxRecord,
};
use statechronicle_ports::observability::{MetricsSink, MutationMetric, MutationOutcome};
use statechronicle_ports::state_index::StateIndex;

use crate::error::CommitError;
use crate::roots::event_root;

/// Backend-agnostic store set used by [`persist`].
///
/// Holds the driven ports as `Send + Sync` trait objects so the persistence
/// path composes in any runtime. Kept distinct from the executor crate's own
/// ports struct to avoid coupling the two lanes.
pub struct CommitPorts {
    /// Append-only commit store.
    pub commit_store: Box<dyn CommitStore>,
    /// Append-only event store.
    pub event_store: Box<dyn EventStore>,
    /// Read-only current-state index (§27).
    pub state_index: Box<dyn StateIndex>,
    /// Optional event/commit publisher, used when present.
    pub event_publisher: Option<Box<dyn EventPublisher>>,
}

/// A committed event paired with the state type that shaped its projection.
#[derive(Debug, Clone, Copy)]
pub struct CommittedEvent<'event> {
    /// The committed event.
    pub event: &'event Event,
    /// The state type of the event's resource, needed to derive the index
    /// projection (the event body itself does not carry `StateType`).
    pub state_type: StateType,
}

/// Result of a durable persistence attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurablePersistResult {
    /// A new mutation was committed.
    Committed { commit_id: CommitId },
    /// The idempotency key already referred to this committed mutation.
    Replay { commit_id: CommitId },
}

/// Input to [`persist_durable_verified`].
pub struct DurableCommitRequest<'request> {
    /// Authenticated principal used for the mandatory actor binding check.
    pub principal: &'request AuthenticatedPrincipal,
    /// Canonical validated intent that owns the mutation.
    pub intent: &'request statechronicle_domain::intent::Intent,
    /// Digest of the canonical intent/payload.
    pub payload_digest: &'request statechronicle_core::digest::ContentDigest,
    /// Events covered by the signed commit.
    pub entries: &'request [CommittedEvent<'request>],
    /// Signed tenant commit to append.
    pub commit: &'request Signed<Commit>,
    /// Deterministic projections derived from `entries`.
    pub projections: &'request [StateProjection],
    /// Outbox rows to insert before commit.
    pub outbox: &'request [OutboxRecord],
}

/// Cryptographic verifier required by the verified durable-ingress helper.
/// Implementations should resolve the commit key through the deployment's
/// tenant-scoped registry/KMS before checking the detached signature.
pub trait SignedCommitVerifier: Send + Sync {
    /// Verifies the signature and trust policy for one prepared commit.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is unknown, revoked, out of scope, or the
    /// detached signature does not verify.
    fn verify(&self, commit: &Signed<Commit>) -> Result<(), String>;
}

/// Internal implementation for the verified durable write path.
///
/// External callers must use [`persist_durable_verified`] (or its metrics
/// variant). Keeping the signature/trust check at the public boundary prevents
/// a composition root from accidentally persisting a cryptographically
/// unverified commit.
///
/// # Errors
///
/// Returns [`CommitError`] when authentication, event-root validation,
/// idempotency, any durable write, or the final transaction commit fails.
pub(crate) async fn persist_durable(
    ledger: &dyn LedgerStore,
    authorizer: &dyn Authorizer,
    request: DurableCommitRequest<'_>,
) -> Result<DurablePersistResult, CommitError> {
    let (tenant, events) = validate_request(authorizer, &request)
        .await
        .map_err(RequestFailure::into_legacy)?;
    match write_transaction(ledger, &request, &tenant, &events)
        .await
        .map_err(|error| CommitError::Store(error.source.to_string()))?
    {
        TransactionDecision::Committed => Ok(DurablePersistResult::Committed {
            commit_id: request.commit.body.commit_id.clone(),
        }),
        TransactionDecision::Replay(commit_id) => Ok(DurablePersistResult::Replay { commit_id }),
        TransactionDecision::InProgress { .. } => Err(CommitError::Store(String::from(
            "idempotency reservation in progress",
        ))),
        TransactionDecision::Conflict => Err(CommitError::Store(String::from(
            "idempotency payload conflict",
        ))),
    }
}

pub(crate) enum RequestFailure {
    Invalid(CommitError),
    Authorization(AuthorizationError),
}

impl From<CommitError> for RequestFailure {
    fn from(error: CommitError) -> Self {
        Self::Invalid(error)
    }
}

impl From<AuthorizationError> for RequestFailure {
    fn from(error: AuthorizationError) -> Self {
        Self::Authorization(error)
    }
}

impl RequestFailure {
    fn into_legacy(self) -> CommitError {
        match self {
            Self::Invalid(error) => error,
            Self::Authorization(error) => CommitError::Store(error.to_string()),
        }
    }
}

/// One preflight owner for legacy writes and verified recovery records.
pub(crate) async fn validate_request(
    authorizer: &dyn Authorizer,
    request: &DurableCommitRequest<'_>,
) -> Result<(TenantId, Vec<Event>), RequestFailure> {
    validate_durable_intent(request.intent)?;
    let tenant = validate_tenant_commit(&request.commit.body)?;
    let canonical_digest = canonicalize_and_digest(request.intent).map_err(CommitError::Core)?;
    if canonical_digest != *request.payload_digest {
        return Err(CommitError::PayloadDigestMismatch.into());
    }
    authorize_intent(authorizer, request.principal, request.intent, tenant).await?;
    Ok((tenant.clone(), validate_effects(request, tenant)?))
}

pub(crate) async fn authorize_intent(
    authorizer: &dyn Authorizer,
    principal: &AuthenticatedPrincipal,
    intent: &statechronicle_domain::intent::Intent,
    tenant: &TenantId,
) -> Result<(), AuthorizationError> {
    validate_durable_scope(intent, principal, tenant)?;
    authorizer
        .authorize(AuthorizationContext {
            principal,
            tenant,
            intent,
            resource: &intent.resource_id,
        })
        .await
}

pub(crate) fn validate_effects(
    request: &DurableCommitRequest<'_>,
    tenant: &TenantId,
) -> Result<Vec<Event>, CommitError> {
    validate_effects_with_binding(
        request.entries,
        request.commit,
        request.projections,
        request.outbox,
        tenant,
        |event| validate_event_binding(event, request.intent, tenant),
    )
}

/// Shared complete effect validation; the caller owns single/group membership.
pub(crate) fn validate_effects_with_binding(
    entries: &[CommittedEvent<'_>],
    commit: &Signed<Commit>,
    projections: &[StateProjection],
    outbox_records: &[OutboxRecord],
    tenant: &TenantId,
    bind: impl Fn(&Event) -> Result<(), CommitError>,
) -> Result<Vec<Event>, CommitError> {
    let events: Vec<Event> = entries.iter().map(|entry| entry.event.clone()).collect();
    if events.is_empty() {
        return Err(CommitError::InvalidEvent(String::from(
            "durable commit must contain at least one event",
        )));
    }
    if events.len() > MAX_EVENTS_PER_COMMIT {
        return Err(CommitError::InvalidEvent(format!(
            "event count {} exceeds durable limit {MAX_EVENTS_PER_COMMIT}",
            events.len()
        )));
    }
    let event_bytes = events
        .iter()
        .map(|event| bcs::to_bytes(event).map(|bytes| bytes.len()))
        .try_fold(0usize, |total, bytes| {
            bytes.map(|size| total.saturating_add(size))
        })
        .map_err(|error| CommitError::InvalidEvent(error.to_string()))?;
    check_size("event_batch", MAX_EVENT_BATCH_BYTES, event_bytes)
        .map_err(|error| CommitError::InvalidEvent(error.to_string()))?;
    let commit_bytes =
        bcs::to_bytes(commit).map_err(|error| CommitError::InvalidEvent(error.to_string()))?;
    check_size("commit", MAX_COMMIT_BYTES, commit_bytes.len())
        .map_err(|error| CommitError::InvalidEvent(error.to_string()))?;
    validate_committed_events(entries)?;
    for event in &events {
        bind(event)?;
    }
    validate_projection_bindings(&commit.body.commit_id, entries, projections)?;
    for projection in projections {
        if projection.tenant_id != *tenant || projection.last_commit_id != commit.body.commit_id {
            return Err(CommitError::Store(String::from(
                "durable projection is not bound to the commit scope",
            )));
        }
    }
    validate_outbox_records(outbox_records)?;
    for outbox in outbox_records {
        if outbox.tenant != *tenant || outbox.commit_id != commit.body.commit_id {
            return Err(CommitError::Store(String::from(
                "durable outbox record is not bound to the commit scope",
            )));
        }
        validate_outbox_record(outbox)?;
    }
    let event_count = u64::try_from(events.len())
        .map_err(|error| CommitError::InvalidEvent(format!("event count overflow: {error}")))?;
    if event_count != commit.body.event_count
        || event_root(&events)? != commit.body.event_merkle_root
    {
        return Err(CommitError::EventRootMismatch);
    }

    Ok(events)
}

pub(crate) enum TransactionDecision {
    Committed,
    Replay(CommitId),
    InProgress {
        attempt_id: String,
        lease_expires_at_unix: i64,
    },
    Conflict,
}

#[derive(Clone, Copy)]
pub(crate) enum TransactionPhase {
    Begin,
    BeforeCommit,
    Commit,
    Rollback,
}

pub(crate) struct TransactionFailure {
    pub(crate) source: LedgerStoreError,
    pub(crate) phase: TransactionPhase,
    pub(crate) rollback_error: Option<LedgerStoreError>,
}

pub(crate) async fn abort(
    transaction: Box<dyn LedgerTransaction>,
    source: LedgerStoreError,
) -> TransactionFailure {
    TransactionFailure {
        source,
        phase: TransactionPhase::BeforeCommit,
        rollback_error: transaction.rollback().await.err(),
    }
}

pub(crate) async fn rollback_only(
    transaction: Box<dyn LedgerTransaction>,
) -> Result<(), TransactionFailure> {
    transaction
        .rollback()
        .await
        .map_err(|source| TransactionFailure {
            source,
            phase: TransactionPhase::Rollback,
            rollback_error: None,
        })
}

/// One atomic write owner. Preserve stage and rollback uncertainty for the new
/// surface; legacy callers intentionally retain their original error mapping.
pub(crate) async fn write_transaction<S: LedgerStore + ?Sized>(
    ledger: &S,
    request: &DurableCommitRequest<'_>,
    tenant: &TenantId,
    events: &[Event],
) -> Result<TransactionDecision, TransactionFailure> {
    let mut transaction = ledger
        .begin(tenant)
        .await
        .map_err(|source| TransactionFailure {
            source,
            phase: TransactionPhase::Begin,
            rollback_error: None,
        })?;
    let claim = match transaction
        .claim_idempotency(tenant, request.intent, request.payload_digest)
        .await
    {
        Ok(claim) => claim,
        Err(error) => return Err(abort(transaction, error).await),
    };
    let attempt_id = match claim {
        IdempotencyClaim::NewReservation { attempt_id } => attempt_id,
        IdempotencyClaim::Committed { commit_id } => {
            rollback_only(transaction).await?;
            return Ok(TransactionDecision::Replay(commit_id));
        }
        IdempotencyClaim::InProgress {
            attempt_id,
            lease_expires_at_unix,
        } => {
            rollback_only(transaction).await?;
            return Ok(TransactionDecision::InProgress {
                attempt_id,
                lease_expires_at_unix,
            });
        }
        IdempotencyClaim::ConflictDifferentPayload => {
            rollback_only(transaction).await?;
            return Ok(TransactionDecision::Conflict);
        }
    };
    if let Err(error) = transaction.append_events(events).await {
        return Err(abort(transaction, error).await);
    }
    if let Err(error) = transaction.append_commit(request.commit).await {
        return Err(abort(transaction, error).await);
    }
    for projection in request.projections {
        if let Err(error) = transaction.upsert_projection(projection).await {
            return Err(abort(transaction, error).await);
        }
    }
    for outbox in request.outbox {
        if let Err(error) = transaction.enqueue_outbox(outbox).await {
            return Err(abort(transaction, error).await);
        }
    }
    if let Err(error) = transaction
        .finalize_idempotency(
            tenant,
            &request.intent.intent_id,
            &attempt_id,
            &request.commit.body.commit_id,
        )
        .await
    {
        return Err(abort(transaction, error).await);
    }
    transaction
        .commit()
        .await
        .map_err(|source| TransactionFailure {
            source,
            phase: TransactionPhase::Commit,
            rollback_error: None,
        })?;
    Ok(TransactionDecision::Committed)
}

/// Validates every event and rejects duplicate identities before persistence.
fn validate_committed_events(entries: &[CommittedEvent<'_>]) -> Result<(), CommitError> {
    let mut event_ids = BTreeSet::new();
    for entry in entries {
        validate_committed_event(entry)?;
        if !event_ids.insert(entry.event.event_id.clone()) {
            return Err(CommitError::InvalidEvent(format!(
                "duplicate event id `{}` in durable commit",
                entry.event.event_id
            )));
        }
    }
    Ok(())
}

/// Binds every event dimension to the authenticated intent and commit scope.
pub(crate) fn validate_event_binding(
    event: &Event,
    intent: &statechronicle_domain::intent::Intent,
    tenant: &TenantId,
) -> Result<(), CommitError> {
    if event.tenant_id != *tenant
        || event.intent_id != intent.intent_id
        || event.actor != intent.actor
        || event.operation != intent.operation
        || event.resource_id != intent.resource_id
        || intent.state_type != Some(event.after.state.state_type())
    {
        return Err(CommitError::Store(String::from(
            "durable event is not bound to the authenticated intent scope or operation",
        )));
    }
    if let ResourceState::Custom(custom) = &event.after.state {
        let domain = intent
            .inputs
            .get("profile")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| CommitError::InvalidEvent(String::from("missing custom domain")))?;
        let domain = ProfileId::new(String::from(domain))?;
        if intent.state_type != Some(StateType::Custom)
            || custom.domain != domain
            || intent
                .inputs
                .get("schema_version")
                .and_then(serde_json::Value::as_u64)
                != Some(u64::from(custom.schema_version))
        {
            return Err(CommitError::InvalidEvent(String::from(
                "custom event domain/schema does not match intent",
            )));
        }
    }
    Ok(())
}

/// Validates the event-local integrity fields that are not covered by a
/// commit's scope metadata. The enclosing commit root/signature binds the
/// event bytes, but a valid signature must never make an internally malformed
/// event acceptable to a durable adapter.
fn validate_committed_event(entry: &CommittedEvent<'_>) -> Result<(), CommitError> {
    let event = entry.event;
    if event.schema != EVENT_SCHEMA {
        return Err(CommitError::InvalidEvent(String::from(
            "event schema is not the supported v0 schema",
        )));
    }
    EventId::new(event.event_id.0.clone())
        .map_err(|error| CommitError::InvalidEvent(format!("invalid event id: {error}")))?;
    IntentId::new(event.intent_id.0.clone())
        .map_err(|error| CommitError::InvalidEvent(format!("invalid intent id: {error}")))?;
    let expected_after = event.before.version.checked_add(1).ok_or_else(|| {
        CommitError::InvalidEvent(String::from("event before-state version overflows"))
    })?;
    if event.after.version != expected_after {
        return Err(CommitError::InvalidEvent(String::from(
            "event versions must advance by exactly one",
        )));
    }
    if entry.state_type != event.before.state.state_type()
        || entry.state_type != event.after.state.state_type()
    {
        return Err(CommitError::InvalidEvent(String::from(
            "event state type does not match committed entry",
        )));
    }
    if let ResourceState::Custom(after) = &event.after.state {
        let ResourceState::Custom(before) = &event.before.state else {
            return Err(CommitError::InvalidEvent(String::from(
                "custom before-state required",
            )));
        };
        ProfileId::new(after.domain.0.clone())?;
        if after.schema_version == 0
            || before.domain != after.domain
            || before.schema_version != after.schema_version
        {
            return Err(CommitError::InvalidEvent(String::from(
                "custom transition changes domain/schema",
            )));
        }
    }
    if canonicalize_and_digest(&event.before.state).map_err(CommitError::Core)?
        != event.before.state_hash
        || canonicalize_and_digest(&event.after.state).map_err(CommitError::Core)?
            != event.after.state_hash
    {
        return Err(CommitError::InvalidEvent(String::from(
            "event state digest does not match state",
        )));
    }
    Ok(())
}

/// Re-runs the canonical intent boundary because this API accepts the domain
/// body for compatibility. Callers should prefer passing an intent produced by
/// `statechronicle-intent`; no durable claim is allowed for an unchecked body.
pub(crate) fn validate_durable_intent(
    intent: &statechronicle_domain::intent::Intent,
) -> Result<(), CommitError> {
    ValidatedIntent::try_from_intent(intent.clone(), None)
        .map(|_| ())
        .map_err(|error| CommitError::Store(format!("invalid durable intent: {error}")))
}

/// Validates caller-supplied projections against the committed entries. An
/// empty slice is allowed for adapters that derive projections internally;
/// any supplied slice must be the exact deterministic projection set.
fn validate_projection_bindings(
    commit_id: &CommitId,
    entries: &[CommittedEvent<'_>],
    projections: &[StateProjection],
) -> Result<(), CommitError> {
    if entries
        .iter()
        .any(|entry| entry.state_type != entry.event.after.state.state_type())
    {
        return Err(CommitError::Store(String::from(
            "committed event state type does not match its typed after-state",
        )));
    }
    if projections.is_empty() {
        return Ok(());
    }
    let expected = projections_for(commit_id, entries);
    if projections != expected.as_slice() {
        return Err(CommitError::Store(String::from(
            "durable projections do not match committed event entries",
        )));
    }
    Ok(())
}

/// Validates an outbox record before any transaction or adapter call.
fn validate_outbox_record(record: &OutboxRecord) -> Result<(), CommitError> {
    if record.delivery_key.is_empty()
        || record.delivery_key.len() > MAX_QUOTA_KEY_BYTES
        || record.delivery_key.chars().any(char::is_control)
        || hash_bytes(&record.payload) != record.payload_digest
    {
        return Err(CommitError::Store(String::from(
            "durable outbox payload or delivery key is invalid",
        )));
    }
    check_size(
        "outbox_payload",
        MAX_OUTBOX_PAYLOAD_BYTES,
        record.payload.len(),
    )
    .map_err(|error| CommitError::InvalidEvent(error.to_string()))
}

/// Validates all outbox rows and rejects duplicate delivery identities before
/// the adapter can observe a partially ambiguous batch.
fn validate_outbox_records(records: &[OutboxRecord]) -> Result<(), CommitError> {
    let mut delivery_keys = BTreeSet::new();
    for record in records {
        validate_outbox_record(record)?;
        if !delivery_keys.insert(record.delivery_key.as_str()) {
            return Err(CommitError::Store(String::from(
                "durable outbox contains duplicate delivery keys",
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_durable_scope(
    intent: &statechronicle_domain::intent::Intent,
    principal: &AuthenticatedPrincipal,
    commit_tenant: &TenantId,
) -> Result<(), AuthorizationError> {
    if intent.tenant_id != *commit_tenant
        || principal.tenant != *commit_tenant
        || principal.subject != intent.actor
    {
        return Err(AuthorizationError::ContextMismatch(String::from(
            "principal, intent, and commit tenant/actor are not consistently bound",
        )));
    }
    Ok(())
}

/// Verifies a prepared commit and then persists it through the durable ledger
/// boundary. This compatibility route verifies before idempotency claiming,
/// but its replay result contains only a store-supplied commit id. For checked
/// original records and structured uncertainty use [`crate::durable`].
///
/// # Errors
///
/// Returns [`CommitError::Store`] when signature/trust verification fails, or
/// the same persistence errors as the internal durable write implementation.
pub async fn persist_durable_verified(
    ledger: &dyn LedgerStore,
    authorizer: &dyn Authorizer,
    verifier: &dyn SignedCommitVerifier,
    request: DurableCommitRequest<'_>,
) -> Result<DurablePersistResult, CommitError> {
    verifier
        .verify(request.commit)
        .map_err(CommitError::Store)?;
    persist_durable(ledger, authorizer, request).await
}

/// Verified durable persistence wrapper with privacy-safe mutation metrics.
///
/// This compatibility helper retains the legacy result contract: commit
/// trust verification and canonical payload-digest validation both happen
/// before the idempotency reservation, while telemetry remains non-fatal.
///
/// # Errors
///
/// Returns the same [`CommitError`] as [`persist_durable_verified`].
pub async fn persist_durable_verified_with_metrics(
    ledger: &dyn LedgerStore,
    authorizer: &dyn Authorizer,
    verifier: &dyn SignedCommitVerifier,
    metrics: &dyn MetricsSink,
    request: DurableCommitRequest<'_>,
) -> Result<DurablePersistResult, CommitError> {
    let tenant = validate_tenant_commit(&request.commit.body)?.clone();
    let operation = request.intent.operation.clone();
    let started = std::time::Instant::now();
    let result = persist_durable_verified(ledger, authorizer, verifier, request).await;
    let outcome = match &result {
        Ok(DurablePersistResult::Committed { .. }) => MutationOutcome::Committed,
        Ok(DurablePersistResult::Replay { .. }) => MutationOutcome::Replay,
        Err(CommitError::Store(message))
            if message.contains("unavailable") || message.contains("in progress") =>
        {
            MutationOutcome::RetryableFailure
        }
        Err(_) => MutationOutcome::Rejected,
    };
    metrics
        .record_mutation(MutationMetric {
            tenant,
            operation,
            outcome,
            latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
        .await;
    result
}

/// Persists a signed commit and its events, fail-closed on validation.
///
/// The commit is stored via [`CommitStore`], the events are appended via
/// [`EventStore`], and both are published through [`EventPublisher`] when one
/// is present. Current-state projections are derived via [`projections_for`]
/// for the composition root's index adapter (see module docs).
///
/// **Validation.** The commit body's declared `event_count` and
/// `event_merkle_root` are re-derived from the supplied `entries` and compared
/// fail-closed against the signed commit before anything is written: a count
/// mismatch or root mismatch aborts with [`CommitError::EventRootMismatch`]
/// and no store write occurs.
///
/// **Caller contract (adapter-transactional writes).** [`persist`] issues the
/// three store writes (`put_commit`, `append_events`, and the publisher calls)
/// as separate port calls and does NOT wrap them in a transaction: the commit
/// and event stores are independent append-only adapters. The caller (the
/// composition root) is responsible for making these writes transactional at
/// the adapter layer — e.g. by staging them inside its `TransactionManager` so
/// the commit, its events, and the derived projections commit or roll back
/// together. [`persist`] is deterministic and side-effect-free on any
/// validation failure, so a caller that gates the write on a successful
/// [`persist`] return keeps the lanes consistent.
///
/// # Errors
///
/// Returns [`CommitError::Store`] when the commit is not tenant-scoped (global
/// checkpoints carry tenant roots, not direct events), or a store/publisher
/// rejects the write; [`CommitError::EventRootMismatch`] when `entries` does
/// not match the commit's declared event count or Merkle root;
/// [`CommitError::InvalidEvent`] when the event count does not fit the `u64`
/// field; and [`CommitError::Core`] or [`CommitError::EmptyBatch`] when the
/// event root cannot be recomputed.
pub async fn persist(
    ports: &CommitPorts,
    commit: &Signed<Commit>,
    entries: &[CommittedEvent<'_>],
) -> Result<(), CommitError> {
    let tenant = validate_tenant_commit(&commit.body)?;
    let events: Vec<Event> = entries.iter().map(|entry| entry.event.clone()).collect();
    let event_count = u64::try_from(events.len()).map_err(|err| {
        CommitError::InvalidEvent(format!("event count does not fit in u64: {err}"))
    })?;
    if event_count != commit.body.event_count {
        return Err(CommitError::EventRootMismatch);
    }
    validate_committed_events(entries)?;
    for event in &events {
        if event.tenant_id != *tenant {
            return Err(CommitError::Store(String::from(
                "event tenant does not match commit tenant scope",
            )));
        }
    }
    let computed_root = event_root(&events)?;
    if computed_root != commit.body.event_merkle_root {
        return Err(CommitError::EventRootMismatch);
    }

    ports
        .commit_store
        .put_commit(tenant, commit)
        .await
        .map_err(|err| CommitError::Store(err.to_string()))?;
    ports
        .event_store
        .append_events(tenant, &events)
        .await
        .map_err(|err| CommitError::Store(err.to_string()))?;
    if let Some(publisher) = &ports.event_publisher {
        publisher
            .publish_events(tenant, &events)
            .await
            .map_err(|err| CommitError::Store(err.to_string()))?;
        publisher
            .publish_commit(tenant, commit)
            .await
            .map_err(|err| CommitError::Store(err.to_string()))?;
    }
    // Projection derivation for the composition root's index adapter.
    let _projections = projections_for(&commit.body.commit_id, entries);
    Ok(())
}

/// Derives the current-state projections of a commit's events (protocol §9).
///
/// Each projection is a deterministic function of its event's after-state:
/// version, state hash, state payload, last event id, and last commit id all
/// come from the event/commit; the state type comes from the caller.
pub fn projections_for<'event>(
    commit_id: &CommitId,
    entries: &[CommittedEvent<'event>],
) -> Vec<StateProjection> {
    entries
        .iter()
        .map(|entry| {
            let event = entry.event;
            StateProjection {
                tenant_id: event.tenant_id.clone(),
                resource_id: event.resource_id.clone(),
                state_type: entry.state_type,
                version: event.after.version,
                last_event_id: event.event_id.clone(),
                last_commit_id: commit_id.clone(),
                state_hash: event.after.state_hash.clone(),
                state: event.after.state.clone(),
            }
        })
        .collect()
}

/// Validates a tenant commit's schema and structural identifiers and returns
/// its tenant scope. Shared by persistence and protected-plan signing.
///
/// This does not verify a signature, signer trust, event/state roots, parent
/// history, current head or authorization. It grants no commit authority.
///
/// # Errors
///
/// Returns [`CommitError::Store`] when the commit is not tenant-scoped or its
/// tenant id is missing.
pub fn validate_tenant_commit(body: &Commit) -> Result<&TenantId, CommitError> {
    if body.schema != COMMIT_SCHEMA {
        return Err(CommitError::Store(String::from(
            "commit schema is not the supported v0 schema",
        )));
    }
    CommitId::new(body.commit_id.0.clone())
        .map_err(|error| CommitError::Store(format!("invalid commit id: {error}")))?;
    if let Some(parent) = &body.parent_commit_id {
        CommitId::new(parent.0.clone())
            .map_err(|error| CommitError::Store(format!("invalid parent commit id: {error}")))?;
    }
    ProfileId::new(body.profile.0.clone())
        .map_err(|error| CommitError::Store(format!("invalid commit profile: {error}")))?;
    if body.executor.0.is_empty()
        || body.executor.0.chars().count() > MAX_ID_LENGTH
        || body.executor.0.chars().any(char::is_control)
    {
        return Err(CommitError::Store(String::from(
            "commit executor has an invalid subject identifier",
        )));
    }
    if body.scope.kind != ScopeKind::Tenant {
        return Err(CommitError::Store(String::from(
            "commit persistence requires a tenant-scoped commit; global checkpoint commits contain tenant roots, not direct events",
        )));
    }
    let tenant = body.scope.tenant_id.as_ref().ok_or_else(|| {
        CommitError::Store(String::from(
            "tenant-scoped commit is missing its tenant id",
        ))
    })?;
    if tenant.0.is_empty()
        || tenant.0.chars().count() > MAX_ID_LENGTH
        || tenant.0.chars().any(char::is_control)
    {
        return Err(CommitError::Store(String::from(
            "tenant-scoped commit has an invalid tenant identifier",
        )));
    }
    Ok(tenant)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use statechronicle_core::digest::hash_bytes;
    use statechronicle_domain::commit::{CommitScope, ProfileId};
    use statechronicle_domain::event::StateCommitment;
    use statechronicle_domain::ids::IntentId;
    use statechronicle_domain::intent::{Intent, Nonce, Operation};
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::resource_state::{ResourceState, UniqueAssetState};
    use statechronicle_domain::state_type::StateType;
    use statechronicle_domain::status::Status;
    use statechronicle_domain::subject::SubjectId;

    fn intent(tenant: &str, actor: &str) -> Intent {
        Intent::new(
            TenantId(String::from(tenant)),
            IntentId::new(String::from("int_scope_test")).unwrap(),
            Operation::from_static("asset.transfer"),
            SubjectId(String::from(actor)),
            ResourceId(String::from("asset:sword")),
            Some(StateType::UniqueAsset),
            0,
            std::collections::BTreeMap::new(),
            None,
            DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            None,
            Nonce::from_bytes(vec![1]).unwrap(),
        )
    }

    fn make_principal(tenant: &str, actor: &str) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: SubjectId(String::from(actor)),
            credential_id: String::from("session-scope-test"),
            tenant: TenantId(String::from(tenant)),
        }
    }

    fn commitment(version: u64) -> StateCommitment {
        let state = ResourceState::UniqueAsset(UniqueAssetState {
            owner: SubjectId(String::from("account:alice")),
            status: Status::from_static("active"),
            trade_id: None,
        });
        StateCommitment {
            version,
            state_hash: canonicalize_and_digest(&state).unwrap(),
            state,
        }
    }

    #[test]
    fn durable_scope_requires_intent_principal_and_commit_tenant_match() {
        let intent = intent("game", "account:alice");
        let principal = make_principal("game", "account:alice");
        assert!(
            validate_durable_scope(&intent, &principal, &TenantId(String::from("game"))).is_ok()
        );
        assert!(
            validate_durable_scope(&intent, &principal, &TenantId(String::from("other-game")))
                .is_err()
        );
        assert!(
            validate_durable_scope(
                &intent,
                &make_principal("other-game", "account:alice"),
                &TenantId(String::from("game"))
            )
            .is_err()
        );
        assert!(
            validate_durable_scope(
                &intent,
                &make_principal("game", "account:mallory"),
                &TenantId(String::from("game"))
            )
            .is_err()
        );
    }

    #[test]
    fn commit_tenant_rejects_malformed_scope_identifiers() {
        let mut body = Commit::new(
            CommitScope::tenant(TenantId(String::from("game"))),
            CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            None,
            1,
            0,
            hash_bytes(b"events"),
            hash_bytes(b"previous"),
            hash_bytes(b"next"),
            DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            SubjectId(String::from("service:ledger")),
            ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
        );
        body.scope.tenant_id = Some(TenantId(String::from("bad\nissuer")));
        assert!(validate_tenant_commit(&body).is_err());
        body.scope.tenant_id = Some(TenantId(String::new()));
        assert!(validate_tenant_commit(&body).is_err());

        body.scope.tenant_id = Some(TenantId(String::from("game")));
        body.commit_id = CommitId(String::from("invalid"));
        assert!(validate_tenant_commit(&body).is_err());

        body.commit_id = CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap();
        body.parent_commit_id = Some(CommitId(String::from("invalid")));
        assert!(validate_tenant_commit(&body).is_err());
        body.parent_commit_id = None;
        body.profile = ProfileId(String::from("bad\nprofile"));
        assert!(validate_tenant_commit(&body).is_err());
        body.profile = ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap();
        body.executor = SubjectId(String::from("bad\nexecutor"));
        assert!(validate_tenant_commit(&body).is_err());
        body.executor = SubjectId(String::from("service:ledger"));
        body.schema = String::from("statechronicle.commit.v99");
        assert!(validate_tenant_commit(&body).is_err());
    }

    #[test]
    fn durable_intent_validation_rejects_unchecked_schema() {
        let mut invalid = intent("game", "account:alice");
        invalid.schema = String::from("statechronicle.intent.v99");
        assert!(validate_durable_intent(&invalid).is_err());
    }

    #[test]
    fn projection_bindings_reject_forged_payloads() {
        let event = Event::new(
            TenantId(String::from("game")),
            statechronicle_domain::ids::EventId::new(String::from(
                "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4",
            ))
            .unwrap(),
            statechronicle_domain::ids::IntentId::new(String::from("int_scope_test")).unwrap(),
            Operation::from_static("asset.transfer"),
            ResourceId(String::from("asset:sword")),
            SubjectId(String::from("account:alice")),
            commitment(0),
            commitment(1),
            None,
            SubjectId(String::from("service:ledger")),
            DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        let commit_id = CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap();
        let entry = CommittedEvent {
            event: &event,
            state_type: StateType::UniqueAsset,
        };
        let mut forged = projections_for(&commit_id, std::slice::from_ref(&entry));
        let forged_projection = forged.first_mut().unwrap();
        forged_projection.version = forged_projection.version.saturating_add(1);
        assert!(validate_projection_bindings(&commit_id, &[entry], &forged).is_err());

        let mismatched_entry = CommittedEvent {
            event: &event,
            state_type: StateType::FungibleBalance,
        };
        assert!(validate_projection_bindings(&commit_id, &[mismatched_entry], &[]).is_err());

        let mut malformed_event = event.clone();
        malformed_event.after.state_hash = hash_bytes(b"tampered");
        let malformed_entry = CommittedEvent {
            event: &malformed_event,
            state_type: StateType::UniqueAsset,
        };
        assert!(validate_committed_event(&malformed_entry).is_err());

        let duplicate_entry = CommittedEvent {
            event: &event,
            state_type: StateType::UniqueAsset,
        };
        assert!(validate_committed_events(&[entry, duplicate_entry]).is_err());

        let mut wrong_resource = event.clone();
        let mut substituted_intent = intent("game", "account:alice");
        substituted_intent.state_type = Some(StateType::Custom);
        assert!(validate_event_binding(&event, &substituted_intent, &event.tenant_id).is_err());
        substituted_intent.state_type = Some(StateType::FungibleBalance);
        assert!(validate_event_binding(&event, &substituted_intent, &event.tenant_id).is_err());
        wrong_resource.resource_id = ResourceId(String::from("asset:other"));
        assert!(
            validate_event_binding(
                &wrong_resource,
                &intent("game", "account:alice"),
                &TenantId(String::from("game")),
            )
            .is_err()
        );
    }

    #[test]
    fn outbox_validation_rejects_digest_and_empty_key() {
        let record = OutboxRecord {
            delivery_key: String::new(),
            tenant: TenantId(String::from("game")),
            commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            payload_digest: hash_bytes(b"other"),
            payload: b"payload".to_vec(),
        };
        assert!(validate_outbox_record(&record).is_err());

        let oversized = OutboxRecord {
            delivery_key: "x".repeat(MAX_QUOTA_KEY_BYTES + 1),
            tenant: TenantId(String::from("game")),
            commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            payload_digest: hash_bytes(b"payload"),
            payload: b"payload".to_vec(),
        };
        assert!(validate_outbox_record(&oversized).is_err());

        let mut control = oversized;
        control.delivery_key = String::from("delivery\nkey");
        assert!(validate_outbox_record(&control).is_err());

        let first = OutboxRecord {
            delivery_key: String::from("delivery-1"),
            tenant: TenantId(String::from("game")),
            commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            payload_digest: hash_bytes(b"payload"),
            payload: b"payload".to_vec(),
        };
        let second = first.clone();
        assert!(validate_outbox_records(&[first, second]).is_err());
    }
}
