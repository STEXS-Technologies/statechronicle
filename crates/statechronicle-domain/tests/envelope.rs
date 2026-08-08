//! Integration tests for the ADR-004 signed-envelope wire path.
//!
//! Exercises the end-to-end path over real domain types: build an `Intent`
//! (and a `Commit`), sign the BCS canonical bytes of the *body*, wrap body and
//! signature in `Signed<T>`, canonicalize the envelope, and verify that (a) the
//! envelope round-trips through BCS and JSON, and (b) the detached signature
//! verifies against the body bytes, while a tampered canonical body fails
//! verification (ADR-004 §2, §5).

#![allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use statechronicle_core::canonicalize::canonicalize;
use statechronicle_core::digest::hash_bytes;
use statechronicle_core::signature::{sign, verify};

use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle_domain::ids::{CommitId, IntentId};
use statechronicle_domain::intent::{
    Intent, KeyId, Nonce, Operation, SignatureAlg, SignatureBlock,
};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

const FIXED_SEED: [u8; 32] = [42u8; 32];

fn fixed_key() -> SigningKey {
    SigningKey::from_bytes(&FIXED_SEED)
}

fn sample_created_at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn sample_intent() -> Intent {
    let inputs = BTreeMap::from([
        (
            String::from("from_owner"),
            serde_json::json!("account:example:player_123"),
        ),
        (
            String::from("to_owner"),
            serde_json::json!("account:example:player_456"),
        ),
    ]);
    Intent::new(
        TenantId(String::from("acme.game.alpha")),
        IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
        Operation::new(String::from("asset.transfer")).unwrap(),
        SubjectId(String::from("account:example:player_123")),
        ResourceId(String::from("asset:sword_001")),
        Some(StateType::UniqueAsset),
        41,
        inputs,
        None,
        sample_created_at(),
        None,
        Nonce::from_bytes(vec![7, 8, 9]).unwrap(),
    )
}

fn sample_commit() -> Commit {
    let root = hash_bytes(b"state-root");
    Commit::new(
        CommitScope::tenant(TenantId(String::from("acme.game.alpha"))),
        CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
        None,
        1,
        1,
        root.clone(),
        hash_bytes(b"previous-root"),
        root,
        sample_created_at(),
        SubjectId(String::from("service:statechronicle.example.net")),
        ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
    )
}

fn sign_body<T: serde::Serialize>(body: &T, key: &SigningKey) -> SignatureBlock {
    let canonical = canonicalize(body).unwrap();
    let signature = sign(&canonical, key);
    SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
        sig: signature,
    }
}

fn sample_intent_without_inputs() -> Intent {
    let mut intent = sample_intent();
    intent.inputs = BTreeMap::new();
    intent
}

#[test]
fn signed_intent_canonicalizes_and_verifies() {
    let key = fixed_key();
    // Empty inputs make the intent BCS-decodable (BCS is not self-describing,
    // so `serde_json::Value` payloads cannot be decoded).
    let intent = sample_intent_without_inputs();

    // The signature covers only the body's canonical bytes (ADR-004 §2).
    let body_bytes = canonicalize(&intent).unwrap();
    let digest = hash_bytes(&body_bytes);
    assert_eq!(digest.as_bytes().len(), 32);

    let envelope = Signed::new(intent.clone(), sign_body(&intent, &key));

    // The full envelope canonicalizes and BCS-round-trips.
    let envelope_bytes = canonicalize(&envelope).unwrap();
    let decoded: Signed<Intent> = bcs::from_bytes(&envelope_bytes).unwrap();
    assert_eq!(decoded, envelope);

    // The detached signature verifies against the body bytes.
    assert!(verify(&body_bytes, &key.verifying_key(), &envelope.signature.sig).is_ok());
}

#[test]
fn tampered_body_fails_verification() {
    let key = fixed_key();
    let intent = sample_intent();
    let envelope = Signed::new(intent.clone(), sign_body(&intent, &key));

    let body_bytes = canonicalize(&intent).unwrap();
    let mut tampered = body_bytes;
    tampered.push(0x00);

    assert!(verify(&tampered, &key.verifying_key(), &envelope.signature.sig).is_err());
}

#[test]
fn signed_intent_serde_json_view() {
    let key = fixed_key();
    let intent = sample_intent();
    let envelope = Signed::new(intent.clone(), sign_body(&intent, &key));

    let json = serde_json::to_string(&envelope).unwrap();
    assert!(json.contains("\"schema\":\"statechronicle.intent.v0\""));
    let decoded: Signed<Intent> = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, envelope);
}

#[test]
fn signed_commit_canonicalizes_and_verifies() {
    let key = fixed_key();
    let commit = sample_commit();

    let body_bytes = canonicalize(&commit).unwrap();
    let envelope = Signed::new(commit.clone(), sign_body(&commit, &key));

    let envelope_bytes = canonicalize(&envelope).unwrap();
    let decoded: Signed<Commit> = bcs::from_bytes(&envelope_bytes).unwrap();
    assert_eq!(decoded, envelope);

    assert!(verify(&body_bytes, &key.verifying_key(), &envelope.signature.sig).is_ok());
}

#[test]
fn signed_commit_tampered_body_fails_verification() {
    let key = fixed_key();
    let commit = sample_commit();
    let envelope = Signed::new(commit.clone(), sign_body(&commit, &key));

    let body_bytes = canonicalize(&commit).unwrap();
    let mut tampered = body_bytes;
    tampered.push(0x00);

    assert!(verify(&tampered, &key.verifying_key(), &envelope.signature.sig).is_err());
}

#[test]
fn signed_commit_serde_json_view() {
    let key = fixed_key();
    let commit = sample_commit();
    let envelope = Signed::new(commit.clone(), sign_body(&commit, &key));

    let json = serde_json::to_string(&envelope).unwrap();
    assert!(json.contains("\"schema\":\"statechronicle.commit.v0\""));
    let decoded: Signed<Commit> = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, envelope);
}
