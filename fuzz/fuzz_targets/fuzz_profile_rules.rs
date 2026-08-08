#![no_main]

use std::collections::BTreeMap;

use libfuzzer_sys::fuzz_target;

use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::ids::{CommitId, EventId};
use statechronicle_domain::intent::Operation;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::status::Status;
use statechronicle_domain::tenant::TenantId;

use statechronicle_profiles::registry::ProfileRegistry;

// The profile rule gate is total over arbitrary bytes: every registered rule
// set must return a `Result` — never panic — for any operation name and any
// JSON payload derived from the input.
fuzz_target!(|data: &[u8]| {
    // Split the input in half: the operation-name prefix and the JSON payload
    // suffix. Both halves are arbitrary; the payload half may not be JSON at
    // all, in which case there is nothing to run through the gate.
    let midpoint = data.len() / 2;
    let (name_bytes, payload_bytes) = data.split_at(midpoint);

    // The status newtype parser must be total over arbitrary bytes: any string
    // either validates or is rejected, never panics, and round-trips when Ok.
    if let Ok(status) = Status::try_from_str(&String::from_utf8_lossy(name_bytes)) {
        assert!(matches!(
            Status::try_from_str(status.as_str()),
            Ok(ref r) if r == &status
        ));
    }

    // The operation newtype is constructed via its validated accessor, failing
    // closed (skipped) on names the profile gate would never see.
    let Ok(operation) = Operation::new(String::from_utf8_lossy(name_bytes).into_owned()) else {
        return;
    };
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(payload_bytes) else {
        return;
    };

    let Some(current) = projection(payload) else {
        return;
    };
    let inputs = BTreeMap::new();

    let registry = ProfileRegistry::baseline();
    for state_type in [
        StateType::UniqueAsset,
        StateType::ConsumableStack,
        StateType::FungibleBalance,
        StateType::Entitlement,
        StateType::MeteredResource,
        StateType::Listing,
        StateType::Escrow,
    ] {
        if let Some(rules) = registry.get(state_type) {
            let _ = rules.check(&operation, Some(&current), &inputs);
            let _ = rules.check(&operation, None, &inputs);
        }
    }
    let _ = registry
        .paid_unique_asset()
        .check(&operation, Some(&current), &inputs);
    let _ = registry
        .paid_unique_asset()
        .check(&operation, None, &inputs);
});

/// Builds a projection over the arbitrary payload with fixed stable ids.
fn projection(state: serde_json::Value) -> Option<StateProjection> {
    let event_id = EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).ok()?;
    let commit_id = CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).ok()?;
    Some(StateProjection {
        tenant_id: TenantId(String::from("tenant.fuzz")),
        resource_id: ResourceId(String::from("res:fuzz_001")),
        state_type: StateType::UniqueAsset,
        version: 1,
        last_event_id: event_id,
        last_commit_id: commit_id,
        state_hash: ContentDigest::new([0u8; 32]),
        state,
    })
}
