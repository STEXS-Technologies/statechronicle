//! Deterministic ordering.
//!
//! Orders events within a commit deterministically so replays reproduce
//! identical histories (protocol §13.3). The canonical key is
//! `(resource_id, after.version)` — the resource first, then the resource's
//! state version, which reflects that resource's progression — with
//! `event_id` (unique) as the final tie-breaker. The key is total, so the
//! same event set always sorts identically independent of input order.
//!
//! Batching (in [`crate::batch::CommitBatch`]) appends events in call order;
//! this module is standalone and reusable: the executor's parallel lane can
//! order a batch deterministically before this crate consumes it.

use std::collections::HashSet;

use statechronicle_domain::event::Event;

use crate::error::CommitError;

/// Sorts `events` in place into canonical commit order.
///
/// Canonical key: `(resource_id, after.version)` ascending, tie-broken by
/// `event_id` ascending. The comparison is total and deterministic, so any
/// input order of the same event set yields the same sorted sequence.
///
/// # Errors
///
/// This function is total for the v0 baseline: it always returns `Ok`. The
/// `Result` shape keeps the API uniform with [`validate_order`] and reserves
/// room for future validation failures without a breaking change.
#[allow(clippy::unnecessary_wraps)]
pub fn sort_events(events: &mut [Event]) -> Result<(), CommitError> {
    events.sort_by(|left, right| {
        left.resource_id
            .0
            .cmp(&right.resource_id.0)
            .then_with(|| left.after.version.cmp(&right.after.version))
            .then_with(|| left.event_id.0.cmp(&right.event_id.0))
    });
    Ok(())
}

/// Returns a new [`Event`] vector in canonical commit order.
///
/// Convenience wrapper around [`sort_events`]; the input vector is consumed
/// and its elements are moved into the sorted result.
///
/// # Errors
///
/// Returns [`CommitError::Core`] when an event cannot be canonicalized (this
/// wrapper reserves the error channel that [`sort_events`]'s `Result` shape
/// provides for future validation).
pub fn order_events(mut events: Vec<Event>) -> Result<Vec<Event>, CommitError> {
    sort_events(&mut events)?;
    Ok(events)
}

