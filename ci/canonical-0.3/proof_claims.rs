//! Candidate-only external consumer; all StateChronicle imports are public.
//! Expectations are fixed independently of original-version proof payloads.
mod common;
#[path = "../examples/common/proof_bench.rs"]
mod proof_bench;

use serde::Deserialize;
use statechronicle::domain::proof::{NonMembershipProofBundle, ResourceStateProof};
use statechronicle::proof::claim::{ClaimProof, ExpectedClaim, VerificationPoint};
use statechronicle::proof::error::ProofError;
use statechronicle::{Commit, ResourceId, Signed, SubjectId, TenantId};

#[derive(Deserialize)]
struct Corpus {
    signed_commit: Signed<Commit>,
    state_proof: ResourceStateProof,
    absence_proof: NonMembershipProofBundle,
}

#[tokio::test]
async fn original_proofs_establish_only_the_independently_requested_claim() {
    let corpus: Corpus = serde_json::from_str(include_str!("../../../wire/history.json")).unwrap();
    let tenant = TenantId(String::from("acme.game.alpha"));
    let ownership = ExpectedClaim::ownership(
        tenant.clone(),
        ResourceId(String::from("asset:wire")),
        SubjectId(String::from("account:bob")),
    );
    let absent = ExpectedClaim::absence(tenant.clone(), ResourceId(String::from("asset:absent")));
    let current = proof_bench::service(tenant.clone(), &corpus.signed_commit, false);
    let trusted_key = common::fixed_key().verifying_key();
    let verified = current
        .verify_current(
            ClaimProof::State(&corpus.state_proof),
            &ownership,
            &trusted_key,
        )
        .await
        .unwrap();
    assert_eq!(verified.point(), VerificationPoint::Current);
    assert_eq!(verified.expectation(), &ownership);
    assert_eq!(verified.commit(), &corpus.state_proof.commit);
    current
        .verify_current(
            ClaimProof::Absence(&corpus.absence_proof),
            &absent,
            &trusted_key,
        )
        .await
        .unwrap();
    let stale = proof_bench::service(tenant, &corpus.signed_commit, true);
    assert!(matches!(
        stale
            .verify_current(
                ClaimProof::State(&corpus.state_proof),
                &ownership,
                &trusted_key
            )
            .await,
        Err(ProofError::NonCanonicalCommit { .. })
    ));
    let pinned = stale
        .verify_historical(
            ClaimProof::State(&corpus.state_proof),
            &ownership,
            &corpus.signed_commit.body.commit_id,
            &trusted_key,
        )
        .await
        .unwrap();
    assert_eq!(pinned.point(), VerificationPoint::Historical);
    let mut substituted = corpus.absence_proof.clone();
    substituted.resource_id = ownership.resource().clone();
    let wrong_absence =
        ExpectedClaim::absence(ownership.tenant().clone(), ownership.resource().clone());
    assert!(matches!(
        current
            .verify_current(
                ClaimProof::Absence(&substituted),
                &wrong_absence,
                &trusted_key
            )
            .await,
        Err(ProofError::KeyMismatch { .. })
    ));
}
