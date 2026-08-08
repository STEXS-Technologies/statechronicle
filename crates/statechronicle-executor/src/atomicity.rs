//! Multi-resource atomic transactions (protocol §18.3).
//!
//! A transaction commits all affected state transitions or none; every
//! affected state record is validated against its expected version. The
//! executor returns batches of events that must be internally consistent:
//! distinct intent ids (except a transfer pair), distinct event ids, and a
//! single tenant scope.
//!
//! **Transfer pair invariant (§20.5).** An atomic transfer is a pair of events
//! that share one accepted intent id: a source debit and a destination credit.
//! The pair is the atomic unit: the source debit and destination credit are
//! both net-zero (debit == credit) and reference the same resource and the same
//! amount. Only `stack.transfer` / `balance.transfer` may share an intent id;
//! any other multi-event intent is internally inconsistent.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use statechronicle_core::amount::Amount;
use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::IntentId;
use statechronicle_domain::intent::Operation;
use statechronicle_domain::tenant::TenantId;
use statechronicle_profiles::consumable_stack::op as stack_op;
use statechronicle_profiles::fungible_balance::op as balance_op;
use statechronicle_profiles::keys;

use crate::error::ExecutorError;

/// Validates the internal consistency of an event batch (protocol §18.3).
///
/// A batch is internally valid only when:
///
/// * every event id is distinct: events are uniquely identifiable within the
///   tenant's history;
/// * every event shares the same tenant scope: a commit is scoped to exactly
///   one tenant (protocol §13.1);
/// * every intent id is distinct *except* that a `stack.transfer` /
///   `balance.transfer` may appear exactly twice as an atomic debit + credit
///   pair sharing one intent id, the same resource, and net-zero amounts
///   (§20.5, §18.3).
///
/// # Errors
///
/// Returns [`ExecutorError::AtomicityViolation`] when the batch contains a
/// duplicate event id, events from different tenants, or a multi-event intent
/// that is not a valid transfer pair, and
/// [`ExecutorError::TransferMismatch`] when a transfer pair's net-zero (debit
/// equals credit) or amount invariant is violated.
pub fn validate_batch_consistency(events: &[Event]) -> Result<(), ExecutorError> {
    let mut intent_to_events: BTreeMap<IntentId, Vec<&Event>> = BTreeMap::new();
    let mut event_ids = BTreeSet::new();
    let mut first_tenant: Option<&str> = None;

    for event in events {
        if !event_ids.insert(&event.event_id) {
            return Err(ExecutorError::AtomicityViolation(format!(
                "duplicate event id `{}` in batch",
                event.event_id.as_str()
            )));
        }
        let tenant = event.tenant_id.0.as_str();
        if let Some(first) = first_tenant {
            if first != tenant {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "mixed tenant scopes in batch (first tenant `{first}`)"
                )));
            }
        } else {
            first_tenant = Some(tenant);
        }
        intent_to_events
            .entry(event.intent_id.clone())
            .or_default()
            .push(event);
    }

    for (intent_id, group) in intent_to_events {
        if group.len() == 1 {
            continue;
        }
        validate_transfer_pair(&intent_id, &group)?;
    }
    Ok(())
}

/// A tenant-scoped group of events produced by one leg of a cross-tenant
/// transaction (protocol §8.2, §18.3).
///
/// This is an internal return type of [`crate::pipeline::Executor::execute_cross_tenant`];
/// it is not a wire object and carries no serde derives.
#[derive(Debug)]
pub struct TenantEventGroup {
    /// The tenant scope of this leg's events.
    pub tenant: TenantId,
    /// The events produced for this tenant leg, in execution order.
    pub events: Vec<Event>,
}

