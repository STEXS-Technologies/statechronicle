//! Domain-separated state keys (ADR-005 §2).
//!
//! The SMT key space is 256-bit SHA-256 images over a length-prefixed,
//! domain-tagged composite of `(tenant_id, resource_id[, subject_id])`:
//!
//! ```text
//! key = SHA-256( 0x00 || u64le(len(tenant))   || tenant
//!             || 0x01 || u64le(len(resource))  || resource
//!             || [ 0x02 || u64le(len(subject)) || subject ] )
//! ```
//!
//! `tenant_id` stays in the preimage even though trees are per-tenant
//! (defense-in-depth against cross-tenant key collisions). Subject-held types
//! (fungible balance, consumable stack, meter, entitlement) append the
//! subject; owner-based unique assets do not.

use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};

use sha2::{Digest as _, Sha256};

/// Leading domain byte for the key preimage.
pub const KEY_DOMAIN_PREFIX: u8 = 0x00;
/// Tag separating tenant from resource in the preimage.
pub const KEY_RESOURCE_TAG: u8 = 0x01;
/// Tag separating resource from subject in the preimage.
pub const KEY_SUBJECT_TAG: u8 = 0x02;

/// An opaque 256-bit SMT key derived from domain IDs.
///
/// The bytes are a SHA-256 image, so key distribution is uniform regardless
/// of adversarial ID choice (§16.2 rationale).
#[derive(Clone, Copy)]
pub struct StateKey([u8; 32]);

impl StateKey {
    /// Wraps raw 256-bit key bytes.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the 32 raw key bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the bit at `position`, counted from the most significant bit
    /// of the key: position 0 is the MSB of byte 0 (the top of the tree),
    /// position 255 is the LSB of byte 31 (the leaf-adjacent decision).
    pub fn bit_at(&self, position: usize) -> bool {
        let byte_index = position.wrapping_shr(3);
        let bit_in_byte = position & 7;
        let Some(byte) = self.0.get(byte_index) else {
            return false;
        };
        let shift = 7usize.wrapping_sub(bit_in_byte);
        (byte.wrapping_shr(shift as u32)) & 1 == 1
    }

    /// Derives the key for an owner-based (non subject-held) resource.
    ///
    /// `tenant_id` and `resource_id` are canonical UTF-8 of the domain
    /// newtypes.
    pub fn for_resource(tenant_id: &str, resource_id: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update([KEY_DOMAIN_PREFIX]);
        hasher.update(u64_len_prefix(tenant_id));
        hasher.update(tenant_id.as_bytes());
        hasher.update([KEY_RESOURCE_TAG]);
        hasher.update(u64_len_prefix(resource_id));
        hasher.update(resource_id.as_bytes());
        Self(hasher.finalize().into())
    }

    /// Derives the key for a subject-held resource (balance, stack, meter,
    /// entitlement).
    ///
    /// `tenant_id`, `resource_id`, and `subject_id` are canonical UTF-8 of
    /// the domain newtypes.
    pub fn for_subject_held(tenant_id: &str, resource_id: &str, subject_id: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update([KEY_DOMAIN_PREFIX]);
        hasher.update(u64_len_prefix(tenant_id));
        hasher.update(tenant_id.as_bytes());
        hasher.update([KEY_RESOURCE_TAG]);
        hasher.update(u64_len_prefix(resource_id));
        hasher.update(resource_id.as_bytes());
        hasher.update([KEY_SUBJECT_TAG]);
        hasher.update(u64_len_prefix(subject_id));
        hasher.update(subject_id.as_bytes());
        Self(hasher.finalize().into())
    }
}

/// Encodes a byte length as a little-endian u64 (ADR-005 §2 length-prefixing).
const fn u64_len_prefix(value: &str) -> [u8; 8] {
    // `len()` is bounded by the 32-bit address space, so the cast to u64 is
    // lossless on every supported target.
    (value.len() as u64).to_le_bytes()
}

impl fmt::Debug for StateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("StateKey").field(&hex_str(&self.0)).finish()
    }
}

impl PartialEq for StateKey {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for StateKey {}

impl PartialOrd for StateKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StateKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl Hash for StateKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Display for StateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex_str(&self.0))
    }
}

/// Formats a byte array as lowercase hex without allocating per call site.
fn hex_str(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(char::from_digit((byte.wrapping_shr(4)) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::{StateKey, hex_str};
    use crate::sparse_merkle::EMPTY_LEAF_HASH;

    #[test]
    fn for_resource_known_answer() {
        let key = StateKey::for_resource("tenant:acme", "asset:sword_001");
        assert_eq!(
            hex_str(key.as_bytes()),
            "0421c524d1c882a9eaa2f91e189a126e6c1b0cd2f66fb53beee2016ee8ddd14a"
        );
    }

    #[test]
    fn for_subject_held_known_answer() {
        let key = StateKey::for_subject_held(
            "tenant:acme",
            "asset:sword_001",
            "account:example:player_123",
        );
        assert_eq!(
            hex_str(key.as_bytes()),
            "f0d26f5ba7d7380ceff5e6f33c78ed1df06556499435a657dbb9f3f3b0d0907d"
        );
    }

    #[test]
    fn subject_held_differs_from_resource_key() {
        let owner = StateKey::for_resource("tenant:acme", "asset:sword_001");
        let held = StateKey::for_subject_held("tenant:acme", "asset:sword_001", "player_1");
        assert_ne!(owner, held);
    }

    #[test]
    fn keys_are_32_bytes_and_deterministic() {
        let a = StateKey::for_resource("tenant:acme", "asset:sword_001");
        let b = StateKey::for_resource("tenant:acme", "asset:sword_001");
        assert_eq!(a, b);
        assert_eq!(a.as_bytes().len(), 32);
    }

    #[test]
    fn bit_at_matches_byte_layout() {
        let key = StateKey::new(EMPTY_LEAF_HASH);
        let bytes = key.as_bytes();
        // Position 0 is the MSB of byte 0; position 255 is the LSB of byte 31.
        assert_eq!(key.bit_at(0), bytes[0] & 0x80 == 0x80);
        assert_eq!(key.bit_at(7), bytes[0] & 1 == 1);
        assert_eq!(key.bit_at(8), bytes[1] & 0x80 == 0x80);
        assert_eq!(key.bit_at(255), bytes[31] & 1 == 1);
    }
}
