#![no_main]

use libfuzzer_sys::fuzz_target;

use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateUpdate};
use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::ids::CommitId;
use statechronicle_domain::intent::{KeyId, SignatureAlg, SignatureBlock};
use statechronicle_domain::proof::{CommitRef, NonMembershipProofBundle};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_proof::inclusion::{sparse_proof_from_non_membership, steps_from_sparse_proof};
use statechronicle_proof::verify::verify_non_membership;

// The non-membership proof lane must never panic on arbitrary input, and its
// core guarantees must hold for any leaf set:
// - a genuine absent-key non-membership bundle verifies through the dense
//   wire form against the same root (reusing the accumulator's own path
//   verifier),
// - the dense wire roundtrip preserves the accumulator's level-tagged steps,
// - a tampered (non-empty) leaf can never verify against the same root —
//   the empty-leaf assertion fails closed.
fuzz_target!(|data: &[u8]| {
    // Derive a deterministic batch of (key, digest) updates from the input.
    let mut updates = Vec::new();
    for chunk in data.chunks(64).take(32) {
        let mut key = [0u8; 32];
        let mut digest = [0u8; 32];
        for (i, byte) in chunk.iter().take(32).enumerate() {
            key[i] = *byte;
        }
        for (i, byte) in chunk.iter().skip(32).take(32).enumerate() {
            digest[i] = *byte;
        }
        updates.push(StateUpdate::new(StateKey::new(key), digest));
    }

    let mut acc = StateAccumulator::empty();
    let Ok(root) = acc.insert_batch(&updates) else {
        return;
    };

    // Probe an absent key; the guard skips the (rare) present-probe case
    // where no non-membership proof exists.
    let probe = StateKey::new([0x42u8; 32]);
    if let Some(non_membership) = acc.prove_non_membership(&probe) {
        let sparse = sparse_proof_from_non_membership(&non_membership);

        // The dense wire roundtrip preserves the accumulator's steps.
        let steps = steps_from_sparse_proof(&sparse).unwrap();
        assert_eq!(steps, non_membership.steps);

        let bundle = NonMembershipProofBundle::new(
            TenantId(String::from("acme.game.alpha")),
            ResourceId(String::from("asset:sword_001")),
            ContentDigest::new(*probe.as_bytes()),
            CommitRef {
                commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
                sequence: 1,
                state_root: ContentDigest::new(*root.as_bytes()),
                signature: SignatureBlock {
                    alg: SignatureAlg::Ed25519,
                    key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
                    sig: statechronicle_core::signature::Signature::from_bytes([0u8; 64]),
                },
            },
            sparse,
        );

        // Genuine absence verifies against the same root.
        assert!(verify_non_membership(&bundle, &root, &probe).is_ok());

        // A tampered (non-empty) leaf always fails closed.
        let mut tampered = bundle;
        tampered.state_non_membership_proof.leaf_hash = ContentDigest::new([0x5au8; 32]);
        assert!(verify_non_membership(&tampered, &root, &probe).is_err());
    }
});
