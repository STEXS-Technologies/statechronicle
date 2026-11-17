//! Read-only public consumer without intent-store or transaction-manager adapters.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn
)]
#[path = "support/readonly.rs"]
mod fixture;
use fixture::*;
use statechronicle::ResourceId;
use statechronicle::executor::pipeline::PlayerBatchItem;

#[tokio::test]
async fn readonly_composition_authenticates_and_prepares_without_write_adapters() {
    let trace = Trace::default();
    let preparer = preparer(&trace);
    let (authorization, keys) = policies(&trace);
    let request = request("one");
    let principal = principal();
    let prepared = preparer
        .prepare_player(&request, &principal, &authorization, &keys)
        .await
        .unwrap();
    assert_eq!(prepared.validated(), &request);
    assert_eq!(prepared.principal(), &principal);
    assert_eq!(prepared.events().len(), 1);
    assert_eq!(prepared.events().first().unwrap().after.version, 1);
    assert_eq!(
        *trace.lock().unwrap(),
        [
            "key",
            "authorize",
            "clock",
            "signature",
            "tenant",
            "state",
            "event",
            "clock"
        ]
    );
    // No ledger result or replay shortcut: another preparation produces a fresh plan.
    let again = preparer
        .prepare_player(&request, &principal, &authorization, &keys)
        .await
        .unwrap();
    assert_ne!(
        prepared.events().first().unwrap().event_id,
        again.events().first().unwrap().event_id
    );
}

#[tokio::test]
async fn readonly_batch_checks_all_ingress_before_state_and_returns_complete_group() {
    let trace = Trace::default();
    let preparer = preparer(&trace);
    let (authorization, keys) = policies(&trace);
    let principal = principal();
    let requests = [request("one"), request("two")];
    let items: Vec<_> = requests
        .iter()
        .map(|validated| PlayerBatchItem {
            validated,
            principal: &principal,
        })
        .collect();
    let batch = preparer
        .prepare_player_batch(&items, &authorization, &keys)
        .await
        .unwrap();
    assert_eq!(batch.mutations().len(), 2);
    assert_eq!(batch.events().count(), 2);
    assert_eq!(
        *trace.lock().unwrap(),
        [
            "authorize",
            "key",
            "authorize",
            "key",
            "clock",
            "signature",
            "tenant",
            "state",
            "event",
            "clock",
            "clock",
            "signature",
            "tenant",
            "state",
            "event",
            "clock"
        ]
    );
    trace.lock().unwrap().clear();
    let duplicate = [*items.first().unwrap(), *items.first().unwrap()];
    assert!(
        preparer
            .prepare_player_batch(&duplicate, &authorization, &keys)
            .await
            .is_err()
    );
    assert_eq!(
        *trace.lock().unwrap(),
        ["authorize", "key", "authorize", "key"]
    );
}

#[tokio::test]
async fn readonly_refusals_preserve_single_and_batch_gate_order() {
    let trace = Trace::default();
    let preparer = preparer(&trace);
    let denied = Policy {
        trace: trace.clone(),
        deny: true,
    };
    let (authorization, keys) = policies(&trace);
    let principal = principal();
    let request = request("refusal");
    // Single keys precede authorization; batch authorization precedes keys.
    assert!(
        preparer
            .prepare_player(&request, &principal, &denied, &denied)
            .await
            .is_err()
    );
    assert_eq!(*trace.lock().unwrap(), ["key"]);
    trace.lock().unwrap().clear();
    let items = [PlayerBatchItem {
        validated: &request,
        principal: &principal,
    }];
    assert!(
        preparer
            .prepare_player_batch(&items, &denied, &denied)
            .await
            .is_err()
    );
    assert_eq!(*trace.lock().unwrap(), ["authorize"]);
    trace.lock().unwrap().clear();
    let mut forged = request.clone();
    forged.signature = None;
    assert!(
        preparer
            .prepare_player(&forged, &principal, &authorization, &keys)
            .await
            .is_err()
    );
    assert!(trace.lock().unwrap().is_empty());
    // An authenticated but modified body still reaches real signature verification.
    forged = request;
    forged.intent.resource_id = ResourceId(String::from("asset:substituted"));
    assert!(
        preparer
            .prepare_player(&forged, &principal, &authorization, &keys)
            .await
            .is_err()
    );
    assert_eq!(
        *trace.lock().unwrap(),
        ["key", "authorize", "clock", "signature"]
    );
}
