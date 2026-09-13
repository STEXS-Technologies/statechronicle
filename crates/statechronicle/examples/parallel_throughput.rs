//! Multi-thread scaling probe for independent StateChronicle resources.
//!
//! Each worker owns an executor and in-memory ports, so this measures shardable
//! workload scaling rather than lock-sharing one fake store. Set
//! `STATECHRONICLE_WORKERS` and `STATECHRONICLE_PARALLEL_ITERS` (default
//! workers = available CPUs, iterations = 32,000 per worker).

#![allow(clippy::expect_used, clippy::arithmetic_side_effects)]

mod common;

use std::hint::black_box;
use std::thread;
use std::time::Instant;

use statechronicle::domain::ids::IntentId;
use statechronicle::domain::intent::{Intent, Nonce, Operation};
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::state_type::StateType;
use statechronicle::domain::subject::SubjectId;

use common::Harness;

fn run_worker(worker: usize, iterations: u64) -> u8 {
    let harness = Harness::new();
    let actor = SubjectId(String::from("account:bench:player"));
    let operation = Operation::from_static("asset.mint");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async move {
        let mut checksum = 0u8;
        for index in 0..iterations {
            let global_index = (worker as u64) * iterations + index;
            let intent = Intent::builder()
                .tenant(harness.tenant())
                .intent_id(IntentId::new(format!("int_{global_index:020}")).expect("intent id"))
                .operation(operation.clone())
                .actor(actor.clone())
                .resource(ResourceId(format!("asset:parallel_{global_index:020}")))
                .state_type(StateType::UniqueAsset)
                .expected_version(0)
                .created_at(harness.now())
                .nonce(Nonce::from_bytes(vec![0]).expect("nonce"))
                .input("to_owner", serde_json::json!("account:bench:player"))
                .build()
                .expect("intent");
            let validated = harness.sign(intent, None);
            let event = harness.run(&validated, StateType::UniqueAsset).await;
            checksum ^= event
                .event_id
                .as_str()
                .as_bytes()
                .first()
                .copied()
                .unwrap_or_default();
            black_box(event);
        }
        checksum
    })
}

fn main() {
    let workers = std::env::var("STATECHRONICLE_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
        });
    let per_worker = std::env::var("STATECHRONICLE_PARALLEL_ITERS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(32_000);
    let start = Instant::now();
    let checksum = (0..workers)
        .map(|worker| thread::spawn(move || run_worker(worker, per_worker)))
        .map(|handle| handle.join().expect("worker"))
        .fold(0u8, |sum, value| sum ^ value);
    let total = (workers as u128) * u128::from(per_worker);
    let rate = (total * 1_000_000_000) / start.elapsed().as_nanos().max(1);
    println!(
        "workers={workers} iterations={total} elapsed_ms={} operations_per_sec={rate} checksum={checksum}",
        start.elapsed().as_millis()
    );
}
