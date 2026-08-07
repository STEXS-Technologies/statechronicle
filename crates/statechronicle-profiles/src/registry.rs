//! Baseline profile registry.
//!
//! Maps profile identifiers to their state type and rule set (protocol §10,
//! §20). The [`ProfileRules`] trait is the single gate every state transition
//! passes through; the [`ProfileRegistry`] resolves a resource's state type to
//! its rule set and keeps the paid unique asset overlay reachable separately.

use std::collections::BTreeMap;

use statechronicle_core::amount::Amount;
use statechronicle_domain::authority::AggregationPolicy;
use statechronicle_domain::intent::Operation;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;

use crate::consumable_stack::ConsumableStackRules;
use crate::entitlement::EntitlementRules;
use crate::error::ProfileError;
use crate::fungible_balance::FungibleBalanceRules;
use crate::marketplace::{EscrowRules, ListingRules};
use crate::meter::MeterRules;
use crate::paid_unique_asset::PaidUniqueAssetRules;
use crate::unique_asset::UniqueAssetRules;

/// A rule set governing one profile over a resource state type.
///
/// Implementations are stateless and `Sync`, so a single static instance
/// serves every resource of the profile. [`check`](Self::check) is pure,
/// deterministic, and fail-closed: it never mutates state, never reads global
/// context, and returns an error for any unknown operation, invalid
/// transition, or malformed input instead of panicking.
pub trait ProfileRules: Sync {
    /// The state type this profile governs.
    fn state_type(&self) -> StateType;

    /// The registry-wide profile identifier.
    fn profile_id(&self) -> &'static str;

    /// All operations this profile accepts, in protocol wire form.
    fn allowed_operations(&self) -> &'static [&'static str];

    /// Validates an operation against the current projection and inputs.
    ///
    /// `current` is `None` when the resource does not exist yet (a create or
    /// mint operation); `Some` carries the projected payload the operation
    /// would mutate. The `inputs` map is the profile-defined intent input map,
    /// which also carries the acting subject under the `actor` key where a
    /// profile checks ownership or consent.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for unknown operations, transitions not
    /// permitted from the current state, missing or malformed inputs, and any
    /// invariant violation (quantity bounds, ownership, transferability,
    /// float ban, consent).
    fn check(
        &self,
        operation: &Operation,
        current: Option<&StateProjection>,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), ProfileError>;

    /// Whether the operation MUST carry an authority binding
    /// (protocol §11.2, ADR-006 §36 Q5 / deferral item 4).
    ///
    /// When this returns `true` for an operation, the executor rejects the
    /// intent with [`ExecutorError::AuthorityMissing`] unless it binds an
    /// authority proof. The default is `false` (authority optional; the
    /// profile's transition and consent rules govern), matching v0 behavior.
    fn requires_authority(&self, _operation: &Operation) -> bool {
        false
    }

    /// The aggregation policy applied to the deployment's authority set for
    /// this operation (protocol §18.1 step 8, ADR-006 §36 Q5).
    ///
    /// [`AggregationPolicy::RequireAll`] requires every configured authority
    /// member to allow the operation; [`AggregationPolicy::AnyOf`] passes when
    /// at least one does. The default is [`AggregationPolicy::RequireAll`].
    fn authority_policy(&self, _operation: &Operation) -> AggregationPolicy {
        AggregationPolicy::RequireAll
    }
}

/// Baseline profile registry.
///
/// Resolves a resource's state type to its rule set. The baseline maps all
/// seven protocol state types (protocol §10). The paid unique asset rules are
/// a distinct profile over the [`StateType::UniqueAsset`] state type, so they
/// are registered separately under `paid_unique_asset()` rather than replacing
/// the plain unique asset rules.
#[derive(Clone, Copy)]
pub struct ProfileRegistry {
    unique_asset: &'static dyn ProfileRules,
    consumable_stack: &'static dyn ProfileRules,
    fungible_balance: &'static dyn ProfileRules,
    entitlement: &'static dyn ProfileRules,
    metered_resource: &'static dyn ProfileRules,
    listing: &'static dyn ProfileRules,
    escrow: &'static dyn ProfileRules,
    paid_unique_asset: &'static dyn ProfileRules,
}

/// Reads a required non-empty string input.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidInput`] when the input is missing, is not a
/// string, or is empty.
pub(crate) fn input_str<'input>(
    inputs: &'input BTreeMap<String, serde_json::Value>,
    key: &str,
) -> Result<&'input str, ProfileError> {
    let value = inputs
        .get(key)
        .ok_or_else(|| ProfileError::InvalidInput(format!("missing input `{key}`")))?;
    let text = value
        .as_str()
        .ok_or_else(|| ProfileError::InvalidInput(format!("input `{key}` must be a string")))?;
    if text.is_empty() {
        return Err(ProfileError::InvalidInput(format!(
            "input `{key}` must not be empty"
        )));
    }
    Ok(text)
}

/// Reads a required string field from a projection's state payload.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidInput`] when the payload has no `key` field
/// or it is not a string.
pub(crate) fn state_str<'projection>(
    projection: &'projection StateProjection,
    key: &str,
) -> Result<&'projection str, ProfileError> {
    let value = projection
        .state
        .get(key)
        .ok_or_else(|| ProfileError::InvalidInput(format!("state payload has no `{key}`")))?;
    value
        .as_str()
        .ok_or_else(|| ProfileError::InvalidInput(format!("`{key}` must be a string")))
}

