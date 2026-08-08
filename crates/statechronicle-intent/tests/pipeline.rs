//! Integration tests for the intent parsing/validation → signed-envelope path.
//!
//! Exercises the end-to-end pipeline over real domain types: build a canonical
//! JSON intent payload, parse it into a `RawIntent`, validate it into a
//! `ValidatedIntent`, canonicalize the body, hash it, sign it with an Ed25519
//! key, wrap body and signature in `Signed<Intent>`, and verify the detached
//! signature (ADR-004 §2, §5). Also covers §11.2 idempotency semantics.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use statechronicle_core::canonicalize::canonicalize;
use statechronicle_core::digest::hash_bytes;
use statechronicle_core::signature::{sign, verify};

use statechronicle_domain::intent::{INTENT_SCHEMA, KeyId, SignatureAlg, SignatureBlock};
use statechronicle_domain::signed::Signed;

use statechronicle_intent::parse::{parse_intent, parse_intent_str};
use statechronicle_intent::validate::validate;

const FIXED_SEED: [u8; 32] = [42u8; 32];

fn fixed_key() -> SigningKey {
    SigningKey::from_bytes(&FIXED_SEED)
}

fn sample_payload() -> serde_json::Value {
    serde_json::json!({
        "schema": INTENT_SCHEMA,
        "tenant_id": "acme.game.alpha",
        "intent_id": "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2",
        "operation": "asset.transfer",
        "actor": "account:example:player_123",
        "resource_id": "asset:sword_001",
        "state_type": "unique_asset",
        "expected_version": 41,
        "inputs": {
            "from_owner": "account:example:player_123",
            "to_owner": "account:example:player_456",
        },
        "created_at": "2026-07-14T00:00:00Z",
        "expires_at": "2026-07-14T00:05:00Z",
        "nonce": "b64u:AAME",
    })
}

fn parse_payload(payload: &serde_json::Value) -> statechronicle_intent::raw::RawIntent {
    let bytes = serde_json::to_vec(payload).unwrap();
    parse_intent(&bytes).unwrap()
}

#[test]
fn parse_then_validate_then_sign_and_verify_end_to_end() {
    let key = fixed_key();
    let payload = sample_payload();

    // Parse: JSON → RawIntent (stage separation: no field validation yet).
    let raw = parse_payload(&payload);
    assert_eq!(raw.schema, INTENT_SCHEMA);

    // Validate: RawIntent → ValidatedIntent (schema + newtype + expiry).
    let validated = validate(&raw).unwrap();
    assert_eq!(validated.idempotency_key.intent_id.as_str(), raw.intent_id);

    // Canonicalize: BCS canonical bytes of the body (ADR-004).
    let body_bytes = canonicalize(&validated.intent).unwrap();
    assert!(!body_bytes.is_empty());

    // Hash: sha256 digest of the canonical bytes (§17).
    let digest = hash_bytes(&body_bytes);
    assert_eq!(digest.as_bytes().len(), 32);
    let expected_hex = format!("sha256:{}", hex::encode(digest.as_bytes()));
    assert_eq!(digest.as_str(), expected_hex);

    // Sign: detached Ed25519 signature over the canonical body bytes.
    let signature = sign(&body_bytes, &key);
    let block = SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
        sig: signature,
    };
    let envelope = Signed::new(validated.intent, block);

    // Verify: the detached signature checks out against the body bytes.
    assert!(verify(&body_bytes, &key.verifying_key(), &envelope.signature.sig).is_ok());

    // A tampered canonical body must fail verification.
    let mut tampered = body_bytes;
    tampered.push(0x00);
    assert!(verify(&tampered, &key.verifying_key(), &envelope.signature.sig).is_err());

    // The envelope canonicalizes deterministically.
    assert_eq!(
        canonicalize(&envelope).unwrap(),
        canonicalize(&envelope).unwrap()
    );
}

#[test]
fn signed_intent_roundtrips_through_bcs_with_empty_inputs() {
    let key = fixed_key();
    let mut payload = sample_payload();
    // BCS is not self-describing: `serde_json::Value` payloads in `inputs`
    // encode but do not decode, so use an empty inputs map for the BCS
    // round-trip (matching the domain envelope test).
    payload["inputs"] = serde_json::json!({});

    let raw = parse_payload(&payload);
    let validated = validate(&raw).unwrap();

    let body_bytes = canonicalize(&validated.intent).unwrap();
    let block = SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
        sig: sign(&body_bytes, &key),
    };
    let envelope = Signed::new(validated.intent, block);

    let envelope_bytes = canonicalize(&envelope).unwrap();
    let decoded: Signed<statechronicle_domain::intent::Intent> =
        bcs::from_bytes(&envelope_bytes).unwrap();
    assert_eq!(decoded, envelope);

    // Re-validated envelope still verifies against the original body.
    let decoded_body_bytes = canonicalize(&decoded.body).unwrap();
    assert!(
        verify(
            &decoded_body_bytes,
            &key.verifying_key(),
            &decoded.signature.sig
        )
        .is_ok()
    );
}

