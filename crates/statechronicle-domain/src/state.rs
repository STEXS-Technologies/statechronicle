//! State projections over event history (protocol §9).
//!
//! Current state is a deterministic projection of the append-only event
//! history:
//!
//! ```text
//! previous state + valid committed event = next state
//! ```
//!
//! A [`StateProjection`](crate::state::StateProjection) is derived, cacheable, and indexable. It is never the
//! source of truth (protocol §9). `StateType` shapes each projection's rules;
//! the profile-specific payload (`owner`/`status`, `balance`/`unit`,
//! `quantity`, ...) lives in the typed `state` [`ResourceState`](crate::resource_state::ResourceState) value.

use serde::{Deserialize, Serialize};

use statechronicle_core::digest::ContentDigest;

use crate::ids::{CommitId, EventId};
use crate::resource::ResourceId;
use crate::resource_state::ResourceState;
use crate::state_type::StateType;
use crate::tenant::TenantId;

/// A derived projection of a resource's current state (protocol §9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateProjection {
    /// The tenant scope of the resource.
    pub tenant_id: TenantId,
    /// The resource being projected.
    pub resource_id: ResourceId,
    /// The state type shaping the projection's rules.
    pub state_type: StateType,
    /// The resource version at this projection.
    pub version: u64,
    /// The last event that mutated the resource.
    pub last_event_id: EventId,
    /// The last commit that included a mutation of the resource.
    pub last_commit_id: CommitId,
    /// Canonical digest of the projected state payload.
    pub state_hash: ContentDigest,
    /// The typed, profile-defined projected state payload.
    pub state: ResourceState,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::resource_state::{FungibleBalanceState, UniqueAssetState};
    use crate::status::Status;
    use crate::subject::SubjectId;
    use statechronicle_core::amount::Amount;
    use statechronicle_core::canonicalize::canonicalize_and_digest;

    fn sample_projection(
        resource_id: &str,
        state_type: StateType,
        version: u64,
        state: ResourceState,
    ) -> StateProjection {
        let state_hash = canonicalize_and_digest(&state).unwrap();
        StateProjection {
            tenant_id: TenantId(String::from("acme.game.alpha")),
            resource_id: ResourceId(String::from(resource_id)),
            state_type,
            version,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash,
            state,
        }
    }

    #[test]
    fn unique_asset_projection_roundtrips() {
        let projection = sample_projection(
            "asset:sword_001",
            StateType::UniqueAsset,
            42,
            ResourceState::UniqueAsset(UniqueAssetState {
                owner: SubjectId(String::from("account:example:player_456")),
                status: Status::from_static("active"),
                trade_id: None,
            }),
        );

        let json = serde_json::to_string(&projection).unwrap();
        let decoded: StateProjection = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, projection);

        // `state` is a typed struct, so BCS both encodes and decodes it.
        let first = bcs::to_bytes(&projection).unwrap();
        let second = bcs::to_bytes(&projection).unwrap();
        assert_eq!(first, second);
        let _: StateProjection = bcs::from_bytes(&first).unwrap();
    }

    #[test]
    fn fungible_balance_projection_roundtrips() {
        let projection = sample_projection(
            "currency:gold",
            StateType::FungibleBalance,
            88,
            ResourceState::FungibleBalance(FungibleBalanceState {
                subject: SubjectId(String::from("account:example:player_123")),
                balance: Amount::from_u64(125000),
                unit: String::from("gold_minor"),
            }),
        );

        let json = serde_json::to_string(&projection).unwrap();
        let decoded: StateProjection = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, projection);

        let first = bcs::to_bytes(&projection).unwrap();
        let second = bcs::to_bytes(&projection).unwrap();
        assert_eq!(first, second);
        let _: StateProjection = bcs::from_bytes(&first).unwrap();
    }
}