/// Parses a non-negative integer stored as a canonical integer string.
///
/// The protocol bans floating-point economic state (§10.3), so any
/// float-formatted string (`.` or exponent markers) is rejected with
/// [`ProfileError::FloatForbidden`]; anything else that does not parse as a
/// canonical non-negative integer is rejected with
/// [`ProfileError::InvalidInput`].
///
/// # Errors
///
/// Returns [`ProfileError::InvalidInput`] when the value is missing, not a
/// string, or not a canonical non-negative integer, and
/// [`ProfileError::FloatForbidden`] for float-formatted strings.
pub(crate) fn parse_amount_str(
    value: &serde_json::Value,
    key: &str,
) -> Result<Amount, ProfileError> {
    let text = value
        .as_str()
        .ok_or_else(|| ProfileError::InvalidInput(format!("`{key}` must be an integer string")))?;
    if text.is_empty() {
        return Err(ProfileError::InvalidInput(format!(
            "`{key}` must be an integer string"
        )));
    }
    if text.contains(['.', 'e', 'E']) {
        return Err(ProfileError::FloatForbidden);
    }
    Amount::try_from_str(text)
        .map_err(|_source| ProfileError::InvalidInput(format!("`{key}` must be an integer string")))
}

/// Requires the operation to act on an existing resource.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] with `from` set to `unborn`
/// when `current` is `None`.
pub(crate) fn require_current<'projection>(
    current: Option<&'projection StateProjection>,
    operation: &str,
) -> Result<&'projection StateProjection, ProfileError> {
    current.ok_or_else(|| ProfileError::InvalidTransition {
        from: String::from("unborn"),
        operation: String::from(operation),
    })
}

/// Parses a required non-negative integer-string input.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidInput`] when the input is missing or not a
/// canonical non-negative integer string, and [`ProfileError::FloatForbidden`]
/// for float-formatted strings.
pub(crate) fn input_amount(
    inputs: &BTreeMap<String, serde_json::Value>,
    key: &str,
) -> Result<Amount, ProfileError> {
    let value = inputs
        .get(key)
        .ok_or_else(|| ProfileError::InvalidInput(format!("missing input `{key}`")))?;
    parse_amount_str(value, key)
}

/// Requires the operation to act on a resource that does not exist yet.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] with `from` set to `existing`
/// when `current` is `Some`.
pub(crate) fn require_unborn(
    current: Option<&StateProjection>,
    operation: &str,
) -> Result<(), ProfileError> {
    if current.is_some() {
        return Err(ProfileError::InvalidTransition {
            from: String::from("existing"),
            operation: String::from(operation),
        });
    }
    Ok(())
}

/// Requires the operation's source state and returns the projection.
///
/// # Errors
///
/// Returns [`ProfileError::InvalidTransition`] when the resource does not
/// exist yet (`from` is reported as `unborn`) or its `status` is not one of
/// `allowed_from`.
pub(crate) fn require_from<'projection>(
    current: Option<&'projection StateProjection>,
    operation: &str,
    allowed_from: &[&str],
) -> Result<&'projection StateProjection, ProfileError> {
    let projection = require_current(current, operation)?;
    let from = state_str(projection, "status")?;
    if !allowed_from.contains(&from) {
        return Err(ProfileError::InvalidTransition {
            from: String::from(from),
            operation: String::from(operation),
        });
    }
    Ok(projection)
}

impl ProfileRegistry {
    /// Constructs the baseline registry covering all seven state types.
    pub fn baseline() -> Self {
        Self {
            unique_asset: &UniqueAssetRules,
            consumable_stack: &ConsumableStackRules,
            fungible_balance: &FungibleBalanceRules,
            entitlement: &EntitlementRules,
            metered_resource: &MeterRules,
            listing: &ListingRules,
            escrow: &EscrowRules,
            paid_unique_asset: &PaidUniqueAssetRules,
        }
    }

    /// Returns the rule set registered for a state type.
    ///
    /// Every baseline [`StateType`] is registered, so this returns `Some` for
    /// all seven protocol state types. The `Option` return is part of the
    /// public registry contract so future custom state types can resolve to
    /// no registered rule set.
    #[allow(clippy::unnecessary_wraps)]
    pub fn get(&self, state_type: StateType) -> Option<&'static dyn ProfileRules> {
        match state_type {
            StateType::UniqueAsset => Some(self.unique_asset),
            StateType::ConsumableStack => Some(self.consumable_stack),
            StateType::FungibleBalance => Some(self.fungible_balance),
            StateType::Entitlement => Some(self.entitlement),
            StateType::MeteredResource => Some(self.metered_resource),
            StateType::Listing => Some(self.listing),
            StateType::Escrow => Some(self.escrow),
        }
    }

    /// Returns the paid unique asset rule set (protocol §20.3).
    pub fn paid_unique_asset(&self) -> &'static dyn ProfileRules {
        self.paid_unique_asset
    }

    /// Builds a registry identical to the baseline except the `unique_asset`
    /// slot is served by `rules`.
    ///
    /// This is a test-support constructor for integration tests that need to
    /// exercise a unique-asset profile overriding authority aggregation or
    /// mandatory-ness (e.g. an any-of policy). It is `#[doc(hidden)]` because
    /// production deployments compose profiles through the registry's baseline
    /// registration, not per-slot overrides.
    #[doc(hidden)]
    pub fn with_unique_asset(rules: &'static dyn ProfileRules) -> Self {
        let baseline = Self::baseline();
        Self {
            unique_asset: rules,
            ..baseline
        }
    }
}
