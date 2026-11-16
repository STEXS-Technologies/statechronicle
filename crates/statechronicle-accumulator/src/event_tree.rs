//! Event Merkle roots shared by commit formation and historical verification.
use crate::sparse_merkle::{LEAF_NODE_TAG, internal_hash};
use sha2::{Digest as _, Sha256};
use statechronicle_core::{
    canonicalize::canonicalize_and_digest, digest::ContentDigest, error::StateChronicleError,
};
use statechronicle_domain::event::Event;

/// Computes the v0 ordered event Merkle root without changing event bytes.
/// Leaves use `H(0x11 || digest(event))`; internal nodes use `0x10` and odd
/// levels duplicate their final node. Empty input has no event root (`None`).
/// This primitive imposes no admission limits; callers own their input bounds.
///
/// # Errors
/// Returns the canonical serialization error for a malformed event.
pub fn event_merkle_root(events: &[Event]) -> Result<Option<ContentDigest>, StateChronicleError> {
    if events.is_empty() {
        return Ok(None);
    }
    let mut level: Vec<[u8; 32]> = Vec::with_capacity(events.len());
    for event in events {
        let digest = canonicalize_and_digest(event)?;
        level.push(event_leaf_hash(*digest.as_bytes()));
    }
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        for chunk in level.chunks(2) {
            let Some(left) = chunk.first().copied() else {
                break;
            };
            let right = chunk.get(1).copied().unwrap_or(left);
            next.push(internal_hash(left, right));
        }
        level = next;
    }
    let Some(root) = level.first().copied() else {
        return Ok(None);
    };
    Ok(Some(ContentDigest::new(root)))
}

/// `H(0x11 || event_digest)`: the event tree leaf, mirroring the
/// accumulator's `LEAF_NODE_TAG` domain separation (ADR-005 §2). Events have
/// no SMT key, so the leaf covers only the event's canonical digest.
fn event_leaf_hash(event_digest: [u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([LEAF_NODE_TAG]);
    hasher.update(event_digest);
    hasher.finalize().into()
}
