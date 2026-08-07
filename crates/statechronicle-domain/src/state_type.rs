//! Resource state types.
//!
//! Discriminates the profile-backed state model: unique, stack, fungible,
//! entitlement, meter, listing, and escrow (protocol §10.1–§10.6). The set is
//! closed for the v0 baseline; profile-defined custom state types land in the
//! profiles crate (protocol §10).

use serde::{Deserialize, Serialize};

/// The closed set of baseline state types for the v0 protocol.
///
/// Each variant shapes a projection's rules in its profile (protocol §10):
/// `UniqueAsset` has an `owner`; balances, stacks, entitlements, and meters
/// are subject-held quantities or access; listings and escrow positions are
/// temporary control constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateType {
    /// A unique asset with one current owner or controller (§10.1).
    UniqueAsset,
    /// A count of stackable units held by a subject (§10.2).
    ConsumableStack,
    /// A numerical amount of a resource held by a subject (§10.3).
    FungibleBalance,
    /// Access, license, membership, or claim status (§10.4).
    Entitlement,
    /// A refillable, time-bound, or usage-limited counter (§10.5).
    ///
    /// The canonical serde name is `metered_resource`; protocol §10.5 examples
    /// abbreviate the `state_type` as `meter`.
    MeteredResource,
    /// A temporary control constraint around a sale, trade, or auction (§10.6).
    Listing,
    /// A temporary control constraint around a settlement workflow (§10.6).
    Escrow,
}

impl StateType {
    /// Returns the canonical snake_case name used by the wire format.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UniqueAsset => "unique_asset",
            Self::ConsumableStack => "consumable_stack",
            Self::FungibleBalance => "fungible_balance",
            Self::Entitlement => "entitlement",
            Self::MeteredResource => "metered_resource",
            Self::Listing => "listing",
            Self::Escrow => "escrow",
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrips_all_variants() {
        for variant in [
            StateType::UniqueAsset,
            StateType::ConsumableStack,
            StateType::FungibleBalance,
            StateType::Entitlement,
            StateType::MeteredResource,
            StateType::Listing,
            StateType::Escrow,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let decoded: StateType = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, variant);
        }
    }

    #[test]
    fn as_str_returns_snake_case_names() {
        assert_eq!(StateType::UniqueAsset.as_str(), "unique_asset");
        assert_eq!(StateType::ConsumableStack.as_str(), "consumable_stack");
        assert_eq!(StateType::FungibleBalance.as_str(), "fungible_balance");
        assert_eq!(StateType::Entitlement.as_str(), "entitlement");
        assert_eq!(StateType::MeteredResource.as_str(), "metered_resource");
        assert_eq!(StateType::Listing.as_str(), "listing");
        assert_eq!(StateType::Escrow.as_str(), "escrow");
    }

    #[test]
    fn serde_uses_snake_case_wire_names() {
        assert_eq!(
            serde_json::to_string(&StateType::UniqueAsset).unwrap(),
            "\"unique_asset\""
        );
        assert_eq!(
            serde_json::to_string(&StateType::FungibleBalance).unwrap(),
            "\"fungible_balance\""
        );
    }

    #[test]
    fn unknown_string_is_rejected() {
        let result = serde_json::from_str::<StateType>("\"profile_custom\"");
        assert!(result.is_err());
    }
}