#[test]
fn validate_parses_embedded_signature_block() {
    let key = fixed_key();
    let mut payload = sample_payload();

    // Sign the canonical body of the (unsigned) intent first.
    let raw = parse_payload(&payload);
    let validated = validate(&raw).unwrap();
    let body_bytes = canonicalize(&validated.intent).unwrap();
    let sig = sign(&body_bytes, &key);

    // Embed the signature in the wire payload using the protocol's b64u form.
    let sig_value = serde_json::to_value(sig).unwrap();
    payload["signature"] = serde_json::json!({
        "alg": "ed25519",
        "key_id": "did:key:z6Mk...#key-1",
        "sig": sig_value,
    });

    // Re-parse and validate: the signature block is parsed and preserved.
    let raw_with_sig = parse_payload(&payload);
    let validated_with_sig = validate(&raw_with_sig).unwrap();
    let block = validated_with_sig.signature.as_ref().unwrap();
    assert_eq!(block.alg, SignatureAlg::Ed25519);
    assert_eq!(block.key_id.as_str(), "did:key:z6Mk...#key-1");
    assert_eq!(block.sig, sig);

    // The embedded signature verifies against the canonical body bytes.
    assert!(verify(&body_bytes, &key.verifying_key(), &block.sig).is_ok());
}

#[test]
fn idempotency_key_is_stable_across_reparse() {
    let payload = sample_payload();
    let first = validate(&parse_payload(&payload)).unwrap();
    let second = validate(&parse_payload(&payload)).unwrap();

    // Replaying the same accepted intent yields the same idempotency tuple
    // (protocol §11.2).
    assert_eq!(first.idempotency_key, second.idempotency_key);

    // A conflicting intent with a different intent_id must differ.
    let mut other = payload;
    other["intent_id"] = serde_json::json!("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K3");
    let third = validate(&parse_payload(&other)).unwrap();
    assert_ne!(first.idempotency_key, third.idempotency_key);
    assert_eq!(
        first.idempotency_key.tenant_id,
        third.idempotency_key.tenant_id
    );
    assert_eq!(
        first.idempotency_key.operation,
        third.idempotency_key.operation
    );
}

#[test]
fn idempotency_tuple_fields_come_from_payload() {
    let payload = sample_payload();
    let validated = validate(&parse_payload(&payload)).unwrap();
    let key = &validated.idempotency_key;

    assert_eq!(key.tenant_id.0, "acme.game.alpha");
    assert_eq!(key.intent_id.as_str(), "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2");
    assert_eq!(key.actor.0, "account:example:player_123");
    assert_eq!(key.resource_id.0, "asset:sword_001");
    assert_eq!(key.operation.as_str(), "asset.transfer");
}

#[test]
fn parse_intent_str_agrees_with_parse_intent() {
    let text = serde_json::to_string(&sample_payload()).unwrap();
    let from_str = parse_intent_str(&text).unwrap();
    let from_bytes = parse_intent(text.as_bytes()).unwrap();
    assert_eq!(from_str, from_bytes);

    let validated_str = validate(&from_str).unwrap();
    let validated_bytes = validate(&from_bytes).unwrap();
    assert_eq!(validated_str, validated_bytes);
}

#[test]
fn oversized_payload_fails_closed() {
    use statechronicle_core::limits::MAX_INTENT_BYTES;
    use statechronicle_intent::error::IntentError;

    let oversized = vec![b'{'; MAX_INTENT_BYTES + 1];
    let error = parse_intent(&oversized).unwrap_err();
    assert!(matches!(
        error,
        IntentError::SizeLimitExceeded { name, limit, actual }
        if name == "intent" && limit == MAX_INTENT_BYTES && actual == MAX_INTENT_BYTES + 1
    ));
}

#[test]
fn invalid_json_fails_closed() {
    use statechronicle_intent::error::IntentError;

    let error = parse_intent(b"{ not json").unwrap_err();
    assert!(matches!(error, IntentError::InvalidJson { .. }));
}

#[test]
fn wrong_schema_fails_closed() {
    use statechronicle_intent::error::IntentError;

    let mut payload = sample_payload();
    payload["schema"] = serde_json::json!("statechronicle.intent.v999");
    let raw = parse_payload(&payload);
    let error = validate(&raw).unwrap_err();
    assert!(matches!(
        error,
        IntentError::InvalidSchema { found, expected }
        if found == "statechronicle.intent.v999" && expected == INTENT_SCHEMA
    ));
}

#[test]
fn validated_intent_reflects_expiry_window() {
    let validated = validate(&parse_payload(&sample_payload())).unwrap();
    let before = DateTime::parse_from_rfc3339("2026-07-14T00:04:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let after = DateTime::parse_from_rfc3339("2026-07-14T00:06:00Z")
        .unwrap()
        .with_timezone(&Utc);

    assert!(!validated.is_expired(before));
    assert!(validated.is_expired(after));
    assert_eq!(
        validated.intent.expires_at.unwrap().to_rfc3339(),
        "2026-07-14T00:05:00+00:00"
    );
}

#[test]
fn validated_intent_json_view_roundtrips() {
    let validated = validate(&parse_payload(&sample_payload())).unwrap();
    let json = serde_json::to_string(&validated).unwrap();
    assert!(json.contains("\"schema\":\"statechronicle.intent.v0\""));

    let decoded: statechronicle_intent::validated::ValidatedIntent =
        serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, validated);
    assert_eq!(decoded.idempotency_key, validated.idempotency_key);
}

#[test]
fn nonce_is_preserved_through_the_pipeline() {
    use statechronicle_domain::intent::Nonce;

    let validated = validate(&parse_payload(&sample_payload())).unwrap();
    assert_eq!(
        validated.intent.nonce,
        Nonce::from_bytes(vec![0, 3, 4]).unwrap()
    );
    assert_eq!(validated.intent.inputs.len(), 2);

    let mut inputs: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    inputs.insert(
        String::from("to_owner"),
        serde_json::json!("account:example:player_456"),
    );
    assert_eq!(
        validated.intent.inputs.get("to_owner"),
        inputs.get("to_owner")
    );
}
