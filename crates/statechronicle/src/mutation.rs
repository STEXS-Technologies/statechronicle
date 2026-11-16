//! Protected mutation composition: prepare single requests through
//! [`crate::mutation::Preparer::prepare_player`], bind and sign with [`crate::mutation::SignedMutation`],
//! commit atomically, and resolve retries through [`crate::mutation::recover`].
//! Batches use [`crate::mutation::Preparer::prepare_player_batch`] and [`crate::mutation::SignedBatch`];
//! every member is retained and recovered as one complete atomic group.
//!
//! Signing is not durable success. Only the independently verified ledger
//! receipt permits post-commit dispatch. Storage must still enforce current
//! authorization, expected state, canonical head and atomic complete effects.

pub use statechronicle_commit::durable::{
    CommittedMutation, DurableError, DurableOutcome, MutationIdentity, RecoveryOutcome, recover,
};
pub use statechronicle_executor::preparation::{
    PreparationConfig, PreparationPorts, PreparedBatch, PreparedMutation, Preparer,
};

use statechronicle_commit::durable::commit_verified;
use statechronicle_commit::error::CommitError;
use statechronicle_commit::persist::{
    CommittedEvent, DurableCommitRequest, SignedCommitVerifier, projections_for,
    validate_tenant_commit,
};
use statechronicle_commit::roots::event_root;
use statechronicle_commit::sign::{CommitSigner, sign_commit_with_signer};
use statechronicle_domain::commit::Commit;
use statechronicle_domain::intent::KeyId;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_ports::authorization::Authorizer;
use statechronicle_ports::ledger_store::{LedgerRecovery, OutboxRecord};

/// An owned signed plan whose events and principal come only from a protected
/// [`PreparedMutation`]. The commit root/count and tenant are bound
/// before invoking the signer. Projections are derived internally, not supplied
/// separately by a caller. This type is neither a receipt nor proof of freshness.
///
/// Private fields and no deserialization prevent substituting events, payload,
/// principal, projections or outbox after binding. It can be retained across an
/// unknown acknowledgement; reconcile the original identity through [`crate::mutation::recover`]
/// before any retry decision. It grants no authority to dispatch notifications.
///
/// ```compile_fail
/// use statechronicle::mutation::SignedMutation;
/// let forged = SignedMutation { prepared: todo!(), commit: todo!() };
/// ```
#[derive(Debug)]
pub struct SignedMutation {
    prepared: PreparedMutation,
    commit: Signed<Commit>,
    projections: Vec<StateProjection>,
    outbox: Vec<OutboxRecord>,
}

impl SignedMutation {
    /// Bind and sign a commit body covering exactly this prepared event set.
    ///
    /// Build `body` with the existing commit builder from `prepared.events()`
    /// and independently obtained canonical history/state context. Root/head
    /// correctness and concurrency are enforced by the ledger at commit time;
    /// these host-supplied context values are not made authoritative by signing.
    /// `CommitSigner` may use a remote KMS. Its returned signature and signer
    /// trust are independently verified on every commit attempt. Event producer
    /// and commit signer are distinct roles; their identities need not be equal.
    ///
    /// Outbox rows are owned unchanged; integrity/scope/limits are validated by
    /// the shared persistence owner before any ledger operation. Signing itself
    /// creates no idempotency claim, ledger write or notification dispatch.
    ///
    /// # Errors
    ///
    /// Returns structural or prepared-event binding errors before calling the
    /// signer, or the injected signer's error. No durable receipt is issued.
    pub fn sign(
        prepared: PreparedMutation,
        body: &Commit,
        key_id: KeyId,
        signer: &dyn CommitSigner,
        outbox: Vec<OutboxRecord>,
    ) -> Result<Self, CommitError> {
        let tenant = validate_tenant_commit(body)?;
        if *tenant != prepared.validated().intent.tenant_id {
            return Err(CommitError::MixedTenant);
        }
        let count = u64::try_from(prepared.events().len())
            .map_err(|error| CommitError::InvalidEvent(error.to_string()))?;
        if count != body.event_count || event_root(prepared.events())? != body.event_merkle_root {
            return Err(CommitError::EventRootMismatch);
        }
        let commit = sign_commit_with_signer(body, key_id, signer)?;
        let projections = projections_for(&commit.body.commit_id, &entries(&prepared));
        Ok(Self {
            prepared,
            commit,
            projections,
            outbox,
        })
    }

    /// Original protected request and complete uncommitted event set.
    pub const fn prepared(&self) -> &PreparedMutation {
        &self.prepared
    }

    /// Signed commit bound to this event set. Signer trust is checked at commit.
    pub const fn signed_commit(&self) -> &Signed<Commit> {
        &self.commit
    }

    /// Commit this exact plan through one atomic ledger boundary and require
    /// verified original record readback. No replacement events, principal,
    /// digest or projection set can be passed to this method.
    ///
    /// Reauthorizes at commit time, independently checks signer trust, and
    /// preserves verified replay and unknown-acknowledgement semantics. It
    /// returns no notification dispatch capability or unqualified success.
    ///
    /// # Errors
    ///
    /// Returns the shared [`DurableError`] variants. An unknown result requires
    /// original-identity lookup, never a new plan or intent identity.
    pub async fn commit<S: LedgerRecovery + ?Sized>(
        &self,
        ledger: &S,
        authorizer: &dyn Authorizer,
        verifier: &dyn SignedCommitVerifier,
    ) -> Result<DurableOutcome, DurableError> {
        commit_verified(
            ledger,
            authorizer,
            verifier,
            DurableCommitRequest {
                principal: self.prepared.principal(),
                intent: &self.prepared.validated().intent,
                payload_digest: self.prepared.payload_digest(),
                entries: &entries(&self.prepared),
                commit: &self.commit,
                projections: &self.projections,
                outbox: &self.outbox,
            },
        )
        .await
    }
}

