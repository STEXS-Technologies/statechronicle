//! Fail-closed conflict rules (protocol §18.2).
//!
//! Pure, deterministic gates evaluated by the pipeline before any transition
//! is accepted. Every function here is total: malformed input produces an
//! [`ExecutorError`] variant, never a panic. Each gate mirrors one line of
//! protocol §18.2's "must fail closed" list.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

use statechronicle_domain::intent::{Intent, Operation};
use statechronicle_domain::state::StateProjection;

use crate::error::ExecutorError;

/// A unique asset status that blocks mutations: the asset is locked against
/// transfer, burn, and listing (only `asset.unlock` and `asset.restrict`
/// escape).
const LOCKED: &str = "locked";
/// A unique asset status that blocks mutations: the asset is held in escrow
/// (only `asset.release` and `asset.restrict` escape).
const ESCROWED: &str = "escrowed";
/// A unique asset status that blocks mutations: the asset is listed for sale
/// (only `asset.delist`, `asset.redeem`, and `asset.restrict` escape).
const LISTED: &str = "listed";
/// Terminal statuses: no operation may ever leave them (protocol §20.1).
const TERMINAL_STATUSES: &[&str] = &[
    "burned",
    "expired",
    "tombstoned",
    "revoked",
    "redeemed",
    "sold",
    "cancelled",
    "released",
    "refunded",
];

/// Exceptional statuses a paid unique asset may be restricted into; only
/// `asset.restore` (or `entitlement.restore` for entitlements) escapes.
const EXCEPTIONAL_STATUSES: &[&str] = &[
    "restricted",
    "quarantined",
    "unsupported",
    "legal_hold",
    "fraud_lock",
    "policy_restricted",
];

/// Checks the intent's `expected_version` against the current projection
/// (protocol §18.2 "expected_version does not match current version").
///
/// A `None` current projection is acceptable only when `expected_version` is
/// zero (a create or mint); otherwise the resource does not exist and the
/// check fails closed with [`ExecutorError::ResourceNotFound`].
///
/// # Errors
///
/// Returns [`ExecutorError::ResourceNotFound`] when `current` is `None` and
/// `expected_version` is positive, and
/// [`ExecutorError::ExpectedVersionMismatch`] when `current.version` differs
/// from `expected_version`.
pub fn check_expected_version(
    intent: &Intent,
    current: Option<&StateProjection>,
) -> Result<(), ExecutorError> {
    let expected = intent.expected_version;
    match current {
        None if expected > 0 => Err(ExecutorError::ResourceNotFound {
            resource: intent.resource_id.0.clone(),
        }),
        None => Ok(()),
        Some(projection) if projection.version != expected => {
            Err(ExecutorError::ExpectedVersionMismatch {
                resource: intent.resource_id.0.clone(),
                expected,
                actual: projection.version,
            })
        }
        Some(_) => Ok(()),
    }
}

/// Checks that the intent carries a non-empty tenant scope (protocol §18.2
/// "Tenant scope is missing, ambiguous, or not authorized").
///
/// # Errors
///
/// Returns [`ExecutorError::TenantScopeMissing`] when `intent.tenant_id` is
/// empty.
pub const fn check_tenant_scope(intent: &Intent) -> Result<(), ExecutorError> {
    if intent.tenant_id.0.is_empty() {
        return Err(ExecutorError::TenantScopeMissing);
    }
    Ok(())
}

