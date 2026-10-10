//! Standard/portable protocol equivalence and malformed-input boundaries.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use ed25519_dalek::SigningKey;
use proptest::prelude::*;
use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateUpdate};
use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::signature::sign;
use statechronicle_domain::commit::ProfileId;
use statechronicle_domain::custom::CustomState;
use statechronicle_domain::resource_state::ResourceState;
use statechronicle_portable::codec::{CodecError, Decoder, Encoder};
use statechronicle_portable::integrity::{
    PathStep, custom_state_hash, resource_key, verify_inclusion, verify_non_membership,
    verify_signature,
};

fn steps(input: &[statechronicle_accumulator::proof::PathStep]) -> Vec<PathStep> {
    input
        .iter()
        .map(|step| PathStep {
            level: u8::try_from(step.level).unwrap(),
            sibling: step.sibling,
        })
        .collect()
}

#[test]
fn standard_sparse_proofs_and_ed25519_are_portable() {
    let key = StateKey::for_resource("tenant", "configuration");
    let state = [0x42; 32];
    let mut tree = StateAccumulator::empty();
    tree.insert_batch(&[
        StateUpdate::new(key, state),
        StateUpdate::new(StateKey::for_resource("tenant", "equipment"), [0x13; 32]),
    ])
    .unwrap();
    let proof = tree.prove_inclusion(&key).unwrap();
    let path = steps(&proof.steps);
    assert_eq!(resource_key("tenant", "configuration"), *key.as_bytes());
    assert!(verify_inclusion(
        tree.root().as_bytes(),
        key.as_bytes(),
        &state,
        &path
    ));
    assert!(!verify_inclusion(
        tree.root().as_bytes(),
        key.as_bytes(),
        &[0; 32],
        &path
    ));
    let missing = StateKey::for_resource("tenant", "missing");
    let absence = tree.prove_non_membership(&missing).unwrap();
    assert!(verify_non_membership(
        tree.root().as_bytes(),
        missing.as_bytes(),
        &steps(&absence.steps)
    ));
    assert!(!verify_non_membership(
        tree.root().as_bytes(),
        key.as_bytes(),
        &path
    ));
    let duplicate = [PathStep {
        level: 0,
        sibling: [0; 32],
    }; 2];
    assert!(!verify_inclusion(
        tree.root().as_bytes(),
        key.as_bytes(),
        &state,
        &duplicate
    ));
    let unordered = [
        PathStep {
            level: 2,
            sibling: [0; 32],
        },
        PathStep {
            level: 1,
            sibling: [0; 32],
        },
    ];
    assert!(!verify_non_membership(
        tree.root().as_bytes(),
        missing.as_bytes(),
        &unordered
    ));
    let signing = SigningKey::from_bytes(&[0x25; 32]);
    let body = bcs::to_bytes(&("example.configuration", 1_u32, state)).unwrap();
    let signature = sign(&body, &signing);
    assert!(verify_signature(
        &body,
        &signing.verifying_key().to_bytes(),
        signature.as_bytes()
    ));
    assert!(!verify_signature(
        b"changed",
        &signing.verifying_key().to_bytes(),
        signature.as_bytes()
    ));
    assert!(!verify_signature(&body, &[0; 32], signature.as_bytes()));
}

#[test]
fn canonical_boundaries_and_atomic_primitive_writes() {
    for bytes in [&[0x80, 0][..], &[0x81, 0][..]] {
        assert_eq!(Decoder::new(bytes).uleb(), Err(CodecError::NonCanonical));
    }
    assert_eq!(Decoder::new(&[0xff; 5]).uleb(), Err(CodecError::Overflow));
    assert_eq!(Decoder::new(&[0x80]).uleb(), Err(CodecError::Truncated));
    assert_eq!(Decoder::new(&[2]).boolean(), Err(CodecError::NonCanonical));
    assert_eq!(Decoder::new(&[1]).finish(), Err(CodecError::Trailing));
    assert_eq!(Decoder::new(&[1, 0xff]).string(8), Err(CodecError::Utf8));
    assert_eq!(
        Decoder::new(&[4, 1, 2, 3, 4]).bytes(3),
        Err(CodecError::Capacity)
    );
    let mut buffer = [0x55; 2];
    let mut output = Encoder::new(&mut buffer);
    assert_eq!(output.bytes(b"ab"), Err(CodecError::Capacity));
    assert!(output.written().is_empty());
    assert_eq!(buffer, [0x55; 2]);
}

proptest! {
    #[test]
    fn fixed_fields_and_custom_envelope_match_standard(
        revision in any::<u32>(), enabled in any::<bool>(), amount in any::<u64>(),
        payload in proptest::collection::vec(any::<u8>(), 0..512), schema in 1_u32..u32::MAX,
    ) {
        let mut buffer = [0_u8; 1024];
        let mut output = Encoder::new(&mut buffer);
        output.u32(revision).unwrap(); output.boolean(enabled).unwrap();
        output.u64(amount).unwrap(); output.bytes(&payload).unwrap();
        prop_assert_eq!(output.written(), bcs::to_bytes(&(revision, enabled, amount, &payload)).unwrap());
        let mut input = Decoder::new(output.written());
        prop_assert_eq!(input.u32().unwrap(), revision);
        prop_assert_eq!(input.boolean().unwrap(), enabled);
        prop_assert_eq!(input.u64().unwrap(), amount);
        prop_assert_eq!(input.bytes(512).unwrap(), &payload);
        prop_assert_eq!(input.finish(), Ok(()));
        let state = ResourceState::Custom(CustomState {
            domain: ProfileId::new(String::from("example.configuration")).unwrap(),
            schema_version: schema, payload: payload.clone(),
        });
        prop_assert_eq!(custom_state_hash("example.configuration", schema, &payload).unwrap(),
            *canonicalize_and_digest(&state).unwrap().as_bytes());
        prop_assert_ne!(custom_state_hash("example.equipment", schema, &payload).unwrap(),
            custom_state_hash("example.configuration", schema, &payload).unwrap());
    }
    #[test]
    fn uleb_matches_bcs_sequence_prefix(value in any::<u32>()) {
        let mut buffer = [0; 5];
        let mut output = Encoder::new(&mut buffer);
        output.uleb(value).unwrap();
        let mut input = Decoder::new(output.written());
        prop_assert_eq!(input.uleb(), Ok(value));
        prop_assert_eq!(input.finish(), Ok(()));
        // Independent shortest-length calculation.
        let expected = if value < 1 << 7 { 1 } else if value < 1 << 14 { 2 }
            else if value < 1 << 21 { 3 } else if value < 1 << 28 { 4 } else { 5 };
        prop_assert_eq!(output.written().len(), expected);
    }
}
