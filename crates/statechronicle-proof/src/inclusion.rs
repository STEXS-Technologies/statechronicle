//! Inclusion proofs.
//!
//! Proves that an event or state entry is included in a commit or root.
//! The accumulator produces level-tagged [`InclusionProof`]s (ADR-005); the
//! domain's portable [`SparseMerkleProof`] wire form (§16.2) is untagged, so
//! this module converts between the two encodings.
//!
//! Wire encoding: the domain `path` is **dense**: exactly
//! [`TREE_DEPTH`] sibling hashes in ascending level order (leaf-adjacent
//! first, index `level`), with empty levels filled by the precomputed
//! default-subtree hashes. A dense path is the minimal encoding the untagged
//! wire format can verify: it pins the level of every sibling, so the
//! verifier never has to guess which levels hold empty subtrees (the compact
//! flattened encoding is ambiguous and cannot be verified soundly against an
//! arbitrary root).

use statechronicle_accumulator::proof::{InclusionProof, NonMembershipProof, PathStep};
use statechronicle_accumulator::sparse_merkle::{TREE_DEPTH, default_hash};
use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::proof::{SPARSE_MERKLE_V0, SparseMerkleProof};

use crate::error::ProofError;

/// Converts an accumulator inclusion proof into the dense v0 wire form.
///
/// The resulting [`SparseMerkleProof`] carries `kind = sparse_merkle_v0`,
/// a dense 256-entry `path` (ascending level order, default-filled), and the
/// authenticated leaf hash.
pub fn sparse_proof_from_inclusion(proof: &InclusionProof) -> SparseMerkleProof {
    SparseMerkleProof::new(
        dense_path(&proof.steps),
        ContentDigest::new(proof.leaf_hash),
    )
}

/// Converts an accumulator non-membership proof into the dense v0 wire form.
///
/// A non-membership proof authenticates an absent slot, so its leaf hash is
/// the empty-leaf constant and the dense path is the inclusion path of that
/// empty leaf. The resulting [`SparseMerkleProof`] carries
/// `kind = sparse_merkle_v0`, a dense 256-entry `path` (ascending level order,
/// default-filled), and the empty-leaf hash.
pub fn sparse_proof_from_non_membership(proof: &NonMembershipProof) -> SparseMerkleProof {
    SparseMerkleProof::new(
        dense_path(&proof.steps),
        ContentDigest::new(proof.leaf_hash),
    )
}

/// Encodes level-tagged steps as a dense 256-entry path.
///
/// Every level whose sibling differs from its default-subtree hash is filled
/// at its exact level index; all other levels carry their default-subtree
/// hash. This is the minimal unambiguous dense wire encoding (§16.2).
fn dense_path(steps: &[PathStep]) -> Vec<ContentDigest> {
    let mut path: Vec<ContentDigest> = (0..TREE_DEPTH)
        .map(|level| ContentDigest::new(default_hash(level)))
        .collect();
    for step in steps {
        if let Some(slot) = path.get_mut(step.level) {
            *slot = ContentDigest::new(step.sibling);
        }
    }
    path
}

