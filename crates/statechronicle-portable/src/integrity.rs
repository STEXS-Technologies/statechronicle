//! Protocol-compatible hashes, signatures and sparse-Merkle verification.
//!
//! These functions authenticate bytes under caller-supplied keys and roots.
//! Signer trust, canonical-head freshness and profile/schema interpretation
//! remain consumer policies. No journal or tree allocation is performed here.

use crate::codec::{CodecError, uleb_bytes};
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest as _, Sha256};

/// SHA-256 digest of canonical bytes.
#[must_use]
pub fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Existing resource-key derivation, including tenant domain separation.
#[must_use]
pub fn resource_key(tenant: &str, resource: &str) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update([0]);
    digest.update((tenant.len() as u64).to_le_bytes());
    digest.update(tenant.as_bytes());
    digest.update([1]);
    digest.update((resource.len() as u64).to_le_bytes());
    digest.update(resource.as_bytes());
    digest.finalize().into()
}

/// Hashes the existing BCS `ResourceState::Custom` envelope without buffering it.
/// The caller must validate domain identity, schema and native payload semantics.
/// # Errors
/// Returns `Overflow` for a sequence length exceeding the BCS u32 bound.
pub fn custom_state_hash(
    domain: &str,
    schema: u32,
    payload: &[u8],
) -> Result<[u8; 32], CodecError> {
    let mut digest = Sha256::new();
    digest.update([7]);
    for (bytes, is_domain) in [(domain.as_bytes(), true), (payload, false)] {
        let length = u32::try_from(bytes.len()).map_err(|_overflow| CodecError::Overflow)?;
        let (prefix, count) = uleb_bytes(length);
        digest.update(prefix.get(..count).ok_or(CodecError::Overflow)?);
        digest.update(bytes);
        if is_domain {
            digest.update(schema.to_le_bytes());
        }
    }
    Ok(digest.finalize().into())
}

/// Strict Ed25519 verification over canonical protocol body bytes.
/// This operation allocates nothing; the caller owns trust/key selection.
#[must_use]
pub fn verify_signature(body: &[u8], key: &[u8; 32], signature: &[u8; 64]) -> bool {
    let Ok(verifier) = VerifyingKey::from_bytes(key) else {
        return false;
    };
    verifier
        .verify_strict(body, &Signature::from_bytes(signature))
        .is_ok()
}

/// One non-default sibling in an ascending, leaf-to-root sparse proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathStep {
    /// Leaf-adjacent decision is zero, root decision is 255.
    pub level: u8,
    /// Protocol node digest of the sibling subtree.
    pub sibling: [u8; 32],
}

/// Verifies an inclusion proof with at most 256 borrowed siblings. Malformed
/// paths (duplicate or unordered levels) fail closed. Work and stack use are
/// bounded independently of domain data; no tree or default-hash table is built.
#[must_use]
pub fn verify_inclusion(
    root: &[u8; 32],
    key: &[u8; 32],
    state: &[u8; 32],
    steps: &[PathStep],
) -> bool {
    verify_path(root, key, node_hash(0x11, key, state), steps)
}
/// Verifies absence at the caller's expected key under the given trusted root.
#[must_use]
pub fn verify_non_membership(root: &[u8; 32], key: &[u8; 32], steps: &[PathStep]) -> bool {
    verify_path(root, key, node_hash(0x11, &[0; 32], &[0; 32]), steps)
}

fn node_hash(tag: u8, left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update([tag]);
    digest.update(left);
    digest.update(right);
    digest.finalize().into()
}
fn verify_path(root: &[u8; 32], key: &[u8; 32], leaf: [u8; 32], steps: &[PathStep]) -> bool {
    if steps.len() > 256
        || steps.windows(2).any(|pair| match pair {
            [first, second] => first.level >= second.level,
            _ => true,
        })
    {
        return false;
    }
    let mut path = steps.iter().peekable();
    let mut current = leaf;
    let mut default = node_hash(0x11, &[0; 32], &[0; 32]);
    for level in 0_u16..256 {
        let sibling = if path
            .peek()
            .is_some_and(|step| u16::from(step.level) == level)
        {
            match path.next() {
                Some(step) => step.sibling,
                None => return false,
            }
        } else {
            default
        };
        let byte_position = usize::from(255_u16.saturating_sub(level) / 8);
        let Some(byte) = key.get(byte_position) else {
            return false;
        };
        let right = byte.wrapping_shr(u32::from(level % 8)) & 1 == 1;
        current = if right {
            node_hash(0x10, &sibling, &current)
        } else {
            node_hash(0x10, &current, &sibling)
        };
        default = node_hash(0x10, &default, &default);
    }
    path.next().is_none() && current == *root
}
