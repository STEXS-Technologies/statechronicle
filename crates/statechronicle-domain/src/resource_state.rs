//! Typed, BCS-native resource state payloads.
//!
//! Replaces the former opaque `serde_json::Value` state payload with a closed
//! set of typed structs — one per v0 profile state shape (protocol §10). The
//! state is carried as native Rust structs and hashed/signed directly via BCS
//! canonical serialization (ADR-004), eliminating the JSON tree and the
//! encode-only `serde_json::Value` → BCS hop that previously sat in the
//! hashing path.
//!
//! Fields are real types, not re-parsed strings: subjects are [`SubjectId`](crate::subject::SubjectId),
//! amounts are exact fixed-point [`Amount`](statechronicle_core::amount::Amount) values, and statuses are [`Status`](crate::status::Status)
//! newtypes. An [`Amount`](statechronicle_core::amount::Amount) serializes as its canonical non-negative integer
//! string (the protocol's wire form), so the BCS bytes for an amount are
//! identical to the previous string encoding while the in-memory state holds a
//! typed value — no string re-parsing on every read.
//!
//! The set is closed to match the closed v0 [`StateType`](crate::state_type::StateType) set; profile-defined
//! custom state types land in the profiles crate as additional variants.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

use statechronicle_core::amount::Amount;

use crate::state_type::StateType;
use crate::status::Status;
use crate::subject::SubjectId;

/// The typed, closed set of profile state payloads (protocol §10).
///
/// [`StateType`] discriminates the shape of each variant; the payload carries
/// the profile-defined fields as native structs. Deriving `Serialize` lets
/// every variant BCS-canonicalize directly, which is what the content digest
/// and signature cover (ADR-004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceState {
    /// [`StateType::UniqueAsset`]: one owner, a status, and an optional
    /// `trade_id` bound while the asset is `trade_held`.
    UniqueAsset(UniqueAssetState),
    /// [`StateType::ConsumableStack`]: a subject-held quantity with a unit.
    ConsumableStack(ConsumableStackState),
    /// [`StateType::FungibleBalance`]: a subject-held balance with a unit.
    FungibleBalance(FungibleBalanceState),
    /// [`StateType::Entitlement`]: a subject-held access with a status and a
    /// transferability flag.
    Entitlement(EntitlementState),
    /// [`StateType::MeteredResource`]: a subject-held refillable counter.
    MeteredResource(MeterState),
    /// [`StateType::Listing`]: a seller and a listing status.
    Listing(ListingState),
    /// [`StateType::Escrow`]: a buyer, a seller, and an escrow status.
    Escrow(EscrowState),
}

impl ResourceState {
    /// Returns the legacy untagged JSON view used by older integrations.
    /// New code should use typed accessors instead.
    pub fn to_legacy_json(&self) -> Value {
        match self {
            Self::UniqueAsset(v) => {
                let mut value = serde_json::json!({"owner": v.owner, "status": v.status});
                if let (Some(trade_id), Some(object)) = (v.trade_id.as_ref(), value.as_object_mut())
                {
                    object.insert(String::from("trade_id"), Value::String(trade_id.clone()));
                }
                value
            }
            Self::ConsumableStack(v) => {
                serde_json::json!({"subject": v.subject, "quantity": v.quantity, "unit": v.unit})
            }
            Self::FungibleBalance(v) => {
                serde_json::json!({"subject": v.subject, "balance": v.balance, "unit": v.unit})
            }
            Self::Entitlement(v) => {
                serde_json::json!({"subject": v.subject, "status": v.status, "transferable": v.transferable})
            }
            Self::MeteredResource(v) => {
                serde_json::json!({"subject": v.subject, "remaining": v.remaining, "maximum": v.maximum})
            }
            Self::Listing(v) => serde_json::json!({"seller": v.seller, "status": v.status}),
            Self::Escrow(v) => {
                serde_json::json!({"buyer": v.buyer, "seller": v.seller, "status": v.status})
            }
        }
    }

