#[path = "common/mod.rs"]
#[allow(dead_code)]
mod common;
#[path = "ledger.rs"]
#[allow(dead_code)]
mod ledger;

use ledger::{Allow, MemoryLedger, Step, Verify};
use statechronicle::commit::error::CommitError;
use statechronicle::commit::persist::{CommittedEvent, projections_for};
use statechronicle::commit::sign::CommitSigner;
use statechronicle::core::digest::hash_bytes;
use statechronicle::core::signature::{Signature, sign};
use statechronicle::domain::intent::{Intent, Nonce};
use statechronicle::mutation::{
    CommittedMutation, DurableError, DurableOutcome, RecoveryOutcome, SignedMutation, recover,
};
use statechronicle::ports::authorization::{AuthenticatedPrincipal, DenyAllAuthorizer};
use statechronicle::ports::key_registry::{KeyRecord, KeyStatus, MemoryKeyRegistry};
use statechronicle::ports::ledger_store::OutboxRecord;
use statechronicle::{IntentId, KeyId, Operation, ResourceId, StateType, SubjectId};
use std::future::Future;
use std::task::{Context, Poll, Waker};

struct Signer;
impl CommitSigner for Signer {
    fn sign_bytes(&self, _: &KeyId, bytes: &[u8]) -> Result<Signature, CommitError> {
        Ok(sign(bytes, &common::fixed_key()))
    }
}
fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("non-yielding fixture unexpectedly pending"),
    }
}
fn counts(ledger: &MemoryLedger) -> [usize; 6] {
    let data = ledger.data.lock().unwrap();
    [
        data.events.len(),
        data.commits.len(),
        data.projections.len(),
        data.outbox.len(),
        data.identities.len(),
        data.mutations.len(),
    ]
}

fn main() {
    let data: &[u8] = &[8, 1, 2, 3, 4];
    let byte = |index| data.get(index).copied().unwrap_or(0);
    let harness = common::Harness::new();
    let ledger = MemoryLedger::default();
    let principal = AuthenticatedPrincipal {
        subject: SubjectId(String::from("account:alice")),
        tenant: harness.tenant(),
        credential_id: String::from("fuzz-session"),
    };
    let intent = Intent::builder()
        .tenant(harness.tenant())
        .intent_id(IntentId::new(format!("int_fuzz_{}", byte(2))).unwrap())
        .operation(Operation::from_static("asset.mint"))
        .actor(principal.subject.clone())
        .resource(ResourceId(format!("asset:fuzz:{}", byte(3))))
        .state_type(StateType::UniqueAsset)
        .expected_version(0)
        .inputs(
            [(
                String::from("to_owner"),
                serde_json::json!(format!("account:owner:{}", byte(4))),
            )]
            .into(),
        )
        .created_at(harness.now())
        .nonce(
            Nonce::from_bytes(if data.is_empty() {
                vec![0]
            } else {
                data.iter().copied().take(64).collect()
            })
            .unwrap(),
        )
        .build()
        .unwrap();
    let request = harness.sign(intent, None);
    let registry = MemoryKeyRegistry::new();
    registry
        .register(KeyRecord {
            key_id: harness.key_id(),
            tenant: harness.tenant(),
            subject: Some(principal.subject.clone()),
            operations: vec![request.intent.operation.clone()],
            valid_from: harness.now(),
            valid_until: None,
            status: KeyStatus::Active,
        })
        .unwrap();
    let prepared = ready(
        harness
            .executor
            .prepare_player(&request, &principal, &Allow, &registry),
    )
    .unwrap();
    let (commit, _) = harness.commit_events(prepared.events());
    let events = prepared.events().to_vec();
    let entries: Vec<_> = events
        .iter()
        .map(|event| CommittedEvent {
            event,
            state_type: event.after.state.state_type(),
        })
        .collect();
    let projections = projections_for(&commit.body.commit_id, &entries);
    let outbox: Vec<_> = (0..=byte(1) % 3)
        .map(|index| {
            let payload = [
                data.iter().copied().take(64).collect::<Vec<_>>(),
                vec![index],
            ]
            .concat();
            OutboxRecord {
                delivery_key: format!("fuzz-delivery-{index}"),
                tenant: harness.tenant(),
                commit_id: commit.body.commit_id.clone(),
                payload_digest: hash_bytes(&payload),
                payload,
            }
        })
        .collect();
    let signed = SignedMutation::sign(
        prepared,
        &commit.body,
        harness.key_id(),
        &Signer,
        outbox.clone(),
    )
    .unwrap();

    let only = std::env::args().nth(1);
    let allocation = only.as_ref().is_some_and(|s| s.starts_with("alloc-"));
    let selected = only.as_deref().map(|s| s.strip_prefix("alloc-").unwrap_or(s));
    let mut output = Vec::new();
    let mut run = |name: &str, action: &mut dyn FnMut()| {
        if selected.is_some_and(|s| s != name) { return; }
        if allocation {
            for _ in 0..1000 { action(); }
            return;
        }
        for _ in 0..10 { action(); }
        let mut samples = Vec::with_capacity(1000);
        for _ in 0..1000 {
            let start = std::time::Instant::now();
            action();
            samples.push(start.elapsed().as_nanos() as u64);
        }
        samples.sort_unstable();
        output.push(serde_json::json!({"workload":name,"sample_count":samples.len(),
            "median_ns":samples[500],"p95_ns":samples[949],"p99_ns":samples[989],"samples_ns":samples});
    };
    run("control", &mut || { std::hint::black_box(1u8); });
    run("durable_commit_including_empty_adapter", &mut || {
        let fresh = MemoryLedger::default();
        assert!(matches!(ready(signed.commit(&fresh, &Allow, &Verify)).unwrap(), DurableOutcome::Committed(_)));
        assert_eq!(counts(&fresh), [1,1,1,outbox.len(),1,1]);
    });
    assert!(matches!(ready(signed.commit(&ledger, &Allow, &Verify)).unwrap(), DurableOutcome::Committed(_)));
    run("durable_replay", &mut || {
        assert!(matches!(ready(signed.commit(&ledger, &Allow, &Verify)).unwrap(), DurableOutcome::Replay(_)));
    });
    run("durable_recovery", &mut || {
        assert!(matches!(ready(recover(&ledger, &Allow, &Verify, &principal, &request.intent)).unwrap(), RecoveryOutcome::Committed(_)));
    });
    run("durable_denied", &mut || {
        assert!(ready(signed.commit(&ledger, &DenyAllAuthorizer, &Verify)).is_err());
    });
    let mut conflict = request.intent.clone();
    conflict.nonce = Nonce::from_bytes(vec![255]).unwrap();
    run("durable_conflicting_identity", &mut || {
        assert!(matches!(ready(recover(&ledger, &Allow, &Verify, &principal, &conflict)), Err(DurableError::PayloadConflict)));
    });
    run("durable_unknown_and_recovery", &mut || {
        let fresh = MemoryLedger { lose_commit_response: true, ..MemoryLedger::default() };
        assert!(matches!(ready(signed.commit(&fresh, &Allow, &Verify)), Err(DurableError::Unknown { .. })));
        assert!(matches!(ready(recover(&fresh, &Allow, &Verify, &principal, &request.intent)).unwrap(), RecoveryOutcome::Committed(_)));
        assert_eq!(counts(&fresh), [1,1,1,outbox.len(),1,1]);
    });
    run("durable_eight_concurrent_exact_retries_including_threads", &mut || {
        let fresh = MemoryLedger::default();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let fresh = &fresh;
                let signed = &signed;
                scope.spawn(move || { let _ = ready(signed.commit(fresh, &Allow, &Verify)); });
            }
        });
        assert!(matches!(ready(recover(&fresh, &Allow, &Verify, &principal, &request.intent)).unwrap(), RecoveryOutcome::Committed(_)));
        assert_eq!(counts(&fresh), [1,1,1,outbox.len(),1,1]);
    });
    claims::bench(&mut run);
    if !allocation { println!("{}", serde_json::json!({"workloads":output}); }
}