fn entries(prepared: &PreparedMutation) -> Vec<CommittedEvent<'_>> {
    prepared
        .events()
        .iter()
        .map(|event| CommittedEvent {
            event,
            state_type: event.after.state.state_type(),
        })
        .collect()
}

pub use statechronicle_commit::durable_batch::{
    BatchIdentity, BatchMember, BatchRecoveryOutcome, CommittedBatch, DurableBatchError,
    DurableBatchOutcome, recover_batch,
};
use statechronicle_commit::durable_batch::{DurableBatchRequest, commit_batch_verified};
use statechronicle_ports::ledger_batch::LedgerBatchRecovery;

/// An owned signed plan for the ENTIRE protected prepared batch. There is no
/// owned leg extraction and no method that commits a successful subset.
/// Signing is not a receipt; atomic group retention and verified original
/// readback are mandatory at the ledger boundary.
///
/// ```compile_fail
/// use statechronicle::mutation::SignedBatch;
/// let forged = SignedBatch { prepared: todo!(), commit: todo!() };
/// ```
#[derive(Debug)]
pub struct SignedBatch {
    prepared: PreparedBatch,
    commit: Signed<Commit>,
    projections: Vec<StateProjection>,
    outbox: Vec<OutboxRecord>,
}
impl SignedBatch {
    /// Bind every prepared event to one signed tenant commit. Host-supplied
    /// history/root context is checked by the ledger at commit time. Event
    /// producer and commit signer remain distinct roles.
    /// # Errors
    /// Refuses scope/root/count mismatches before calling the signer, or
    /// returns the signer's error. No durable effects or claims are made.
    pub fn sign(
        prepared: PreparedBatch,
        body: &Commit,
        key_id: KeyId,
        signer: &dyn CommitSigner,
        outbox: Vec<OutboxRecord>,
    ) -> Result<Self, CommitError> {
        let tenant = validate_tenant_commit(body)?;
        if prepared
            .mutations()
            .iter()
            .any(|member| member.validated().intent.tenant_id != *tenant)
        {
            return Err(CommitError::MixedTenant);
        }
        let events: Vec<_> = prepared.events().cloned().collect();
        let count = u64::try_from(events.len())
            .map_err(|error| CommitError::InvalidEvent(error.to_string()))?;
        if count != body.event_count || event_root(&events)? != body.event_merkle_root {
            return Err(CommitError::EventRootMismatch);
        }
        let commit = sign_commit_with_signer(body, key_id, signer)?;
        let projections = projections_for(&commit.body.commit_id, &batch_entries(&prepared));
        Ok(Self {
            prepared,
            commit,
            projections,
            outbox,
        })
    }
    /// Entire original protected batch; borrowed legs cannot be committed alone.
    pub const fn prepared(&self) -> &PreparedBatch {
        &self.prepared
    }
    /// Signed commit covering the complete event set, not durable success.
    pub const fn signed_commit(&self) -> &Signed<Commit> {
        &self.commit
    }
    /// Reauthorize and atomically commit the whole group. Every member shares
    /// the existing intent idempotency namespace; original group membership
    /// cannot be changed on replay. Retain this plan after any unknown result.
    /// # Errors
    /// Returns refusal, conflict, corruption or explicit unknown with complete
    /// group identity. No successful subset is returned or dispatched.
    pub async fn commit<S: LedgerBatchRecovery + ?Sized>(
        &self,
        ledger: &S,
        authorizer: &dyn Authorizer,
        verifier: &dyn SignedCommitVerifier,
    ) -> Result<DurableBatchOutcome, DurableBatchError> {
        commit_batch_verified(
            ledger,
            authorizer,
            verifier,
            DurableBatchRequest {
                members: &batch_members(&self.prepared),
                entries: &batch_entries(&self.prepared),
                commit: &self.commit,
                projections: &self.projections,
                outbox: &self.outbox,
            },
        )
        .await
    }
    /// Resolve the original complete group without reducing, signing, claiming
    /// or dispatching. A lookup failure does not make an unknown write safe.
    /// # Errors
    /// Reauthorizes every member and rejects partial, substituted or corrupt
    /// evidence; preserves typed policy and storage causes.
    pub async fn recover<S: LedgerBatchRecovery + ?Sized>(
        &self,
        ledger: &S,
        authorizer: &dyn Authorizer,
        verifier: &dyn SignedCommitVerifier,
    ) -> Result<BatchRecoveryOutcome, DurableBatchError> {
        recover_batch(ledger, authorizer, verifier, &batch_members(&self.prepared)).await
    }
}
fn batch_members(prepared: &PreparedBatch) -> Vec<BatchMember<'_>> {
    prepared
        .mutations()
        .iter()
        .map(|member| BatchMember {
            principal: member.principal(),
            intent: &member.validated().intent,
            payload_digest: member.payload_digest(),
        })
        .collect()
}
fn batch_entries(prepared: &PreparedBatch) -> Vec<CommittedEvent<'_>> {
    prepared
        .events()
        .map(|event| CommittedEvent {
            event,
            state_type: event.after.state.state_type(),
        })
        .collect()
}