/// Checks that the operation's `from_owner` input matches the resource's
/// current owner or holder (protocol §18.2 "Current owner, holder, balance,
/// quantity, or subject does not match operation input").
///
/// The owner identity is read from the projection's `owner` field, falling
/// back to `subject` for subject-held types. When the operation supplies no
/// `from_owner` input, the check passes: the profile's own rules decide
/// whether ownership consent is required. The `_intent` parameter is reserved
/// for pipeline-level context (the resource and actor are already bound into
/// the intent) and currently carries no additional gate.
///
/// # Errors
///
/// Returns [`ExecutorError::ActorMismatch`] when `from_owner` is present and
/// differs from the current owner or holder.
pub fn check_owner(
    _intent: &Intent,
    current: Option<&StateProjection>,
    inputs: &BTreeMap<String, Value>,
) -> Result<(), ExecutorError> {
    let Some(projection) = current else {
        return Ok(());
    };
    let Some(actual) = inputs.get("from_owner").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(expected) = projection
        .state
        .get("owner")
        .and_then(Value::as_str)
        .or_else(|| projection.state.get("subject").and_then(Value::as_str))
    else {
        return Ok(());
    };
    if actual != expected {
        return Err(ExecutorError::ActorMismatch {
            expected: String::from(expected),
            actual: String::from(actual),
        });
    }
    Ok(())
}

/// Checks that the resource's status does not block the operation
/// (protocol §18.2 "Resource is locked, burned, revoked, or escrowed in a way
/// that blocks the operation").
///
/// The gate is fail-closed and mirrors the profiles' status conventions
/// (`statechronicle-profiles` §20): terminal statuses block every operation;
/// non-terminal blocking statuses (locked, escrowed, listed, suspended,
/// exceptional) block every operation except the status's own escape
/// operations; payloads without a `status` field (stacks, balances, meters)
/// are governed entirely by the profile's quantity rules and pass.
///
/// # Errors
///
/// Returns [`ExecutorError::ResourceLocked`] when the resource's status blocks
/// the operation or the status is unrecognized (fail closed).
pub fn check_resource_availability(
    current: &StateProjection,
    operation: &Operation,
) -> Result<(), ExecutorError> {
    let Some(status) = current.state.get("status").and_then(Value::as_str) else {
        return Ok(());
    };
    if is_free_status(status) {
        return Ok(());
    }
    if TERMINAL_STATUSES.contains(&status) {
        return Err(blocked(current));
    }
    if EXCEPTIONAL_STATUSES.contains(&status) && operation.as_str() == "asset.restore" {
        return Ok(());
    }
    let escapes = status_escapes(status);
    if escapes.contains(&operation.as_str()) {
        return Ok(());
    }
    Err(blocked(current))
}

/// Checks whether the intent expired before acceptance (protocol §18.2
/// "Intent expired before acceptance").
///
/// An intent without an expiry never expires. An intent is expired when
/// `expires_at` is present and not strictly after `now`.
///
/// # Errors
///
/// Returns [`ExecutorError::Expired`] when `expires_at` is present and not
/// strictly after `now`.
pub fn check_expiry(intent: &Intent, now: DateTime<Utc>) -> Result<(), ExecutorError> {
    if intent.expires_at.is_some_and(|expiry| expiry <= now) {
        return Err(ExecutorError::Expired {
            intent_id: intent.intent_id.0.clone(),
        });
    }
    Ok(())
}

/// Checks idempotency of a replayed `intent_id` (protocol §18.2 "Duplicate
/// `intent_id` with different payload").
///
/// Replaying the same accepted intent (equal full payload) succeeds; replaying
/// a conflicting intent under the same `intent_id` fails closed.
///
/// # Errors
///
/// Returns [`ExecutorError::DuplicateIntent`] when `existing` differs from
/// `incoming` in any field.
pub fn check_idempotency_existing(
    existing: &Intent,
    incoming: &Intent,
) -> Result<(), ExecutorError> {
    if existing == incoming {
        return Ok(());
    }
    Err(ExecutorError::DuplicateIntent {
        intent_id: incoming.intent_id.0.clone(),
    })
}

/// Returns whether a status is always available for mutation.
fn is_free_status(status: &str) -> bool {
    matches!(status, "active" | "granted")
}

/// Returns the escape operations permitted from a non-terminal blocking status.
fn status_escapes(status: &str) -> &'static [&'static str] {
    match status {
        LOCKED => &["asset.unlock", "asset.restrict"],
        ESCROWED => &["asset.release", "asset.restrict"],
        LISTED => &["asset.delist", "asset.redeem", "asset.restrict"],
        "suspended" => &["entitlement.restore", "entitlement.expire"],
        "granted" => &[
            "entitlement.activate",
            "entitlement.expire",
            "entitlement.revoke",
            "entitlement.transfer",
        ],
        // Unreachable for terminal/exceptional/free statuses; fail closed by
        // returning no escapes so any operation is blocked.
        _ => &[],
    }
}

