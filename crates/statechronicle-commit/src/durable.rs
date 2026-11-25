//! Verified durable outcomes and authoritative recovery.
//!
//! This is the advanced commit boundary, not an ingress authenticator or an
//! application reducer. A composition root must bind a prepared, authenticated
//! event plan before signing it. Atomicity and canonical-history membership
//! remain contracts of the independently qualified ledger adapter.

use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::commit::Commit;
use statechronicle_domain::ids::{CommitId, IntentId};
use statechronicle_domain::intent::Intent;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::tenant::TenantId;
use statechronicle_ports::authorization::{AuthenticatedPrincipal, AuthorizationError, Authorizer};
use statechronicle_ports::ledger_store::{
    LedgerLookup, LedgerMutation, LedgerRecovery, LedgerStoreError,
};

use crate::error::CommitError;
use crate::persist::{
    CommittedEvent, DurableCommitRequest, RequestFailure, SignedCommitVerifier,
    TransactionDecision, TransactionFailure, TransactionPhase, authorize_intent, projections_for,
    validate_durable_intent, validate_effects, validate_request, validate_tenant_commit,
    write_transaction,
};

/// Original lookup identity, derived from the complete canonical intent body.
/// The digest binds actor, resource, operation and all other body fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationIdentity {
    tenant: TenantId,
    intent_id: IntentId,
    payload_digest: ContentDigest,
}

impl MutationIdentity {
    pub(crate) fn with_digest(intent: &Intent, payload_digest: ContentDigest) -> Self {
        Self {
            tenant: intent.tenant_id.clone(),
            intent_id: intent.intent_id.clone(),
            payload_digest,
        }
    }

    pub(crate) fn from_intent(intent: &Intent) -> Result<Self, CommitError> {
        Ok(Self {
            tenant: intent.tenant_id.clone(),
            intent_id: intent.intent_id.clone(),
            payload_digest: canonicalize_and_digest(intent).map_err(CommitError::Core)?,
        })
    }

    /// Original tenant scope.
    pub const fn tenant(&self) -> &TenantId {
        &self.tenant
    }
    /// Original intent id; never replace this after an unknown acknowledgement.
    pub const fn intent_id(&self) -> &IntentId {
        &self.intent_id
    }
    /// Digest of the complete original intent body.
    pub const fn payload_digest(&self) -> &ContentDigest {
        &self.payload_digest
    }
}

/// An immutable original ledger record checked against independently supplied
/// intent/principal, signer policy, event root and deterministic effects.
///
/// A valid signature cannot prove adapter atomicity, complete outbox retention,
/// power-loss durability, or application command semantics. Those remain the
/// prepared-plan, signer and [`LedgerRecovery`] trust boundaries. This receipt
/// is historical: later commits may have advanced the canonical head.
///
/// ```compile_fail
/// use statechronicle_commit::durable::CommittedMutation;
/// let forged = CommittedMutation { record: todo!(), identity: todo!() };
/// ```
#[derive(Debug)]
pub struct CommittedMutation {
    identity: MutationIdentity,
    record: Box<LedgerMutation>,
}

impl CommittedMutation {
    /// Verified original retry/lookup identity.
    pub const fn identity(&self) -> &MutationIdentity {
        &self.identity
    }
    /// Original signed commit, checked under the supplied independent policy.
    pub const fn commit(&self) -> &Signed<Commit> {
        &self.record.commit
    }
    /// Verified original immutable mutation effects, not today's projections.
    pub const fn record(&self) -> &LedgerMutation {
        &self.record
    }
}

/// Confirmed durable write, never an empty-event planning result.
#[derive(Debug)]
pub enum DurableOutcome {
    /// This transaction committed; its original record was read back and checked.
    Committed(CommittedMutation),
    /// The original transaction had already committed; its record was checked.
    Replay(CommittedMutation),
}

