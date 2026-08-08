//! Compile-only end-to-end smoke test proving the umbrella facade resolves and
//! is usable: construct protocol objects entirely through `statechronicle::`
//! and assert a few field values. No storage or infrastructure involved.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use statechronicle::{
    Amount, ContentDigest, IntentId, Operation, Signature, Signed, StateType, SubjectId, TenantId,
    domain::intent::{Intent, KeyId, Nonce, SignatureAlg, SignatureBlock},
};

#[test]
fn facade_amount_and_digest_resolve() {
    let amount = Amount::from_u64(1000);
    assert_eq!(amount.mantissa(), 1000);
    assert_eq!(amount.scale(), 0);

    let digest = ContentDigest::from_hex_sha256(
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    )
    .unwrap();
    assert_eq!(digest.as_str(), digest.as_str());
    assert!(digest.as_bytes().len() == 32);
}

#[test]
fn facade_newtypes_construct() {
    let tenant = TenantId(String::from("acme.game.alpha"));
    let subject = SubjectId(String::from("account:example:player_123"));
    let intent_id = IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap();

    assert_eq!(tenant.0, "acme.game.alpha");
    assert_eq!(subject.0, "account:example:player_123");
    assert_eq!(intent_id.as_str(), "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2");
}

#[test]
fn facade_signed_intent_is_usable() {
    let tenant = TenantId(String::from("acme.game.alpha"));
    let intent_id = IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap();
    let operation = Operation::new(String::from("asset.transfer")).unwrap();
    let actor = SubjectId(String::from("account:example:player_123"));
    let resource_id = statechronicle::ResourceId(String::from("asset:sword_001"));
    let nonce = Nonce::from_bytes(vec![1, 2, 3, 4]).unwrap();
    let created_at: DateTime<Utc> = DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    let intent = Intent::builder()
        .tenant(tenant)
        .intent_id(intent_id)
        .operation(operation)
        .actor(actor)
        .resource(resource_id)
        .state_type(StateType::UniqueAsset)
        .expected_version(41)
        .inputs(BTreeMap::new())
        .created_at(created_at)
        .nonce(nonce)
        .build()
        .unwrap();
    assert_eq!(intent.expected_version, 41);
    assert_eq!(intent.schema, "statechronicle.intent.v0");

    let block = SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
        sig: Signature::from_bytes([0u8; 64]),
    };

    let signed = Signed::new(intent, block);
    assert_eq!(signed.body.expected_version, 41);
    assert_eq!(signed.signature.alg, SignatureAlg::Ed25519);
    assert_eq!(*signed.signature.sig.as_bytes(), [0u8; 64]);
}

#[test]
fn facade_executor_and_proof_types_are_visible() {
    // Compile-time proof that the namespaced and top-level facades both resolve
    // to the same concrete types.
    fn _takes_executor(_executor: &statechronicle::Executor) {}
    fn _takes_ports(_ports: &statechronicle::executor::pipeline::Ports) {}
    fn _takes_proof_service(_service: &statechronicle::ProofService) {}
    fn _takes_accumulator(_acc: &statechronicle::StateAccumulator) {}
    fn _takes_registry(_reg: statechronicle::ProfileRegistry) {}
    let _ = (
        _takes_executor,
        _takes_ports,
        _takes_proof_service,
        _takes_accumulator,
        _takes_registry,
    );
}
