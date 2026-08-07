//! Tenant checkpoint commits (protocol §13.4).
//!
//! A deployment that batches many tenants together may publish global
//! checkpoint commits that contain tenant roots rather than directly
//! containing all events. The wire shape is the
//! `statechronicle.global_checkpoint.v0` schema: a sequence number, one
//! [`TenantRootEntry`] per anchored tenant (carrying the tenant's tip
//! `commit_id` and `state_root`), and a `tenant_merkle_root` over the sorted
//! `(tenant_id, state_root)` pairs committed by the accumulator's
//! [`CheckpointRoot`]. The root is a pure function of the pair set, so it is
//! independent of entry order; the per-entry `commit_id` is anchored next to
//! the root but is deliberately *not* part of the merkle root derivation.
//!
//! Note on object identity: the domain `Commit` type exposes a
//! global-checkpoint *scope* (`statechronicle_domain::commit::CommitScope::global_checkpoint`),
//! which lets a `Commit` be scoped globally. This module's dedicated
//! [`GlobalCheckpoint`] is the distinct §13.4 wire shape with its own schema
//! (unlike `statechronicle_domain::commit::COMMIT_SCHEMA`): a tenant-scoped
//! `Commit` (carrying direct events) and a global checkpoint (carrying tenant
//! roots) are deliberately different objects.
//!
//! Global checkpoints are optional and must never weaken tenant-level
//! verification: each entry carries the tenant's authoritative commit id and
//! state root, and the merkle root commits the whole set (ADR-005 §8.1).
//!
//! Signing mirrors [`crate::sign`]: the body is BCS-canonicalized (the
//! `bcs::to_bytes` wire encoding via `statechronicle_core::canonicalize`) and
//! signed with the ADR-004 structural envelope so the signature never covers
//! a `signature` field.

use chrono::{DateTime, Utc};
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use statechronicle_accumulator::checkpoint::CheckpointRoot;
use statechronicle_accumulator::sparse_merkle::StateRoot;
use statechronicle_core::canonicalize::canonicalize;
use statechronicle_core::digest::ContentDigest;
use statechronicle_core::signature::{sign, verify};

use statechronicle_domain::ids::CommitId;
use statechronicle_domain::intent::{KeyId, SignatureAlg, SignatureBlock};
use statechronicle_domain::signed::Signed;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use crate::error::CommitError;

/// Schema identifier for v0 global checkpoints (protocol §13.4).
pub const GLOBAL_CHECKPOINT_SCHEMA: &str = "statechronicle.global_checkpoint.v0";

/// One tenant's anchored root inside a global checkpoint (protocol §13.4).
///
/// The `state_root` carries the tenant's authoritative state root and the
/// `commit_id` identifies the tenant's tip commit at checkpoint time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantRootEntry {
    /// The tenant whose history is anchored.
    pub tenant_id: TenantId,
    /// The tenant's tip commit id at checkpoint time.
    pub commit_id: CommitId,
    /// The tenant's state root committed under this checkpoint.
    ///
    /// [`StateRoot`] does not implement serde (the accumulator keeps serde
    /// out of its dependency graph), so it is serialized as its raw 32 bytes
    /// via [`serialize_state_root`] / [`deserialize_state_root`].
    #[serde(
        serialize_with = "serialize_state_root",
        deserialize_with = "deserialize_state_root"
    )]
    pub state_root: StateRoot,
}

/// Serializes a [`StateRoot`] as its raw 32 bytes.
///
/// The protocol §13.4 logical view renders `state_root` as a digest string;
/// the wire bytes here are the raw root bytes, which BCS length-prefixes.
fn serialize_state_root<S>(root: &StateRoot, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_bytes(root.as_bytes())
}

