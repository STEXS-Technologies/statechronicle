//! Compatibility entry points for profile-owned atomic resource invariants.

use crate::error::ExecutorError;
use statechronicle_domain::event::Event;
use statechronicle_domain::intent::Intent;
use statechronicle_profiles::atomicity;

pub use statechronicle_profiles::atomicity::{
    SettleLeg, TenantEventGroup, TradeManifest, ValueLeg, declares_value_leg,
};

/// Checks the baseline atomic event batch invariants.
/// # Errors
/// Returns an atomicity or transfer error for inconsistent event groups.
pub fn validate_batch_consistency(events: &[Event]) -> Result<(), ExecutorError> {
    atomicity::validate_batch_consistency(events).map_err(ExecutorError::from)
}
/// Checks declared settlement and bundle invariants.
/// # Errors
/// Returns an atomicity or transfer error for inconsistent settlement groups.
pub fn validate_settle_batch(events: &[Event], intents: &[Intent]) -> Result<(), ExecutorError> {
    atomicity::validate_settle_batch(events, intents).map_err(ExecutorError::from)
}
/// Checks scope and identity consistency across tenant groups.
/// # Errors
/// Returns an atomicity or transfer error for inconsistent groups.
pub fn validate_cross_tenant_consistency(groups: &[TenantEventGroup]) -> Result<(), ExecutorError> {
    atomicity::validate_cross_tenant_consistency(groups).map_err(ExecutorError::from)
}
/// Checks the explicit cross-tenant economic manifest.
/// # Errors
/// Returns an atomicity or transfer error for an inconsistent manifest.
pub fn validate_cross_tenant_trade(
    groups: &[TenantEventGroup],
    manifest: &TradeManifest,
    intents: &[Intent],
) -> Result<(), ExecutorError> {
    atomicity::validate_cross_tenant_trade(groups, manifest, intents).map_err(ExecutorError::from)
}
