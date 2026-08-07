//! Profile rule error type.
//!
//! Invariant violations raised by profile rule evaluation, built with
//! `thiserror`. Every variant carries structured context so callers can fail
//! closed without string matching (protocol §20).

use statechronicle_core::amount::Amount;

/// Errors raised by profile rule evaluation.
///
/// Returned by [`crate::registry::ProfileRules::check`] when an operation is
/// unknown, violates a profile's transition table, or fails an
/// input/quantity/ownership invariant. Rule evaluation is fail-closed: any
/// malformed or unexpected input produces one of these variants instead of
/// panicking.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    /// The operation is not registered for this profile.
    ///
    /// Raised when the operation name does not appear in the profile's
    /// `allowed_operations`.
    #[error("unknown operation `{0}`")]
    UnknownOperation(String),

    /// The operation is not permitted from the resource's current state.
    ///
    /// Raised when a transition table forbids the operation from the current
    /// status, or when the operation requires an existing resource and none
    /// exists yet (or vice versa).
    #[error("invalid transition from `{from}` via `{operation}`")]
    InvalidTransition {
        /// The current state name the transition is attempted from, or
        /// `unborn` when the resource does not exist yet.
        from: String,
        /// The operation name being attempted.
        operation: String,
    },

    /// An input field is missing or malformed.
    ///
    /// Raised when a required input key is absent, has the wrong JSON type, or
    /// is an empty string.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// An amount field must be strictly positive.
    ///
    /// Raised when a credit, debit, consume, transfer, or spend amount is
    /// zero or negative.
    #[error("amount must be positive")]
    NonPositiveAmount,

    /// A debit or consumption exceeds the quantity available.
    #[error("insufficient quantity: requested {requested}, available {available}")]
    InsufficientQuantity {
        /// The quantity currently available in the projected state.
        available: Amount,
        /// The quantity requested by the operation.
        requested: Amount,
    },

    /// A hard delete was attempted without the required consent.
    ///
    /// Paid unique assets refuse hard deletion unless the current owner
    /// consents (protocol §20.3).
    #[error("hard delete is forbidden")]
    HardDeleteForbidden,

    /// A floating-point value appeared where integer-only is required.
    ///
    /// The protocol bans floating-point economic state (§10.3): quantities,
    /// balances, and meter values must be canonical non-negative integer
    /// strings.
    #[error("floating-point values are forbidden")]
    FloatForbidden,

    /// An actor or input does not match the resource's current owner.
    #[error("ownership mismatch: expected `{expected}`, got `{actual}`")]
    OwnershipMismatch {
        /// The expected owner from the projected state.
        expected: String,
        /// The actual owner or actor observed in the operation inputs.
        actual: String,
    },

    /// A transfer was attempted on a resource that is not transferable.
    ///
    /// Entitlements are transferable only when their state payload carries
    /// `"transferable": true`.
    #[error("resource is not transferable")]
    NotTransferable,
}