/// Validates that `events` are in canonical order with no duplicates.
///
/// Asserts non-decreasing canonical keys and rejects duplicate `event_id`
/// and duplicate `(resource_id, version)` pairs fail-closed.
///
/// # Errors
///
/// Returns [`CommitError::OutOfOrder`] when a later event sorts before an
/// earlier one, [`CommitError::DuplicateEventId`] when the same event id
/// appears twice, and [`CommitError::DuplicateCanonicalKey`] when two
/// different events share the same `(resource_id, after.version)`.
pub fn validate_order(events: &[Event]) -> Result<(), CommitError> {
    let mut seen_event_ids: HashSet<&str> = HashSet::new();
    let mut seen_keys: HashSet<(&str, u64)> = HashSet::new();
    let mut previous: Option<(&str, u64)> = None;
    for event in events {
        let key = (event.resource_id.0.as_str(), event.after.version);
        if !seen_event_ids.insert(event.event_id.0.as_str()) {
            return Err(CommitError::DuplicateEventId {
                event_id: String::from(event.event_id.as_str()),
            });
        }
        if !seen_keys.insert(key) {
            return Err(CommitError::DuplicateCanonicalKey {
                resource_id: String::from(event.resource_id.0.as_str()),
                version: event.after.version,
            });
        }
        if let Some(previous_key) = previous
            && previous_key > key
        {
            return Err(CommitError::OutOfOrder {
                resource_id: String::from(event.resource_id.0.as_str()),
                version: event.after.version,
            });
        }
        previous = Some(key);
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use statechronicle_core::canonicalize::canonicalize_and_digest;
    use statechronicle_domain::event::StateCommitment;
    use statechronicle_domain::ids::{EventId, IntentId};
    use statechronicle_domain::intent::Operation;
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::subject::SubjectId;
    use statechronicle_domain::tenant::TenantId;

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn commitment(version: u64) -> StateCommitment {
        let state = serde_json::json!({ "owner": "account:stexs:player_123" });
        StateCommitment {
            version,
            state_hash: canonicalize_and_digest(&state).unwrap(),
            state,
        }
    }

    fn test_event(id: &str, resource: &str, version: u64) -> Event {
        Event::new(
            TenantId(String::from("stexs.game.alpha")),
            EventId::new(format!("evt_{id}")).unwrap(),
            IntentId::new(format!("int_{id}")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            ResourceId(String::from(resource)),
            SubjectId(String::from("account:stexs:player_123")),
            commitment(version.saturating_sub(1)),
            commitment(version),
            None,
            SubjectId(String::from("service:statechronicle.stexs.net")),
            timestamp(),
        )
    }

    fn event_ids(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .map(|event| event.event_id.0.clone())
            .collect()
    }

    #[test]
    fn empty_input_is_ok() {
        let mut empty: Vec<Event> = Vec::new();
        assert!(sort_events(&mut empty).is_ok());
        let ordered = order_events(Vec::new()).unwrap();
        assert!(ordered.is_empty());
        assert!(validate_order(&empty).is_ok());
    }

    #[test]
    fn permutations_sort_identically() {
        // Same set, four different input orders → the same canonical order.
        let a = test_event("a", "asset:beta", 2);
        let b = test_event("b", "asset:alpha", 5);
        let c = test_event("c", "asset:alpha", 1);
        let d = test_event("d", "asset:gamma", 1);
        let mut orders = vec![
            vec![a.clone(), b.clone(), c.clone(), d.clone()],
            vec![d.clone(), c.clone(), b.clone(), a.clone()],
            vec![c.clone(), a.clone(), d.clone(), b.clone()],
            vec![b.clone(), d.clone(), a.clone(), c.clone()],
        ];
        let expected = event_ids(&order_events(vec![a, b, c, d]).unwrap());
        for order in &mut orders {
            sort_events(order).unwrap();
            assert_eq!(event_ids(order), expected, "permutation diverged");
        }
    }

    #[test]
    fn per_resource_version_is_monotonic() {
        let mut events = vec![
            test_event("e", "asset:beta", 2),
            test_event("f", "asset:beta", 7),
            test_event("g", "asset:alpha", 4),
            test_event("h", "asset:alpha", 1),
            test_event("i", "asset:beta", 3),
        ];
        sort_events(&mut events).unwrap();
        for pair in events.windows(2) {
            let (left, right) = (&pair[0], &pair[1]);
            let key_order = left
                .resource_id
                .0
                .cmp(&right.resource_id.0)
                .then_with(|| left.after.version.cmp(&right.after.version));
            assert!(key_order.is_le());
        }
    }

    #[test]
    fn duplicate_event_id_is_rejected() {
        let events = vec![
            test_event("dup", "asset:alpha", 1),
            test_event("dup", "asset:beta", 2),
        ];
        let error = validate_order(&events).unwrap_err();
        assert!(matches!(
            error,
            CommitError::DuplicateEventId { event_id } if event_id == "evt_dup"
        ));
    }

    #[test]
    fn duplicate_canonical_key_is_rejected() {
        let events = vec![
            test_event("one", "asset:alpha", 3),
            test_event("two", "asset:alpha", 3),
        ];
        let error = validate_order(&events).unwrap_err();
        assert!(matches!(
            error,
            CommitError::DuplicateCanonicalKey { resource_id, version }
            if resource_id == "asset:alpha" && version == 3
        ));
    }

    #[test]
    fn unsorted_input_is_rejected() {
        let mut events = vec![
            test_event("a", "asset:alpha", 1),
            test_event("b", "asset:alpha", 2),
        ];
        sort_events(&mut events).unwrap();
        events.reverse();
        let error = validate_order(&events).unwrap_err();
        assert!(matches!(error, CommitError::OutOfOrder { .. }));
    }

    #[test]
    fn canonical_order_matches_validate_order() {
        let mut events = vec![
            test_event("x", "asset:zeta", 9),
            test_event("y", "asset:alpha", 1),
            test_event("z", "asset:alpha", 2),
        ];
        sort_events(&mut events).unwrap();
        assert!(validate_order(&events).is_ok());
    }
}
