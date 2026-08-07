//! Integration tests for `ContentDigest` through the public API.

#![allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]

use std::str::FromStr;

use statechronicle_core::digest::{
    ContentDigest, DIGEST_BYTE_LEN, DIGEST_HEX_LEN, DIGEST_PREFIX, hash_bytes,
};
use statechronicle_core::error::StateChronicleError;

#[test]
fn parse_and_display_roundtrip() {
    let digest = hash_bytes(b"integration parse/display");
    let rendered = digest.to_string();
    assert!(rendered.starts_with(DIGEST_PREFIX));

    let reparsed = ContentDigest::from_hex_sha256(&rendered).unwrap();
    assert_eq!(reparsed, digest);
    assert_eq!(reparsed.as_str(), digest.as_str());
}

#[test]
fn from_str_matches_from_hex_sha256() {
    let digest = hash_bytes(b"from-str integration");
    let parsed = ContentDigest::from_str(digest.as_str()).unwrap();
    assert_eq!(parsed, digest);
}

#[test]
fn serde_roundtrips_through_string_form() {
    let digest = hash_bytes(b"serde integration");
    let json = serde_json::to_string(&digest).unwrap();
    assert!(json.starts_with("\"sha256:"));

    let decoded: ContentDigest = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, digest);
}

#[test]
fn serde_rejects_invalid_digest() {
    let result = serde_json::from_str::<ContentDigest>("\"sha256:zz\"");
    assert!(result.is_err());
}

#[test]
fn known_answer_empty_string() {
    // sha256 of the empty string, the standard first known-answer vector.
    let digest = hash_bytes(b"");
    assert_eq!(
        digest.as_str(),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn known_answer_abc() {
    // sha256("abc"), the canonical second known-answer vector.
    let digest = hash_bytes(b"abc");
    assert_eq!(
        digest.as_str(),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn digest_bytes_are_32_and_hex_is_64() {
    let digest = hash_bytes(b"length check");
    assert_eq!(digest.as_bytes().len(), DIGEST_BYTE_LEN);
    assert_eq!(digest.as_str().len(), DIGEST_PREFIX.len() + DIGEST_HEX_LEN);
}

#[test]
fn rejects_wrong_prefix() {
    let raw = format!("md5:{}", "0".repeat(DIGEST_HEX_LEN));
    assert!(matches!(
        ContentDigest::from_hex_sha256(&raw),
        Err(StateChronicleError::InvalidDigest(_))
    ));
}

#[test]
fn rejects_missing_prefix() {
    let raw = "0".repeat(DIGEST_HEX_LEN);
    assert!(ContentDigest::from_hex_sha256(&raw).is_err());
}

#[test]
fn rejects_uppercase_hex() {
    let raw = format!("{DIGEST_PREFIX}{}", "A".repeat(DIGEST_HEX_LEN));
    assert!(ContentDigest::from_hex_sha256(&raw).is_err());
}

#[test]
fn rejects_wrong_length() {
    let too_short = format!(
        "{DIGEST_PREFIX}{}",
        "a".repeat(DIGEST_HEX_LEN.saturating_sub(1))
    );
    assert!(ContentDigest::from_hex_sha256(&too_short).is_err());

    let too_long = format!(
        "{DIGEST_PREFIX}{}",
        "a".repeat(DIGEST_HEX_LEN.saturating_add(1))
    );
    assert!(ContentDigest::from_hex_sha256(&too_long).is_err());
}

#[test]
fn rejects_non_hex_characters() {
    let raw = format!("{DIGEST_PREFIX}{}", "z".repeat(DIGEST_HEX_LEN));
    assert!(ContentDigest::from_hex_sha256(&raw).is_err());
}
