//! Independent claim expectations and explicit historical/current verification.
//!
//! Expected addresses come from the caller, never the supplied proof. Signer
//! trust is also a caller policy: resolve the supplied verifying key under the
//! expected tenant and the appropriate historical/current trust policy.

use chrono::{DateTime, Utc};
use ed25519_dalek::VerifyingKey;
use statechronicle_accumulator::key::StateKey;
use statechronicle_core::digest::{ContentDigest, hash_bytes};
use statechronicle_domain::{
    commit::Commit,
    ids::CommitId,
    proof::{
        CommitRef, NonMembershipProofBundle, NonMembershipProofBundleV1, ResourceStateProof,
        ResourceStateProofV1,
    },
    resource::ResourceId,
    resource_state::ResourceState,
    signed::Signed,
    subject::SubjectId,
    tenant::TenantId,
};
use statechronicle_ports::{
    commit_store::{CanonicalHead, CommitReader, CommitReaderV1, CommitStoreError},
    signature_policy::{SignaturePolicy, SignatureUse},
};

use crate::{
    bundle::{check_claim_identity, commit_ref, derive_state_key},
    error::ProofError,
    service::ProofService,
    verify::{verify_bundle, verify_non_membership_bundle, verify_ownership},
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Assertion {
    State { owner: Option<SubjectId> },
    Absence,
}

/// The caller's independently requested address and assertion.
///
/// Constructors derive the exact state key from trusted tenant/resource/holder
/// inputs. They cannot pair one resource's name with another resource's key.
/// Keep these inputs independent of the untrusted proof being checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedClaim {
    tenant: TenantId,
    resource: ResourceId,
    subject: Option<SubjectId>,
    key: StateKey,
    assertion: Assertion,
}

/// Caller-selected pin for one exact v1 commit envelope.
///
/// The digest must come from independently retained commit evidence; deriving
/// it from the proof's own `signed_commit_record` would not pin the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedCommitV1 {
    commit_id: CommitId,
    signed_record_digest: ContentDigest,
}

impl ExpectedCommitV1 {
    /// Pins a commit id and the digest of its independently retained exact envelope.
    pub const fn new(commit_id: CommitId, signed_record_digest: ContentDigest) -> Self {
        Self {
            commit_id,
            signed_record_digest,
        }
    }

    /// Independently expected canonical commit id.
    pub const fn commit_id(&self) -> &CommitId {
        &self.commit_id
    }

    /// Digest of the independently retained exact signed envelope bytes.
    pub const fn signed_record_digest(&self) -> &ContentDigest {
        &self.signed_record_digest
    }
}

impl ExpectedClaim {
    /// Requests present resource-keyed state, without asserting its owner.
    pub fn state(tenant: TenantId, resource: ResourceId) -> Self {
        Self::address(tenant, resource, None, Assertion::State { owner: None })
    }

    /// Requests resource-keyed state owned by an independently expected subject.
    pub fn ownership(tenant: TenantId, resource: ResourceId, owner: SubjectId) -> Self {
        Self::address(
            tenant,
            resource,
            None,
            Assertion::State { owner: Some(owner) },
        )
    }

    /// Requests present state at the independently expected holder's address.
    pub fn subject_state(tenant: TenantId, resource: ResourceId, subject: SubjectId) -> Self {
        Self::address(
            tenant,
            resource,
            Some(subject),
            Assertion::State { owner: None },
        )
    }

    /// Requests absence at the independently expected resource address.
    pub fn absence(tenant: TenantId, resource: ResourceId) -> Self {
        Self::address(tenant, resource, None, Assertion::Absence)
    }

    /// Requests absence at the independently expected holder's address.
    pub fn subject_absence(tenant: TenantId, resource: ResourceId, subject: SubjectId) -> Self {
        Self::address(tenant, resource, Some(subject), Assertion::Absence)
    }

    fn address(
        tenant: TenantId,
        resource: ResourceId,
        subject: Option<SubjectId>,
        assertion: Assertion,
    ) -> Self {
        let key = match &subject {
            Some(holder) => StateKey::for_subject_held(&tenant.0, &resource.0, &holder.0),
            None => StateKey::for_resource(&tenant.0, &resource.0),
        };
        Self {
            tenant,
            resource,
            subject,
            key,
            assertion,
        }
    }

