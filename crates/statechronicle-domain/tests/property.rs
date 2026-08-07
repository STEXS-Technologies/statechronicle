//! Property tests (proptest) for the domain types.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::str::FromStr;

use ed25519_dalek::SigningKey;
use proptest::prelude::*;

use statechronicle_core::signature::{sign, verify};

use statechronicle_domain::ids::IntentId;
use statechronicle_domain::intent::Nonce;

proptest! {
    // (a) For any bytes 0..64, a nonce constructed from them round-trips
    // through its `b64u:` string form and back to an equal nonce.
    #[test]
    fn nonce_roundtrips_through_b64u_string(
        data in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let nonce = Nonce::from_bytes(data.clone()).unwrap();
        assert_eq!(nonce.as_bytes(), data.as_slice());

        let encoded = nonce.to_b64u_string();
        assert!(encoded.starts_with("b64u:"));

        let reparsed = Nonce::from_b64u_str(&encoded).unwrap();
        assert_eq!(reparsed, nonce);
    }

    // (b) For any valid prefixed id string, FromStr then Display then FromStr
    // yields an equal id.
    #[test]
    fn id_roundtrips_through_display_and_from_str(
        body in "[A-Za-z0-9]{1,124}",
    ) {
        let value = format!("int_{body}");
        let id = IntentId::new(value).unwrap();

        let rendered = id.to_string();
        assert_eq!(rendered.len(), body.len().saturating_add(4));

        let reparsed = IntentId::from_str(&rendered).unwrap();
        assert_eq!(reparsed, id);
    }

    // (c) For any canonical bytes, a signature made with a fixed key verifies
    // under that same key.
    #[test]
    fn sign_then_verify_succeeds(
        canonical in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let key = SigningKey::from_bytes(&[23u8; 32]);
        let signature = sign(&canonical, &key);
        assert!(verify(&canonical, &key.verifying_key(), &signature).is_ok());
    }
}
