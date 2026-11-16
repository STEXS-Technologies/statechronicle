//! Complete protected batch commit/replay/recovery and bounded atomic failures.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod common;
#[path = "support/durable.rs"]
mod ledger;
use async_trait::async_trait;
use common::Harness;
use ledger::{Allow, MemoryLedger, Step, Verify};
use statechronicle::commit::{error::CommitError, sign::CommitSigner};
use statechronicle::core::{
    canonicalize::canonicalize_and_digest,
    signature::{Signature, sign},
};
use statechronicle::domain::intent::{KeyId, Nonce};
use statechronicle::mutation::{
    BatchMember, BatchRecoveryOutcome, DurableBatchOutcome, DurableError, SignedBatch,
    recover_batch,
};
use statechronicle::ports::{
    authorization::{AuthenticatedPrincipal, AuthorizationContext, AuthorizationError, Authorizer},
    key_registry::{KeyRecord, KeyStatus, MemoryKeyRegistry},
    ledger_store::OutboxRecord,
};
use statechronicle::{
    Intent, IntentId, Operation, PlayerBatchItem, ResourceId, StateType, SubjectId,
};
struct Signer;
impl CommitSigner for Signer {
    fn sign_bytes(&self, _: &KeyId, bytes: &[u8]) -> Result<Signature, CommitError> {
        Ok(sign(bytes, &common::fixed_key()))
    }
}
async fn plan(names: &[&str], nonce: u8) -> SignedBatch {
    plan_after(names, nonce, None).await
}
async fn plan_after(names: &[&str], nonce: u8, previous: Option<&SignedBatch>) -> SignedBatch {
    let h = Harness::new();
    let principal = AuthenticatedPrincipal {
        subject: SubjectId(String::from("account:alice")),
        tenant: h.tenant(),
        credential_id: String::from("test-session"),
    };
    let requests: Vec<_> = names
        .iter()
        .map(|name| {
            h.sign(
                Intent::builder()
                    .tenant(h.tenant())
                    .intent_id(IntentId::new(format!("int_batch_{name}")).unwrap())
                    .operation(Operation::from_static("asset.mint"))
                    .actor(principal.subject.clone())
                    .resource(ResourceId(format!("asset:{name}")))
                    .state_type(StateType::UniqueAsset)
                    .expected_version(0)
                    .inputs([(String::from("to_owner"), serde_json::json!("account:alice"))].into())
                    .created_at(h.now())
                    .nonce(Nonce::from_bytes(vec![nonce]).unwrap())
                    .build()
                    .unwrap(),
                None,
            )
        })
        .collect();
    let registry = MemoryKeyRegistry::new();
    registry
        .register(KeyRecord {
            key_id: h.key_id(),
            tenant: h.tenant(),
            subject: Some(principal.subject.clone()),
            operations: vec![Operation::from_static("asset.mint")],
            valid_from: h.now(),
            valid_until: None,
            status: KeyStatus::Active,
        })
        .unwrap();
    let items: Vec<_> = requests
        .iter()
        .map(|validated| PlayerBatchItem {
            validated,
            principal: &principal,
        })
        .collect();
    let prepared = h
        .executor
        .prepare_player_batch(&items, &Allow, &registry)
        .await
        .unwrap();
    let events: Vec<_> = prepared.events().cloned().collect();
    let (mut commit, _) = h.commit_events(&events);
    if let Some(previous) = previous {
        let all: Vec<_> = previous
            .prepared()
            .events()
            .cloned()
            .chain(events.iter().cloned())
            .collect();
        let mut tree = statechronicle::accumulator::sparse_merkle::StateAccumulator::empty();
        tree.insert_batch(&statechronicle::commit::roots::state_root_updates(&all).unwrap())
            .unwrap();
        commit.body.commit_id =
            statechronicle::CommitId::new(String::from("cmt_batch_second")).unwrap();
        commit.body.sequence = 2;
        commit.body.parent_commit_id = Some(previous.signed_commit().body.commit_id.clone());
        commit.body.previous_state_root = previous.signed_commit().body.next_state_root.clone();
        commit.body.next_state_root = statechronicle::ContentDigest::new(*tree.root().as_bytes());
    }
    let outbox = vec![OutboxRecord {
        delivery_key: String::from("whole-batch-notice"),
        tenant: h.tenant(),
        commit_id: commit.body.commit_id.clone(),
        payload: vec![1, 2, 3],
        payload_digest: statechronicle::core::digest::hash_bytes(&[1, 2, 3]),
    }];
    SignedBatch::sign(prepared, &commit.body, h.key_id(), &Signer, outbox).unwrap()
}
fn unchanged(ledger: &MemoryLedger) {
    let data = ledger.data.lock().unwrap();
    assert!(data.events.is_empty());
    assert!(data.commits.is_empty());
    assert!(data.projections.is_empty());
    assert!(data.outbox.is_empty());
    assert!(data.identities.is_empty());
    assert!(data.batches.is_empty());
    assert!(data.mutations.is_empty());
}
#[tokio::test]
async fn complete_group_commits_replays_and_recovers_through_fresh_handle() {
    let plan = plan(&["a", "b"], 1).await;
    let ledger = MemoryLedger::default();
    assert!(matches!(
        plan.recover(&ledger, &Allow, &Verify).await.unwrap(),
        BatchRecoveryOutcome::NotFound(_)
    ));
    let receipt = match plan.commit(&ledger, &Allow, &Verify).await.unwrap() {
        DurableBatchOutcome::Committed(receipt) => receipt,
        _ => panic!("expected new batch"),
    };
    assert_eq!(receipt.identity().members().len(), 2);
    assert_eq!(receipt.record().events.len(), 2);
    assert_eq!(receipt.record().intents.len(), 2);
    let reopened = MemoryLedger {
        data: ledger.data.clone(),
        reservations: ledger.reservations.clone(),
        ..Default::default()
    };
    let recovered = match plan.recover(&reopened, &Allow, &Verify).await.unwrap() {
        BatchRecoveryOutcome::Committed(original) => original,
        BatchRecoveryOutcome::NotFound(_) | BatchRecoveryOutcome::InProgress { .. } => {
            panic!("missing original")
        }
    };
    assert_eq!(recovered.record(), receipt.record());
    assert!(matches!(
        plan.commit(&reopened, &Allow, &Verify).await.unwrap(),
        DurableBatchOutcome::Replay(_)
    ));
    let data = ledger.data.lock().unwrap();
    assert_eq!(data.events.len(), 2);
    assert_eq!(data.commits.len(), 1);
    assert_eq!(data.outbox.len(), 1);
    assert_eq!(data.identities.len(), 2);
    assert_eq!(data.batches.len(), 2);
    assert!(data.mutations.is_empty());
}
#[tokio::test]
async fn lost_ack_retains_entire_identity_and_recovers_original_without_new_effects() {
    let plan = plan(&["a", "b"], 1).await;
    let ledger = MemoryLedger {
        lose_commit_response: true,
        ..Default::default()
    };
    let error = plan.commit(&ledger, &Allow, &Verify).await.unwrap_err();
    assert!(matches!(error.cause(), DurableError::Unknown { .. }));
    assert_eq!(error.identity().unwrap().members().len(), 2);
    assert!(matches!(
        plan.recover(&ledger, &Allow, &Verify).await.unwrap(),
        BatchRecoveryOutcome::Committed(_)
    ));
    assert!(matches!(
        plan.commit(&ledger, &Allow, &Verify).await.unwrap(),
        DurableBatchOutcome::Replay(_)
    ));
    assert_eq!(ledger.data.lock().unwrap().commits.len(), 1);
}
#[tokio::test]
async fn each_precommit_failure_rolls_back_every_member_and_effect() {
    for step in [
        Step::Begin,
        Step::Claim,
        Step::Events,
        Step::CommitRow,
        Step::Projection,
        Step::Outbox,
        Step::RetainBatch,
        Step::Finalize,
    ] {
        let plan = plan(&["a", "b"], 1).await;
        let ledger = MemoryLedger {
            faults: vec![step],
            ..Default::default()
        };
        let error = plan.commit(&ledger, &Allow, &Verify).await.unwrap_err();
        assert!(
            matches!(error.cause(), DurableError::Unavailable(_)),
            "{step:?}: {error:?}"
        );
        unchanged(&ledger);
        assert!(ledger.reservations.lock().unwrap().is_empty());
        let retry = MemoryLedger {
            data: ledger.data.clone(),
            reservations: ledger.reservations.clone(),
            ..Default::default()
        };
        assert!(matches!(
            plan.commit(&retry, &Allow, &Verify).await.unwrap(),
            DurableBatchOutcome::Committed(_)
        ));
    }
}
#[tokio::test]
async fn final_commit_and_rollback_failures_never_become_safe_failure() {
    for faults in [vec![Step::Commit], vec![Step::Events, Step::Rollback]] {
        let plan = plan(&["a", "b"], 1).await;
        let ledger = MemoryLedger {
            faults,
            ..Default::default()
        };
        let error = plan.commit(&ledger, &Allow, &Verify).await.unwrap_err();
        assert!(matches!(error.cause(), DurableError::Unknown { .. }));
        assert_eq!(error.identity().unwrap().members().len(), 2);
        unchanged(&ledger);
    }
}
#[tokio::test]
async fn cancellation_at_each_write_boundary_exposes_no_subset() {
    for step in [
        Step::Claim,
        Step::Events,
        Step::CommitRow,
        Step::Projection,
        Step::Outbox,
        Step::RetainBatch,
        Step::Finalize,
        Step::Commit,
    ] {
        let plan = std::sync::Arc::new(plan(&["a", "b"], 1).await);
        let ledger = MemoryLedger {
            pause: Some(step),
            ..Default::default()
        };
        let task = tokio::spawn({
            let ledger = ledger.clone();
            let plan = plan.clone();
            async move { plan.commit(&ledger, &Allow, &Verify).await }
        });
        loop {
            if ledger.calls.lock().unwrap().contains(&step) {
                break;
            }
            tokio::task::yield_now().await;
        }
        unchanged(&ledger);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        unchanged(&ledger);
        assert!(ledger.reservations.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn subsets_reordering_supersets_and_changed_payload_never_replay_as_a_new_group() {
    let original = plan(&["a", "b"], 1).await;
    let ledger = MemoryLedger::default();
    original.commit(&ledger, &Allow, &Verify).await.unwrap();
    for (names, nonce) in [
        (vec!["a"], 1),
        (vec!["b", "a"], 1),
        (vec!["a", "b", "c"], 1),
        (vec!["a", "b"], 2),
    ] {
        let different = plan(&names, nonce).await;
        let error = different
            .commit(&ledger, &Allow, &Verify)
            .await
            .unwrap_err();
        assert!(
            matches!(error.cause(), DurableError::PayloadConflict),
            "{names:?}: {error:?}"
        );
        assert!(ledger.reservations.lock().unwrap().is_empty());
    }
    assert_eq!(ledger.data.lock().unwrap().identities.len(), 2);
    assert_eq!(ledger.data.lock().unwrap().commits.len(), 1);
    let leg = &original.prepared().mutations()[0];
    let digest = canonicalize_and_digest(&leg.validated().intent).unwrap();
    let error = recover_batch(
        &ledger,
        &Allow,
        &Verify,
        &[BatchMember {
            principal: leg.principal(),
            intent: &leg.validated().intent,
            payload_digest: &digest,
        }],
    )
    .await
    .unwrap_err();
    assert!(matches!(error.cause(), DurableError::PayloadConflict));
}
struct DenySecond;
#[async_trait]
impl Authorizer for DenySecond {
    async fn authorize(&self, context: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
        if context.intent.intent_id.as_str().ends_with("_b") {
            Err(AuthorizationError::Denied)
        } else {
            Ok(())
        }
    }
}
#[tokio::test]
async fn every_member_is_reauthorized_before_any_ledger_call_on_write_or_recovery() {
    let plan = plan(&["a", "b"], 1).await;
    let ledger = MemoryLedger::default();
    for error in [
        plan.commit(&ledger, &DenySecond, &Verify)
            .await
            .unwrap_err(),
        plan.recover(&ledger, &DenySecond, &Verify)
            .await
            .unwrap_err(),
    ] {
        assert!(matches!(
            error.cause(),
            DurableError::Authorization(AuthorizationError::Denied)
        ));
    }
    assert!(ledger.calls.lock().unwrap().is_empty());
    unchanged(&ledger);
}
#[tokio::test]
async fn partial_corrupt_or_unavailable_readback_never_issues_a_receipt() {
    for damage in 0..5 {
        let plan = plan(&["a", "b"], 1).await;
        let ledger = MemoryLedger::default();
        plan.commit(&ledger, &Allow, &Verify).await.unwrap();
        {
            let mut data = ledger.data.lock().unwrap();
            match damage {
                0 => {
                    data.batches.values_mut().for_each(|record| {
                        record.events.pop();
                    });
                }
                1 => {
                    data.batches.values_mut().for_each(|record| {
                        record.projections.clear();
                    });
                }
                2 => {
                    let key = data.batches.keys().last().unwrap().clone();
                    data.batches.remove(&key);
                }
                3 => {
                    data.batches.values_mut().for_each(|record| {
                        record.intents.reverse();
                    });
                }
                _ => {
                    data.batches.values_mut().for_each(|record| {
                        record.commit.signature.sig = Signature::from_bytes([0; 64]);
                    });
                }
            }
        }
        assert!(plan.recover(&ledger, &Allow, &Verify).await.is_err());
        let error = plan.commit(&ledger, &Allow, &Verify).await.unwrap_err();
        assert!(matches!(
            error.cause(),
            DurableError::Unknown { .. } | DurableError::PayloadConflict
        ));
        assert_eq!(ledger.data.lock().unwrap().commits.len(), 1);
    }
    let plan = plan(&["a", "b"], 1).await;
    let ledger = MemoryLedger {
        faults: vec![Step::Lookup],
        ..Default::default()
    };
    assert!(matches!(
        plan.commit(&ledger, &Allow, &Verify)
            .await
            .unwrap_err()
            .cause(),
        DurableError::Unknown { .. }
    ));
}

#[tokio::test]
async fn later_member_failures_and_second_member_readback_are_checked() {
    for step in [Step::Claim, Step::Projection, Step::Finalize] {
        let plan = plan(&["a", "b"], 1).await;
        let ledger = MemoryLedger {
            fail_at: Some((step, 2)),
            ..Default::default()
        };
        assert!(matches!(
            plan.commit(&ledger, &Allow, &Verify)
                .await
                .unwrap_err()
                .cause(),
            DurableError::Unavailable(_)
        ));
        unchanged(&ledger);
        assert!(ledger.reservations.lock().unwrap().is_empty());
    }
    let plan = plan(&["a", "b"], 1).await;
    let ledger = MemoryLedger {
        fail_at: Some((Step::Lookup, 2)),
        ..Default::default()
    };
    let error = plan.commit(&ledger, &Allow, &Verify).await.unwrap_err();
    assert!(matches!(error.cause(), DurableError::Unknown { .. }));
    assert_eq!(ledger.data.lock().unwrap().identities.len(), 2);
}
#[tokio::test]
async fn same_group_contention_has_one_writer_and_only_complete_replay() {
    for count in [2, 8, 32] {
        let plan = std::sync::Arc::new(plan(&["a", "b"], 1).await);
        let ledger = MemoryLedger {
            yield_each_step: true,
            ..Default::default()
        };
        let barrier = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(count));
        let mut tasks = Vec::new();
        for _ in 0..count {
            let ledger = ledger.clone();
            let plan = plan.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                barrier.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                while barrier.load(std::sync::atomic::Ordering::SeqCst) != 0 {
                    tokio::task::yield_now().await;
                }
                plan.commit(&ledger, &Allow, &Verify).await
            }));
        }
        let mut writers: usize = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(DurableBatchOutcome::Committed(_)) => writers = writers.saturating_add(1),
                Ok(DurableBatchOutcome::Replay(_)) => {}
                Err(error) => assert!(
                    matches!(error.cause(), DurableError::InProgress { .. }),
                    "{error:?}"
                ),
            }
        }
        assert_eq!(writers, 1);
        {
            let stored = ledger.data.lock().unwrap();
            assert_eq!(stored.commits.len(), 1);
            assert_eq!(stored.events.len(), 2);
            assert_eq!(stored.identities.len(), 2);
            assert_eq!(stored.batches.len(), 2);
        }
        assert!(matches!(
            plan.recover(&ledger, &Allow, &Verify).await.unwrap(),
            BatchRecoveryOutcome::Committed(_)
        ));
    }
}

