//! Compare compatibility and read-only entry points through identical read callbacks.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn
)]
mod common;
#[path = "support/readonly.rs"]
mod fixture;
use fixture::*;
use statechronicle::executor::pipeline::{Executor, PlayerBatchItem, Ports};

fn legacy(trace: &Trace) -> Executor {
    let config = config(trace);
    Executor::builder()
        .ports(Ports {
            intent_store: Box::new(common::FakeIntentStore::default()),
            state_index: Box::new(Reads(trace.clone())),
            tenant_store: Box::new(Reads(trace.clone())),
            trustgrant: Vec::new(),
            transaction_manager: Box::new(common::FakeTransactionManager::default()),
        })
        .profiles(config.profiles)
        .executor(config.executor)
        .clock(config.now)
        .event_id_gen(config.event_id_fn)
        .intent_verifier(config.intent_verifier)
        .build()
        .unwrap()
}

#[tokio::test]
async fn readonly_and_compatibility_preparation_match_events_errors_and_callbacks() {
    let principal = principal();
    for scenario in [
        "valid",
        "missing_signature",
        "changed_body",
        "wrong_actor",
        "denied",
        "revoked",
    ] {
        let mut request = request("parity");
        if scenario == "missing_signature" {
            request.signature = None;
        }
        if scenario == "changed_body" {
            request.intent.resource_id.0.push_str("changed");
        }
        let mut actor = principal.clone();
        if scenario == "wrong_actor" {
            actor.subject.0.push_str("other");
        }
        let new_trace = Trace::default();
        let old_trace = Trace::default();
        let (mut new_auth, mut new_keys) = policies(&new_trace);
        let (mut old_auth, mut old_keys) = policies(&old_trace);
        new_auth.deny = scenario == "denied";
        old_auth.deny = new_auth.deny;
        new_keys.deny = scenario == "revoked";
        old_keys.deny = new_keys.deny;
        let new = preparer(&new_trace)
            .prepare_player(&request, &actor, &new_auth, &new_keys)
            .await;
        let old = legacy(&old_trace)
            .prepare_player(&request, &actor, &old_auth, &old_keys)
            .await;
        match (new, old) {
            (Ok(new), Ok(old)) => {
                assert_eq!(new.events(), old.events(), "{scenario}");
                assert_eq!(new.payload_digest(), old.payload_digest());
                assert_eq!(new.principal(), old.principal());
            }
            (Err(new), Err(old)) => assert_eq!(new.to_string(), old.to_string(), "{scenario}"),
            other => panic!("different preparation outcomes for {scenario}: {other:?}"),
        }
        assert_eq!(
            *new_trace.lock().unwrap(),
            *old_trace.lock().unwrap(),
            "{scenario}"
        );
    }
}

#[tokio::test]
async fn readonly_and_compatibility_batches_match_success_and_late_leg_refusal() {
    let principal = principal();
    for corrupt_second in [false, true] {
        let first = request("first");
        let mut second = request("second");
        if corrupt_second {
            second.intent.resource_id.0.push_str("changed");
        }
        let items = [
            PlayerBatchItem {
                validated: &first,
                principal: &principal,
            },
            PlayerBatchItem {
                validated: &second,
                principal: &principal,
            },
        ];
        let new_trace = Trace::default();
        let old_trace = Trace::default();
        let (new_auth, new_keys) = policies(&new_trace);
        let (old_auth, old_keys) = policies(&old_trace);
        let new = preparer(&new_trace)
            .prepare_player_batch(&items, &new_auth, &new_keys)
            .await;
        let old = legacy(&old_trace)
            .prepare_player_batch(&items, &old_auth, &old_keys)
            .await;
        match (new, old) {
            (Ok(new), Ok(old)) => assert_eq!(
                new.events().collect::<Vec<_>>(),
                old.events().collect::<Vec<_>>()
            ),
            (Err(new), Err(old)) => assert_eq!(new.to_string(), old.to_string()),
            other => panic!("different batch outcomes: {other:?}"),
        }
        assert_eq!(*new_trace.lock().unwrap(), *old_trace.lock().unwrap());
    }
}
