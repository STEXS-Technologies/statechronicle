//! Property tests (proptest) for commit root computation.
//!
//! Generates arbitrary event sequences and checks: `compute_state_root` never
//! panics, the state root is deterministic for identical inputs, and the root
//! is independent of insertion order (the SMT root is a pure function of the
//! `(key → digest)` set, ADR-005). Events are derived from the input bytes,
//! alternating owner-based and subject-held payloads so both key derivations
//! are exercised.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use chrono::{DateTime, Utc};
use proptest::prelude::*;

use statechronicle_accumulator::sparse_merkle::StateRoot;

use statechronicle_core::canonicalize::canonicalize_and_digest;

use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::{CommitId, EventId, IntentId};
use statechronicle_domain::intent::Operation;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_commit::checkpoint::{TenantRootEntry, build_global_checkpoint};
use statechronicle_commit::ordering::{order_events, validate_order};
use statechronicle_commit::roots::{compute_state_root, state_root_updates};

fn timestamp() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:04Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn executor() -> SubjectId {
    SubjectId(String::from("service:statechronicle.stexs.net"))
}

/// Reorders `events` with a deterministic pseudo-random permutation keyed by
/// `data` bytes, so the same input always permutes the same way.
fn shuffle(events: &mut [Event], data: &[u8]) {
    if events.len() < 2 || data.is_empty() {
        return;
    }
    for index in 0..events.len() {
        let byte = data[index % data.len()];
        let other = (usize::from(byte) * 2654435761usize) % events.len();
        events.swap(index, other);
    }
}

/// Builds a set of checkpoint entries from arbitrary bytes: one entry per
/// 48-byte chunk with a distinct tenant, a valid commit id, and a 32-byte root.
fn build_entries(data: &[u8]) -> Vec<TenantRootEntry> {
    data.chunks(48)
        .enumerate()
        .filter_map(|(index, chunk)| {
            let root: [u8; 32] = chunk.get(..32)?.try_into().ok()?;
            Some(TenantRootEntry {
                tenant_id: TenantId(format!("tenant:{index:06}")),
                commit_id: CommitId::new(format!("cmt_{index:020}")).ok()?,
                state_root: StateRoot::new(root),
            })
        })
        .collect()
}

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
    let owner = format!("account:stexs:player_{}", index % 17);
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
    let version = u64::try_from(index).ok()?;
    let after = StateCommitment {
        version,
        state_hash: state_hash.clone(),
        state,
    };
    let before = StateCommitment {
        version,
        state_hash,
        state: serde_json::json!({}),
    };
    Some(Event::new(
        TenantId(String::from("stexs.game.alpha")),
        event_id,
        intent_id,
        operation,
        ResourceId(resource),
        SubjectId(String::from("account:stexs:player_0")),
        before,
        after,
        None,
        SubjectId(String::from("service:statechronicle.stexs.net")),
        DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
            .ok()?
            .with_timezone(&Utc),
    ))
}

proptest! {
    // (a) Root computation never panics on arbitrary event sequences; any
    // outcome is either a valid root or a typed error.
    #[test]
    fn state_root_never_panics(data in prop::collection::vec(any::<u8>(), 0..2048)) {
        let events = build_events(&data);
        if events.is_empty() {
            return Ok(());
        }
        let Ok(updates) = state_root_updates(&events) else {
            return Ok(());
        };
        drop(compute_state_root(&updates));
    }

    // (b) Determinism: the same events always produce the same state root.
    #[test]
    fn state_root_is_deterministic(data in prop::collection::vec(any::<u8>(), 0..2048)) {
        let events = build_events(&data);
        if events.is_empty() {
            return Ok(());
        }
        let Ok(updates) = state_root_updates(&events) else {
            return Ok(());
        };
        let Ok(first) = compute_state_root(&updates) else {
            return Ok(());
        };
        let Ok(second) = compute_state_root(&updates) else {
            return Ok(());
        };
        assert_eq!(first, second);
    }

    // (c) Sorted-by-key insertion invariant: the root is a pure function of
    // the update set, independent of insertion order (ADR-005).
    #[test]
    fn state_root_is_order_independent(data in prop::collection::vec(any::<u8>(), 0..2048)) {
        let events = build_events(&data);
        if events.is_empty() {
            return Ok(());
        }
        let Ok(updates) = state_root_updates(&events) else {
            return Ok(());
        };
        let mut reversed = updates.clone();
        reversed.reverse();
        let Ok(root) = compute_state_root(&updates) else {
            return Ok(());
        };
        let Ok(other) = compute_state_root(&reversed) else {
            return Ok(());
        };
        assert_eq!(root, other);
    }

    // (d) Ordering determinism: every permutation of the same event set sorts
    // to the identical canonical sequence, and the result validates.
    #[test]
    fn ordering_is_deterministic_under_permutation(data in prop::collection::vec(any::<u8>(), 0..2048)) {
        let events = build_events(&data);
        if events.is_empty() {
            return Ok(());
        }
        let mut shuffled = events.clone();
        shuffle(&mut shuffled, &data);
        let Ok(sorted) = order_events(events) else {
            return Ok(());
        };
        let Ok(resorted) = order_events(shuffled) else {
            return Ok(());
        };
        let ids: Vec<String> = sorted.iter().map(|e| e.event_id.0.clone()).collect();
        let resorted_ids: Vec<String> = resorted.iter().map(|e| e.event_id.0.clone()).collect();
        assert_eq!(ids, resorted_ids);
        assert!(validate_order(&sorted).is_ok());
    }

    // (e) Global checkpoint root determinism: the tenant merkle root is a pure
    // function of the (tenant_id, state_root) set, so any entry order of the
    // same set yields the same root (ADR-005 §8.1).
    #[test]
    fn global_checkpoint_root_is_order_independent(data in prop::collection::vec(any::<u8>(), 0..2048)) {
        let entries = build_entries(&data);
        if entries.is_empty() {
            return Ok(());
        }
        let Ok(checkpoint) = build_global_checkpoint(entries.clone(), 7, timestamp(), executor()) else {
            return Ok(());
        };
        let mut reversed = entries;
        reversed.reverse();
        let Ok(other) = build_global_checkpoint(reversed, 7, timestamp(), executor()) else {
            return Ok(());
        };
        assert_eq!(checkpoint.tenant_merkle_root, other.tenant_merkle_root);
    }
}