    /// Independently expected tenant.
    pub const fn tenant(&self) -> &TenantId {
        &self.tenant
    }
    /// Independently expected resource.
    pub const fn resource(&self) -> &ResourceId {
        &self.resource
    }
    /// Independently expected holder for subject-keyed state.
    pub const fn subject(&self) -> Option<&SubjectId> {
        self.subject.as_ref()
    }
    /// Independently expected owner, when ownership was requested.
    pub const fn owner(&self) -> Option<&SubjectId> {
        match &self.assertion {
            Assertion::State { owner } => owner.as_ref(),
            Assertion::Absence => None,
        }
    }
    /// Exact independently derived sparse-Merkle key.
    pub const fn key(&self) -> StateKey {
        self.key
    }
    /// Whether the requested assertion is absence rather than present state.
    pub const fn is_absence(&self) -> bool {
        matches!(self.assertion, Assertion::Absence)
    }

    fn check(&self, proof: ClaimProof<'_>) -> Result<(), ProofError> {
        let (tenant, resource) = match proof {
            ClaimProof::State(state) => (&state.tenant_id, &state.resource_id),
            ClaimProof::Absence(absence) => (&absence.tenant_id, &absence.resource_id),
        };
        check_claim_identity(tenant, resource, &self.tenant, &self.resource)?;
        match (proof, &self.assertion) {
            (ClaimProof::State(state), Assertion::State { .. }) => {
                let key = derive_state_key(state)?;
                if key != self.key {
                    return Err(ProofError::KeyMismatch {
                        expected: self.key.to_string(),
                        actual: key.to_string(),
                    });
                }
            }
            (ClaimProof::Absence(absence), Assertion::Absence) => {
                let key = StateKey::new(*absence.claimed_key.as_bytes());
                if key != self.key {
                    return Err(ProofError::KeyMismatch {
                        expected: self.key.to_string(),
                        actual: key.to_string(),
                    });
                }
            }
            _ => {
                return Err(ProofError::InvalidState(String::from(
                    "proof kind does not match the independently expected claim",
                )));
            }
        }
        Ok(())
    }
}

/// Untrusted evidence offered for the independently expected claim.
#[derive(Debug, Clone, Copy)]
pub enum ClaimProof<'proof> {
    /// Present state or ownership evidence.
    State(&'proof ResourceStateProof),
    /// Absence evidence.
    Absence(&'proof NonMembershipProofBundle),
}

/// Untrusted v1 proof for a caller-selected claim.
#[derive(Debug, Clone, Copy)]
pub enum ClaimProofV1<'proof> {
    /// Present state or ownership evidence.
    State(&'proof ResourceStateProofV1),
    /// Absence evidence.
    Absence(&'proof NonMembershipProofBundleV1),
}

impl<'proof> ClaimProofV1<'proof> {
    const fn embedded_commit_id(self) -> &'proof CommitId {
        match self {
            Self::State(p) => &p.commit.commit_id,
            Self::Absence(p) => &p.commit.commit_id,
        }
    }
    const fn state(self) -> Option<&'proof ResourceState> {
        match self {
            Self::State(p) => Some(&p.claimed_state),
            Self::Absence(_) => None,
        }
    }
}

impl<'proof> ClaimProof<'proof> {
    const fn commit(self) -> &'proof CommitRef {
        match self {
            Self::State(proof) => &proof.commit,
            Self::Absence(proof) => &proof.commit,
        }
    }

    const fn state(self) -> Option<&'proof ResourceState> {
        match self {
            Self::State(proof) => Some(&proof.claimed_state),
            Self::Absence(_) => None,
        }
    }
}

/// Temporal scope of a successful verification, pinned by its commit reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationPoint {
    /// Verified at the caller's independently selected historical commit.
    Historical,
    /// Matched the authoritative head at the final linearizable head read.
    /// It does not promise that the head remains unchanged after that read.
    Current,
}

