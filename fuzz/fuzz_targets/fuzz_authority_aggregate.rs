#![no_main]

use libfuzzer_sys::fuzz_target;

use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::authority::{AggregationPolicy, aggregate_evaluation_digest};

// Multi-authority aggregation must be total and deterministic over arbitrary
// sub-digest sets: chunking arbitrary bytes into 32-byte sub-digests and
// aggregating them must never panic, and recomputing the same set must yield
// the same aggregate digest.
fuzz_target!(|data: &[u8]| {
    let mut digests = Vec::new();
    for chunk in data.chunks(32) {
        let mut bytes = [0u8; 32];
        for (dst, src) in bytes.iter_mut().zip(chunk.iter()) {
            *dst = *src;
        }
        digests.push(ContentDigest::new(bytes));
    }
    let first = aggregate_evaluation_digest(AggregationPolicy::RequireAll, &digests);
    // Total: always yields a valid 32-byte digest and never panics.
    assert_eq!(first.as_bytes().len(), 32);
    // Deterministic: the same set always yields the same digest.
    let second = aggregate_evaluation_digest(AggregationPolicy::RequireAll, &digests);
    assert_eq!(first, second);
});
