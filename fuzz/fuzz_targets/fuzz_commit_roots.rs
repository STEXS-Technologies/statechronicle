#![no_main]

use libfuzzer_sys::fuzz_target;

use chrono::{DateTime, Utc};

use statechronicle_core::canonicalize::canonicalize_and_digest;

use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::{EventId, IntentId};
use statechronicle_domain::intent::Operation;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_commit::roots::{compute_state_root, event_root, state_root_updates};

// The commit root primitives must never panic on arbitrary input: event Merkle
// roots and state roots are pure functions of their inputs, so any outcome is
// either a valid root or a typed error. Determinism and order independence
// hold for every derived update set.
fuzz_target!(|data: &[u8]| {
    let events = build_events(data);

    // event_root never panics and is deterministic.
    if let Ok(root) = event_root(&events) {
        let again = event_root(&events);
        assert!(again.is_ok());
        assert_eq!(root, again.unwrap());
    }

    // state_root_updates + compute_state_root never panic, are deterministic,
    // and are independent of insertion order.
    if let Ok(updates) = state_root_updates(&events)
        && let Ok(root) = compute_state_root(&updates)
    {
        let again = compute_state_root(&updates);
        assert!(again.is_ok());
        assert_eq!(root, again.unwrap());

        let mut reversed = updates.clone();
        reversed.reverse();
        if let Ok(other) = compute_state_root(&reversed) {
            assert_eq!(root, other);
        }
    }
});

/// Builds a deterministic event sequence from arbitrary bytes. Every field is
/// constructed via its `Result`-returning constructor, which fails closed on
/// malformed input; unparseable chunks are skipped.
fn build_events(data: &[u8]) -> Vec<Event> {
    data.chunks(48)
        .enumerate()
        .filter_map(|(index, chunk)| build_event(chunk, index))
        .collect()
}

fn build_event(chunk: &[u8], index: usize) -> Option<Event> {
    let body: String = chunk
        .iter()
        .map(|&byte| char::from(byte % 96 + 32))
        .collect();
    if body.is_empty() {
        return None;
    }
    let event_id = EventId::new(format!("evt_{index}_{body}")).ok()?;
    let intent_id = IntentId::new(format!("int_{index}_{body}")).ok()?;
    let operation = Operation::new(String::from("asset.transfer")).ok()?;
    let owner = format!("account:example:player_{}", index % 17);
    // Every third event is subject-held, exercising the `for_subject_held`
    // key derivation; the rest are owner-based (`for_resource`).
    let (state, resource) = if index.is_multiple_of(3) {
        (
            serde_json::json!({ "subject": &owner, "quantity": "1", "unit": "items" }),
            format!("stack:item_{index}"),
        )
    } else {
        (
            serde_json::json!({ "owner": &owner, "status": "active" }),
            format!("asset:item_{index}"),
        )
    };
    let state_hash = canonicalize_and_digest(&state).ok()?;
    let after = StateCommitment {
        version: index as u64,
        state_hash: state_hash.clone(),
        state,
    };
    let before = StateCommitment {
        version: after.version,
        state_hash,
        state: serde_json::json!({}),
    };
    Some(Event::new(
        TenantId(String::from("acme.game.alpha")),
        event_id,
        intent_id,
        operation,
        ResourceId(resource),
        SubjectId(String::from("account:example:player_0")),
        before,
        after,
        None,
        SubjectId(String::from("service:statechronicle.example.net")),
        DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
            .ok()?
            .with_timezone(&Utc),
    ))
}