/// A claim established by verification, with no public or deserializing constructor.
///
/// The commit reference is the verified root/sequence/identity. Even a current
/// result is pinned to that observation; retaining it does not preserve freshness.
///
/// ```compile_fail,E0451
/// use statechronicle_proof::claim::{ExpectedClaim, VerifiedClaim, VerificationPoint};
/// use statechronicle_domain::proof::CommitRef;
/// fn fabricate(expectation: ExpectedClaim, commit: CommitRef) -> VerifiedClaim {
///     VerifiedClaim { expectation, commit, point: VerificationPoint::Current, state: None }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedClaim {
    expectation: ExpectedClaim,
    commit: CommitRef,
    point: VerificationPoint,
    state: Option<ResourceState>,
}

/// Claim verified against an exact v1 signed-envelope pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedClaimV1 {
    expectation: ExpectedClaim,
    commit: ExpectedCommitV1,
    point: VerificationPoint,
    state: Option<ResourceState>,
}

impl VerifiedClaimV1 {
    /// Independently requested claim.
    pub const fn expectation(&self) -> &ExpectedClaim {
        &self.expectation
    }
    /// Exact stored envelope pin used for verification.
    pub const fn commit(&self) -> &ExpectedCommitV1 {
        &self.commit
    }
    /// Current or historical verification scope.
    pub const fn point(&self) -> VerificationPoint {
        self.point
    }
    /// Verified state payload, or `None` for absence.
    pub const fn state(&self) -> Option<&ResourceState> {
        self.state.as_ref()
    }
}

impl VerifiedClaim {
    /// The exact independently supplied claim that was established.
    pub const fn expectation(&self) -> &ExpectedClaim {
        &self.expectation
    }
    /// The signed commit reference to which the verified claim is pinned.
    pub const fn commit(&self) -> &CommitRef {
        &self.commit
    }
    /// Whether verification established historical or observed-current state.
    pub const fn point(&self) -> VerificationPoint {
        self.point
    }
    /// Verified state payload, or `None` for an absence claim.
    /// This owned copy remains bound to verification if the input is changed.
    pub const fn state(&self) -> Option<&ResourceState> {
        self.state.as_ref()
    }
}

impl<R: CommitReader + ?Sized> Verification<'_, R> {
    async fn verify_current(
        &self,
        proof: ClaimProof<'_>,
        expected: &ExpectedClaim,
        trusted_key: &VerifyingKey,
    ) -> Result<VerifiedClaim, ProofError> {
        let (signed, commit) = self.verify_expected(proof, expected, trusted_key).await?;
        ensure_canonical(self.reader, expected.tenant(), &signed).await?;
        Ok(VerifiedClaim {
            expectation: expected.clone(),
            commit,
            point: VerificationPoint::Current,
            state: proof.state().cloned(),
        })
    }

    async fn verify_historical(
        &self,
        proof: ClaimProof<'_>,
        expected: &ExpectedClaim,
        pinned_commit: &CommitId,
        trusted_key: &VerifyingKey,
    ) -> Result<VerifiedClaim, ProofError> {
        if &proof.commit().commit_id != pinned_commit {
            return Err(ProofError::CommitRefMismatch {
                expected: pinned_commit.0.clone(),
                actual: proof.commit().commit_id.0.clone(),
            });
        }
        let (_, commit) = self.verify_expected(proof, expected, trusted_key).await?;
        Ok(VerifiedClaim {
            expectation: expected.clone(),
            commit,
            point: VerificationPoint::Historical,
            state: proof.state().cloned(),
        })
    }

    async fn verify_expected(
        &self,
        proof: ClaimProof<'_>,
        expected: &ExpectedClaim,
        trusted_key: &VerifyingKey,
    ) -> Result<(Signed<Commit>, CommitRef), ProofError> {
        expected.check(proof)?;
        let signed = require_commit(
            self.reader
                .read_commit(expected.tenant(), &proof.commit().commit_id)
                .await,
        )?;
        if proof.commit().signature != signed.signature {
            return Err(ProofError::CommitSignature(String::from(
                "proof commit signature does not match the stored signed commit",
            )));
        }
        let reference = commit_ref(&signed)?;
        match proof {
            ClaimProof::State(state) => {
                verify_bundle(state, &signed, trusted_key, &expected.key)?;
                if let Some(owner) = expected.owner() {
                    verify_ownership(state, &owner.0)?;
                }
            }
            ClaimProof::Absence(absence) => {
                verify_non_membership_bundle(absence, &signed, trusted_key, &expected.key)?
            }
        }
        Ok((signed, reference))
    }
}

