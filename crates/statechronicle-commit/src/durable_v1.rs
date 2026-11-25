//! Read-only recovery of exact-byte v1 mutations.
//!
//! Recovery verifies the original signed intent in historical mode, rechecks
//! current caller authorization, and verifies the retained commit under an
//! independently selected tenant authority. It never claims an idempotency
//! key or dispatches effects.

use chrono::{DateTime, Utc};
use statechronicle_core::{
    digest::{ContentDigest, hash_bytes},
    signature_provider::SignaturePurpose,
    signature_wire::decode_v1_envelope,
};
use statechronicle_domain::{commit::Commit, ids::CommitId, intent::Intent, subject::SubjectId};
use statechronicle_intent::v1_body::{INTENT_V1_SCHEMA, decode_v1_intent_body};
use statechronicle_ports::{
    authorization::{AuthenticatedPrincipal, Authorizer},
    ledger_v1::{LedgerLookupV1, LedgerMutationV1, LedgerRecoveryV1},
    signature_policy::{
        ResolvedSignaturePolicy, SignatureAuthority, SignaturePolicy, SignaturePolicyQuery,
        SignatureUse,
    },
};

use crate::{
    durable::{DurableError, MutationIdentity, read_error},
    error::CommitError,
    persist::{
        CommitEffects, CommittedEvent, authorize_intent, validate_durable_intent,
        validate_effects_for_body, validate_event_binding, validate_tenant_commit,
    },
    sign_v1::verify_commit_v1_with_policy,
};

/// Trusted context required to verify an original v1 mutation.
pub struct MutationTrustV1<'policy> {
    /// Authenticated caller being authorized to recover this intent.
    pub principal: &'policy AuthenticatedPrincipal,
    /// Independently configured authority for the tenant commit chain.
    pub commit_authority: &'policy SubjectId,
    /// Trusted policy and key-set resolver.
    pub signature_policy: &'policy dyn SignaturePolicy,
    /// Trusted reference time for active or historical policy resolution.
    pub now: DateTime<Utc>,
}

/// Verified original v1 record, preserving its exact canonical signed bytes.
#[derive(Debug)]
pub struct CommittedMutationV1 {
    identity: MutationIdentity,
    intent: Intent,
    commit_body: Commit,
    record: Box<LedgerMutationV1>,
}

impl CommittedMutationV1 {
    /// Verified original retry identity, including the v1 intent-body digest.
    pub const fn identity(&self) -> &MutationIdentity {
        &self.identity
    }

    /// Decoded intent whose original envelope and signature were verified.
    pub const fn intent(&self) -> &Intent {
        &self.intent
    }

    /// Independently verified commit body.
    pub const fn commit_body(&self) -> &Commit {
        &self.commit_body
    }

    /// Exact original signed actor-intent envelope retained by the adapter.
    pub fn intent_record(&self) -> &[u8] {
        &self.record.intent_record
    }

    /// Exact original signed tenant-commit envelope retained by the adapter.
    pub fn commit_record(&self) -> &[u8] {
        &self.record.commit_record
    }

    /// Complete verified original effects, not current projections or outbox state.
    pub const fn record(&self) -> &LedgerMutationV1 {
        &self.record
    }
}

/// Read-only recovery result for an exact-byte v1 mutation.
#[derive(Debug)]
pub enum RecoveryOutcomeV1 {
    /// No visible matching row at this read's linearization point.
    NotFound(MutationIdentity),
    /// The matching original reservation has not finalized.
    InProgress {
        /// Original identity to retain.
        identity: MutationIdentity,
        /// Adapter attempt identity.
        attempt_id: String,
        /// Lease expiry does not establish rollback.
        lease_expires_at_unix: i64,
    },
    /// Verified original mutation with exact v1 signed bytes.
    Committed(Box<CommittedMutationV1>),
}

