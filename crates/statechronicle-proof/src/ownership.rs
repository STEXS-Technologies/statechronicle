//! Ownership proofs.
//!
//! Prove the current owner of a resource, binding the proof to the owning
//! subject. Ownership proofs reuse the [`ResourceStateProof`] envelope and
//! assert that the claimed state's `owner` field equals the bound subject
//! (protocol §29 step 8). The builder lives in [`crate::bundle`] and is
//! re-exported here for callers that reason in "ownership proof" terms.

pub use crate::bundle::{build_ownership_proof, owner_of};

/// The JSON field carrying the owner identity in claimed state payloads.
pub const OWNER_FIELD: &str = "owner";

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn owner_field_constant_is_documented_convention() {
        assert_eq!(OWNER_FIELD, "owner");
    }

    #[test]
    fn owner_of_reads_and_validates() {
        assert_eq!(
            owner_of(&serde_json::json!({ "owner": "account:example:player_456" })).unwrap(),
            "account:example:player_456"
        );
        assert!(owner_of(&serde_json::json!({ "status": "active" })).is_err());
        assert!(owner_of(&serde_json::json!({ "owner": 42 })).is_err());
        assert!(owner_of(&serde_json::json!({ "owner": "" })).is_err());
    }
}