/// Verify supplied claims using only immutable commit lookup and optional head reads.
///
/// No proof index, projection index, snapshot store or commit writer is required.
/// All expectation, signature, envelope, root and final head checks share the
/// implementation used by [`ProofService`]. Caller-selected keys must already
/// satisfy independent tenant/time/revocation policy. A valid signature alone
/// does not establish that policy or physical truth. Host read implementations
/// must not persist data or dispatch effects; no sandbox is implied.
///
/// ```compile_fail
/// # fn check(verifier: statechronicle_proof::claim::ClaimVerifier) {
/// verifier.put_commit(todo!(), todo!());
/// # }
/// ```
pub struct ClaimVerifier {
    reader: Box<dyn CommitReader>,
}

/// Read-only typed service for exact v1 commit records.
pub struct ClaimVerifierV1 {
    reader: Box<dyn CommitReaderV1>,
}

impl ClaimVerifier {
    /// Compose a read-only claim verifier; no permissive signer policy is supplied.
    pub fn new(reader: Box<dyn CommitReader>) -> Self {
        Self { reader }
    }
}

impl ClaimVerifierV1 {
    /// Compose a verifier over exact v1 commit records.
    pub fn new(reader: Box<dyn CommitReaderV1>) -> Self {
        Self { reader }
    }

    /// Verify a v1 claim at current canonical head using an independently
    /// configured tenant commit authority and active signature policy.
    ///
    /// # Errors
    /// Returns claim, record, signature, state-root, storage, or canonical-head errors.
    pub async fn verify_current_v1(
        &self,
        proof: ClaimProofV1<'_>,
        expected: &ExpectedClaim,
        commit_authority: &SubjectId,
        policy: &dyn SignaturePolicy,
        now: DateTime<Utc>,
    ) -> Result<VerifiedClaimV1, ProofError> {
        let reader = self.reader.as_ref();
        expected.check_v1(proof)?;
        let head = reader
            .read_canonical_head_v1(expected.tenant())
            .await
            .map_err(|e| ProofError::Store(e.to_string()))?
            .ok_or(ProofError::CanonicalHeadUnavailable)?;
        if proof.embedded_commit_id() != &head.commit_id {
            return Err(ProofError::CommitRefMismatch {
                expected: head.commit_id.0.clone(),
                actual: proof.embedded_commit_id().0.clone(),
            });
        }
        let record = read_v1_record(reader, expected.tenant(), &head.commit_id).await?;
        let pin = ExpectedCommitV1::new(head.commit_id.clone(), hash_bytes(&record));
        let signed = verify_v1(
            proof,
            expected,
            &pin,
            commit_authority,
            policy,
            now,
            SignatureUse::Active,
        )
        .await?;
        ensure_canonical_v1(reader, expected.tenant(), &head, &signed).await?;
        Ok(VerifiedClaimV1 {
            expectation: expected.clone(),
            commit: pin,
            point: VerificationPoint::Current,
            state: proof.state().cloned(),
        })
    }

    /// Verify a v1 claim at an independently selected historical commit.
    /// The digest pin is always computed from the exact record loaded by the
    /// reader under the expected tenant and caller-supplied commit id.
    ///
    /// # Errors
    /// Returns claim, pin, record, signature, state-root, or storage errors.
    #[allow(clippy::too_many_arguments)]
    pub async fn verify_historical_v1(
        &self,
        proof: ClaimProofV1<'_>,
        expected: &ExpectedClaim,
        pinned_commit: &CommitId,
        commit_authority: &SubjectId,
        policy: &dyn SignaturePolicy,
        now: DateTime<Utc>,
    ) -> Result<VerifiedClaimV1, ProofError> {
        let reader = self.reader.as_ref();
        expected.check_v1(proof)?;
        if proof.embedded_commit_id() != pinned_commit {
            return Err(ProofError::CommitRefMismatch {
                expected: pinned_commit.0.clone(),
                actual: proof.embedded_commit_id().0.clone(),
            });
        }
        let record = read_v1_record(reader, expected.tenant(), pinned_commit).await?;
        let pin = ExpectedCommitV1::new(pinned_commit.clone(), hash_bytes(&record));
        let _signed = verify_v1(
            proof,
            expected,
            &pin,
            commit_authority,
            policy,
            now,
            SignatureUse::Historical,
        )
        .await?;
        Ok(VerifiedClaimV1 {
            expectation: expected.clone(),
            commit: pin,
            point: VerificationPoint::Historical,
            state: proof.state().cloned(),
        })
    }
}

