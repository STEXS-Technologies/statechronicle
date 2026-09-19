//! Single-process throughput probe for the pure StateChronicle hot path.
//!
//! Run with `cargo run --release -p statechronicle --example throughput`.
//! Set `STATECHRONICLE_THROUGHPUT_ITERS` to change the iteration count.

#![allow(clippy::expect_used, clippy::arithmetic_side_effects)]

use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

use serde_json::json;
use statechronicle::core::canonicalize::canonicalize_and_digest;
use statechronicle::domain::intent::Operation;
use statechronicle::executor::transition;

fn main() {
    let iterations = std::env::var("STATECHRONICLE_THROUGHPUT_ITERS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1_000_000);
    let minimum_rate = std::env::var("STATECHRONICLE_THROUGHPUT_MIN_OPS_PER_SEC")
        .ok()
        .and_then(|value| value.parse::<u128>().ok())
        .unwrap_or(0);

    let operation = Operation::from_static("asset.mint");
    let mut inputs = BTreeMap::new();
    inputs.insert(String::from("to_owner"), json!("account:bench:player"));

    let start = Instant::now();
    let mut checksum = 0u8;
    for _ in 0..iterations {
        let state = transition::apply(None, &operation, &inputs).expect("valid transition");
        let digest = canonicalize_and_digest(black_box(&state)).expect("canonical state");
        checksum ^= digest.as_bytes()[0];
        black_box(state);
    }
    let elapsed = start.elapsed();
    let nanos = elapsed.as_nanos().max(1);
    let rate = (u128::from(iterations) * 1_000_000_000) / nanos;
    println!(
        "iterations={iterations} elapsed_ms={} operations_per_sec={rate} checksum={checksum}",
        elapsed.as_millis()
    );
    if rate < minimum_rate {
        eprintln!("throughput {rate} operations/s is below required {minimum_rate} operations/s");
        std::process::exit(1);
    }
}
