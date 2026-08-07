//! Property tests (proptest) for the core protocol primitives.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use ed25519_dalek::SigningKey;
use proptest::prelude::*;

use statechronicle_core::canonicalize::canonicalize;
use statechronicle_core::digest::{ContentDigest, hash_bytes};
use statechronicle_core::signature::{sign, verify};

#[derive(serde::Deserialize, serde::Serialize, Debug, PartialEq, Eq)]
struct Record {
    tenant_id: String,
    version: u64,
    tags: Vec<String>,
}

proptest! {
    // (a) For any byte vector, `hash_bytes` is deterministic and its string
    // form round-trips back into an equal digest.
    #[test]
    fn hash_bytes_is_deterministic_and_roundtrips(
        data in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let digest = hash_bytes(&data);
        assert_eq!(digest.as_bytes().len(), 32);

        let reparsed = ContentDigest::from_hex_sha256(digest.as_str()).unwrap();
        assert_eq!(reparsed, digest);

        // Determinism: hashing the same bytes twice yields the same digest.
        assert_eq!(hash_bytes(&data), digest);
    }

    // (b) For any struct of arbitrary strings/ints/vecs, canonicalization then
    // BCS decoding round-trips to an equal value.
    #[test]
    fn canonicalize_bcs_roundtrips(
        tenant_id in "\\PC*",
        version in any::<u64>(),
        tags in prop::collection::vec("\\PC*", 0..8),
    ) {
        let record = Record { tenant_id, version, tags };
        let bytes = canonicalize(&record).unwrap();
        let decoded: Record = bcs::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, record);
    }

    // (c) For any canonical bytes, a signature made with a key verifies under
    // that same key.
    #[test]
    fn sign_then_verify_succeeds(
        canonical in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let key = SigningKey::from_bytes(&[23u8; 32]);
        let signature = sign(&canonical, &key);
        assert!(verify(&canonical, &key.verifying_key(), &signature).is_ok());
    }
}
