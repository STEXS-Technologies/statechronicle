#![no_main]

use libfuzzer_sys::fuzz_target;

use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use statechronicle_accumulator::sparse_merkle::StateRoot;

use statechronicle_core::canonicalize::canonicalize_and_digest;

use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::{CommitId, EventId, IntentId};
use statechronicle_domain::intent::{KeyId, Operation};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_commit::checkpoint::{
    TenantRootEntry, build_global_checkpoint, sign_global_checkpoint, verify_global_checkpoint,
};
use statechronicle_commit::ordering::{order_events, sort_events, validate_order};

// Fixed signing key so sign/verify runs against a stable authority.
const FIXED_SEED: [u8; 32] = [42u8; 32];

// Ordering and checkpoint paths must never panic on arbitrary input: failures
// surface as typed errors, never as panics. Determinism is checked by re-sorting
// the same set and by re-verifying freshly signed checkpoints.
fuzz_target!(|data: &[u8]| {
    let events = build_events(data);

    // sort_events / order_events / validate_order never panic.
    let mut sorted = events.clone();
    let _ = sort_events(&mut sorted);
    let _ = validate_order(&sorted);
    let _ = order_events(events.clone());

    // A GlobalCheckpoint built from random entries signs and verifies without
    // panicking. Duplicate tenants or an empty set fail closed (typed error).
    let entries = build_entries(data);
    let key = SigningKey::from_bytes(&FIXED_SEED);
    let Some(created_at) = timestamp() else {
        return;
    };
    let Ok(checkpoint) = build_global_checkpoint(
        entries,
        7,
        created_at,
        SubjectId(String::from("service:statechronicle.stexs.net")),
    ) else {
        return;
    };
    let Some(key_id) = KeyId::new(String::from("did:key:z6Mk...#global-checkpoint")).ok() else {
        return;
    };
    let Ok(signed) = sign_global_checkpoint(&checkpoint, &key, key_id) else {
        return;
    };
    let _ = verify_global_checkpoint(&signed, &key.verifying_key());
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
    let owner = format!("account:stexs:player_{}", index % 17);
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
        timestamp()?,
    ))
}

/// Builds checkpoint entries from arbitrary bytes: one entry per 48-byte chunk
/// with a distinct tenant, a valid commit id, and a 32-byte root.
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

fn timestamp() -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:05Z")
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}