/// Builds a fail-closed `ResourceLocked` error for a blocked operation.
fn blocked(current: &StateProjection) -> ExecutorError {
    ExecutorError::ResourceLocked {
        resource: current.resource_id.0.clone(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;
    use statechronicle_core::digest::ContentDigest;
    use statechronicle_domain::ids::{CommitId, EventId, IntentId};
    use statechronicle_domain::intent::{Nonce, Operation};
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::state_type::StateType;
    use statechronicle_domain::subject::SubjectId;
    use statechronicle_domain::tenant::TenantId;

    fn op(name: &str) -> Operation {
        Operation::new(String::from(name)).unwrap()
    }

    fn inputs(entries: &[(&str, Value)]) -> BTreeMap<String, Value> {
        entries
            .iter()
            .map(|(key, value)| (String::from(*key), value.clone()))
            .collect()
    }

    fn projection(state_type: StateType, state: Value) -> StateProjection {
        StateProjection {
            tenant_id: TenantId(String::from("tenant.test")),
            resource_id: ResourceId(String::from("asset:sword_001")),
            state_type,
            version: 5,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state,
        }
    }

    fn asset(status: &str, owner: &str) -> StateProjection {
        projection(
            StateType::UniqueAsset,
            json!({ "owner": owner, "status": status }),
        )
    }

    fn sample_intent() -> Intent {
        Intent::new(
            TenantId(String::from("tenant.test")),
            IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
            op("asset.transfer"),
            SubjectId(String::from("account:example:player_123")),
            ResourceId(String::from("asset:sword_001")),
            Some(StateType::UniqueAsset),
            5,
            BTreeMap::new(),
            None,
            DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            None,
            Nonce::from_bytes(vec![1]).unwrap(),
        )
    }

    #[test]
    fn expected_version_none_with_zero_is_ok() {
        let mut intent = sample_intent();
        intent.expected_version = 0;
        assert!(check_expected_version(&intent, None).is_ok());
    }

    #[test]
    fn expected_version_none_with_positive_is_resource_not_found() {
        let intent = sample_intent();
        assert!(matches!(
            check_expected_version(&intent, None),
            Err(ExecutorError::ResourceNotFound { resource })
            if resource == "asset:sword_001"
        ));
    }

    #[test]
    fn expected_version_mismatch_fails_closed() {
        let intent = sample_intent();
        let current = asset("active", "alice");
        assert!(check_expected_version(&intent, Some(&current)).is_ok());

        let mut stale = sample_intent();
        stale.expected_version = 4;
        assert!(matches!(
            check_expected_version(&stale, Some(&current)),
            Err(ExecutorError::ExpectedVersionMismatch { resource, expected, actual })
            if resource == "asset:sword_001" && expected == 4 && actual == 5
        ));
    }

    #[test]
    fn tenant_scope_empty_fails_closed() {
        let mut intent = sample_intent();
        intent.tenant_id = TenantId(String::new());
        assert!(matches!(
            check_tenant_scope(&intent),
            Err(ExecutorError::TenantScopeMissing)
        ));
        assert!(check_tenant_scope(&sample_intent()).is_ok());
    }

    #[test]
    fn owner_mismatch_fails_closed() {
        let current = asset("active", "alice");
        let inputs = inputs(&[("from_owner", json!("mallory"))]);
        let intent = sample_intent();
        assert!(matches!(
            check_owner(&intent, Some(&current), &inputs),
            Err(ExecutorError::ActorMismatch { expected, actual })
            if expected == "alice" && actual == "mallory"
        ));
    }

    #[test]
    fn owner_match_and_missing_input_pass() {
        let current = asset("active", "alice");
        let intent = sample_intent();
        let matching = inputs(&[("from_owner", json!("alice"))]);
        assert!(check_owner(&intent, Some(&current), &matching).is_ok());
        assert!(check_owner(&intent, Some(&current), &BTreeMap::new()).is_ok());
        assert!(check_owner(&intent, None, &BTreeMap::new()).is_ok());
    }

    #[test]
    fn subject_held_owner_uses_subject_field() {
        let balance = projection(
            StateType::FungibleBalance,
            json!({
                "subject": "account:example:player_123",
                "balance": "100",
                "unit": "gold_minor",
            }),
        );
        let intent = sample_intent();
        let inputs = inputs(&[("from_owner", json!("other"))]);
        assert!(matches!(
            check_owner(&intent, Some(&balance), &inputs),
            Err(ExecutorError::ActorMismatch { .. })
        ));
    }

    #[test]
    fn resource_availability_active_passes() {
        let current = asset("active", "alice");
        for name in ["asset.transfer", "asset.lock", "asset.burn"] {
            assert!(
                check_resource_availability(&current, &op(name)).is_ok(),
                "{name} should be allowed from active"
            );
        }
    }

    #[test]
    fn resource_availability_locked_blocks_mutations() {
        let current = asset("locked", "alice");
        assert!(
            check_resource_availability(&current, &op("asset.unlock")).is_ok(),
            "unlock escapes locked"
        );
        assert!(matches!(
            check_resource_availability(&current, &op("asset.transfer")),
            Err(ExecutorError::ResourceLocked { resource })
            if resource == "asset:sword_001"
        ));
        assert!(check_resource_availability(&current, &op("asset.restrict")).is_ok());
    }

    #[test]
    fn resource_availability_terminal_blocks_everything() {
        for status in TERMINAL_STATUSES {
            let current = asset(status, "alice");
            assert!(
                check_resource_availability(&current, &op("asset.restore")).is_err(),
                "restore must be blocked from terminal `{status}`"
            );
            assert!(
                check_resource_availability(&current, &op("asset.transfer")).is_err(),
                "transfer must be blocked from terminal `{status}`"
            );
        }
    }

    #[test]
    fn resource_availability_no_status_passes() {
        let stack = projection(
            StateType::ConsumableStack,
            json!({ "subject": "alice", "quantity": "10", "unit": "arrows" }),
        );
        assert!(check_resource_availability(&stack, &op("stack.debit")).is_ok());
    }

    #[test]
    fn resource_availability_unknown_status_blocks() {
        let current = asset("weird_state", "alice");
        assert!(matches!(
            check_resource_availability(&current, &op("asset.transfer")),
            Err(ExecutorError::ResourceLocked { .. })
        ));
    }

    #[test]
    fn expiry_checks_against_now() {
        let mut intent = sample_intent();
        intent.expires_at = Some(
            DateTime::parse_from_rfc3339("2026-07-14T00:05:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        let before = DateTime::parse_from_rfc3339("2026-07-14T00:04:59Z")
            .unwrap()
            .with_timezone(&Utc);
        let at = DateTime::parse_from_rfc3339("2026-07-14T00:05:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(check_expiry(&intent, before).is_ok());
        assert!(matches!(
            check_expiry(&intent, at),
            Err(ExecutorError::Expired { intent_id })
            if intent_id == "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2"
        ));
    }

    #[test]
    fn intent_without_expiry_never_expires() {
        let intent = sample_intent();
        let far_future = DateTime::parse_from_rfc3339("2999-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(check_expiry(&intent, far_future).is_ok());
    }

    #[test]
    fn idempotency_equal_payload_ok_conflict_fails() {
        let existing = sample_intent();
        let mut incoming = sample_intent();
        assert!(check_idempotency_existing(&existing, &incoming).is_ok());

        incoming.inputs = inputs(&[("to_owner", json!("bob"))]);
        assert!(matches!(
            check_idempotency_existing(&existing, &incoming),
            Err(ExecutorError::DuplicateIntent { intent_id })
            if intent_id == "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2"
        ));
    }
}