impl ClaimVerifier {
    /// Verifies an independent state, ownership or absence claim at current head.
    ///
    /// The commit is loaded under the expected tenant, then signature, key,
    /// state and claim are verified before one final canonical-head read. The
    /// adapter must provide immutable commit lookups and a linearizable head
    /// read (see [`statechronicle_ports::commit_store::CommitStore`]). A head
    /// advance before that read rejects the proof; an advance afterward cannot
    /// change the returned result's explicit commit pin.
    ///
    /// `trusted_key` must come from independent signer policy for the expected
    /// tenant; cryptographic validity alone does not establish key authorization.
    /// This does not authenticate unsigned event or trade-summary metadata.
    ///
    /// # Errors
    /// Returns expectation/kind, store, signature and root verification errors;
    /// missing head support fails with [`ProofError::CanonicalHeadUnavailable`]
    /// and a different head with [`ProofError::NonCanonicalCommit`].
    pub async fn verify_current(
        &self,
        proof: ClaimProof<'_>,
        expected: &ExpectedClaim,
        trusted_key: &VerifyingKey,
    ) -> Result<VerifiedClaim, ProofError> {
        Verification {
            reader: self.reader.as_ref(),
        }
        .verify_current(proof, expected, trusted_key)
        .await
    }
    /// Verifies a claim at an independently selected historical commit.
    ///
    /// No head lookup or freshness guarantee is implied. Resolve `trusted_key`
    /// independently under the expected tenant's historical signer policy.
    ///
    /// # Errors
    /// Returns [`ProofError::CommitRefMismatch`] for a different requested pin,
    /// or the expectation, store, signature and root errors of current checking.
    pub async fn verify_historical(
        &self,
        proof: ClaimProof<'_>,
        expected: &ExpectedClaim,
        pinned_commit: &CommitId,
        trusted_key: &VerifyingKey,
    ) -> Result<VerifiedClaim, ProofError> {
        Verification {
            reader: self.reader.as_ref(),
        }
        .verify_historical(proof, expected, pinned_commit, trusted_key)
        .await
    }
}

impl ExpectedClaim {
    fn check_v1(&self, proof: ClaimProofV1<'_>) -> Result<(), ProofError> {
        let (tenant, resource, absent_key) = match proof {
            ClaimProofV1::State(p) => (&p.tenant_id, &p.resource_id, None),
            ClaimProofV1::Absence(p) => (
                &p.tenant_id,
                &p.resource_id,
                Some(StateKey::new(*p.claimed_key.as_bytes())),
            ),
        };
        check_claim_identity(tenant, resource, &self.tenant, &self.resource)?;
        if let Some(key) = absent_key
            && key != self.key
        {
            return Err(ProofError::KeyMismatch {
                expected: self.key.to_string(),
                actual: key.to_string(),
            });
        }
        match (proof, &self.assertion) {
            (ClaimProofV1::State(p), Assertion::State { .. }) => {
                let derived_key = crate::bundle::derive_state_key_from_parts(
                    &p.tenant_id,
                    &p.resource_id,
                    &p.claimed_state,
                )?;
                if derived_key != self.key {
                    return Err(ProofError::KeyMismatch {
                        expected: self.key.to_string(),
                        actual: derived_key.to_string(),
                    });
                }
            }
            (ClaimProofV1::Absence(_), Assertion::Absence) => {}
            _ => {
                return Err(ProofError::InvalidState(String::from(
                    "proof kind does not match the independently expected claim",
                )));
            }
        }
        Ok(())
    }
}

