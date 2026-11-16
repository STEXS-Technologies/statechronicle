//! Supplied original-version proofs verified through a commit-read-only consumer.
#![allow(clippy::unwrap_used, clippy::panic)]
use std::sync::{Arc, Mutex};

use ed25519_dalek::SigningKey;
use serde::Deserialize;
use statechronicle::domain::proof::{NonMembershipProofBundle, ResourceStateProof};
use statechronicle::ports::commit_store::{
    CanonicalHead, CommitReadFuture, CommitReader, CommitStoreError,
};
use statechronicle::proof::claim::{ClaimProof, ClaimVerifier, ExpectedClaim, VerificationPoint};
use statechronicle::proof::error::ProofError;
use statechronicle::{Commit, CommitId, ResourceId, Signed, SubjectId, TenantId};

#[derive(Deserialize)]
struct Corpus {
    signed_commit: Signed<Commit>,
    state_proof: ResourceStateProof,
    absence_proof: NonMembershipProofBundle,
}
fn corpus() -> Corpus {
    serde_json::from_str(include_str!("../../../ci/wire-0.2/history.json")).unwrap()
}
fn expected() -> ExpectedClaim {
    ExpectedClaim::ownership(
        TenantId(String::from("acme.game.alpha")),
        ResourceId(String::from("asset:wire")),
        SubjectId(String::from("account:bob")),
    )
}

#[derive(Clone)]
struct Reads {
    signed: Signed<Commit>,
    head: Arc<Mutex<Option<CanonicalHead>>>,
    trace: Arc<Mutex<Vec<&'static str>>>,
    fail_commit: bool,
    fail_head: bool,
}
impl Reads {
    fn new(signed: &Signed<Commit>) -> Self {
        Self {
            signed: signed.clone(),
            head: Arc::new(Mutex::new(Some(CanonicalHead {
                commit_id: signed.body.commit_id.clone(),
                sequence: signed.body.sequence,
                state_root: signed.body.next_state_root.clone(),
            }))),
            trace: Arc::default(),
            fail_commit: false,
            fail_head: false,
        }
    }
}
impl CommitReader for Reads {
    fn read_commit<'read>(
        &'read self,
        tenant: &'read TenantId,
        commit: &'read CommitId,
    ) -> CommitReadFuture<'read, Option<Signed<Commit>>> {
        Box::pin(async move {
            self.trace.lock().unwrap().push("commit");
            if self.fail_commit {
                return Err(CommitStoreError::Unavailable(String::from("commit read")));
            }
            Ok(
                (tenant == expected().tenant() && commit == &self.signed.body.commit_id)
                    .then(|| self.signed.clone()),
            )
        })
    }
    fn read_canonical_head<'read>(
        &'read self,
        _: &'read TenantId,
    ) -> CommitReadFuture<'read, Option<CanonicalHead>> {
        Box::pin(async move {
            self.trace.lock().unwrap().push("head");
            if self.fail_head {
                return Err(CommitStoreError::Unavailable(String::from("head read")));
            }
            Ok(self.head.lock().unwrap().clone())
        })
    }
}

#[tokio::test]
async fn reader_only_consumer_verifies_present_and_absent_original_claims() {
    let original = corpus();
    let reader = Reads::new(&original.signed_commit);
    let verifier = ClaimVerifier::new(Box::new(reader.clone()));
    let key = SigningKey::from_bytes(&[42; 32]).verifying_key();
    let ownership = expected();
    let current = verifier
        .verify_current(ClaimProof::State(&original.state_proof), &ownership, &key)
        .await
        .unwrap();
    assert_eq!(current.point(), VerificationPoint::Current);
    assert_eq!(current.expectation(), &ownership);
    assert_eq!(current.state(), Some(&original.state_proof.claimed_state));
    assert_eq!(*reader.trace.lock().unwrap(), ["commit", "head"]);
    reader.trace.lock().unwrap().clear();
    let absent = ExpectedClaim::absence(
        ownership.tenant().clone(),
        ResourceId(String::from("asset:absent")),
    );
    verifier
        .verify_current(ClaimProof::Absence(&original.absence_proof), &absent, &key)
        .await
        .unwrap();
    assert_eq!(*reader.trace.lock().unwrap(), ["commit", "head"]);
    reader.trace.lock().unwrap().clear();
    let historical = verifier
        .verify_historical(
            ClaimProof::State(&original.state_proof),
            &ownership,
            &original.signed_commit.body.commit_id,
            &key,
        )
        .await
        .unwrap();
    assert_eq!(historical.point(), VerificationPoint::Historical);
    assert_eq!(*reader.trace.lock().unwrap(), ["commit"]);
}

