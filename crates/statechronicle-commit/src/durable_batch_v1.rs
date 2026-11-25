//! Versioned atomic batches with exact original-record recovery.
use crate::{
    durable::{
        DurableError, MutationIdentity, confirm_single_readback, read_error, transaction_error,
    },
    durable_batch::{self, BatchDecision, BatchMember, BatchWrite, BatchWriteCommit},
    durable_v1::verify_intent_record_v1,
    error::CommitError,
    persist::{CommittedEvent, authorize_intent, projections_for, validate_tenant_commit},
    sign_v1::verify_commit_v1_with_policy,
};
use chrono::{DateTime, Utc};
use statechronicle_core::{
    canonicalize::canonicalize,
    digest::{ContentDigest, hash_bytes},
    limits::{MAX_EVENT_BATCH_BYTES, MAX_EVENTS_PER_COMMIT, check_size},
};
use statechronicle_domain::{
    commit::Commit, event::Event, ids::CommitId, intent::Intent, state::StateProjection,
    subject::SubjectId, tenant::TenantId,
};
use statechronicle_ports::{
    authorization::{AuthenticatedPrincipal, Authorizer},
    ledger_store::OutboxRecord,
    ledger_v1::{
        LedgerBatchRecoveryV1, LedgerBatchV1, LedgerLookupV1, RetainBatchMemberV1, RetainBatchV1,
    },
    signature_policy::{SignatureAuthority, SignaturePolicy, SignatureUse},
};
use std::collections::BTreeSet;

/// One signed v1 member and its independently authenticated principal.
pub struct BatchMemberV1<'request> {
    /// Principal bound to this intent.
    pub principal: &'request AuthenticatedPrincipal,
    /// Exact original actor-intent envelope.
    pub intent_record: &'request [u8],
}
/// Trusted tenant commit authority and policy clock.
pub struct BatchTrustV1<'request> {
    /// Independent commit-chain authority.
    pub commit_authority: &'request SubjectId,
    /// Trusted policy resolver.
    pub signature_policy: &'request dyn SignaturePolicy,
    /// Trusted reference time.
    pub now: DateTime<Utc>,
}
/// Complete atomic v1 write request.
pub struct DurableBatchRequestV1<'request> {
    /// Ordered original signed intents.
    pub members: &'request [BatchMemberV1<'request>],
    /// Exact signed commit envelope.
    pub commit_record: &'request [u8],
    /// Complete ordered events.
    pub entries: &'request [CommittedEvent<'request>],
    /// Complete deterministic projections.
    pub projections: &'request [StateProjection],
    /// Complete original outbox.
    pub outbox: &'request [OutboxRecord],
}

/// Complete ordered retry identity of a v1 batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchIdentityV1 {
    members: Vec<MutationIdentity>,
    digest: ContentDigest,
}
impl BatchIdentityV1 {
    /// Member identities in caller order.
    pub fn members(&self) -> &[MutationIdentity] {
        &self.members
    }
    /// Digest of all ordered exact intent records.
    pub const fn payload_digest(&self) -> &ContentDigest {
        &self.digest
    }
}
/// Refusal or uncertainty, preserving full group identity after preflight.
#[derive(Debug, thiserror::Error)]
#[error("durable v1 batch: {source}")]
pub struct DurableBatchErrorV1 {
    identity: Option<BatchIdentityV1>,
    #[source]
    source: DurableError,
}
impl DurableBatchErrorV1 {
    /// Complete validated group identity, if available.
    pub const fn identity(&self) -> Option<&BatchIdentityV1> {
        self.identity.as_ref()
    }
    /// Typed cause.
    pub const fn cause(&self) -> &DurableError {
        &self.source
    }
}
fn err(message: &str) -> DurableError {
    DurableError::Rejected(CommitError::Store(message.into()))
}
fn with_id(identity: &BatchIdentityV1, source: DurableError) -> DurableBatchErrorV1 {
    DurableBatchErrorV1 {
        identity: Some(identity.clone()),
        source,
    }
}
const fn before_id(source: DurableError) -> DurableBatchErrorV1 {
    DurableBatchErrorV1 {
        identity: None,
        source,
    }
}

