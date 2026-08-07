//! Proofs of state and inclusion (protocol §16).
//!
//! Proof bundles prove current state, historical inclusion, and ownership
//! without replaying the full history. The v0 proof bundle references the
//! enclosing signed commit via [`CommitRef`], whose signature block lets a
//! verifier check the commit signature against the bundle (protocol §16.2,
//! §16.3).

use serde::{Deserialize, Serialize};

use statechronicle_core::digest::ContentDigest;

use crate::authority::AuthorityProof;
use crate::ids::{CommitId, EventId};
use crate::intent::{Operation, SignatureBlock};
use crate::resource::ResourceId;
use crate::tenant::TenantId;

/// Schema identifier for v0 resource state proofs (protocol §16.2).
pub const RESOURCE_STATE_PROOF_SCHEMA: &str = "statechronicle.proof.resource_state.v0";

/// Schema identifier for v0 non-membership proof bundles (protocol §16.2).
pub const NON_MEMBERSHIP_PROOF_SCHEMA: &str = "statechronicle.proof.non_membership.v0";

/// The v0 sparse Merkle proof kind (protocol §16.2 `state_inclusion_proof`).
pub const SPARSE_MERKLE_V0: &str = "sparse_merkle_v0";

/// The purpose of a proof (protocol §16.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofType {
    /// Proves an event is included in a commit.
    Inclusion,
    /// Proves a resource state at a commit.
    State,
    /// Proves a valid before → after transition.
    Transition,
    /// Proves current owner/controller/holder.
    Ownership,
    /// Proves ordered event history for a resource.
    History,
    /// Proves snapshot authenticity and state root.
    Snapshot,
    /// Binds a TrustGrant evaluation to a transition.
    Authority,
    /// Proves a commit belongs to the canonical chain.
    Commit,
    /// Proves a state key holds no state at a commit (absent slot).
    NonMembership,
}

/// A sparse Merkle inclusion proof against a commit's state root.
///
/// `kind` is set to [`SPARSE_MERKLE_V0`] by the constructor; `path` holds the
/// sibling hashes from the leaf to the root and `leaf_hash` is the resource's
/// state hash (protocol §16.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SparseMerkleProof {
    /// The proof encoding kind, always [`SPARSE_MERKLE_V0`] for v0.
    pub kind: String,
    /// Sibling hashes along the leaf-to-root path.
    pub path: Vec<ContentDigest>,
    /// The canonical digest of the claimed leaf state.
    pub leaf_hash: ContentDigest,
}

impl SparseMerkleProof {
    /// Constructs a v0 sparse Merkle proof.
    pub fn new(path: Vec<ContentDigest>, leaf_hash: ContentDigest) -> Self {
        Self {
            kind: String::from(SPARSE_MERKLE_V0),
            path,
            leaf_hash,
        }
    }
}

/// The signed commit block embedded in a proof bundle (protocol §16.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitRef {
    /// The referenced commit id.
    pub commit_id: CommitId,
    /// The commit's sequence number.
    pub sequence: u64,
    /// The commit's next state root.
    pub state_root: ContentDigest,
    /// The detached commit signature over the commit body.
    pub signature: SignatureBlock,
}

/// The latest event reference embedded in a proof bundle (protocol §16.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRef {
    /// The latest event for the resource.
    pub event_id: EventId,
    /// The operation of the latest event.
    pub operation: Operation,
}

/// A portable proof that a resource's state is current (protocol §16.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceStateProof {
    /// Schema identifier, always [`RESOURCE_STATE_PROOF_SCHEMA`] for v0.
    pub schema: String,
    /// The tenant scope of the claimed state.
    pub tenant_id: TenantId,
    /// The resource whose state is claimed.
    pub resource_id: ResourceId,
    /// The claimed current state projection.
    pub claimed_state: serde_json::Value,
    /// The signed commit that pins the state root.
    pub commit: CommitRef,
    /// The sparse Merkle inclusion proof of the claimed leaf.
    pub state_inclusion_proof: SparseMerkleProof,
    /// The latest event for the resource.
    pub latest_event: EventRef,
    /// Optional authority proof binding a TrustGrant evaluation.
    pub authority: Option<AuthorityProof>,
}

impl ResourceStateProof {
    /// Constructs a resource state proof with the v0 schema identifier set.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        resource_id: ResourceId,
        claimed_state: serde_json::Value,
        commit: CommitRef,
        state_inclusion_proof: SparseMerkleProof,
        latest_event: EventRef,
        authority: Option<AuthorityProof>,
    ) -> Self {
        Self {
            schema: String::from(RESOURCE_STATE_PROOF_SCHEMA),
            tenant_id,
            resource_id,
            claimed_state,
            commit,
            state_inclusion_proof,
            latest_event,
            authority,
        }
    }
}

