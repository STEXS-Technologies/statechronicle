//! Size and safety bounds used across the protocol.
//!
//! Enforces upper bounds on payload sizes, nesting depth, and key lengths so
//! parsers and accumulators fail closed instead of exhausting resources.
//!
//! The concrete values below are **provisional protocol defaults**, to be
//! finalized in ADR-004 §7. Protocol §30 only mandates "bounded input sizes";
//! the specific byte/character bounds are ours to define, and [`check_size`] is
//! the single choke point so tightening a bound later is a one-line change.

use crate::error::StateChronicleError;

/// Maximum canonical byte length of an intent body.
pub const MAX_INTENT_BYTES: usize = 64 * 1024;

/// Maximum canonical byte length of an event body.
pub const MAX_EVENT_BYTES: usize = 64 * 1024;

/// Maximum canonical byte length of a commit body.
pub const MAX_COMMIT_BYTES: usize = 1024 * 1024;

/// Maximum character length of a protocol id string (tenant, resource, etc.).
pub const MAX_ID_LENGTH: usize = 128;

/// Checks that `actual` is within `limit`, failing closed otherwise.
///
/// The boundary is inclusive: a value exactly at the limit is accepted.
///
/// # Errors
///
/// Returns [`StateChronicleError::SizeLimitExceeded`] when `actual` exceeds
/// `limit`, carrying the bound `name`, the `limit`, and the `actual` length so
/// callers can fail closed with structured context.
pub fn check_size(name: &str, limit: usize, actual: usize) -> Result<(), StateChronicleError> {
    if actual > limit {
        return Err(StateChronicleError::SizeLimitExceeded {
            name: String::from(name),
            limit,
            actual,
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::{MAX_COMMIT_BYTES, MAX_EVENT_BYTES, MAX_ID_LENGTH, MAX_INTENT_BYTES, check_size};
    use crate::error::StateChronicleError;

    #[test]
    fn check_size_ok_under_limit() {
        assert!(check_size("intent", MAX_INTENT_BYTES, 42).is_ok());
    }

    #[test]
    fn check_size_ok_at_limit() {
        assert!(check_size("intent", MAX_INTENT_BYTES, MAX_INTENT_BYTES).is_ok());
    }

    #[test]
    fn check_size_ok_for_zero_byte_value() {
        assert!(check_size("event", MAX_EVENT_BYTES, 0).is_ok());
    }

    #[test]
    fn check_size_error_over_limit() {
        let error = check_size("commit", 4, 5).unwrap_err();
        assert!(matches!(
            error,
            StateChronicleError::SizeLimitExceeded { name, limit, actual }
            if name == "commit" && limit == 4 && actual == 5
        ));
    }

    #[test]
    fn check_size_commit_limit_is_provisional_default() {
        assert_eq!(MAX_COMMIT_BYTES, 1024 * 1024);
    }

    #[test]
    fn max_id_length_bounds_id_string() {
        // Exactly at the limit is accepted.
        let at_limit = "x".repeat(MAX_ID_LENGTH);
        assert!(check_size("id", MAX_ID_LENGTH, at_limit.len()).is_ok());

        // One past the limit is rejected.
        let over_limit = "x".repeat(MAX_ID_LENGTH.saturating_add(1));
        assert!(check_size("id", MAX_ID_LENGTH, over_limit.len()).is_err());
    }
}
