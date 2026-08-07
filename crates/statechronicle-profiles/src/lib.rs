//! Baseline resource profiles for StateChronicle.
//!
//! The profile registry and per-profile rule sets (protocol §20): unique
//! assets, paid unique assets, consumable stacks, fungible balances,
//! entitlements, meters, and marketplace listings/escrow.

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

/// Baseline profile registry.
pub mod registry;

/// Unique asset profile.
pub mod unique_asset;

/// Paid unique asset profile (no-hard-delete invariants, protocol §20.3).
pub mod paid_unique_asset;

/// Consumable stack profile.
pub mod consumable_stack;

/// Fungible balance profile.
pub mod fungible_balance;

/// Entitlement profile.
pub mod entitlement;

/// Meter profile.
pub mod meter;

/// Marketplace profile (listing/escrow, protocol §20.9).
pub mod marketplace;

/// Profile rule error type.
pub mod error;
