//! Property tests (proptest) for the intent parse/validate pipeline.
//!
//! Generates valid raw intent payloads and checks: parsing and validation
//! never panic on arbitrary bytes, every generated payload validates, the
//! validated intent preserves payload fields, validation is deterministic,
//! and the canonical body digests to a protocol `sha256:` digest.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use chrono::{DateTime, Duration};
use proptest::prelude::*;

use statechronicle_core::canonicalize::canonicalize;
use statechronicle_core::digest::hash_bytes;

use statechronicle_domain::intent::{INTENT_SCHEMA, Nonce};

use statechronicle_intent::parse::parse_intent;
use statechronicle_intent::validate::validate;

/// Generates a fully valid raw intent JSON payload.
fn raw_intent_payload() -> impl Strategy<Value = serde_json::Value> {
    (
        any::<u64>(),                              // expected_version
        0u64..1_000_000u64,                        // expiry offset in seconds
        prop::collection::vec(any::<u8>(), 0..32), // nonce bytes
        "[A-Za-z0-9_]{1,40}",                      // operation body
        "[A-Za-z0-9]{1,60}",                       // intent id body
    )
        .prop_map(
            |(version, expiry_secs, nonce_bytes, operation, intent_body)| {
                let created = DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z").unwrap();
                let expires_at = created + Duration::seconds(expiry_secs as i64 + 1);
                let nonce = Nonce::from_bytes(nonce_bytes).unwrap().to_b64u_string();
                serde_json::json!({
                    "schema": INTENT_SCHEMA,
                    "tenant_id": "stexs.game.alpha",
                    "intent_id": format!("int_{intent_body}"),
                    "operation": operation,
                    "actor": "account:stexs:player_123",
                    "resource_id": "asset:sword_001",
                    "state_type": "unique_asset",
                    "expected_version": version,
                    "inputs": {},
                    "created_at": created.to_rfc3339(),
                    "expires_at": expires_at.to_rfc3339(),
                    "nonce": nonce,
                })
            },
        )
}

proptest! {
    // (a) Parsing arbitrary bytes fails closed and never panics: every outcome
    // is either a valid RawIntent or a SizeLimitExceeded/InvalidJson error.
    #[test]
    fn parse_intent_never_panics(data in prop::collection::vec(any::<u8>(), 0..4096)) {
        drop(parse_intent(&data));
    }

    // (b) Every generated payload parses and validates, and the validated
    // intent preserves the payload fields.
    #[test]
    fn generated_payloads_validate_and_preserve_fields(payload in raw_intent_payload()) {
        let bytes = serde_json::to_vec(&payload).unwrap();
        let raw = parse_intent(&bytes).unwrap();
        let validated = validate(&raw).unwrap();

        assert_eq!(validated.intent.schema, INTENT_SCHEMA);
        assert_eq!(
            validated.intent.expected_version,
            payload["expected_version"].as_u64().unwrap()
        );
        assert_eq!(validated.idempotency_key.tenant_id.0, "stexs.game.alpha");
        assert_eq!(validated.idempotency_key.intent_id.as_str(), raw.intent_id);
        assert_eq!(
            validated.idempotency_key.operation.as_str(),
            payload["operation"].as_str().unwrap()
        );

        // The canonical body hashes to a protocol sha256 digest.
        let canonical = canonicalize(&validated.intent).unwrap();
        let digest = hash_bytes(&canonical);
        assert_eq!(digest.as_bytes().len(), 32);
        assert_eq!(hex::encode(digest.as_bytes()).len(), 64);
        assert_eq!(digest.as_str(), format!("sha256:{}", hex::encode(digest.as_bytes())));
    }

    // (c) Validation is deterministic: the same payload always produces the
    // same validated intent, so replay deduplication is well-defined (§11.2).
    #[test]
    fn validation_is_deterministic(payload in raw_intent_payload()) {
        let bytes = serde_json::to_vec(&payload).unwrap();
        let first = validate(&parse_intent(&bytes).unwrap()).unwrap();
        let second = validate(&parse_intent(&bytes).unwrap()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.idempotency_key, second.idempotency_key);
    }

    // (d) The idempotency tuple mirrors the payload's §11.2 fields.
    #[test]
    fn idempotency_key_matches_payload_tuple(payload in raw_intent_payload()) {
        let validated = validate(&parse_intent(&serde_json::to_vec(&payload).unwrap()).unwrap())
            .unwrap();
        assert_eq!(
            validated.idempotency_key.intent_id.as_str(),
            payload["intent_id"].as_str().unwrap()
        );
        assert_eq!(
            validated.idempotency_key.actor.0,
            payload["actor"].as_str().unwrap()
        );
        assert_eq!(
            validated.idempotency_key.resource_id.0,
            payload["resource_id"].as_str().unwrap()
        );
    }

    // (e) A nonce round-trips: whatever bytes the payload embeds in b64u form
    // are the exact nonce bytes carried by the validated intent.
    #[test]
    fn nonce_bytes_are_preserved(nonce_bytes in prop::collection::vec(any::<u8>(), 0..32)) {
        let nonce = Nonce::from_bytes(nonce_bytes.clone()).unwrap();
        let encoded = nonce.to_b64u_string();
        let reparsed = Nonce::from_b64u_str(&encoded).unwrap();
        assert_eq!(reparsed, nonce);
        assert_eq!(reparsed.as_bytes(), nonce_bytes.as_slice());
    }
}
