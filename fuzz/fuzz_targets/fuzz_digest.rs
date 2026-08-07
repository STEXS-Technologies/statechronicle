#![no_main]

use libfuzzer_sys::fuzz_target;

use statechronicle_core::digest::{ContentDigest, hash_bytes};

// SHA-256 digests are total and deterministic over any input bytes: the digest
// must always be 32 bytes and its canonical string form must parse back to the
// same digest.
fuzz_target!(|data: &[u8]| {
    let digest = hash_bytes(data);
    assert_eq!(digest.as_bytes().len(), 32);

    let reparsed = ContentDigest::from_hex_sha256(digest.as_str());
    assert!(reparsed.is_ok());
    assert_eq!(reparsed.unwrap(), digest);
});