/// Validates cross-tenant consistency across per-tenant event groups
/// (protocol §8.2, §18.3).
///
/// A cross-tenant transaction is consistent when:
///
/// * (a) every event in a group is scoped to that group's tenant: a partition
///   mismatch fails closed;
/// * (b) each group is internally batch-consistent via
///   [`validate_batch_consistency`] (distinct event ids and distinct intent ids
///   except a valid atomic transfer pair);
/// * (c) the transaction is genuinely cross-tenant: at least one intent id
///   appears in two or more distinct tenant groups (the linkage that ties the
///   legs together).
///
/// The existing per-tenant transfer-pair rule ([`validate_transfer_pair`],
/// reached through [`validate_batch_consistency`]) is still enforced within each
/// group; this function introduces no new conservation rules.
///
/// # Errors
///
/// Returns [`ExecutorError::AtomicityViolation`] when a group contains events
/// scoped to a different tenant, when a group is internally inconsistent, or
/// when no intent id links at least two distinct tenant groups.
pub fn validate_cross_tenant_consistency(groups: &[TenantEventGroup]) -> Result<(), ExecutorError> {
    let mut intent_tenants: BTreeMap<&IntentId, BTreeSet<&str>> = BTreeMap::new();
    for group in groups {
        for event in &group.events {
            if event.tenant_id != group.tenant {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "tenant mismatch in cross-tenant batch (event tenant `{}`, group tenant `{}`)",
                    event.tenant_id.0, group.tenant.0
                )));
            }
            intent_tenants
                .entry(&event.intent_id)
                .or_default()
                .insert(group.tenant.0.as_str());
        }
        validate_batch_consistency(&group.events)?;
    }
    if !intent_tenants.values().any(|tenants| tenants.len() >= 2) {
        return Err(ExecutorError::AtomicityViolation(String::from(
            "cross-tenant batch has no cross-tenant intent linkage",
        )));
    }
    Ok(())
}

/// Validates that a multi-event intent group is an atomic transfer pair.
///
/// A transfer pair must contain exactly two events of the same
/// `stack.transfer` / `balance.transfer` operation over the same resource,
/// whose net debit equals net credit (net-zero) with a strictly positive
/// amount (§20.5).
///
/// # Errors
///
/// Returns [`ExecutorError::AtomicityViolation`] when the group is not exactly
/// two events of a transfer operation over the same resource, and
/// [`ExecutorError::TransferMismatch`] when the pair's amounts are malformed or
/// net-zero is violated.
fn validate_transfer_pair(intent_id: &IntentId, group: &[&Event]) -> Result<(), ExecutorError> {
    if group.len() != 2 {
        return Err(duplicate_intent_error(intent_id));
    }
    let Some(first) = group.first() else {
        return Err(duplicate_intent_error(intent_id));
    };
    let Some(second) = group.get(1) else {
        return Err(duplicate_intent_error(intent_id));
    };
    let Some(field) = transfer_field(&first.operation) else {
        return Err(duplicate_intent_error(intent_id));
    };
    if transfer_field(&second.operation) != Some(field) {
        return Err(duplicate_intent_error(intent_id));
    }
    if first.operation != second.operation {
        return Err(duplicate_intent_error(intent_id));
    }
    if first.resource_id != second.resource_id {
        return Err(duplicate_intent_error(intent_id));
    }

    let mut debit = Amount::ZERO;
    let mut credit = Amount::ZERO;
    for event in group {
        let before = commitment_amount(&event.before, field)?;
        let after = commitment_amount(&event.after, field)?;
        if before >= after {
            let delta = before
                .checked_sub(after)
                .ok_or_else(|| ExecutorError::TransferMismatch(String::from("debit overflow")))?;
            debit = debit
                .checked_add(delta)
                .ok_or_else(|| ExecutorError::TransferMismatch(String::from("debit overflow")))?;
        } else {
            let delta = after
                .checked_sub(before)
                .ok_or_else(|| ExecutorError::TransferMismatch(String::from("credit overflow")))?;
            credit = credit
                .checked_add(delta)
                .ok_or_else(|| ExecutorError::TransferMismatch(String::from("credit overflow")))?;
        }
    }
    if debit == Amount::ZERO || debit != credit {
        return Err(ExecutorError::TransferMismatch(format!(
            "net-zero violated for intent `{}`: debit {debit} != credit {credit}",
            intent_id.as_str()
        )));
    }
    Ok(())
}

