//! State proofs.
//!
//! Prove the current state of a resource (for example current owner or
//! balance) from the accumulator root. The v0 bundle is the domain's
//! [`ResourceStateProof`] envelope (protocol §16.2); the builders live in
//! [`crate::bundle`] and are re-exported here for callers that reason in
//! "state proof" terms.

use statechronicle_accumulator::key::StateKey;
use statechronicle_domain::state::StateProjection;

pub use crate::bundle::{build_state_proof, derive_state_key, state_key_for_proof};

/// Returns the owner-based state key implied by a projection.
///
/// For subject-held projections (balance, stack, meter, entitlement) use
/// [`StateKey::for_subject_held`] with the projection's subject instead.
pub fn state_key_for_projection(projection: &StateProjection) -> StateKey {
    StateKey::for_resource(&projection.tenant_id.0, &projection.resource_id.0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_domain::state_type::StateType;
    use statechronicle_domain::tenant::TenantId;

    #[test]
    fn state_key_for_projection_is_resource_keyed() {
        let projection = StateProjection {
            tenant_id: TenantId(String::from("acme.game.alpha")),
            resource_id: statechronicle_domain::resource::ResourceId(String::from(
                "asset:sword_001",
            )),
            state_type: StateType::UniqueAsset,
            version: 1,
            last_event_id: statechronicle_domain::ids::EventId::new(String::from(
                "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4",
            ))
            .unwrap(),
            last_commit_id: statechronicle_domain::ids::CommitId::new(String::from(
                "cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W",
            ))
            .unwrap(),
            state_hash: statechronicle_core::digest::hash_bytes(b"state"),
            state: serde_json::json!({ "owner": "account:example:player_456" }),
        };
        let key = state_key_for_projection(&projection);
        assert_eq!(
            key,
            StateKey::for_resource("acme.game.alpha", "asset:sword_001")
        );
    }
}