/// Deserializes a [`StateRoot`] from its raw 32 bytes.
///
/// # Errors
///
/// Returns a serde error when the serialized form is not exactly 32 bytes.
fn deserialize_state_root<'de, D>(deserializer: D) -> Result<StateRoot, D::Error>
where
    D: Deserializer<'de>,
{
    let bytes = Vec::<u8>::deserialize(deserializer)?;
    let root: [u8; 32] = bytes.try_into().map_err(|decoded: Vec<u8>| {
        serde::de::Error::custom(format!(
            "state root must be exactly 32 bytes, got {}",
            decoded.len()
        ))
    })?;
    Ok(StateRoot::new(root))
}

/// A signed global checkpoint body (protocol §13.4).
///
/// Compact snapshot that anchors each tenant's state root at a point in
/// history. The signature is detached per ADR-004 and lives in
/// [`Signed<GlobalCheckpoint>`]; this body type never carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalCheckpoint {
    /// Schema identifier, always [`GLOBAL_CHECKPOINT_SCHEMA`] for v0.
    pub schema: String,
    /// Monotonic global checkpoint sequence number.
    pub sequence: u64,
    /// One root entry per anchored tenant.
    pub tenant_roots: Vec<TenantRootEntry>,
    /// Merkle root over the sorted `(tenant_id, state_root)` pairs.
    pub tenant_merkle_root: ContentDigest,
    /// When the checkpoint was created (UTC).
    pub created_at: DateTime<Utc>,
    /// The authorized executor that signed the checkpoint.
    pub executor: SubjectId,
}

impl GlobalCheckpoint {
    /// Constructs a global checkpoint with the v0 schema identifier set.
    ///
    /// Callers normally use [`build_global_checkpoint`], which derives the
    /// merkle root; this constructor takes the root as given.
    pub fn new(
        sequence: u64,
        tenant_roots: Vec<TenantRootEntry>,
        tenant_merkle_root: ContentDigest,
        created_at: DateTime<Utc>,
        executor: SubjectId,
    ) -> Self {
        Self {
            schema: String::from(GLOBAL_CHECKPOINT_SCHEMA),
            sequence,
            tenant_roots,
            tenant_merkle_root,
            created_at,
            executor,
        }
    }
}

/// Builds a global checkpoint over `entries`, deriving the tenant merkle root.
///
/// Reuses the accumulator's [`CheckpointRoot::from_tenant_roots`], which sorts
/// the `(tenant_id, state_root)` pairs into canonical leaf order and is a pure
/// function of the pair set (ADR-005 §8.1). Each entry's `commit_id` is carried
/// alongside its root but does not enter the merkle derivation.
///
/// # Errors
///
/// Returns [`CommitError::Accumulator`] when `entries` is empty or two entries
/// share the same tenant (fail-closed, protocol §13.4).
pub fn build_global_checkpoint(
    entries: Vec<TenantRootEntry>,
    sequence: u64,
    created_at: DateTime<Utc>,
    executor: SubjectId,
) -> Result<GlobalCheckpoint, CommitError> {
    let pairs: Vec<(TenantId, StateRoot)> = entries
        .iter()
        .map(|entry| (entry.tenant_id.clone(), entry.state_root))
        .collect();
    let checkpoint_root = CheckpointRoot::from_tenant_roots(&pairs)?;
    let tenant_merkle_root = ContentDigest::new(*checkpoint_root.as_bytes());
    Ok(GlobalCheckpoint::new(
        sequence,
        entries,
        tenant_merkle_root,
        created_at,
        executor,
    ))
}

/// Signs a global checkpoint body and wraps it in the signed envelope.
///
/// Mirrors [`crate::sign::sign_commit`]: BCS-canonicalizes the body (the
/// `bcs::to_bytes` wire encoding) and signs the canonical bytes with the
/// Ed25519 key, covering only the body (ADR-004 §2).
///
/// # Errors
///
/// Returns [`CommitError::Core`] when the body cannot be BCS canonicalized.
pub fn sign_global_checkpoint(
    checkpoint: &GlobalCheckpoint,
    key: &SigningKey,
    key_id: KeyId,
) -> Result<Signed<GlobalCheckpoint>, CommitError> {
    let canonical = canonicalize(checkpoint)?;
    let signature = sign(&canonical, key);
    let block = SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id,
        sig: signature,
    };
    Ok(Signed::new(checkpoint.clone(), block))
}

