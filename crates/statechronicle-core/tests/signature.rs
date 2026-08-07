//! Integration tests for Ed25519 signing/verification through the public API.

#![allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]

use ed25519_dalek::SigningKey;

use statechronicle_core::signature::{B64U_PREFIX, Signature, sign, verify};

const FIXED_SEED: [u8; 32] = [17u8; 32];

fn fixed_key() -> SigningKey {
    SigningKey::from_bytes(&FIXED_SEED)
}

#[test]
fn sign_then_verify_succeeds() {
    let key = fixed_key();
    let canonical = b"integration canonical payload";

    let signature = sign(canonical, &key);
    assert!(verify(canonical, &key.verifying_key(), &signature).is_ok());
}

#[test]
fn tampered_canonical_is_rejected() {
    let key = fixed_key();
    let canonical = b"integration canonical payload";
    let signature = sign(canonical, &key);

    let mut tampered = canonical.to_vec();
    tampered.push(0x00);

    assert!(verify(&tampered, &key.verifying_key(), &signature).is_err());
}

#[test]
fn wrong_key_is_rejected() {
    let key = fixed_key();
    let other_key = SigningKey::from_bytes(&[9u8; 32]);
    let canonical = b"integration canonical payload";

    let signature = sign(canonical, &key);
    assert!(verify(canonical, &other_key.verifying_key(), &signature).is_err());
}

#[test]
fn serde_roundtrips_through_b64u_string() {
    let key = fixed_key();
    let signature = sign(b"serde integration", &key);

    let json = serde_json::to_string(&signature).unwrap();
    assert!(json.starts_with("\"b64u:"));

    let decoded: Signature = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, signature);
}

#[test]
fn invalid_b64u_strings_are_rejected() {
    let not_prefixed = serde_json::from_str::<Signature>("\"QUJD\"");
    assert!(not_prefixed.is_err());

    let not_base64 = serde_json::from_str::<Signature>("\"b64u:%%%\"");
    assert!(not_base64.is_err());

    let wrong_length = serde_json::from_str::<Signature>("\"b64u:QUJD\"");
    assert!(wrong_length.is_err());
}

#[test]
fn display_uses_b64u_string_form() {
    let key = fixed_key();
    let signature = sign(b"display integration", &key);

    let rendered = signature.to_string();
    assert!(rendered.starts_with(B64U_PREFIX));
    assert!(!rendered.starts_with(&format!("{B64U_PREFIX}{B64U_PREFIX}")));
}