/// Verifies and decodes a v1 actor-intent envelope under the requested policy mode.
///
/// The result digest covers the exact canonical v1 body bytes, not the legacy
/// typed `Intent` representation. Active mode also enforces current expiry.
pub(crate) async fn verify_intent_record_v1(
    record: &[u8],
    principal: &AuthenticatedPrincipal,
    policy: &dyn SignaturePolicy,
    usage: SignatureUse,
    now: DateTime<Utc>,
) -> Result<(Intent, ContentDigest), DurableError> {
    let envelope = decode_v1_envelope(record, SignaturePurpose::ActorIntent)
        .map_err(|error| rejected(error.to_string()))?;
    let intent =
        decode_v1_intent_body(envelope.body).map_err(|error| rejected(error.to_string()))?;
    validate_durable_intent(&intent).map_err(DurableError::Rejected)?;
    crate::persist::validate_durable_scope(&intent, principal, &intent.tenant_id)
        .map_err(DurableError::Authorization)?;
    if intent.created_at > now
        || (usage == SignatureUse::Active && intent.expires_at.is_some_and(|expiry| expiry <= now))
    {
        return Err(rejected("v1 intent is not valid at the trusted time"));
    }
    let decision = policy
        .resolve(SignaturePolicyQuery {
            authority: SignatureAuthority::ActorIntent {
                tenant: &principal.tenant,
                actor: &principal.subject,
                operation: &intent.operation,
            },
            usage,
            signed_at: intent.created_at,
            now,
        })
        .await
        .map_err(|error| rejected(error.to_string()))?;
    let ResolvedSignaturePolicy::V1(resolved) = decision else {
        return Err(rejected("v1 intent requires a v1 signature policy"));
    };
    resolved
        .context(SignaturePurpose::ActorIntent, INTENT_V1_SCHEMA)
        .verify_envelope(envelope, resolved.verifier())
        .map_err(|error| rejected(error.to_string()))?;
    Ok((intent, hash_bytes(envelope.body)))
}

fn rejected(message: impl Into<String>) -> DurableError {
    DurableError::Rejected(CommitError::Store(message.into()))
}

