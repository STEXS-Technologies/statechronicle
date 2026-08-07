#![no_main]

use libfuzzer_sys::fuzz_target;
use serde::{Deserialize, Serialize};

use statechronicle_core::canonicalize::canonicalize;

// BCS canonicalization must be deterministic and reversible: canonicalizing a
// value twice yields identical bytes, and `bcs::from_bytes` decodes those bytes
// back to the original value.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Payload {
    bytes: Vec<u8>,
    count: u64,
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }

    let mut count_bytes = [0u8; 8];
    count_bytes.copy_from_slice(&data[..8]);
    let payload = Payload {
        bytes: data[8..].to_vec(),
        count: u64::from_le_bytes(count_bytes),
    };

    if let Ok(bytes) = canonicalize(&payload) {
        // Determinism: a second canonicalization yields identical bytes.
        let again = canonicalize(&payload);
        assert!(again.is_ok());
        assert_eq!(bytes, again.unwrap());

        // Roundtrip: BCS decoding recovers the exact original value.
        let decoded = bcs::from_bytes::<Payload>(&bytes);
        assert!(decoded.is_ok());
        assert_eq!(decoded.unwrap(), payload);
    }
});
