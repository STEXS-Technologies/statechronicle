//! Level-tagged authentication paths for the sparse Merkle accumulator.
//!
//! Both inclusion and non-membership proofs carry the key, the authenticated
//! leaf hash, and the list of non-empty sibling hashes. Each sibling is tagged
//! with the key-bit `level` it applies to: the verifier walks key bits from
//! level 0 (leaf-adjacent) upward, filling the precomputed `default[level]`
//! constants at levels whose sibling subtree is empty (§16.2).

use crate::key::StateKey;

/// One non-empty sibling along an authentication path.
///
/// `level` is the key-bit index (0..=255) whose routing this sibling resolves:
/// the sibling subtree has height `level`, so the verifier uses
/// `default[level]` at that level when the proof omits a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathStep {
    /// The key-bit level this sibling applies to (0 = leaf-adjacent).
    pub level: usize,
    /// The sibling subtree hash (32 bytes).
    pub sibling: [u8; 32],
}

/// Inclusion proof for a state key/value under a state root.
///
/// Shape: `{ key, leaf_hash, steps }` where `leaf_hash` is the authenticated
/// leaf and `steps` are the level-tagged non-empty siblings (ascending level
/// order). The caller additionally checks
/// `claimed_state.hash == leaf_hash` (§16.3 step 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InclusionProof {
    /// The proven state key.
    pub key: StateKey,
    /// The authenticated leaf hash `H(0x11 || key || state_digest)`.
    pub leaf_hash: [u8; 32],
    /// Level-tagged sibling hashes, ascending by level.
    pub steps: Vec<PathStep>,
}

/// Non-membership proof: authenticates that a key's slot holds the empty leaf.
///
/// Identical shape to [`InclusionProof`]; the slot's leaf is the empty-leaf
/// constant, so a non-membership proof verifies through the same path walk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NonMembershipProof {
    /// The key whose slot is claimed empty.
    pub key: StateKey,
    /// The authenticated empty leaf hash (`EMPTY_LEAF_HASH`).
    pub leaf_hash: [u8; 32],
    /// Level-tagged sibling hashes, ascending by level.
    pub steps: Vec<PathStep>,
}

/// Proof that a tenant's state root is committed under a checkpoint root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TenantRootProof {
    /// The tenant whose root is proven.
    pub tenant_id: String,
    /// The claimed tenant state root.
    pub tenant_root: [u8; 32],
    /// The tenant's index in the sorted checkpoint leaf order (0-based).
    pub index: usize,
    /// Level-tagged sibling hashes, ascending by level (0 = leaf level).
    pub steps: Vec<CheckpointStep>,
}

/// One non-empty sibling along a checkpoint Merkle path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckpointStep {
    /// The tree level of the sibling node (0 = a leaf).
    pub level: usize,
    /// The sibling node hash (32 bytes).
    pub sibling: [u8; 32],
}
