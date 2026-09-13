#![no_main]

use chrono::{DateTime, Utc};

use libfuzzer_sys::fuzz_target;

use statechronicle_core::digest::hash_bytes;
use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::{EventId, IntentId};
use statechronicle_domain::intent::Operation;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::resource_state::ResourceState;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_executor::atomicity::validate_settle_batch;

// `validate_settle_batch` is total over arbitrary settle batches: any
// arbitrary partition of valid events plus arbitrary settle intents must
// return a `Result` — never panic. A fixed pool of valid events drives the
// batch-consistency and value-leg checks; the byte stream decides which events
// land in the batch and which settle intents declare them, so every fail-closed
// path (mixed tenant scopes, undeclared groups, value-pair mismatches, bundle
// shape) is reachable.
fuzz_target!(|data: &[u8]| {
    let Some(pool) = build_pool() else {
        return;
    };
    let mut events: Vec<Event> = Vec::new();
    for (event_index, event) in pool.iter().enumerate() {
        let probe = event_index % data.len().max(1);
        if data.get(probe).copied().unwrap_or(0) % 2 == 1 {
            events.push(event.clone());
        }
    }
    let Some(intent_pool) = build_intents() else {
        return;
    };
    let mut settle_intents: Vec<statechronicle_domain::intent::Intent> = Vec::new();
    for (intent_index, intent) in intent_pool.iter().enumerate() {
        let probe = (pool.len() + intent_index) % data.len().max(1);
        if data.get(probe).copied().unwrap_or(0) % 2 == 1 {
            settle_intents.push(intent.clone());
        }
    }
    // Total: the validator never panics on arbitrary events + intents.
    let _ = validate_settle_batch(&events, &settle_intents);
});

/// Builds a fixed pool of valid events with distinct ids, using `.ok()?` so
/// newtype construction never panics on the fixed literals.
fn build_pool() -> Option<Vec<Event>> {
    let mut pool = Vec::new();
    for index in 0..16usize {
        pool.push(Event::new(
            TenantId(String::from("acme.game.alpha")),
            EventId::new(format!("evt_{index:020}")).ok()?,
            IntentId::new(format!("int_{index:08}")).ok()?,
            Operation::new(String::from("trade.settle")).ok()?,
            ResourceId(String::from("asset:sword")),
            SubjectId(String::from("account:example:player")),
            StateCommitment {
                version: 1,
                state_hash: hash_bytes(b"before"),
                state: ResourceState::from_legacy_json(
                    StateType::UniqueAsset,
                    serde_json::json!({ "owner": "alice", "status": "trade_held" }),
                )
                .ok()?,
            },
            StateCommitment {
                version: 2,
                state_hash: hash_bytes(b"after"),
                state: ResourceState::from_legacy_json(
                    StateType::UniqueAsset,
                    serde_json::json!({ "owner": "bob", "status": "active" }),
                )
                .ok()?,
            },
            None,
            SubjectId(String::from("service:statechronicle.example.net")),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
                .ok()?
                .with_timezone(&Utc),
        ));
    }
    Some(pool)
}

/// Builds a fixed pool of settle intents with distinct ids.
fn build_intents() -> Option<Vec<statechronicle_domain::intent::Intent>> {
    let mut intents = Vec::new();
    for index in 0..8usize {
        let mut inputs = std::collections::BTreeMap::new();
        inputs.insert(String::from("from_owner"), serde_json::json!("alice"));
        inputs.insert(String::from("to_owner"), serde_json::json!("bob"));
        intents.push(statechronicle_domain::intent::Intent::new(
            TenantId(String::from("acme.game.alpha")),
            IntentId::new(format!("int_{index:08}")).ok()?,
            Operation::new(String::from("trade.settle")).ok()?,
            SubjectId(String::from("account:example:player")),
            ResourceId(String::from("asset:sword")),
            Some(statechronicle_domain::state_type::StateType::UniqueAsset),
            1,
            inputs,
            None,
            DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
                .ok()?
                .with_timezone(&Utc),
            None,
            statechronicle_domain::intent::Nonce::from_bytes(vec![1]).ok()?,
        ));
    }
    Some(intents)
}