/// A portable proof that a state key holds no state at a commit
/// (protocol §16.2).
///
/// Proves absence: the sparse Merkle proof authenticates that the key's slot
/// holds the empty-leaf constant under the pinned commit's state root. The
/// claimed key is carried as a [`ContentDigest`] wrapping the 32 raw state
/// key bytes, so a verifier can bind the bundle to the caller's `StateKey`
/// without parsing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NonMembershipProofBundle {
    /// Schema identifier, always [`NON_MEMBERSHIP_PROOF_SCHEMA`] for v0.
    pub schema: String,
    /// The tenant scope of the claimed absent state.
    pub tenant_id: TenantId,
    /// The resource whose state is claimed absent.
    pub resource_id: ResourceId,
    /// The claimed absent state key, as a digest of the 32 raw key bytes.
    pub claimed_key: ContentDigest,
    /// The signed commit that pins the state root.
    pub commit: CommitRef,
    /// The sparse Merkle inclusion proof of the empty-leaf constant.
    pub state_non_membership_proof: SparseMerkleProof,
}

impl NonMembershipProofBundle {
    /// Constructs a non-membership proof bundle with the v0 schema identifier
    /// set.
    pub fn new(
        tenant_id: TenantId,
        resource_id: ResourceId,
        claimed_key: ContentDigest,
        commit: CommitRef,
        state_non_membership_proof: SparseMerkleProof,
    ) -> Self {
        Self {
            schema: String::from(NON_MEMBERSHIP_PROOF_SCHEMA),
            tenant_id,
            resource_id,
            claimed_key,
            commit,
            state_non_membership_proof,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::intent::{KeyId, SignatureAlg};
    use statechronicle_core::digest::hash_bytes;
    use statechronicle_core::signature::Signature;

    fn sample_digest() -> ContentDigest {
        hash_bytes(b"leaf")
    }

    fn sample_proof() -> ResourceStateProof {
        let signature = SignatureBlock {
            alg: SignatureAlg::Ed25519,
            key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
            sig: Signature::from_bytes([0u8; 64]),
        };
        ResourceStateProof::new(
            TenantId(String::from("stexs.game.alpha")),
            ResourceId(String::from("asset:sword_001")),
            serde_json::json!({ "owner": "account:stexs:player_456", "status": "active" }),
            CommitRef {
                commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
                sequence: 918273,
                state_root: sample_digest(),
                signature,
            },
            SparseMerkleProof::new(vec![sample_digest(), sample_digest()], sample_digest()),
            EventRef {
                event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
                operation: Operation::new(String::from("asset.transfer")).unwrap(),
            },
            None,
        )
    }

    #[test]
    fn constructor_sets_schema_and_kind() {
        let proof = sample_proof();
        assert_eq!(proof.schema, RESOURCE_STATE_PROOF_SCHEMA);
        assert_eq!(proof.state_inclusion_proof.kind, SPARSE_MERKLE_V0);
    }

    #[test]
    fn proof_type_serde_uses_snake_case() {
        assert_eq!(
            serde_json::to_string(&ProofType::Inclusion).unwrap(),
            "\"inclusion\""
        );
        assert_eq!(
            serde_json::to_string(&ProofType::Commit).unwrap(),
            "\"commit\""
        );
        assert_eq!(
            serde_json::to_string(&ProofType::Ownership).unwrap(),
            "\"ownership\""
        );
        assert_eq!(
            serde_json::to_string(&ProofType::NonMembership).unwrap(),
            "\"non_membership\""
        );
    }

    #[test]
    fn serde_json_roundtrips() {
        let proof = sample_proof();
        let json = serde_json::to_string(&proof).unwrap();
        let decoded: ResourceStateProof = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, proof);
    }

    #[test]
    fn bcs_canonicalization_is_deterministic() {
        // `claimed_state` is `serde_json::Value` — BCS-encodable but not
        // BCS-decodable (BCS is not self-describing, ADR-004) — so the BCS
        // check is encode-side determinism.
        let proof = sample_proof();
        let first = bcs::to_bytes(&proof).unwrap();
        let second = bcs::to_bytes(&proof).unwrap();
        assert_eq!(first, second);
        assert!(!first.is_empty());
    }

    fn sample_non_membership() -> NonMembershipProofBundle {
        let signature = SignatureBlock {
            alg: SignatureAlg::Ed25519,
            key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
            sig: Signature::from_bytes([0u8; 64]),
        };
        NonMembershipProofBundle::new(
            TenantId(String::from("stexs.game.alpha")),
            ResourceId(String::from("asset:sword_001")),
            ContentDigest::new([0xabu8; 32]),
            CommitRef {
                commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
                sequence: 918273,
                state_root: sample_digest(),
                signature,
            },
            SparseMerkleProof::new(vec![sample_digest(); 256], sample_digest()),
        )
    }

    #[test]
    fn non_membership_constructor_sets_schema_and_kind() {
        let bundle = sample_non_membership();
        assert_eq!(bundle.schema, NON_MEMBERSHIP_PROOF_SCHEMA);
        assert_eq!(bundle.state_non_membership_proof.kind, SPARSE_MERKLE_V0);
    }

    #[test]
    fn non_membership_serde_json_roundtrips() {
        let bundle = sample_non_membership();
        let json = serde_json::to_string(&bundle).unwrap();
        let decoded: NonMembershipProofBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, bundle);
    }

    #[test]
    fn non_membership_bcs_roundtrips() {
        // Unlike `ResourceStateProof`, the non-membership bundle carries no
        // `serde_json::Value`, so BCS round-trips fully (both directions).
        let bundle = sample_non_membership();
        let bytes = bcs::to_bytes(&bundle).unwrap();
        let decoded: NonMembershipProofBundle = bcs::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, bundle);
    }
}