#[tokio::test]
async fn current_head_is_required_and_rechecked_while_historical_pin_stays_explicit() {
    let original = corpus();
    let reader = Reads::new(&original.signed_commit);
    let verifier = ClaimVerifier::new(Box::new(reader.clone()));
    let key = SigningKey::from_bytes(&[42; 32]).verifying_key();
    let proof = ClaimProof::State(&original.state_proof);
    let ownership = expected();
    for missing in [false, true] {
        *reader.head.lock().unwrap() = if missing {
            None
        } else {
            Some(CanonicalHead {
                commit_id: CommitId::new(String::from("cmt_later")).unwrap(),
                sequence: 200,
                state_root: original.signed_commit.body.next_state_root.clone(),
            })
        };
        let result = verifier.verify_current(proof, &ownership, &key).await;
        assert!(if missing {
            matches!(result, Err(ProofError::CanonicalHeadUnavailable))
        } else {
            matches!(result, Err(ProofError::NonCanonicalCommit { .. }))
        });
        assert!(
            verifier
                .verify_historical(
                    proof,
                    &ownership,
                    &original.signed_commit.body.commit_id,
                    &key
                )
                .await
                .is_ok()
        );
    }
    reader.trace.lock().unwrap().clear();
    let wrong_pin = CommitId::new(String::from("cmt_wrong_pin")).unwrap();
    assert!(matches!(
        verifier
            .verify_historical(proof, &ownership, &wrong_pin, &key)
            .await,
        Err(ProofError::CommitRefMismatch { .. })
    ));
    assert!(reader.trace.lock().unwrap().is_empty());
}

#[tokio::test]
async fn expectation_and_signature_refusals_precede_final_head_read() {
    let original = corpus();
    let reader = Reads::new(&original.signed_commit);
    let verifier = ClaimVerifier::new(Box::new(reader.clone()));
    let key = SigningKey::from_bytes(&[42; 32]).verifying_key();
    let wrong = ExpectedClaim::state(
        expected().tenant().clone(),
        ResourceId(String::from("asset:other")),
    );
    assert!(
        verifier
            .verify_current(ClaimProof::State(&original.state_proof), &wrong, &key)
            .await
            .is_err()
    );
    assert!(reader.trace.lock().unwrap().is_empty());
    let wrong_signer = SigningKey::from_bytes(&[43; 32]).verifying_key();
    assert!(
        verifier
            .verify_current(
                ClaimProof::State(&original.state_proof),
                &expected(),
                &wrong_signer
            )
            .await
            .is_err()
    );
    assert_eq!(*reader.trace.lock().unwrap(), ["commit"]);
    reader.trace.lock().unwrap().clear();
    let mut substituted = original.absence_proof;
    substituted.resource_id = expected().resource().clone();
    let absence =
        ExpectedClaim::absence(expected().tenant().clone(), expected().resource().clone());
    assert!(matches!(
        verifier
            .verify_current(ClaimProof::Absence(&substituted), &absence, &key)
            .await,
        Err(ProofError::KeyMismatch { .. })
    ));
    assert!(reader.trace.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_reads_are_not_absence_or_verified_current_claims() {
    let original = corpus();
    let key = SigningKey::from_bytes(&[42; 32]).verifying_key();
    let mut reader = Reads::new(&original.signed_commit);
    reader.fail_commit = true;
    let verifier = ClaimVerifier::new(Box::new(reader.clone()));
    assert!(matches!(
        verifier
            .verify_current(ClaimProof::State(&original.state_proof), &expected(), &key)
            .await,
        Err(ProofError::Store(_))
    ));
    assert_eq!(*reader.trace.lock().unwrap(), ["commit"]);
    reader.trace.lock().unwrap().clear();
    reader.fail_commit = false;
    reader.fail_head = true;
    let head_verifier = ClaimVerifier::new(Box::new(reader.clone()));
    assert!(matches!(
        head_verifier
            .verify_current(ClaimProof::State(&original.state_proof), &expected(), &key)
            .await,
        Err(ProofError::Store(_))
    ));
    assert_eq!(*reader.trace.lock().unwrap(), ["commit", "head"]);
    reader.trace.lock().unwrap().clear();
    assert!(
        head_verifier
            .verify_historical(
                ClaimProof::State(&original.state_proof),
                &expected(),
                &original.signed_commit.body.commit_id,
                &key
            )
            .await
            .is_ok()
    );
    assert_eq!(*reader.trace.lock().unwrap(), ["commit"]);
}

struct HistoryOnly(Signed<Commit>);
impl CommitReader for HistoryOnly {
    fn read_commit<'read>(
        &'read self,
        tenant: &'read TenantId,
        commit: &'read CommitId,
    ) -> CommitReadFuture<'read, Option<Signed<Commit>>> {
        Box::pin(async move {
            Ok(
                (tenant == expected().tenant() && commit == &self.0.body.commit_id)
                    .then(|| self.0.clone()),
            )
        })
    }
}

#[tokio::test]
async fn history_only_adapter_requires_no_head_method_and_cannot_prove_current_state() {
    let original = corpus();
    let verifier = ClaimVerifier::new(Box::new(HistoryOnly(original.signed_commit.clone())));
    let key = SigningKey::from_bytes(&[42; 32]).verifying_key();
    let proof = ClaimProof::State(&original.state_proof);
    assert!(
        verifier
            .verify_historical(
                proof,
                &expected(),
                &original.signed_commit.body.commit_id,
                &key
            )
            .await
            .is_ok()
    );
    assert!(matches!(
        verifier.verify_current(proof, &expected(), &key).await,
        Err(ProofError::CanonicalHeadUnavailable)
    ));
}
