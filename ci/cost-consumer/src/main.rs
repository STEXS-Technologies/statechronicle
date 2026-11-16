//! Fixed public-facade workload for reproducible build and binary-size tracking.
use std::hint::black_box;
use statechronicle::{Amount, StateKey};

fn main() {
    let mut checksum = 0_u64;
    for value in 0..100_000 {
        let amount = Amount::from_u64(black_box(value));
        let key = StateKey::for_resource("cost.consumer", "asset:fixed");
        checksum ^= amount.mantissa() as u64 ^ u64::from(key.as_bytes()[0]);
    }
    println!("checksum={}", black_box(checksum));
}
