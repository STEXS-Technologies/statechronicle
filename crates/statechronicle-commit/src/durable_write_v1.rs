//! Single-mutation v1 writes with authoritative original-record readback.

use statechronicle_domain::state::StateProjection;
use statechronicle_ports::{
    authorization::Authorizer,
    ledger_store::OutboxRecord,
    ledger_v1::{LedgerRecoveryV1, RetainMutationV1},
    signature_policy::{SignatureAuthority, SignatureUse},
};

use crate::{
    durable::{DurableError, classify_single_write, confirm_single_readback, transaction_error},
    durable_v1::{
        CommittedMutationV1, MutationTrustV1, RecoveryOutcomeV1, lookup, verify_intent_record_v1,
    },
    error::CommitError,
    persist::{
        CommitEffects, CommitWrite, CommittedEvent, SingleMutationWrite, authorize_intent,
        validate_effects_for_body, validate_event_binding, validate_tenant_commit,
        write_single_transaction,
    },
    sign_v1::verify_commit_v1_with_policy,
};

/// Exact records and effects proposed for one atomic v1 mutation.
pub struct DurableCommitRequestV1<'request> {
    /// Original signed actor intent.
    pub intent_record: &'request [u8],
    /// Original signed tenant commit.
    pub commit_record: &'request [u8],
    /// Complete ordered prepared events.
    pub entries: &'request [CommittedEvent<'request>],
    /// Complete deterministic projections.
    pub projections: &'request [StateProjection],
    /// Complete outbox rows to retain atomically.
    pub outbox: &'request [OutboxRecord],
}

/// A confirmed v1 mutation with independently verified retained evidence.
#[derive(Debug)]
pub enum DurableOutcomeV1 {
    /// This transaction committed and its original record was confirmed.
    Committed(Box<CommittedMutationV1>),
    /// An earlier transaction committed the same intent.
    Replay(Box<CommittedMutationV1>),
}

/// Commits an exact v1 mutation and verifies original record readback.
///
/// # Errors
/// Returns typed rejection, conflict, unavailability, or uncertainty. After
/// an unknown outcome, reconcile against the original signed intent.
pub async fn commit_verified_v1<S: LedgerRecoveryV1 + ?Sized>(
    ledger: &S,
    authorizer: &dyn Authorizer,
    trust: &MutationTrustV1<'_>,
    request: DurableCommitRequestV1<'_>,
) -> Result<DurableOutcomeV1, DurableError> {
    let (intent, payload_digest) = verify_intent_record_v1(
        request.intent_record,
        trust.principal,
        trust.signature_policy,
        SignatureUse::Active,
        trust.now,
    )
    .await?;
    let tenant = &intent.tenant_id;
    authorize_intent(authorizer, trust.principal, &intent, tenant)
        .await
        .map_err(DurableError::Authorization)?;
    let commit = verify_commit_v1_with_policy(
        request.commit_record,
        SignatureAuthority::TenantCommit {
            tenant,
            authority: trust.commit_authority,
        },
        SignatureUse::Active,
        trust.now,
        trust.signature_policy,
    )
    .await
    .map_err(|error| DurableError::Rejected(CommitError::Store(error.to_string())))?;
    validate_tenant_commit(&commit).map_err(DurableError::Rejected)?;
    if request.projections.is_empty() {
        return Err(DurableError::Rejected(CommitError::Store(String::from(
            "v1 mutations require all deterministic projections",
        ))));
    }
    let events = validate_effects_for_body(
        CommitEffects {
            entries: request.entries,
            body: &commit,
            projections: request.projections,
            outbox_records: request.outbox,
        },
        tenant,
        |event| validate_event_binding(event, &intent, tenant),
        || Ok(()), // The typed v1 verifier already checks the full record bound.
    )
    .map_err(DurableError::Rejected)?;
    let identity = crate::durable::MutationIdentity::with_digest(&intent, payload_digest.clone());
    let decision = write_single_transaction(
        ledger,
        SingleMutationWrite {
            intent: &intent,
            payload_digest: &payload_digest,
            commit: CommitWrite::V1(RetainMutationV1 {
                commit: &commit,
                intent_id: &intent.intent_id,
                intent_record: request.intent_record,
                commit_record: request.commit_record,
            }),
            projections: request.projections,
            outbox: request.outbox,
        },
        tenant,
        &events,
    )
    .await
    .map_err(|failure| transaction_error(identity.clone(), failure))?;

    let (committed, original_id) = classify_single_write(decision, &commit.commit_id, &identity)?;
    let readback = lookup(
        ledger,
        trust,
        &intent,
        request.intent_record,
        Some(&original_id),
    )
    .await;
    let receipt = confirm_single_readback(
        readback.map(|outcome| match outcome {
            RecoveryOutcomeV1::Committed(receipt) => Some(receipt),
            RecoveryOutcomeV1::NotFound(_) | RecoveryOutcomeV1::InProgress { .. } => None,
        }),
        identity,
        |receipt| {
            let original = receipt.record();
            !committed
                || (original.intent_record == request.intent_record
                    && original.commit_record == request.commit_record
                    && original.events == events
                    && original.projections == request.projections
                    && original.outbox == request.outbox)
        },
    )?;
    Ok(if committed {
        DurableOutcomeV1::Committed(receipt)
    } else {
        DurableOutcomeV1::Replay(receipt)
    })
}