/// Verified complete original group.
#[derive(Debug)]
pub struct CommittedBatchV1 {
    identity: BatchIdentityV1,
    record: Box<LedgerBatchV1>,
    commit: Commit,
}
impl CommittedBatchV1 {
    /// Verified ordered group identity.
    pub const fn identity(&self) -> &BatchIdentityV1 {
        &self.identity
    }
    /// Exact original group evidence and effects.
    pub const fn record(&self) -> &LedgerBatchV1 {
        &self.record
    }
    /// Verified tenant commit body.
    pub const fn commit_body(&self) -> &Commit {
        &self.commit
    }
}
/// Durable write result.
#[derive(Debug)]
pub enum DurableBatchOutcomeV1 {
    /// Newly committed and read back.
    Committed(Box<CommittedBatchV1>),
    /// Previously committed exact group.
    Replay(Box<CommittedBatchV1>),
}
/// Read-only recovery result.
#[derive(Debug)]
pub enum BatchRecoveryOutcomeV1 {
    /// No visible anchor at the read linearization point.
    NotFound(BatchIdentityV1),
    /// Anchor claim is still reserved.
    InProgress {
        /// Full group identity.
        identity: BatchIdentityV1,
        /// Fenced attempt.
        attempt_id: String,
        /// Lease expiry does not prove rollback.
        lease_expires_at_unix: i64,
    },
    /// Full verified original group.
    Committed(Box<CommittedBatchV1>),
}