async fn verify_retained_effects(
    record: Box<LedgerMutationV1>,
    identity: MutationIdentity,
    expected_intent: Intent,
    expected_intent_record: &[u8],
    trust: &MutationTrustV1<'_>,
    expected_commit_id: Option<&CommitId>,
) -> Result<CommittedMutationV1, DurableError> {
    if record.intent_record.is_empty() {
        return Err(DurableError::Corrupt(String::from(
            "missing retained v1 intent envelope",
        )));
    }
    if record.intent_record != expected_intent_record {
        // If the stored original is a different valid signed body, distinguish
        // a genuine idempotency conflict from corrupt adapter digest metadata.
        let (stored_intent, stored_digest) = verify_intent_record_v1(
            &record.intent_record,
            trust.principal,
            trust.signature_policy,
            SignatureUse::Historical,
            trust.now,
        )
        .await
        .map_err(|error| DurableError::Corrupt(error.to_string()))?;
        if stored_intent.tenant_id != *identity.tenant()
            || stored_intent.intent_id != *identity.intent_id()
            || record.payload_digest != stored_digest
        {
            return Err(DurableError::Corrupt(String::from(
                "retained v1 intent digest or identity is inconsistent",
            )));
        }
        if stored_digest != *identity.payload_digest() {
            return Err(DurableError::PayloadConflict);
        }
        return Err(DurableError::Corrupt(String::from(
            "same v1 intent body has different original signed envelope bytes",
        )));
    }
    if record.payload_digest != *identity.payload_digest() {
        return Err(DurableError::Corrupt(String::from(
            "retained v1 intent digest does not match its signed body",
        )));
    }
    let authority = SignatureAuthority::TenantCommit {
        tenant: identity.tenant(),
        authority: trust.commit_authority,
    };
    let commit = verify_commit_v1_with_policy(
        &record.commit_record,
        authority,
        SignatureUse::Historical,
        trust.now,
        trust.signature_policy,
    )
    .await
    .map_err(|error| DurableError::Corrupt(error.to_string()))?;
    let commit_tenant = validate_tenant_commit(&commit)
        .map_err(|error| DurableError::Corrupt(error.to_string()))?;
    if commit_tenant != identity.tenant()
        || expected_commit_id.is_some_and(|id| *id != commit.commit_id)
    {
        return Err(DurableError::Corrupt(String::from(
            "retained v1 commit does not match the requested tenant or commit id",
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
    if record.projections.is_empty() {
        return Err(DurableError::Corrupt(String::from(
            "lookup is missing original deterministic projections",
        )));
    }
    validate_effects_for_body(
        CommitEffects {
            entries: &entries,
            body: &commit,
            projections: &record.projections,
            outbox_records: &record.outbox,
        },
        commit_tenant,
        |event| validate_event_binding(event, &expected_intent, commit_tenant),
        || Ok(()),
    )
    .map_err(|error| DurableError::Corrupt(error.to_string()))?;
    Ok(CommittedMutationV1 {
        identity,
        intent: expected_intent,
        commit_body: commit,
        record,
    })
}

async fn lookup_verified<S: LedgerRecoveryV1 + ?Sized>(
    ledger: &S,
    trust: &MutationTrustV1<'_>,
    expected_intent: Intent,
    intent_record: &[u8],
    body_digest: ContentDigest,
    expected_commit_id: Option<&CommitId>,
) -> Result<RecoveryOutcomeV1, DurableError> {
    let identity = MutationIdentity::with_digest(&expected_intent, body_digest);
    match ledger
        .lookup_mutation_v1(identity.tenant(), identity.intent_id())
        .await
        .map_err(read_error)?
    {
        LedgerLookupV1::NotFound => Ok(RecoveryOutcomeV1::NotFound(identity)),
        LedgerLookupV1::InProgress {
            payload_digest,
            attempt_id,
            lease_expires_at_unix,
        } => {
            if payload_digest != *identity.payload_digest() {
                return Err(DurableError::PayloadConflict);
            }
            Ok(RecoveryOutcomeV1::InProgress {
                identity,
                attempt_id,
                lease_expires_at_unix,
            })
        }
        LedgerLookupV1::OtherRecord => Err(DurableError::Corrupt(String::from(
            "v1 lookup resolved to another record format or retained group",
        ))),
        LedgerLookupV1::Committed(record) => verify_retained_effects(
            record,
            identity,
            expected_intent,
            intent_record,
            trust,
            expected_commit_id,
        )
        .await
        .map(|receipt| RecoveryOutcomeV1::Committed(Box::new(receipt))),
    }
}

/// Internal authoritative v1 lookup shared by recovery and post-write readback.
pub(crate) async fn lookup<S: LedgerRecoveryV1 + ?Sized>(
    ledger: &S,
    trust: &MutationTrustV1<'_>,
    expected_intent: &Intent,
    intent_record: &[u8],
    expected_commit_id: Option<&CommitId>,
) -> Result<RecoveryOutcomeV1, DurableError> {
    let (decoded, body_digest) = verify_intent_record_v1(
        intent_record,
        trust.principal,
        trust.signature_policy,
        SignatureUse::Historical,
        trust.now,
    )
    .await?;
    if decoded != *expected_intent {
        return Err(DurableError::Rejected(CommitError::PayloadDigestMismatch));
    }
    lookup_verified(
        ledger,
        trust,
        decoded,
        intent_record,
        body_digest,
        expected_commit_id,
    )
    .await
}

/// Reauthorizes and reads back a v1 mutation without claiming or writing anything.
///
/// # Errors
/// Returns typed refusal, conflict, corruption, or unavailable evidence. A
/// failed lookup leaves an earlier unknown transaction outcome unresolved.
pub async fn recover_v1<S: LedgerRecoveryV1 + ?Sized>(
    ledger: &S,
    authorizer: &dyn Authorizer,
    trust: &MutationTrustV1<'_>,
    intent_record: &[u8],
) -> Result<RecoveryOutcomeV1, DurableError> {
    let (intent, digest) = verify_intent_record_v1(
        intent_record,
        trust.principal,
        trust.signature_policy,
        SignatureUse::Historical,
        trust.now,
    )
    .await?;
    authorize_intent(authorizer, trust.principal, &intent, &intent.tenant_id)
        .await
        .map_err(DurableError::Authorization)?;
    lookup_verified(ledger, trust, intent, intent_record, digest, None).await
}

pub use crate::durable_write_v1::{DurableCommitRequestV1, DurableOutcomeV1, commit_verified_v1};