#[tokio::test]
async fn original_batch_survives_later_head_and_outbox_delivery() {
    let first = plan(&["a", "b"], 1).await;
    let ledger = MemoryLedger::default();
    let original = match first.commit(&ledger, &Allow, &Verify).await.unwrap() {
        DurableBatchOutcome::Committed(receipt) => receipt,
        DurableBatchOutcome::Replay(_) => panic!("expected new group"),
    };
    let second = plan_after(&["c", "d"], 1, Some(&first)).await;
    assert!(matches!(
        second.commit(&ledger, &Allow, &Verify).await.unwrap(),
        DurableBatchOutcome::Committed(_)
    ));
    ledger.data.lock().unwrap().outbox.clear();
    let recovered = match first.recover(&ledger, &Allow, &Verify).await.unwrap() {
        BatchRecoveryOutcome::Committed(receipt) => receipt,
        BatchRecoveryOutcome::NotFound(_) | BatchRecoveryOutcome::InProgress { .. } => {
            panic!("lost original group")
        }
    };
    assert_eq!(recovered.record(), original.record());
    assert_eq!(recovered.record().outbox.len(), 1);
    assert!(matches!(
        first.commit(&ledger, &Allow, &Verify).await.unwrap(),
        DurableBatchOutcome::Replay(_)
    ));
    assert_eq!(ledger.data.lock().unwrap().commits.len(), 2);
}
#[tokio::test]
async fn advanced_preflight_cannot_omit_or_substitute_members_events_or_effects() {
    use statechronicle::commit::{
        durable_batch::{DurableBatchRequest, commit_batch_verified},
        persist::{CommittedEvent, projections_for},
    };
    let plan = plan(&["a", "b"], 1).await;
    for damage in 0..7 {
        let mut events: Vec<_> = plan.prepared().events().cloned().collect();
        if damage == 0 {
            events.pop();
        }
        if damage == 1 {
            events[1].intent_id = IntentId::new(String::from("int_other")).unwrap();
        }
        if damage == 2 {
            events[1].after.version = 50;
        }
        let entries: Vec<_> = events
            .iter()
            .map(|event| CommittedEvent {
                event,
                state_type: event.after.state.state_type(),
            })
            .collect();
        let mut projections = projections_for(&plan.signed_commit().body.commit_id, &entries);
        if damage == 3 {
            projections.clear();
        }
        let wrong_digest = statechronicle::core::digest::hash_bytes(&[99]);
        let mut members: Vec<_> = plan
            .prepared()
            .mutations()
            .iter()
            .map(|member| BatchMember {
                principal: member.principal(),
                intent: &member.validated().intent,
                payload_digest: member.payload_digest(),
            })
            .collect();
        if damage == 4 {
            members[1].payload_digest = &wrong_digest;
        }
        if damage == 5 {
            members.pop();
        }
        if damage == 6 {
            members[1].intent = members[0].intent;
            members[1].payload_digest = members[0].payload_digest;
        }
        let ledger = MemoryLedger::default();
        let error = commit_batch_verified(
            &ledger,
            &Allow,
            &Verify,
            DurableBatchRequest {
                members: &members,
                entries: &entries,
                commit: plan.signed_commit(),
                projections: &projections,
                outbox: &[],
            },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error.cause(), DurableError::Rejected(_)),
            "damage {damage}: {error:?}"
        );
        assert!(ledger.calls.lock().unwrap().is_empty());
        unchanged(&ledger);
    }
}
