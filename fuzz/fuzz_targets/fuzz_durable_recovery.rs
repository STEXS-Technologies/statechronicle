#![no_main]
//! Public protected single-mutation/recovery probes with a dev-only atomic fake.
//! Allow is an explicit test policy; no production permissive policy is added.
#[path = "../../crates/statechronicle/tests/common/mod.rs"]
#[allow(dead_code)]
mod common;
#[path = "../../crates/statechronicle/tests/support/durable.rs"]
#[allow(dead_code)]
mod ledger;

use ledger::{Allow, MemoryLedger, Step, Verify};
use libfuzzer_sys::fuzz_target;
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

fuzz_target!(|data: &[u8]| {
    let byte = |index| data.get(index).copied().unwrap_or(0);
    let mode = byte(0) % 10;
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
    assert_eq!(counts(&ledger), [0; 6], "durable.prepare.no_effects");
    let check_receipt = |receipt: &CommittedMutation| {
        assert_eq!(receipt.commit(), &commit);
        assert_eq!(receipt.record().events, events);
        assert_eq!(receipt.record().projections, projections);
        assert_eq!(receipt.record().outbox, outbox);
        assert_eq!(
            receipt.identity().payload_digest(),
            signed.prepared().payload_digest()
        );
        assert_eq!(receipt.identity().intent_id(), &request.intent.intent_id);
        assert_eq!(receipt.identity().tenant(), &harness.tenant());
    };
    // Reauthorization is required even for an already prepared and signed plan.
    assert!(ready(signed.commit(&ledger, &DenyAllAuthorizer, &Verify)).is_err());
    assert_eq!(counts(&ledger), [0; 6]);
    let steps = [
        Step::Begin,
        Step::Claim,
        Step::Events,
        Step::CommitRow,
        Step::Projection,
        Step::Outbox,
        Step::Finalize,
        Step::Commit,
    ];
    let mut faulted = ledger.clone();
    if mode < 8 {
        faulted.faults.push(steps[usize::from(mode)]);
    }
    if mode < 8 && byte(1) & 1 != 0 {
        faulted.faults.push(Step::Rollback);
    }
    faulted.lose_commit_response = mode == 9;
    let attempted = ready(signed.commit(&faulted, &Allow, &Verify));
    if mode < 8 {
        assert!(attempted.is_err(), "durable.fault.no_receipt");
        assert_eq!(
            counts(&ledger),
            [0; 6],
            "durable.rollback.no_partial_effects"
        );
    } else if mode == 9 {
        assert!(
            matches!(attempted, Err(DurableError::Unknown { .. })),
            "durable.ack.unknown"
        );
    } else {
        let DurableOutcome::Committed(receipt) = attempted.unwrap() else {
            panic!("durable.first.committed");
        };
        check_receipt(&receipt);
    }
    // A fresh handle retains the exact atomic authority; it clears injected faults.
    let reopened = MemoryLedger {
        faults: vec![],
        lose_commit_response: false,
        ..faulted.clone()
    };
    let resolve =
        |store: &MemoryLedger| ready(recover(store, &Allow, &Verify, &principal, &request.intent));
    match resolve(&reopened).unwrap() {
        RecoveryOutcome::Committed(receipt) => {
            assert!(mode >= 8);
            check_receipt(&receipt);
        }
        RecoveryOutcome::NotFound(_) => assert!(mode < 8),
        RecoveryOutcome::InProgress { .. } => {
            panic!("fixture transaction must release its reservation")
        }
    }
    let mut unavailable = reopened.clone();
    unavailable.faults = vec![Step::Lookup];
    assert!(
        resolve(&unavailable).is_err(),
        "durable.lookup.unavailable_is_not_receipt"
    );
    let next = ready(signed.commit(&reopened, &Allow, &Verify)).unwrap();
    let receipt = match next {
        DurableOutcome::Committed(receipt) => {
            assert!(mode < 8);
            receipt
        }
        DurableOutcome::Replay(receipt) => {
            assert!(mode >= 8);
            receipt
        }
    };
    check_receipt(&receipt);
    let expected = [1, 1, 1, outbox.len(), 1, 1];
    assert_eq!(counts(&reopened), expected, "durable.retry.exactly_one");
    assert!(ready(signed.commit(&reopened, &DenyAllAuthorizer, &Verify)).is_err());
    assert!(
        ready(recover(
            &reopened,
            &DenyAllAuthorizer,
            &Verify,
            &principal,
            &request.intent
        ))
        .is_err()
    );
    let mut conflict = request.intent.clone();
    let mut nonce = data.iter().copied().take(64).collect::<Vec<_>>();
    if nonce.is_empty() {
        nonce.push(0);
    }
    nonce[0] ^= 255;
    conflict.nonce = Nonce::from_bytes(nonce).unwrap();
    assert!(
        matches!(
            ready(recover(&reopened, &Allow, &Verify, &principal, &conflict)),
            Err(DurableError::PayloadConflict)
        ),
        "durable.retry.conflict"
    );
    let identity = (harness.tenant().0, request.intent.intent_id.0.clone());
    let original = reopened.data.lock().unwrap().mutations[&identity].clone();
    // Every input reaches all corruption classes, with data-dependent contents.
    for corruption in 0..6 {
        let mut corrupt = original.clone();
        match corruption {
            0 => corrupt.payload_digest = hash_bytes(b"wrong-payload"),
            1 => corrupt.events.clear(),
            2 => corrupt.projections.clear(),
            3 => corrupt.outbox[0].payload.push(255),
            4 => corrupt.commit.signature.sig = Signature::from_bytes([0; 64]),
            _ => corrupt.intent = conflict.clone(),
        }
        reopened
            .data
            .lock()
            .unwrap()
            .mutations
            .insert(identity.clone(), corrupt);
        assert!(
            matches!(resolve(&reopened), Err(DurableError::Corrupt(_))),
            "durable.recovery.corrupt_record:{corruption}"
        );
        reopened
            .data
            .lock()
            .unwrap()
            .mutations
            .insert(identity.clone(), original.clone());
    }
    let RecoveryOutcome::Committed(receipt) = resolve(&reopened).unwrap() else {
        panic!("durable.restored.receipt");
    };
    check_receipt(&receipt);
    assert!(matches!(
        ready(signed.commit(&reopened, &Allow, &Verify)).unwrap(),
        DurableOutcome::Replay(_)
    ));
    assert_eq!(
        counts(&reopened),
        expected,
        "durable.replay.no_duplicate_effects"
    );
});
