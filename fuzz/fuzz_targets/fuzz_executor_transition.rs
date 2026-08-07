#![no_main]

use std::collections::BTreeMap;

use libfuzzer_sys::fuzz_target;

use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::ids::{CommitId, EventId};
use statechronicle_domain::intent::Operation;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::tenant::TenantId;

use statechronicle_executor::transition::{apply, transfer_after_state};

// The transition functions are total over arbitrary bytes: any operation name
// and any JSON before-state/inputs payload derived from the input must return
// a `Result` — never panic. The state type of the before-state is derived from
// the operation prefix so each state type's arithmetic is exercised with
// arbitrary payload fields (quantities may be floats, huge integers, garbage,
// or missing entirely — all must fail closed). `transfer_after_state` (the
// destination credit) is exercised alongside `apply` (the source debit) so the
// atomic transfer pair (§20.5) is fuzzed on both halves.
fuzz_target!(|data: &[u8]| {
    let third = data.len() / 3;
    let (name_bytes, rest) = data.split_at(third);
    let (state_bytes, inputs_bytes) = rest.split_at(rest.len() / 2);

    let operation = Operation(String::from_utf8_lossy(name_bytes).into_owned());
    let Ok(state) = serde_json::from_slice::<serde_json::Value>(state_bytes) else {
        return;
    };
    let Ok(inputs) = serde_json::from_slice::<serde_json::Value>(inputs_bytes) else {
        return;
    };
    let inputs: BTreeMap<String, serde_json::Value> = match inputs {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        _ => BTreeMap::new(),
    };

    let Some(before) = projection(state, state_type_for(&operation)) else {
        return;
    };
    let _ = apply(Some(&before), &operation, &inputs);
    let _ = apply(None, &operation, &inputs);
    let _ = transfer_after_state(&before, Some(&before), &operation, &inputs);
    let _ = transfer_after_state(&before, None, &operation, &inputs);
});

/// Derives the state type from the operation prefix, matching the executor's
/// create-time inference.
fn state_type_for(operation: &Operation) -> StateType {
    let name = operation.as_str();
    if name.starts_with("asset.") {
        StateType::UniqueAsset
    } else if name.starts_with("stack.") {
        StateType::ConsumableStack
    } else if name.starts_with("balance.") {
        StateType::FungibleBalance
    } else if name.starts_with("entitlement.") {
        StateType::Entitlement
    } else if name.starts_with("meter.") {
        StateType::MeteredResource
    } else if name.starts_with("listing.") {
        StateType::Listing
    } else if name.starts_with("escrow.") {
        StateType::Escrow
    } else {
        StateType::UniqueAsset
    }
}

/// Builds a projection over the arbitrary payload with fixed stable ids.
fn projection(state: serde_json::Value, state_type: StateType) -> Option<StateProjection> {
    let event_id = EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).ok()?;
    let commit_id = CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).ok()?;
    Some(StateProjection {
        tenant_id: TenantId(String::from("tenant.fuzz")),
        resource_id: ResourceId(String::from("res:fuzz_001")),
        state_type,
        version: 1,
        last_event_id: event_id,
        last_commit_id: commit_id,
        state_hash: ContentDigest::new([0u8; 32]),
        state,
    })
}