/// Verifies a signed global checkpoint's detached signature over the body.
///
/// # Errors
///
/// Returns [`CommitError::Core`] when the body cannot be BCS canonicalized or
/// the signature fails strict Ed25519 verification (ZIP-215 malleability
/// checks).
pub fn verify_global_checkpoint(
    signed: &Signed<GlobalCheckpoint>,
    verifying_key: &VerifyingKey,
) -> Result<(), CommitError> {
    let canonical = canonicalize(&signed.body)?;
    verify(&canonical, verifying_key, &signed.signature.sig)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use statechronicle_accumulator::checkpoint::checkpoint_leaf;

    const FIXED_SEED: [u8; 32] = [42u8; 32];

    fn fixed_key() -> SigningKey {
        SigningKey::from_bytes(&FIXED_SEED)
    }

    fn key_id() -> KeyId {
        KeyId::new(String::from("did:key:z6Mk...#global-checkpoint")).unwrap()
    }

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-14T00:00:03Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn executor() -> SubjectId {
        SubjectId(String::from("service:statechronicle.stexs.net"))
    }

    fn entry(tenant: &str, commit: &str, root: [u8; 32]) -> TenantRootEntry {
        TenantRootEntry {
            tenant_id: TenantId(String::from(tenant)),
            commit_id: CommitId::new(String::from(commit)).unwrap(),
            state_root: StateRoot::new(root),
        }
    }

    fn sample_checkpoint() -> GlobalCheckpoint {
        let entries = vec![
            entry("tenant:alpha", "cmt_alpha_001", [0xaau8; 32]),
            entry("tenant:beta", "cmt_beta_001", [0xbbu8; 32]),
        ];
        build_global_checkpoint(entries, 7, timestamp(), executor()).unwrap()
    }

    #[test]
    fn single_tenant_root_is_its_leaf() {
        let entries = vec![entry("tenant:alpha", "cmt_alpha_001", [0xaau8; 32])];
        let checkpoint = build_global_checkpoint(entries, 1, timestamp(), executor()).unwrap();
        assert_eq!(
            checkpoint.tenant_merkle_root.as_bytes(),
            &checkpoint_leaf("tenant:alpha", [0xaau8; 32])
        );
    }

    #[test]
    fn two_tenant_known_answer() {
        // Known answer shared with the accumulator checkpoint tree: alpha and
        // beta at [0xaa; 32] / [0xbb; 32] root to this digest.
        let checkpoint = sample_checkpoint();
        assert_eq!(
            checkpoint.tenant_merkle_root.as_str(),
            "sha256:fe1c9e911c7a12302a09e57cd02cb218426ba5a734148f64a9748e691710ade6"
        );
    }

    #[test]
    fn root_is_order_independent() {
        let mut entries = vec![
            entry("tenant:alpha", "cmt_alpha_001", [0xaau8; 32]),
            entry("tenant:beta", "cmt_beta_001", [0xbbu8; 32]),
        ];
        let forward = build_global_checkpoint(entries.clone(), 1, timestamp(), executor()).unwrap();
        entries.reverse();
        let reversed = build_global_checkpoint(entries, 1, timestamp(), executor()).unwrap();
        assert_eq!(forward.tenant_merkle_root, reversed.tenant_merkle_root);
    }

    #[test]
    fn root_ignores_entry_commit_id() {
        // The merkle root is a pure function of (tenant_id, state_root); the
        // per-entry commit_id is anchored but not hashed into the root.
        let a = build_global_checkpoint(
            vec![entry("tenant:alpha", "cmt_alpha_001", [0xaau8; 32])],
            1,
            timestamp(),
            executor(),
        )
        .unwrap();
        let b = build_global_checkpoint(
            vec![entry("tenant:alpha", "cmt_alpha_002", [0xaau8; 32])],
            1,
            timestamp(),
            executor(),
        )
        .unwrap();
        assert_eq!(a.tenant_merkle_root, b.tenant_merkle_root);
        assert_eq!(a.tenant_roots[0].commit_id.as_str(), "cmt_alpha_001");
        assert_eq!(b.tenant_roots[0].commit_id.as_str(), "cmt_alpha_002");
    }

    #[test]
    fn empty_entries_are_rejected() {
        let error = build_global_checkpoint(Vec::new(), 1, timestamp(), executor()).unwrap_err();
        assert!(matches!(error, CommitError::Accumulator(_)));
    }

    #[test]
    fn duplicate_tenants_are_rejected() {
        let entries = vec![
            entry("tenant:alpha", "cmt_alpha_001", [0xaau8; 32]),
            entry("tenant:alpha", "cmt_alpha_002", [0xbbu8; 32]),
        ];
        let error = build_global_checkpoint(entries, 1, timestamp(), executor()).unwrap_err();
        assert!(matches!(error, CommitError::Accumulator(_)));
    }

    #[test]
    fn constructor_sets_schema() {
        let checkpoint = sample_checkpoint();
        assert_eq!(checkpoint.schema, GLOBAL_CHECKPOINT_SCHEMA);
        assert_eq!(checkpoint.sequence, 7);
        assert_eq!(checkpoint.executor, executor());
    }

    #[test]
    fn sign_then_verify_succeeds() {
        let checkpoint = sample_checkpoint();
        let key = fixed_key();
        let signed = sign_global_checkpoint(&checkpoint, &key, key_id()).unwrap();
        assert_eq!(signed.body, checkpoint);
        assert_eq!(signed.signature.alg, SignatureAlg::Ed25519);
        assert_eq!(
            signed.signature.key_id.as_str(),
            "did:key:z6Mk...#global-checkpoint"
        );
        assert!(verify_global_checkpoint(&signed, &key.verifying_key()).is_ok());
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let checkpoint = sample_checkpoint();
        let key = fixed_key();
        let signed = sign_global_checkpoint(&checkpoint, &key, key_id()).unwrap();
        let other = SigningKey::from_bytes(&[7u8; 32]);
        assert!(matches!(
            verify_global_checkpoint(&signed, &other.verifying_key()),
            Err(CommitError::Core(_))
        ));
    }

    #[test]
    fn verify_rejects_tampered_body() {
        let checkpoint = sample_checkpoint();
        let key = fixed_key();
        let mut signed = sign_global_checkpoint(&checkpoint, &key, key_id()).unwrap();
        signed.body.sequence = signed.body.sequence.wrapping_add(1);
        assert!(matches!(
            verify_global_checkpoint(&signed, &key.verifying_key()),
            Err(CommitError::Core(_))
        ));
    }

    #[test]
    fn signature_is_deterministic_for_fixed_key_and_body() {
        let checkpoint = sample_checkpoint();
        let key = fixed_key();
        let first = sign_global_checkpoint(&checkpoint, &key, key_id()).unwrap();
        let second = sign_global_checkpoint(&checkpoint, &key, key_id()).unwrap();
        assert_eq!(
            first.signature.sig.as_bytes(),
            second.signature.sig.as_bytes()
        );
    }

    #[test]
    fn serde_json_roundtrips_including_state_root() {
        let checkpoint = sample_checkpoint();
        let json = serde_json::to_string(&checkpoint).unwrap();
        let decoded: GlobalCheckpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, checkpoint);
        assert_eq!(
            decoded.tenant_roots[0].state_root.as_bytes(),
            checkpoint.tenant_roots[0].state_root.as_bytes()
        );
    }

    #[test]
    fn bcs_roundtrips_including_state_root() {
        // The body BCS-canonicalizes deterministically (what signing covers)
        // and decodes back to an equal value.
        let checkpoint = sample_checkpoint();
        let bytes = bcs::to_bytes(&checkpoint).unwrap();
        let decoded: GlobalCheckpoint = bcs::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, checkpoint);
    }
}