/// Returns the amount field name for a transfer operation, or `None` when the
/// operation is not a subject-held transfer.
fn transfer_field(operation: &Operation) -> Option<&'static str> {
    if operation == stack_op::stack_transfer() {
        Some(keys::QUANTITY)
    } else if operation == balance_op::balance_transfer() {
        Some(keys::BALANCE)
    } else {
        None
    }
}

/// Reads the fixed-point [`Amount`] for `field` from a state commitment.
///
/// A missing field (e.g. the empty before-state of a create-on-credit
/// destination) reads as zero.
///
/// # Errors
///
/// Returns [`ExecutorError::TransferMismatch`] when the field is present but
/// not a canonical non-negative decimal integer string.
fn commitment_amount(commitment: &StateCommitment, field: &str) -> Result<Amount, ExecutorError> {
    let Some(text) = commitment.state.get(field).and_then(Value::as_str) else {
        return Ok(Amount::ZERO);
    };
    Amount::try_from_str(text)
        .map_err(|_source| ExecutorError::TransferMismatch(format!("malformed `{field}` amount")))
}

/// Builds the fail-closed duplicate-intent error for a non-transfer group.
fn duplicate_intent_error(intent_id: &IntentId) -> ExecutorError {
    ExecutorError::AtomicityViolation(format!(
        "duplicate intent id `{}` in batch (only transfer pairs may share an intent id)",
        intent_id.as_str()
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use statechronicle_core::digest::hash_bytes;
    use statechronicle_domain::ids::{EventId, IntentId};
    use statechronicle_domain::intent::Operation;
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::subject::SubjectId;
    use statechronicle_domain::tenant::TenantId;

    fn tenant(name: &str) -> TenantId {
        TenantId(String::from(name))
    }

    fn event(id: &str, tenant_name: &str, intent: &str) -> Event {
        Event::new(
            tenant(tenant_name),
            EventId::new(format!("evt_{id}")).unwrap(),
            IntentId::new(format!("int_{intent}")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            ResourceId(String::from("asset:sword_001")),
            SubjectId(String::from("account:example:player_123")),
            statechronicle_domain::event::StateCommitment {
                version: 1,
                state_hash: hash_bytes(b"before"),
                state: serde_json::json!({}),
            },
            statechronicle_domain::event::StateCommitment {
                version: 2,
                state_hash: hash_bytes(b"after"),
                state: serde_json::json!({ "owner": "bob", "status": "active" }),
            },
            None,
            SubjectId(String::from("service:statechronicle.example.net")),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }

    #[test]
    fn consistent_batch_passes() {
        let events = vec![
            event("a", "acme.game.alpha", "1"),
            event("b", "acme.game.alpha", "2"),
        ];
        assert!(validate_batch_consistency(&events).is_ok());
    }

    #[test]
    fn duplicate_intent_id_fails_closed() {
        let events = vec![
            event("a", "acme.game.alpha", "1"),
            event("b", "acme.game.alpha", "1"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("duplicate intent id")
        ));
    }

    #[test]
    fn duplicate_event_id_fails_closed() {
        let events = vec![
            event("a", "acme.game.alpha", "1"),
            event("a", "acme.game.alpha", "2"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("duplicate event id")
        ));
    }

    #[test]
    fn mixed_tenants_fail_closed() {
        let events = vec![
            event("a", "acme.game.alpha", "1"),
            event("b", "acme.game.other", "2"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("mixed tenant scopes")
        ));
    }

    #[test]
    fn empty_batch_passes() {
        assert!(validate_batch_consistency(&[]).is_ok());
    }

    /// Builds a `balance.transfer` event for a subject-held resource with the
    /// given before/after balances (canonical integer strings).
    fn balance_transfer(
        event_id: &str,
        intent: &str,
        subject: &str,
        before: &str,
        after: &str,
    ) -> Event {
        Event::new(
            tenant("acme.game.alpha"),
            EventId::new(format!("evt_{event_id}")).unwrap(),
            IntentId::new(format!("int_{intent}")).unwrap(),
            Operation::new(String::from("balance.transfer")).unwrap(),
            ResourceId(String::from("currency:gold")),
            SubjectId(String::from(subject)),
            StateCommitment {
                version: 1,
                state_hash: hash_bytes(b"before"),
                state: serde_json::json!({
                    "subject": "alice",
                    "balance": before,
                    "unit": "gold_minor",
                }),
            },
            StateCommitment {
                version: 2,
                state_hash: hash_bytes(b"after"),
                state: serde_json::json!({
                    "subject": subject,
                    "balance": after,
                    "unit": "gold_minor",
                }),
            },
            None,
            SubjectId(String::from("service:statechronicle.example.net")),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }

    #[test]
    fn transfer_pair_net_zero_passes() {
        // Source: 100 -> 60 (debit 40). Destination: 0 (create) -> 40 (credit 40).
        let events = vec![
            balance_transfer("src", "t1", "alice", "100", "60"),
            balance_transfer("dst", "t1", "bob", "0", "40"),
        ];
        assert!(validate_batch_consistency(&events).is_ok());
    }

    #[test]
    fn transfer_pair_to_existing_destination_net_zero_passes() {
        // Source: 100 -> 60 (debit 40). Destination: 50 -> 90 (credit 40).
        let events = vec![
            balance_transfer("src", "t1", "alice", "100", "60"),
            balance_transfer("dst", "t1", "bob", "50", "90"),
        ];
        assert!(validate_batch_consistency(&events).is_ok());
    }

    #[test]
    fn transfer_pair_mismatched_amounts_fail_closed() {
        // Source: 100 -> 60 (debit 40). Destination: 0 -> 30 (credit 30).
        let events = vec![
            balance_transfer("src", "t1", "alice", "100", "60"),
            balance_transfer("dst", "t1", "bob", "0", "30"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::TransferMismatch(message))
            if message.contains("net-zero")
        ));
    }

    #[test]
    fn transfer_pair_two_debits_fail_closed() {
        let events = vec![
            balance_transfer("a", "t1", "alice", "100", "60"),
            balance_transfer("b", "t1", "bob", "50", "10"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::TransferMismatch(message))
            if message.contains("net-zero")
        ));
    }

    #[test]
    fn non_transfer_multi_event_rejected() {
        let events = vec![
            event("a", "acme.game.alpha", "1"),
            event("b", "acme.game.alpha", "1"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("duplicate intent id")
        ));
    }

    #[test]
    fn transfer_pair_with_three_events_rejected() {
        let events = vec![
            balance_transfer("a", "t1", "alice", "100", "60"),
            balance_transfer("b", "t1", "bob", "0", "40"),
            balance_transfer("c", "t1", "carol", "0", "1"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("duplicate intent id")
        ));
    }

    #[test]
    fn transfer_pair_different_resources_rejected() {
        let source = balance_transfer("src", "t1", "alice", "100", "60");
        let mut destination = balance_transfer("dst", "t1", "bob", "0", "40");
        destination.resource_id = ResourceId(String::from("currency:silver"));
        assert!(matches!(
            validate_batch_consistency(&[source, destination]),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("duplicate intent id")
        ));
    }

    /// Builds a `balance.transfer` event scoped to an arbitrary tenant, with the
    /// given before/after balances (canonical integer strings).
    fn transfer_event(
        tenant_name: &str,
        event_id: &str,
        intent: &str,
        subject: &str,
        before: &str,
        after: &str,
    ) -> Event {
        Event::new(
            tenant(tenant_name),
            EventId::new(format!("evt_{event_id}")).unwrap(),
            IntentId::new(format!("int_{intent}")).unwrap(),
            Operation::new(String::from("balance.transfer")).unwrap(),
            ResourceId(String::from("currency:gold")),
            SubjectId(String::from(subject)),
            StateCommitment {
                version: 1,
                state_hash: hash_bytes(b"before"),
                state: serde_json::json!({
                    "subject": "alice",
                    "balance": before,
                    "unit": "gold_minor",
                }),
            },
            StateCommitment {
                version: 2,
                state_hash: hash_bytes(b"after"),
                state: serde_json::json!({
                    "subject": subject,
                    "balance": after,
                    "unit": "gold_minor",
                }),
            },
            None,
            SubjectId(String::from("service:statechronicle.example.net")),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }

    #[test]
    fn cross_tenant_valid_pair_passes() {
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.alpha", "a", "x", "alice", "100", "60"),
                    transfer_event("acme.game.alpha", "b", "x", "bob", "0", "40"),
                ],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![
                    transfer_event("acme.game.beta", "c", "x", "carol", "100", "60"),
                    transfer_event("acme.game.beta", "d", "x", "dave", "0", "40"),
                ],
            },
        ];
        assert!(validate_cross_tenant_consistency(&groups).is_ok());
    }

    #[test]
    fn single_tenant_batch_rejected() {
        // Both groups declare the same tenant, so no intent spans two distinct
        // tenants. The batch is not genuinely cross-tenant.
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.alpha", "a", "x", "alice", "100", "60"),
                    transfer_event("acme.game.alpha", "b", "x", "bob", "0", "40"),
                ],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.alpha", "c", "y", "carol", "100", "60"),
                    transfer_event("acme.game.alpha", "d", "y", "dave", "0", "40"),
                ],
            },
        ];
        assert!(matches!(
            validate_cross_tenant_consistency(&groups),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("no cross-tenant intent linkage")
        ));
    }

    #[test]
    fn partition_mismatch_rejected() {
        // The first group declares alpha but holds beta-scoped events.
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.beta", "a", "x", "alice", "100", "60"),
                    transfer_event("acme.game.beta", "b", "x", "bob", "0", "40"),
                ],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![
                    transfer_event("acme.game.beta", "c", "x", "carol", "100", "60"),
                    transfer_event("acme.game.beta", "d", "x", "dave", "0", "40"),
                ],
            },
        ];
        assert!(matches!(
            validate_cross_tenant_consistency(&groups),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("tenant mismatch")
        ));
    }

    #[test]
    fn no_linkage_rejected() {
        // Two tenants, but each intent id appears in exactly one tenant.
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.alpha", "a", "x", "alice", "100", "60"),
                    transfer_event("acme.game.alpha", "b", "x", "bob", "0", "40"),
                ],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![
                    transfer_event("acme.game.beta", "c", "y", "carol", "100", "60"),
                    transfer_event("acme.game.beta", "d", "y", "dave", "0", "40"),
                ],
            },
        ];
        assert!(matches!(
            validate_cross_tenant_consistency(&groups),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("no cross-tenant intent linkage")
        ));
    }

    #[test]
    fn per_tenant_transfer_pair_still_enforced() {
        // Alpha's transfer pair is not net-zero (debit 40 != credit 30); the
        // per-tenant transfer-pair rule still applies across the cross-tenant
        // groups.
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.alpha", "a", "x", "alice", "100", "60"),
                    transfer_event("acme.game.alpha", "b", "x", "bob", "0", "30"),
                ],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![
                    transfer_event("acme.game.beta", "c", "x", "carol", "100", "60"),
                    transfer_event("acme.game.beta", "d", "x", "dave", "0", "40"),
                ],
            },
        ];
        assert!(matches!(
            validate_cross_tenant_consistency(&groups),
            Err(ExecutorError::TransferMismatch(message))
            if message.contains("net-zero")
        ));
    }
}
