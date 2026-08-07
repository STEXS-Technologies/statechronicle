//! Logical-isolation checkpoint root (ADR-005 §2, §8.1).
//!
//! Global checkpoint root = a plain Merkle tree over **sorted**
//! `(TenantId, StateRoot)` pairs. Leaves are
//! `H(0x12 || u64le(len(tenant_id)) || tenant_id || state_root)`, internals
//! are `H(0x13 || left || right)`, and a level with an odd node count
//! duplicates its last node (odd-duplication). Tenant counts are small, so
//! this tree is tiny (`log₂T × 32 B` per tenant-root proof).

use sha2::{Digest as _, Sha256};
use statechronicle_domain::tenant::TenantId;

use crate::error::AccumulatorError;
use crate::proof::{CheckpointStep, TenantRootProof};
use crate::sparse_merkle::StateRoot;

/// Domain tag for checkpoint leaves.
pub const CHECKPOINT_LEAF_TAG: u8 = 0x12;

/// Domain tag for checkpoint internal nodes.
pub const CHECKPOINT_INTERNAL_TAG: u8 = 0x13;

/// A global checkpoint root committing one state root per tenant.
#[derive(Clone, Debug)]
pub struct CheckpointRoot {
    root: [u8; 32],
    /// Sorted `(tenant_id, state_root)` entries (the leaf order).
    entries: Vec<(TenantId, StateRoot)>,
    /// All tree levels, bottom-up: `levels[0]` are the leaves.
    levels: Vec<Vec<[u8; 32]>>,
}

impl CheckpointRoot {
    /// Builds a checkpoint root from a set of `(TenantId, StateRoot)` pairs.
    ///
    /// The pairs are sorted by `TenantId` (canonical byte order) before
    /// hashing; this is a pure function of the pair set, independent of input
    /// order.
    ///
    /// # Errors
    ///
    /// Returns [`AccumulatorError::EmptyTenantRoots`] when `tenant_roots` is
    /// empty, and [`AccumulatorError::DuplicateTenant`] when the same tenant
    /// appears more than once.
    pub fn from_tenant_roots(
        tenant_roots: &[(TenantId, StateRoot)],
    ) -> Result<Self, AccumulatorError> {
        if tenant_roots.is_empty() {
            return Err(AccumulatorError::EmptyTenantRoots);
        }
        let mut entries: Vec<(TenantId, StateRoot)> = tenant_roots.to_vec();
        entries.sort_by_key(|a| a.0.0.clone());
        let mut duplicate: Option<&TenantId> = None;
        let mut previous: Option<&TenantId> = None;
        for (tenant, _) in &entries {
            if previous.is_some_and(|prev| prev == tenant) {
                duplicate = Some(tenant);
                break;
            }
            previous = Some(tenant);
        }
        if let Some(tenant) = duplicate {
            return Err(AccumulatorError::DuplicateTenant(tenant.0.clone()));
        }
        let leaves: Vec<[u8; 32]> = entries
            .iter()
            .map(|(tenant, root)| checkpoint_leaf(tenant.0.as_str(), *root.as_bytes()))
            .collect();
        let levels = build_levels(leaves);
        let Some(root) = levels.last().and_then(|level| level.first()) else {
            return Err(AccumulatorError::EmptyTenantRoots);
        };
        Ok(Self {
            root: *root,
            entries,
            levels,
        })
    }