async fn decode_members(
    members: &[BatchMemberV1<'_>],
    policy: &dyn SignaturePolicy,
    now: DateTime<Utc>,
    usage: SignatureUse,
) -> Result<(Vec<Intent>, Vec<ContentDigest>), DurableError> {
    if members.is_empty() {
        return Err(err("empty durable v1 batch"));
    }
    check_size("batch_intents", MAX_EVENTS_PER_COMMIT, members.len())
        .map_err(|e| DurableError::Rejected(CommitError::Core(e)))?;
    let mut total = 0usize;
    let mut intents = Vec::with_capacity(members.len());
    let mut digests = Vec::with_capacity(members.len());
    let mut seen = BTreeSet::new();
    for member in members {
        total = total.saturating_add(member.intent_record.len());
        check_size("batch_intent_records", MAX_EVENT_BATCH_BYTES, total)
            .map_err(|e| DurableError::Rejected(CommitError::Core(e)))?;
        let (intent, digest) =
            verify_intent_record_v1(member.intent_record, member.principal, policy, usage, now)
                .await?;
        if intents
            .first()
            .is_some_and(|first: &Intent| first.tenant_id != intent.tenant_id)
            || !seen.insert(intent.intent_id.clone())
        {
            return Err(err("mixed tenant or duplicate intent in v1 batch"));
        }
        intents.push(intent);
        digests.push(digest);
    }
    Ok((intents, digests))
}
fn group_identity_v1(
    intents: &[Intent],
    records: &[&[u8]],
    digests: &[ContentDigest],
) -> Result<BatchIdentityV1, DurableError> {
    let members = intents
        .iter()
        .zip(digests)
        .map(|(i, d)| MutationIdentity::with_digest(i, d.clone()))
        .collect();
    let bytes = canonicalize(&("statechronicle.atomic-batch.v1", records))
        .map_err(|e| DurableError::Rejected(CommitError::Core(e)))?;
    check_size("batch_intent_bytes", MAX_EVENT_BATCH_BYTES, bytes.len())
        .map_err(|e| DurableError::Rejected(CommitError::Core(e)))?;
    Ok(BatchIdentityV1 {
        members,
        digest: hash_bytes(&bytes),
    })
}
async fn authorize_members(
    authorizer: &dyn Authorizer,
    members: &[BatchMemberV1<'_>],
    intents: &[Intent],
) -> Result<(), DurableError> {
    for (member, intent) in members.iter().zip(intents) {
        authorize_intent(authorizer, member.principal, intent, &intent.tenant_id)
            .await
            .map_err(DurableError::Authorization)?;
    }
    Ok(())
}
fn check_effects(
    intents: &[Intent],
    entries: &[CommittedEvent<'_>],
    body: &Commit,
    projections: &[StateProjection],
    outbox: &[OutboxRecord],
) -> Result<Vec<Event>, CommitError> {
    let refs: Vec<_> = intents.iter().collect();
    let events = durable_batch::group_effects_v1_body(&refs, entries, body, projections, outbox)?;
    check_size("events", MAX_EVENTS_PER_COMMIT, entries.len()).map_err(CommitError::Core)?;
    if projections != projections_for(&body.commit_id, entries) {
        return Err(CommitError::InvalidEvent(String::from(
            "v1 batch requires complete deterministic projections",
        )));
    }
    Ok(events)
}
async fn verify_commit(
    record: &[u8],
    tenant: &TenantId,
    trust: &BatchTrustV1<'_>,
    usage: SignatureUse,
) -> Result<Commit, DurableError> {
    verify_commit_v1_with_policy(
        record,
        SignatureAuthority::TenantCommit {
            tenant,
            authority: trust.commit_authority,
        },
        usage,
        trust.now,
        trust.signature_policy,
    )
    .await
    .map_err(|e| err(&e.to_string()))
}
async fn lookup<S: LedgerBatchRecoveryV1 + ?Sized>(
    ledger: &S,
    members: &[BatchMemberV1<'_>],
    trust: &BatchTrustV1<'_>,
    identity: &BatchIdentityV1,
    expected: Option<&CommitId>,
    verified_intents: Option<&[Intent]>,
) -> Result<BatchRecoveryOutcomeV1, DurableError> {
    let anchor = identity
        .members
        .first()
        .ok_or_else(|| err("missing batch anchor"))?;
    match ledger
        .lookup_batch_v1(anchor.tenant(), anchor.intent_id())
        .await
        .map_err(read_error)?
    {
        LedgerLookupV1::NotFound => Ok(BatchRecoveryOutcomeV1::NotFound(identity.clone())),
        LedgerLookupV1::InProgress {
            payload_digest,
            attempt_id,
            lease_expires_at_unix,
        } => {
            if payload_digest != *anchor.payload_digest() {
                return Err(DurableError::PayloadConflict);
            }
            Ok(BatchRecoveryOutcomeV1::InProgress {
                identity: identity.clone(),
                attempt_id,
                lease_expires_at_unix,
            })
        }
        LedgerLookupV1::OtherRecord => Err(DurableError::PayloadConflict),
        LedgerLookupV1::Committed(record) => {
            if record.intent_records.len() != members.len()
                || record
                    .intent_records
                    .iter()
                    .zip(members)
                    .any(|(a, b)| a.as_slice() != b.intent_record)
            {
                return Err(DurableError::PayloadConflict);
            }
            let newly_verified;
            let intents = if let Some(intents) = verified_intents {
                intents
            } else {
                let (decoded, digests) = decode_members(
                    members,
                    trust.signature_policy,
                    trust.now,
                    SignatureUse::Historical,
                )
                .await?;
                if !identity
                    .members
                    .iter()
                    .zip(digests)
                    .all(|(member, digest)| member.payload_digest() == &digest)
                {
                    return Err(DurableError::PayloadConflict);
                }
                newly_verified = decoded;
                &newly_verified
            };
            let tenant = anchor.tenant();
            let body = verify_commit(
                &record.commit_record,
                tenant,
                trust,
                SignatureUse::Historical,
            )
            .await
            .map_err(|e| DurableError::Corrupt(e.to_string()))?;
            if expected.is_some_and(|id| id != &body.commit_id)
                || validate_tenant_commit(&body)
                    .map_err(|e| DurableError::Corrupt(e.to_string()))?
                    != tenant
            {
                return Err(DurableError::Corrupt(String::from(
                    "retained v1 batch commit scope or id mismatch",
                )));
            }
            let entries: Vec<_> = record
                .events
                .iter()
                .map(|event| CommittedEvent {
                    event,
                    state_type: event.after.state.state_type(),
                })
                .collect();
            check_effects(
                intents,
                &entries,
                &body,
                &record.projections,
                &record.outbox,
            )
            .map_err(|e| DurableError::Corrupt(e.to_string()))?;
            for member in identity.members.iter().skip(1) {
                match ledger
                    .lookup_batch_v1(member.tenant(), member.intent_id())
                    .await
                    .map_err(read_error)?
                {
                    LedgerLookupV1::Committed(other) if *other == *record => (),
                    LedgerLookupV1::NotFound
                    | LedgerLookupV1::InProgress { .. }
                    | LedgerLookupV1::Committed(_)
                    | LedgerLookupV1::OtherRecord => {
                        return Err(DurableError::Corrupt(String::from(
                            "batch members do not resolve to one complete original group",
                        )));
                    }
                }
            }
            Ok(BatchRecoveryOutcomeV1::Committed(Box::new(
                CommittedBatchV1 {
                    identity: identity.clone(),
                    record,
                    commit: body,
                },
            )))
        }
    }
}
/// Recover the full original group in historical signature mode and reauthorize every member.
///
/// # Errors
/// Returns authorization, policy, corrupt evidence, conflicting group identity,
/// or unavailable ledger errors. Read failure does not resolve an unknown write.
pub async fn recover_batch_v1<S: LedgerBatchRecoveryV1 + ?Sized>(
    ledger: &S,
    authorizer: &dyn Authorizer,
    trust: &BatchTrustV1<'_>,
    members: &[BatchMemberV1<'_>],
) -> Result<BatchRecoveryOutcomeV1, DurableBatchErrorV1> {
    let (intents, digests) = decode_members(
        members,
        trust.signature_policy,
        trust.now,
        SignatureUse::Historical,
    )
    .await
    .map_err(before_id)?;
    authorize_members(authorizer, members, &intents)
        .await
        .map_err(before_id)?;
    let records: Vec<_> = members.iter().map(|m| m.intent_record).collect();
    let id = group_identity_v1(&intents, &records, &digests).map_err(before_id)?;
    lookup(ledger, members, trust, &id, None, Some(&intents))
        .await
        .map_err(|e| with_id(&id, e))
}
/// Verify every active v1 member and commit, write one atomic group, then verify complete readback.
///
/// # Errors
/// Returns typed rejection or conflict before writing; failures after transaction
/// start retain full group identity and report uncertain outcomes for recovery.
pub async fn commit_batch_verified_v1<S: LedgerBatchRecoveryV1 + ?Sized>(
    ledger: &S,
    authorizer: &dyn Authorizer,
    trust: &BatchTrustV1<'_>,
    request: DurableBatchRequestV1<'_>,
) -> Result<DurableBatchOutcomeV1, DurableBatchErrorV1> {
    let (intents, digests) = decode_members(
        request.members,
        trust.signature_policy,
        trust.now,
        SignatureUse::Active,
    )
    .await
    .map_err(before_id)?;
    let records: Vec<_> = request.members.iter().map(|m| m.intent_record).collect();
    let id = group_identity_v1(&intents, &records, &digests).map_err(before_id)?;
    authorize_members(authorizer, request.members, &intents)
        .await
        .map_err(|e| with_id(&id, e))?;
    let anchor = id
        .members
        .first()
        .ok_or_else(|| with_id(&id, err("missing batch anchor")))?;
    let tenant = anchor.tenant();
    let commit = verify_commit(request.commit_record, tenant, trust, SignatureUse::Active)
        .await
        .map_err(|e| with_id(&id, e))?;
    validate_tenant_commit(&commit).map_err(|e| with_id(&id, DurableError::Rejected(e)))?;
    let events = check_effects(
        &intents,
        request.entries,
        &commit,
        request.projections,
        request.outbox,
    )
    .map_err(|e| with_id(&id, DurableError::Rejected(e)))?;
    let retain: Vec<_> = request
        .members
        .iter()
        .zip(&id.members)
        .map(|(member, identity)| RetainBatchMemberV1 {
            intent_id: identity.intent_id(),
            payload_digest: identity.payload_digest(),
            intent_record: member.intent_record,
        })
        .collect();
    let members: Vec<_> = request
        .members
        .iter()
        .zip(&intents)
        .zip(&digests)
        .map(|((member, intent), payload_digest)| BatchMember {
            principal: member.principal,
            intent,
            payload_digest,
        })
        .collect();
    let decision = durable_batch::write_group(
        ledger,
        &members,
        &id.members,
        BatchWrite {
            commit: BatchWriteCommit::V1(RetainBatchV1 {
                commit: &commit,
                commit_record: request.commit_record,
                members: &retain,
            }),
            events: &events,
            projections: request.projections,
            outbox: request.outbox,
        },
    )
    .await
    .map_err(|failure| with_id(&id, transaction_error(anchor.clone(), failure)))?;
    let (committed, commit_id) = match decision {
        BatchDecision::Committed => (true, commit.commit_id),
        BatchDecision::Replay(original) => (false, original),
        BatchDecision::Conflict => return Err(with_id(&id, DurableError::PayloadConflict)),
        BatchDecision::InProgress {
            member,
            attempt_id,
            lease_expires_at_unix,
        } => {
            return Err(with_id(
                &id,
                DurableError::InProgress {
                    identity: member,
                    attempt_id,
                    lease_expires_at_unix,
                },
            ));
        }
    };
    let readback = lookup(ledger, request.members, trust, &id, Some(&commit_id), None).await;
    let receipt = confirm_single_readback(
        readback.map(|outcome| match outcome {
            BatchRecoveryOutcomeV1::Committed(receipt) => Some(receipt),
            BatchRecoveryOutcomeV1::NotFound(_) | BatchRecoveryOutcomeV1::InProgress { .. } => None,
        }),
        anchor.clone(),
        |receipt| {
            !committed
                || (receipt.record.commit_record == request.commit_record
                    && receipt.record.events == events
                    && receipt.record.projections == request.projections
                    && receipt.record.outbox == request.outbox)
        },
    )
    .map_err(|error| with_id(&id, error))?;
    Ok(if committed {
        DurableBatchOutcomeV1::Committed(receipt)
    } else {
        DurableBatchOutcomeV1::Replay(receipt)
    })
}