#[allow(dead_code, unused_imports)]
mod claims {
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
    serde_json::from_str(include_str!("history.json")).unwrap()
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
            if self.fail_head {
                return Err(CommitStoreError::Unavailable(String::from("head read")));
            }
            Ok(self.head.lock().unwrap().clone())
        })
    }
}


pub fn bench(run: &mut impl FnMut(&str, &mut dyn FnMut())) {
    let original = corpus();
    let reader = Reads::new(&original.signed_commit);
    let verifier = ClaimVerifier::new(Box::new(reader.clone()));
    let key = SigningKey::from_bytes(&[42; 32]).verifying_key();
    let ownership = expected();
    let absent = ExpectedClaim::absence(ownership.tenant().clone(), ResourceId(String::from("asset:absent")));
    run("claim_current_ownership", &mut || {
        assert_eq!(super::ready(verifier.verify_current(ClaimProof::State(&original.state_proof), &ownership, &key)).unwrap().point(), VerificationPoint::Current);
    });
    run("claim_historical_ownership", &mut || {
        assert_eq!(super::ready(verifier.verify_historical(ClaimProof::State(&original.state_proof), &ownership, &original.signed_commit.body.commit_id, &key)).unwrap().point(), VerificationPoint::Historical);
    });
    run("claim_current_absence", &mut || {
        assert!(super::ready(verifier.verify_current(ClaimProof::Absence(&original.absence_proof), &absent, &key)).is_ok());
    });
    let bad = SigningKey::from_bytes(&[43; 32]).verifying_key();
    run("claim_wrong_trusted_key", &mut || {
        assert!(super::ready(verifier.verify_current(ClaimProof::State(&original.state_proof), &ownership, &bad)).is_err());
    });
    *reader.head.lock().unwrap() = None;
    run("claim_missing_canonical_head", &mut || {
        assert!(super::ready(verifier.verify_current(ClaimProof::State(&original.state_proof), &ownership, &key)).is_err());
    });
}

}
