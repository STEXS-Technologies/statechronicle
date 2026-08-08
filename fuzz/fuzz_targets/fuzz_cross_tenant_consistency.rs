#![no_main]

use chrono::{DateTime, Utc};

use libfuzzer_sys::fuzz_target;

use statechronicle_core::digest::hash_bytes;
use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::{EventId, IntentId};
use statechronicle_domain::intent::Operation;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_executor::atomicity::{TenantEventGroup, validate_cross_tenant_consistency};

// `validate_cross_tenant_consistency` is total over arbitrary tenant/group
// structures: any partition of valid events into groups (with arbitrary
// declared tenants and intent-sharing patterns) must return a `Result` — never
// panic. A fixed pool of valid events drives the per-group batch-consistency
// and cross-tenant linkage checks; the byte stream decides which events land in
// which group and each group's declared tenant, so every fail-closed path
// (partition mismatch, missing linkage, per-tenant transfer-pair rule) is
// reachable.
fuzz_target!(|data: &[u8]| {
    let Some(pool) = build_pool() else {
        return;
    };
    let mut groups: Vec<TenantEventGroup> = Vec::new();
    let tenant_names = ["acme.game.alpha", "acme.game.beta"];
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
    // Total: the validator never panics on arbitrary groups.
    let _ = validate_cross_tenant_consistency(&groups);
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
            Operation::new(String::from("asset.transfer")).ok()?,
            ResourceId(String::from("asset:sword")),
            SubjectId(String::from("account:example:player")),
            StateCommitment {
                version: 1,
                state_hash: hash_bytes(b"before"),
                state: serde_json::json!({}),
            },
            StateCommitment {
                version: 2,
                state_hash: hash_bytes(b"after"),
                state: serde_json::json!({ "owner": "bob", "status": "active" }),
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
