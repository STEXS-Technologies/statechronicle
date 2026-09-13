#![no_main]

use std::collections::BTreeMap;

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

use statechronicle_executor::atomicity::{
    SettleLeg, TenantEventGroup, TradeManifest, ValueLeg, validate_cross_tenant_trade,
};

// `validate_cross_tenant_trade` is total over arbitrary cross-tenant batches:
// any arbitrary partition of valid events into per-tenant groups plus an
// arbitrary manifest and arbitrary settle intents must return a `Result` —
// never panic. A fixed pool of valid events drives the per-group and
// cross-tenant linkage checks; the byte stream decides which events land in
// which group, each group's declared tenant, the manifest's settle/value legs,
// and the settle intents, so every fail-closed path (partition mismatch,
// settle-leg mismatch, value-pair and settle-declaration mismatch, undeclared
// intent ids) is reachable.
fuzz_target!(|data: &[u8]| {
    let Some(pool) = build_pool() else {
        return;
    };
    let tenant_names = ["acme.game.alpha", "acme.game.beta", "acme.game.gamma"];
    let mut groups: Vec<TenantEventGroup> = Vec::new();
    for (group_index, name) in tenant_names.iter().enumerate() {
        let mut group_events = Vec::new();
        for (event_index, event) in pool.iter().enumerate() {
            let probe = (group_index * pool.len() + event_index) % data.len().max(1);
            if data.get(probe).copied().unwrap_or(0) % 2 == 1 {
                group_events.push(event.clone());
            }
        }
        groups.push(TenantEventGroup {
            tenant: TenantId(String::from(*name)),
            events: group_events,
        });
    }
    let manifest = manifest_from_bytes(data);
    let mut settle_intents: Vec<statechronicle_domain::intent::Intent> = Vec::new();
    for (intent_index, intent) in build_intents().iter().enumerate() {
        let probe = (pool.len() + intent_index) % data.len().max(1);
        if data.get(probe).copied().unwrap_or(0) % 2 == 1 {
            settle_intents.push(intent.clone());
        }
    }
    // Total: the validator never panics on arbitrary groups + manifest.
    let _ = validate_cross_tenant_trade(&groups, &manifest, &settle_intents);
});

/// Builds a manifest from the byte stream: settle legs over a fixed set of
/// (asset, intent id) pairs, plus optionally one declared value leg.
fn manifest_from_bytes(data: &[u8]) -> TradeManifest {
    let assets = ["asset:sword", "asset:shield", "asset:helm"];
    let mut settle_legs = Vec::new();
    for (index, asset) in assets.iter().enumerate() {
        let probe = index % data.len().max(1);
        if data.get(probe).copied().unwrap_or(0) % 2 == 1 {
            settle_legs.push(SettleLeg {
                asset: ResourceId(String::from(*asset)),
                settle_intent_id: IntentId::new(format!("int_{index:08}")).unwrap(),
            });
        }
    }
    let mut value_legs = Vec::new();
    if data.first().copied().unwrap_or(0) % 3 == 0 {
        value_legs.push(ValueLeg {
            resource: ResourceId(String::from("currency:gold")),
            amount: String::from("100"),
            to_subject: SubjectId(String::from("bob")),
        });
    }
    TradeManifest {
        trade_id: String::from("trade_001"),
        settle_legs,
        value_legs,
    }
}

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
fn build_intents() -> Vec<statechronicle_domain::intent::Intent> {
    let mut intents = Vec::new();
    for index in 0..8usize {
        let mut inputs = BTreeMap::new();
        inputs.insert(String::from("from_owner"), serde_json::json!("alice"));
        inputs.insert(String::from("to_owner"), serde_json::json!("bob"));
        intents.push(statechronicle_domain::intent::Intent::new(
            TenantId(String::from("acme.game.alpha")),
            IntentId::new(format!("int_{index:08}")).unwrap(),
            Operation::new(String::from("trade.settle")).unwrap(),
            SubjectId(String::from("account:example:player")),
            ResourceId(String::from("asset:sword")),
            Some(statechronicle_domain::state_type::StateType::UniqueAsset),
            1,
            inputs,
            None,
            DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
                .unwrap()
                .with_timezone(&Utc),
            None,
            statechronicle_domain::intent::Nonce::from_bytes(vec![1]).unwrap(),
        ));
    }
    intents
}