/// One authoritative read. Absence is a point-in-time observation, not proof
/// that an outstanding transaction cannot later commit and not permission to
/// replace an intent identity. No recovery variant creates a reservation.
#[derive(Debug)]
pub enum RecoveryOutcome {
    /// No visible row at this read's linearization point.
    NotFound(MutationIdentity),
    /// Original matching reservation has not finalized.
    InProgress {
        /// Original immutable identity.
        identity: MutationIdentity,
        /// Adapter's active attempt identity.
        attempt_id: String,
        /// Lease timestamp; expiry alone does not establish rollback.
        lease_expires_at_unix: i64,
    },
    /// Verified original canonical mutation, without re-running a reducer.
    Committed(CommittedMutation),
}

/// Actionable refusal or uncertainty at the durable boundary.
#[derive(Debug, thiserror::Error)]
pub enum DurableError {
    /// Input or signer policy was refused before write. Authorization failures
    /// have a separate typed cause.
    #[error("durable request rejected: {0}")]
    Rejected(#[source] CommitError),
    /// Explicit policy failure, preserving denied, unauthenticated, mismatched
    /// scope and temporarily unavailable policy as distinct typed causes.
    #[error("authorization failed: {0}")]
    Authorization(#[source] AuthorizationError),
    /// Same tenant/intent id is bound to a different canonical payload.
    #[error("idempotency payload conflict")]
    PayloadConflict,
    /// Another attempt owns the matching reservation. Do not invent a new id.
    #[error("idempotency reservation in progress")]
    InProgress {
        /// Original identity to retain.
        identity: MutationIdentity,
        /// Active adapter attempt identity.
        attempt_id: String,
        /// Lease timestamp, not proof of rollback.
        lease_expires_at_unix: i64,
    },
    /// An optimistic conflict before final commit, with successful rollback.
    #[error("ledger conflict: {0}")]
    Conflict(#[source] LedgerStoreError),
    /// No commit was attempted, or a pre-commit operation was rolled back.
    /// A recovery-read failure does not resolve a previously unknown write.
    #[error("ledger unavailable: {0}")]
    Unavailable(#[source] LedgerStoreError),
    /// Adapter evidence or an invariant was inconsistent; no receipt is issued.
    #[error("corrupt ledger evidence: {0}")]
    Corrupt(String),
    /// Final commit, rollback or post-commit readback could not be confirmed.
    /// Retain identity and lookup; never infer safe failure or change identity.
    #[error("durable outcome unknown: {source}")]
    Unknown {
        /// Exact original identity for lookup/retry reconciliation.
        identity: MutationIdentity,
        /// Primary adapter error, with its original typed classification.
        #[source]
        source: LedgerStoreError,
        /// Secondary rollback failure, if any.
        rollback_error: Option<LedgerStoreError>,
    },
}

pub(crate) fn transaction_error(
    identity: MutationIdentity,
    failure: TransactionFailure,
) -> DurableError {
    if matches!(
        failure.phase,
        TransactionPhase::Commit | TransactionPhase::Rollback
    ) || failure.rollback_error.is_some()
    {
        return DurableError::Unknown {
            identity,
            source: failure.source,
            rollback_error: failure.rollback_error,
        };
    }
    match failure.source {
        error @ LedgerStoreError::Conflict(_) => DurableError::Conflict(error),
        error @ LedgerStoreError::Unavailable(_) => DurableError::Unavailable(error),
        error @ (LedgerStoreError::Idempotency(_)
        | LedgerStoreError::Closed
        | LedgerStoreError::Invariant(_)) => DurableError::Corrupt(error.to_string()),
    }
}

pub(crate) fn classify_single_write(
    decision: TransactionDecision,
    expected_commit: &CommitId,
    identity: &MutationIdentity,
) -> Result<(bool, CommitId), DurableError> {
    match decision {
        TransactionDecision::Committed => Ok((true, expected_commit.clone())),
        TransactionDecision::Replay(commit_id) => Ok((false, commit_id)),
        TransactionDecision::Conflict => Err(DurableError::PayloadConflict),
        TransactionDecision::InProgress {
            attempt_id,
            lease_expires_at_unix,
        } => Err(DurableError::InProgress {
            identity: identity.clone(),
            attempt_id,
            lease_expires_at_unix,
        }),
    }
}

pub(crate) fn confirm_single_readback<T>(
    result: Result<Option<T>, DurableError>,
    identity: MutationIdentity,
    effects_match: impl FnOnce(&T) -> bool,
) -> Result<T, DurableError> {
    let source = match result {
        Ok(Some(receipt)) if effects_match(&receipt) => return Ok(receipt),
        Ok(Some(_)) => LedgerStoreError::Invariant(String::from(
            "readback differs from acknowledged mutation effects",
        )),
        Ok(None) => LedgerStoreError::Invariant(String::from(
            "finalized mutation missing from authoritative readback",
        )),
        Err(DurableError::Unavailable(source) | DurableError::Conflict(source)) => source,
        Err(error) => LedgerStoreError::Invariant(error.to_string()),
    };
    Err(DurableError::Unknown {
        identity,
        source,
        rollback_error: None,
    })
}

fn verify_record(
    record: Box<LedgerMutation>,
    identity: MutationIdentity,
    principal: &AuthenticatedPrincipal,
    intent: &Intent,
    verifier: &dyn SignedCommitVerifier,
    expected_commit: Option<&CommitId>,
) -> Result<CommittedMutation, DurableError> {
    if record.intent != *intent || record.payload_digest != identity.payload_digest {
        return Err(DurableError::Corrupt(String::from(
            "lookup record does not match requested identity and payload",
        )));
    }
    let tenant = validate_tenant_commit(&record.commit.body)
        .map_err(|e| DurableError::Corrupt(e.to_string()))?;
    if tenant != &identity.tenant
        || expected_commit.is_some_and(|id| *id != record.commit.body.commit_id)
    {
        return Err(DurableError::Corrupt(String::from(
            "lookup commit does not match canonical claim scope and id",
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
    // Recovery must retain all original projections, even when legacy writes
    // allowed an empty supplied list for adapters that derive them internally.
    if record.projections != projections_for(&record.commit.body.commit_id, &entries) {
        return Err(DurableError::Corrupt(String::from(
            "lookup is missing original deterministic projections",
        )));
    }
    validate_effects(
        &DurableCommitRequest {
            principal,
            intent,
            payload_digest: &identity.payload_digest,
            entries: &entries,
            commit: &record.commit,
            projections: &record.projections,
            outbox: &record.outbox,
        },
        tenant,
    )
    .map_err(|error| DurableError::Corrupt(error.to_string()))?;
    Ok(CommittedMutation { identity, record })
}

pub(crate) fn read_error(error: LedgerStoreError) -> DurableError {
    match error {
        source @ LedgerStoreError::Unavailable(_) => DurableError::Unavailable(source),
        source @ LedgerStoreError::Conflict(_) => DurableError::Conflict(source),
        source @ (LedgerStoreError::Idempotency(_)
        | LedgerStoreError::Closed
        | LedgerStoreError::Invariant(_)) => DurableError::Corrupt(source.to_string()),
    }
}

async fn lookup<S: LedgerRecovery + ?Sized>(
    ledger: &S,
    verifier: &dyn SignedCommitVerifier,
    principal: &AuthenticatedPrincipal,
    intent: &Intent,
    identity: MutationIdentity,
    expected_commit: Option<&CommitId>,
) -> Result<RecoveryOutcome, DurableError> {
    match ledger
        .lookup_mutation(&identity.tenant, &identity.intent_id)
        .await
        .map_err(read_error)?
    {
        LedgerLookup::NotFound => Ok(RecoveryOutcome::NotFound(identity)),
        LedgerLookup::InProgress {
            payload_digest,
            attempt_id,
            lease_expires_at_unix,
        } => {
            if payload_digest != identity.payload_digest {
                return Err(DurableError::PayloadConflict);
            }
            Ok(RecoveryOutcome::InProgress {
                identity,
                attempt_id,
                lease_expires_at_unix,
            })
        }
        LedgerLookup::Committed(record) => {
            if record.intent.tenant_id != identity.tenant
                || record.intent.intent_id != identity.intent_id
            {
                return Err(DurableError::Corrupt(String::from(
                    "lookup returned another tenant or intent",
                )));
            }
            if record.payload_digest != identity.payload_digest {
                // A conflicting digest is only authoritative if it actually
                // covers the returned original body; corrupt metadata is not
                // a legitimate conflict with the caller's request.
                if canonicalize_and_digest(&record.intent)
                    .map_err(|error| DurableError::Corrupt(error.to_string()))?
                    != record.payload_digest
                {
                    return Err(DurableError::Corrupt(String::from(
                        "stored intent digest mismatch",
                    )));
                }
                return Err(DurableError::PayloadConflict);
            }
            verify_record(
                record,
                identity,
                principal,
                intent,
                verifier,
                expected_commit,
            )
            .map(RecoveryOutcome::Committed)
        }
    }
}

/// Resolve an original mutation without constructing events, signing a new
/// commit, claiming an identity or dispatching effects. Reauthorizes access to
/// the independently supplied intent before consulting the ledger.
///
/// # Errors
///
/// Returns typed refusal, conflict, corruption or unavailable evidence. A failed
/// lookup never resolves a prior unknown write as safely failed.
pub async fn recover<S: LedgerRecovery + ?Sized>(
    ledger: &S,
    authorizer: &dyn Authorizer,
    verifier: &dyn SignedCommitVerifier,
    principal: &AuthenticatedPrincipal,
    intent: &Intent,
) -> Result<RecoveryOutcome, DurableError> {
    validate_durable_intent(intent).map_err(DurableError::Rejected)?;
    authorize_intent(authorizer, principal, intent, &intent.tenant_id)
        .await
        .map_err(DurableError::Authorization)?;
    let identity = MutationIdentity::from_intent(intent).map_err(DurableError::Rejected)?;
    lookup(ledger, verifier, principal, intent, identity, None).await
}

/// Verify and atomically write a complete prepared commit, then read back and
/// check the original immutable ledger record. A replay claim alone cannot
/// construct a receipt. This advanced API does not itself authenticate ingress
/// signatures or recompute application reduction; bind a prepared plan first.
///
/// # Errors
///
/// Any error from final commit is `Unknown`, even if its text sounds like a
/// refusal. Failed readback after commit/replay also remains unknown. Pre-commit
/// errors retain conflict/unavailable/corruption and rollback uncertainty.
pub async fn commit_verified<S: LedgerRecovery + ?Sized>(
    ledger: &S,
    authorizer: &dyn Authorizer,
    verifier: &dyn SignedCommitVerifier,
    request: DurableCommitRequest<'_>,
) -> Result<DurableOutcome, DurableError> {
    verifier
        .verify(request.commit)
        .map_err(|e| DurableError::Rejected(CommitError::Store(e)))?;
    let (tenant, events) = validate_request(authorizer, &request)
        .await
        .map_err(|failure| match failure {
            RequestFailure::Invalid(error) => DurableError::Rejected(error),
            RequestFailure::Authorization(error) => DurableError::Authorization(error),
        })?;
    // Preflight already recomputed and checked this digest. Retain that
    // authenticated value instead of serializing the body a second time.
    let identity = MutationIdentity {
        tenant: tenant.clone(),
        intent_id: request.intent.intent_id.clone(),
        payload_digest: request.payload_digest.clone(),
    };
    let decision = write_transaction(ledger, &request, &tenant, &events)
        .await
        .map_err(|failure| transaction_error(identity.clone(), failure))?;
    let (committed, original_id) =
        classify_single_write(decision, &request.commit.body.commit_id, &identity)?;
    let result = lookup(
        ledger,
        verifier,
        request.principal,
        request.intent,
        identity.clone(),
        Some(&original_id),
    )
    .await;
    let receipt = confirm_single_readback(
        result.map(|outcome| match outcome {
            RecoveryOutcome::Committed(receipt) => Some(receipt),
            RecoveryOutcome::NotFound(_) | RecoveryOutcome::InProgress { .. } => None,
        }),
        identity,
        |receipt| {
            !committed
                || (receipt.record.commit == *request.commit
                    && receipt.record.events == events
                    && receipt.record.outbox == request.outbox)
        },
    )?;
    Ok(if committed {
        DurableOutcome::Committed(receipt)
    } else {
        DurableOutcome::Replay(receipt)
    })
}
