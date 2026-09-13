//! Single-process executor throughput probe.
//!
//! This exercises intent construction/signing, executor validation and
//! authentication, transition execution, event creation, and in-memory index
//! application. Set `STATECHRONICLE_E2E_ITERS` (default 10,000). Set
//! `STATECHRONICLE_E2E_COMMIT=1` to also form and verify one signed commit per
//! operation.

#![allow(
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use std::hint::black_box;
use std::time::Instant;

use statechronicle::domain::ids::IntentId;
use statechronicle::domain::intent::{Intent, Nonce, Operation};
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::state_type::StateType;
use statechronicle::domain::subject::SubjectId;
use statechronicle::intent::validated::ValidatedIntent;

use common::Harness;

fn make_intent(
    harness: &Harness,
    actor: &SubjectId,
    operation: &Operation,
    index: u64,
) -> ValidatedIntent {
    let intent = Intent::builder()
        .tenant(harness.tenant())
        .intent_id(IntentId::new(format!("int_{index:020}")).expect("intent id"))
        .operation(operation.clone())
        .actor(actor.clone())
        .resource(ResourceId(format!("asset:bench_{index:020}")))
        .state_type(StateType::UniqueAsset)
        .expected_version(0)
        .created_at(harness.now())
        .nonce(Nonce::from_bytes(vec![0]).expect("nonce"))
        .input("to_owner", serde_json::json!("account:bench:player"))
        .build()
        .expect("intent");
    harness.sign(intent, None)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let iterations = std::env::var("STATECHRONICLE_E2E_ITERS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(10_000);
    let include_commit = std::env::var("STATECHRONICLE_E2E_COMMIT").as_deref() == Ok("1");
    let batch_size = std::env::var("STATECHRONICLE_E2E_BATCH")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let harness = Harness::new();
    let actor = SubjectId(String::from("account:bench:player"));
    let operation = Operation::from_static("asset.mint");
    let start = Instant::now();
    let mut checksum = 0u8;

    let mut index = 0u64;
    while index < iterations {
        let count = batch_size.min((iterations - index) as usize);
        let intents: Vec<_> = (0..count)
            .map(|offset| make_intent(&harness, &actor, &operation, index + offset as u64))
            .collect();
        let events = if batch_size == 1 {
            vec![harness.run(&intents[0], StateType::UniqueAsset).await]
        } else {
            let events = harness
                .executor
                .execute_batch(&intents)
                .await
                .expect("batch");
            for event in &events {
                harness.index.apply(event, StateType::UniqueAsset);
            }
            events
        };
        if include_commit {
            let (commit, _) = harness.commit_events(&events);
            checksum ^= commit
                .signature
                .sig
                .as_bytes()
                .first()
                .copied()
                .unwrap_or_default();
        } else {
            checksum ^= events
                .first()
                .and_then(|event| event.event_id.as_str().as_bytes().first())
                .copied()
                .unwrap_or_default();
        }
        black_box(events);
        index += count as u64;
    }

    let elapsed = start.elapsed();
    let nanos = elapsed.as_nanos().max(1);
    let rate = (u128::from(iterations) * 1_000_000_000) / nanos;
    println!(
        "iterations={iterations} batch={batch_size} commit={include_commit} elapsed_ms={} operations_per_sec={rate} checksum={checksum}",
        elapsed.as_millis()
    );
}