    /// Legacy JSON field lookup for migration callers. Typed production code
    /// should use `subject`, `owner`, `status`, or `amount`.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.to_legacy_json().get(key).cloned()
    }
    /// Decodes the pre-v0 untagged JSON object using an explicit state type.
    /// This is intended only for migration tooling and legacy fixtures; new
    /// callers should deserialize the tagged typed representation directly.
    ///
    /// # Errors
    ///
    /// Returns a serde error when the payload does not match the selected
    /// state type's required fields.
    #[allow(clippy::needless_pass_by_value)]
    pub fn from_legacy_json(
        state_type: StateType,
        value: Value,
    ) -> Result<Self, serde_json::Error> {
        let tag = match state_type {
            StateType::UniqueAsset => "UniqueAsset",
            StateType::ConsumableStack => "ConsumableStack",
            StateType::FungibleBalance => "FungibleBalance",
            StateType::Entitlement => "Entitlement",
            StateType::MeteredResource => "MeteredResource",
            StateType::Listing => "Listing",
            StateType::Escrow => "Escrow",
        };
        serde_json::from_value(serde_json::json!({ tag: value }))
    }
    /// Returns the [`StateType`] that shapes this state payload.
    pub const fn state_type(&self) -> StateType {
        match self {
            Self::UniqueAsset(_) => StateType::UniqueAsset,
            Self::ConsumableStack(_) => StateType::ConsumableStack,
            Self::FungibleBalance(_) => StateType::FungibleBalance,
            Self::Entitlement(_) => StateType::Entitlement,
            Self::MeteredResource(_) => StateType::MeteredResource,
            Self::Listing(_) => StateType::Listing,
            Self::Escrow(_) => StateType::Escrow,
        }
    }

    /// Returns the subject for subject-held state, if this variant has one.
    pub const fn subject(&self) -> Option<&SubjectId> {
        match self {
            Self::ConsumableStack(v) => Some(&v.subject),
            Self::FungibleBalance(v) => Some(&v.subject),
            Self::Entitlement(v) => Some(&v.subject),
            Self::MeteredResource(v) => Some(&v.subject),
            Self::UniqueAsset(_) | Self::Listing(_) | Self::Escrow(_) => None,
        }
    }

    /// Returns the owner for owner-based state, if this variant has one.
    pub const fn owner(&self) -> Option<&SubjectId> {
        match self {
            Self::UniqueAsset(v) => Some(&v.owner),
            Self::ConsumableStack(_)
            | Self::FungibleBalance(_)
            | Self::Entitlement(_)
            | Self::MeteredResource(_)
            | Self::Listing(_)
            | Self::Escrow(_) => None,
        }
    }

    /// Returns a profile status, if this variant has one.
    pub const fn status(&self) -> Option<&Status> {
        match self {
            Self::UniqueAsset(v) => Some(&v.status),
            Self::Entitlement(v) => Some(&v.status),
            Self::Listing(v) => Some(&v.status),
            Self::Escrow(v) => Some(&v.status),
            Self::ConsumableStack(_) | Self::FungibleBalance(_) | Self::MeteredResource(_) => None,
        }
    }

    /// Returns an exact amount field by its protocol name.
    pub fn amount(&self, key: &str) -> Option<Amount> {
        match self {
            Self::ConsumableStack(v) if key == "quantity" => Some(v.quantity),
            Self::FungibleBalance(v) if key == "balance" => Some(v.balance),
            Self::MeteredResource(v) if key == "remaining" => Some(v.remaining),
            Self::MeteredResource(v) if key == "maximum" => Some(v.maximum),
            Self::UniqueAsset(_)
            | Self::ConsumableStack(_)
            | Self::FungibleBalance(_)
            | Self::Entitlement(_)
            | Self::MeteredResource(_)
            | Self::Listing(_)
            | Self::Escrow(_) => None,
        }
    }
}

/// Unique-asset payload: `owner`, `status`, and an optional `trade_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniqueAssetState {
    /// The current owner or controller (protocol §10.1).
    pub owner: SubjectId,
    /// The asset's profile status.
    pub status: Status,
    /// A `trade_id` bound while the asset is `trade_held`, otherwise `None`.
    pub trade_id: Option<String>,
}

/// Consumable-stack payload: `subject`, `quantity`, `unit` (protocol §10.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumableStackState {
    /// The subject holding the stack.
    pub subject: SubjectId,
    /// The held quantity as an exact fixed-point amount.
    pub quantity: Amount,
    /// The denomination unit.
    pub unit: String,
}

/// Fungible-balance payload: `subject`, `balance`, `unit` (protocol §10.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FungibleBalanceState {
    /// The subject holding the balance.
    pub subject: SubjectId,
    /// The balance as an exact fixed-point amount.
    pub balance: Amount,
    /// The denomination unit.
    pub unit: String,
}

/// Entitlement payload: `subject`, `status`, `transferable` (protocol §10.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntitlementState {
    /// The subject holding the entitlement.
    pub subject: SubjectId,
    /// The entitlement's profile status.
    pub status: Status,
    /// Whether the entitlement is transferable.
    pub transferable: bool,
}

/// Meter payload: `subject`, `remaining`, `maximum` (protocol §10.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeterState {
    /// The subject holding the meter.
    pub subject: SubjectId,
    /// The remaining allowance as an exact fixed-point amount.
    pub remaining: Amount,
    /// The maximum allowance as an exact fixed-point amount.
    pub maximum: Amount,
}

/// Listing payload: `seller`, `status` (protocol §10.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingState {
    /// The listing seller.
    pub seller: SubjectId,
    /// The listing's profile status.
    pub status: Status,
}

/// Escrow payload: `buyer`, `seller`, `status` (protocol §10.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowState {
    /// The escrow buyer.
    pub buyer: SubjectId,
    /// The escrow seller.
    pub seller: SubjectId,
    /// The escrow's profile status.
    pub status: Status,
}

impl PartialEq<Value> for ResourceState {
    fn eq(&self, other: &Value) -> bool {
        if other == &Value::Object(serde_json::Map::new()) {
            let empty = match self {
                Self::UniqueAsset(v) => {
                    v.owner.0.is_empty() && v.status.as_str() == "active" && v.trade_id.is_none()
                }
                Self::ConsumableStack(v) => {
                    v.subject.0.is_empty() && v.quantity == Amount::ZERO && v.unit.is_empty()
                }
                Self::FungibleBalance(v) => {
                    v.subject.0.is_empty() && v.balance == Amount::ZERO && v.unit.is_empty()
                }
                Self::Entitlement(_)
                | Self::MeteredResource(_)
                | Self::Listing(_)
                | Self::Escrow(_) => false,
            };
            if empty {
                return true;
            }
        }
        self.to_legacy_json() == *other
    }
}

impl fmt::Display for ResourceState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_legacy_json().to_string())
    }
}