async fn read_v1_record(
    reader: &dyn CommitReaderV1,
    tenant: &TenantId,
    commit: &CommitId,
) -> Result<Vec<u8>, ProofError> {
    let record = reader
        .read_commit_record_v1(tenant, commit)
        .await
        .map_err(|error| ProofError::Store(error.to_string()))?
        .ok_or(ProofError::NotFound)?;
    crate::verify::check_v1_envelope_size(&record)?;
    Ok(record)
}

#[allow(clippy::too_many_arguments)]
async fn verify_v1(
    proof: ClaimProofV1<'_>,
    expected: &ExpectedClaim,
    pin: &ExpectedCommitV1,
    authority: &SubjectId,
    policy: &dyn SignaturePolicy,
    now: DateTime<Utc>,
    usage: SignatureUse,
) -> Result<Commit, ProofError> {
    match proof {
        ClaimProofV1::State(p) => {
            crate::verify::verify_resource_state_v1_with_use(
                p, expected, pin, authority, policy, now, usage,
            )
            .await
        }
        ClaimProofV1::Absence(p) => {
            crate::verify::verify_non_membership_v1_with_use(
                p, expected, pin, authority, policy, now, usage,
            )
            .await
        }
    }
}

async fn ensure_canonical_v1(
    reader: &dyn CommitReaderV1,
    tenant: &TenantId,
    observed: &CanonicalHead,
    signed: &Commit,
) -> Result<(), ProofError> {
    let final_head = reader
        .read_canonical_head_v1(tenant)
        .await
        .map_err(|e| ProofError::Store(e.to_string()))?
        .ok_or(ProofError::CanonicalHeadUnavailable)?;
    if &final_head != observed
        || final_head.commit_id != signed.commit_id
        || final_head.sequence != signed.sequence
        || final_head.state_root != signed.next_state_root
    {
        return Err(ProofError::NonCanonicalCommit {
            tenant: tenant.0.clone(),
            commit_id: signed.commit_id.0.clone(),
        });
    }
    Ok(())
}

impl ProofService {
    /// Compatibility entry point sharing [`ClaimVerifier::verify_current`] checks.
    ///
    /// # Errors
    /// Returns the same expectation, store, signature, root and temporal-scope errors.
    pub async fn verify_current(
        &self,
        proof: ClaimProof<'_>,
        expected: &ExpectedClaim,
        trusted_key: &VerifyingKey,
    ) -> Result<VerifiedClaim, ProofError> {
        Verification {
            reader: self.ports().commit_store.as_ref(),
        }
        .verify_current(proof, expected, trusted_key)
        .await
    }
    /// Compatibility entry point sharing [`ClaimVerifier::verify_historical`] checks.
    ///
    /// # Errors
    /// Returns the same expectation, store, signature, root and temporal-scope errors.
    pub async fn verify_historical(
        &self,
        proof: ClaimProof<'_>,
        expected: &ExpectedClaim,
        pinned_commit: &CommitId,
        trusted_key: &VerifyingKey,
    ) -> Result<VerifiedClaim, ProofError> {
        Verification {
            reader: self.ports().commit_store.as_ref(),
        }
        .verify_historical(proof, expected, pinned_commit, trusted_key)
        .await
    }
}

struct Verification<'read, R: ?Sized> {
    reader: &'read R,
}

pub(crate) fn require_commit(
    result: Result<Option<Signed<Commit>>, CommitStoreError>,
) -> Result<Signed<Commit>, ProofError> {
    result
        .map_err(|error| ProofError::Store(error.to_string()))?
        .ok_or(ProofError::NotFound)
}

pub(crate) async fn ensure_canonical<R: CommitReader + ?Sized>(
    reader: &R,
    tenant: &TenantId,
    signed: &Signed<Commit>,
) -> Result<(), ProofError> {
    let head = reader
        .read_canonical_head(tenant)
        .await
        .map_err(|err| ProofError::Store(err.to_string()))?
        .ok_or(ProofError::CanonicalHeadUnavailable)?;
    if head.commit_id != signed.body.commit_id
        || head.sequence != signed.body.sequence
        || head.state_root != signed.body.next_state_root
    {
        return Err(ProofError::NonCanonicalCommit {
            tenant: tenant.0.clone(),
            commit_id: signed.body.commit_id.0.clone(),
        });
    }
    Ok(())
}
