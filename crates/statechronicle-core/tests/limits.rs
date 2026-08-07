//! Integration tests for size-limit enforcement through the public API.

#![allow(clippy::panic, clippy::unwrap_used, clippy::indexing_slicing)]

use statechronicle_core::error::StateChronicleError;
use statechronicle_core::limits::{
    MAX_COMMIT_BYTES, MAX_EVENT_BYTES, MAX_ID_LENGTH, MAX_INTENT_BYTES, check_size,
};

#[test]
fn check_size_accepts_values_under_or_at_limit() {
    assert!(check_size("intent", MAX_INTENT_BYTES, 1).is_ok());
    assert!(check_size("intent", MAX_INTENT_BYTES, MAX_INTENT_BYTES).is_ok());
    assert!(check_size("event", MAX_EVENT_BYTES, MAX_EVENT_BYTES).is_ok());
    assert!(check_size("commit", MAX_COMMIT_BYTES, MAX_COMMIT_BYTES).is_ok());
}

#[test]
fn check_size_rejects_values_over_limit_with_fields() {
    let error = check_size(
        "intent",
        MAX_INTENT_BYTES,
        MAX_INTENT_BYTES.saturating_add(1),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        StateChronicleError::SizeLimitExceeded { name, limit, actual }
        if name == "intent"
            && limit == MAX_INTENT_BYTES
            && actual == MAX_INTENT_BYTES.saturating_add(1)
    ));
}

#[test]
fn provisional_defaults_are_bounded() {
    // Protocol §30 mandates bounded input sizes; these are the provisional
    // defaults to be finalized in ADR-004 §7.
    const _: () = assert!(MAX_INTENT_BYTES > 0);
    const _: () = assert!(MAX_EVENT_BYTES > 0);
    const _: () = assert!(MAX_COMMIT_BYTES >= MAX_INTENT_BYTES);
    const _: () = assert!(MAX_ID_LENGTH > 0);
}

#[test]
fn max_id_length_bounds_an_id_string() {
    // Exactly at the limit is accepted.
    let at_limit = "x".repeat(MAX_ID_LENGTH);
    assert!(check_size("id", MAX_ID_LENGTH, at_limit.len()).is_ok());

    // One past the limit is rejected with the bound fields filled in.
    let over_limit = "x".repeat(MAX_ID_LENGTH.saturating_add(1));
    let error = check_size("id", MAX_ID_LENGTH, over_limit.len()).unwrap_err();
    assert!(matches!(
        error,
        StateChronicleError::SizeLimitExceeded { name, limit, actual }
        if name == "id" && limit == MAX_ID_LENGTH && actual == MAX_ID_LENGTH.saturating_add(1)
    ));
}