/// Rebuilds the accumulator's level-tagged steps from a dense v0 path.
///
/// Every dense-path level whose sibling differs from its default-subtree hash
/// becomes one [`PathStep`] at that exact level, in ascending level order.
///
/// # Errors
///
/// Returns [`ProofError::UnsupportedKind`] when `sparse.kind` is not
/// [`SPARSE_MERKLE_V0`], and [`ProofError::InvalidPathLength`] when the path
/// does not hold exactly [`TREE_DEPTH`] entries.
pub fn steps_from_sparse_proof(sparse: &SparseMerkleProof) -> Result<Vec<PathStep>, ProofError> {
    if sparse.kind != SPARSE_MERKLE_V0 {
        return Err(ProofError::UnsupportedKind(sparse.kind.clone()));
    }
    if sparse.path.len() != TREE_DEPTH {
        return Err(ProofError::InvalidPathLength {
            expected: TREE_DEPTH,
            actual: sparse.path.len(),
        });
    }
    let mut steps = Vec::new();
    for (level, digest) in sparse.path.iter().enumerate() {
        if digest.as_bytes() != &default_hash(level) {
            steps.push(PathStep {
                level,
                sibling: *digest.as_bytes(),
            });
        }
    }
    Ok(steps)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use statechronicle_accumulator::key::StateKey;
    use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateUpdate};

    fn update(key_byte: u8, digest_byte: u8) -> StateUpdate {
        StateUpdate::new(StateKey::new([key_byte; 32]), [digest_byte; 32])
    }

    #[test]
    fn dense_conversion_roundtrips_steps() {
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[update(0x01, 0xa1), update(0x02, 0xa2), update(0x80, 0xa3)])
            .unwrap();
        let proof = acc.prove_inclusion(&StateKey::new([0x01; 32])).unwrap();
        let sparse = sparse_proof_from_inclusion(&proof);

        assert_eq!(sparse.kind, SPARSE_MERKLE_V0);
        assert_eq!(sparse.path.len(), TREE_DEPTH);
        assert_eq!(sparse.leaf_hash.as_bytes(), &proof.leaf_hash);

        let steps = steps_from_sparse_proof(&sparse).unwrap();
        assert_eq!(steps, proof.steps);
    }

    #[test]
    fn sparse_proof_has_defaults_at_empty_levels() {
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[update(0x01, 0xa1)]).unwrap();
        let proof = acc.prove_inclusion(&StateKey::new([0x01; 32])).unwrap();
        let sparse = sparse_proof_from_inclusion(&proof);

        // Every level that is not a genuine step must carry its default hash.
        let step_levels: std::collections::BTreeSet<usize> =
            proof.steps.iter().map(|step| step.level).collect();
        for (level, digest) in sparse.path.iter().enumerate() {
            if step_levels.contains(&level) {
                assert_ne!(digest.as_bytes(), &default_hash(level));
            } else {
                assert_eq!(digest.as_bytes(), &default_hash(level));
            }
        }
    }

    #[test]
    fn steps_from_sparse_rejects_wrong_kind() {
        // `SparseMerkleProof::new` forces the v0 kind, so construct the
        // wrong-kind case by setting the public `kind` field directly.
        let mut sparse = SparseMerkleProof::new(
            vec![ContentDigest::new([0u8; 32])],
            ContentDigest::new([1u8; 32]),
        );
        sparse.kind = String::from("jellyfish_v1");
        assert!(matches!(
            steps_from_sparse_proof(&sparse),
            Err(ProofError::UnsupportedKind(_))
        ));
    }

    #[test]
    fn steps_from_sparse_rejects_wrong_length() {
        let sparse = SparseMerkleProof::new(
            (0..2).map(|i| ContentDigest::new([i; 32])).collect(),
            ContentDigest::new([1u8; 32]),
        );
        assert!(matches!(
            steps_from_sparse_proof(&sparse),
            Err(ProofError::InvalidPathLength {
                expected: 256,
                actual: 2
            })
        ));
    }

    #[test]
    fn non_membership_dense_conversion_uses_empty_leaf() {
        use statechronicle_accumulator::sparse_merkle::EMPTY_LEAF_HASH;
        let mut acc = StateAccumulator::empty();
        acc.insert_batch(&[update(0x01, 0xa1), update(0x02, 0xa2)])
            .unwrap();
        let absent = StateKey::new([0x03; 32]);
        let proof = acc.prove_non_membership(&absent).unwrap();
        let sparse = sparse_proof_from_non_membership(&proof);

        assert_eq!(sparse.kind, SPARSE_MERKLE_V0);
        assert_eq!(sparse.path.len(), TREE_DEPTH);
        assert_eq!(sparse.leaf_hash.as_bytes(), &EMPTY_LEAF_HASH);

        // The dense path round-trips back to the accumulator's steps.
        let steps = steps_from_sparse_proof(&sparse).unwrap();
        assert_eq!(steps, proof.steps);
    }
}