    /// Returns the checkpoint root bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.root
    }

    /// Returns the tenant's state root under this checkpoint, if present.
    pub fn tenant_root(&self, tenant_id: &TenantId) -> Option<StateRoot> {
        self.entries
            .iter()
            .find(|(tenant, _)| tenant == tenant_id)
            .map(|(_, root)| *root)
    }

    /// Produces a proof that `tenant_id`'s root is committed under this
    /// checkpoint, or `None` when the tenant is absent.
    pub fn prove_tenant_root(&self, tenant_id: &TenantId) -> Option<TenantRootProof> {
        let index = self
            .entries
            .iter()
            .position(|(tenant, _)| tenant == tenant_id)?;
        let (_, tenant_root) = self.entries.get(index)?;
        let mut steps = Vec::new();
        let mut node_index = index;
        for (level, nodes) in self.levels.iter().enumerate() {
            if nodes.len() <= 1 {
                break;
            }
            let Some(path_node) = nodes.get(node_index) else {
                break;
            };
            let sibling = if node_index & 1 == 0 {
                // Even index: sibling is the right neighbour, or self when it
                // is the duplicated last node.
                let right = node_index.checked_add(1).unwrap_or(node_index);
                if right >= nodes.len() {
                    *path_node
                } else {
                    *nodes.get(right).unwrap_or(path_node)
                }
            } else {
                let left = node_index.checked_sub(1).unwrap_or(node_index);
                *nodes.get(left).unwrap_or(path_node)
            };
            steps.push(CheckpointStep { level, sibling });
            node_index = node_index.wrapping_shr(1);
        }
        Some(TenantRootProof {
            tenant_id: tenant_id.0.clone(),
            tenant_root: *tenant_root.as_bytes(),
            index,
            steps,
        })
    }

    /// Verifies a tenant-root proof against `root`.
    pub fn verify_tenant_root(root: &CheckpointRoot, proof: &TenantRootProof) -> bool {
        let leaf = checkpoint_leaf(&proof.tenant_id, proof.tenant_root);
        let mut acc = leaf;
        let mut node_index = proof.index;
        for step in &proof.steps {
            let (left, right) = if node_index & 1 == 0 {
                (acc, step.sibling)
            } else {
                (step.sibling, acc)
            };
            acc = checkpoint_internal(left, right);
            node_index = node_index.wrapping_shr(1);
        }
        acc == root.root
    }
}

/// `H(0x12 || u64le(len(tenant)) || tenant || root)`.
pub fn checkpoint_leaf(tenant_id: &str, root: [u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([CHECKPOINT_LEAF_TAG]);
    hasher.update((tenant_id.len() as u64).to_le_bytes());
    hasher.update(tenant_id.as_bytes());
    hasher.update(root);
    hasher.finalize().into()
}

/// `H(0x13 || left || right)`.
pub fn checkpoint_internal(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([CHECKPOINT_INTERNAL_TAG]);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

/// Builds the checkpoint tree bottom-up, duplicating the last node of an odd
/// level.
fn build_levels(leaves: Vec<[u8; 32]>) -> Vec<Vec<[u8; 32]>> {
    let mut levels = vec![leaves];
    while levels.last().is_some_and(|level| level.len() > 1) {
        let Some(current) = levels.last() else {
            break;
        };
        let mut next = Vec::with_capacity(current.len().div_ceil(2));
        for pair in current.chunks(2) {
            let first = pair.first().copied().unwrap_or([0u8; 32]);
            let second = pair.get(1).copied().unwrap_or(first);
            next.push(checkpoint_internal(first, second));
        }
        levels.push(next);
    }
    levels
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::CheckpointRoot;
    use crate::error::AccumulatorError;
    use crate::sparse_merkle::StateRoot;
    use statechronicle_domain::tenant::TenantId;

    #[test]
    fn empty_roots_rejected() {
        let error = CheckpointRoot::from_tenant_roots(&[]).unwrap_err();
        assert_eq!(error, AccumulatorError::EmptyTenantRoots);
    }

    #[test]
    fn duplicate_tenant_rejected() {
        let pairs = [
            (
                TenantId(String::from("tenant:alpha")),
                StateRoot::new([0xaau8; 32]),
            ),
            (
                TenantId(String::from("tenant:alpha")),
                StateRoot::new([0xbbu8; 32]),
            ),
        ];
        let error = CheckpointRoot::from_tenant_roots(&pairs).unwrap_err();
        assert_eq!(
            error,
            AccumulatorError::DuplicateTenant(String::from("tenant:alpha"))
        );
    }

    #[test]
    fn single_tenant_root_is_its_leaf() {
        let pairs = [(
            TenantId(String::from("tenant:alpha")),
            StateRoot::new([0xaau8; 32]),
        )];
        let checkpoint = CheckpointRoot::from_tenant_roots(&pairs).unwrap();
        assert_eq!(
            checkpoint.as_bytes(),
            &super::checkpoint_leaf("tenant:alpha", [0xaau8; 32])
        );
    }

    #[test]
    fn two_tenant_known_answer() {
        let pairs = [
            (
                TenantId(String::from("tenant:alpha")),
                StateRoot::new([0xaau8; 32]),
            ),
            (
                TenantId(String::from("tenant:beta")),
                StateRoot::new([0xbbu8; 32]),
            ),
        ];
        let checkpoint = CheckpointRoot::from_tenant_roots(&pairs).unwrap();
        assert_eq!(
            hex(checkpoint.as_bytes()),
            "fe1c9e911c7a12302a09e57cd02cb218426ba5a734148f64a9748e691710ade6"
        );
    }

    fn hex(bytes: &[u8; 32]) -> String {
        hex::encode(bytes)
    }
}
