//! Integration tests for BCS canonicalization through the public API.

#![allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]

use statechronicle_core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle_core::digest::hash_bytes;
use statechronicle_core::error::StateChronicleError;

#[derive(serde::Deserialize, serde::Serialize, Debug, PartialEq, Eq)]
struct NestedRecord {
    tenant_id: String,
    version: u64,
    tags: Vec<String>,
    owner: Owner,
}

#[derive(serde::Deserialize, serde::Serialize, Debug, PartialEq, Eq)]
struct Owner {
    name: String,
    verified: bool,
    aliases: Option<Vec<String>>,
}

fn sample_record() -> NestedRecord {
    NestedRecord {
        tenant_id: String::from("tenant:acme"),
        version: 7,
        tags: vec![
            String::from("rare"),
            String::from("bound"),
            String::from("unbound"),
        ],
        owner: Owner {
            name: String::from("warden:kara"),
            verified: true,
            aliases: Some(vec![String::from("kara-the-firm")]),
        },
    }
}

#[test]
fn canonicalize_is_deterministic_across_calls() {
    let record = sample_record();

    let first = canonicalize(&record).unwrap();
    let second = canonicalize(&record).unwrap();

    assert_eq!(first, second);
    assert!(!first.is_empty());
}

#[test]
fn canonicalize_roundtrips_through_bcs() {
    let record = sample_record();
    let bytes = canonicalize(&record).unwrap();

    let decoded: NestedRecord = bcs::from_bytes(&bytes).unwrap();
    assert_eq!(decoded, record);
}

#[test]
fn canonicalize_and_digest_equals_hash_of_canonical_bytes() {
    let record = sample_record();
    let bytes = canonicalize(&record).unwrap();

    let digest = canonicalize_and_digest(&record).unwrap();
    assert_eq!(digest.as_bytes(), hash_bytes(&bytes).as_bytes());
}

#[test]
fn canonicalize_rejects_floats() {
    #[derive(serde::Serialize)]
    struct FloatPayload {
        amount: f64,
    }

    let payload = FloatPayload { amount: 1.5 };
    let error = canonicalize(&payload).unwrap_err();

    assert!(matches!(
        error,
        StateChronicleError::Canonicalization { .. }
    ));
}
