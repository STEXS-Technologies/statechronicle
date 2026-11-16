//! Explicit reference producer; only execute against the pinned original release.
#[path = "../tests/common/mod.rs"]
mod common;

use common::{Harness, tenant};
use serde_json::json;
use statechronicle::accumulator::key::StateKey;
use statechronicle::core::canonicalize::canonicalize;
use statechronicle::proof::bundle::{build_non_membership_proof, build_state_proof};
use statechronicle::{ResourceId, StateProjection, StateType};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let harness = Harness::new();
    let mut events = Vec::new();
    for (id, operation, actor, version, inputs) in [
        ("int_wire_mint", "asset.mint", "account:alice", 0,
         json!({"to_owner": "account:alice"})),
        ("int_wire_transfer", "asset.transfer", "account:alice", 1,
         json!({"from_owner": "account:alice", "to_owner": "account:bob"})),
    ] {
        let payload = json!({
            "schema": "statechronicle.intent.v0", "tenant_id": "acme.game.alpha",
            "intent_id": id, "operation": operation, "actor": actor,
            "resource_id": "asset:wire", "state_type": "unique_asset",
            "expected_version": version, "inputs": inputs,
            "created_at": "2026-07-14T00:00:00Z", "nonce": "b64u:AAME"
        });
        let authority = (version > 0).then(|| harness.authority());
        let intent = harness.accept(&payload, authority);
        events.push(harness.run(&intent, StateType::UniqueAsset).await);
    }
    let (signed, accumulator) = harness.commit_events(&events);
    let event = events.last().unwrap();
    let projection = StateProjection {
        tenant_id: tenant(), resource_id: ResourceId("asset:wire".into()),
        state_type: StateType::UniqueAsset, version: event.after.version,
        last_event_id: event.event_id.clone(), last_commit_id: signed.body.commit_id.clone(),
        state_hash: event.after.state_hash.clone(), state: event.after.state.clone(),
    };
    let key = StateKey::for_resource(&tenant().0, "asset:wire");
    let inclusion = accumulator.prove_inclusion(&key).unwrap();
    let proof = build_state_proof(&projection, &signed, &inclusion,
                                &event.operation, None, key).unwrap();
    let absent = StateKey::for_resource(&tenant().0, "asset:absent");
    let absence = build_non_membership_proof(&tenant(), &ResourceId("asset:absent".into()),
        &absent, &signed, &accumulator.prove_non_membership(&absent).unwrap()).unwrap();
    let corpus = json!({
        "producer_commit": "c418e98b91720188158c531cd5c2c4b8ab71cbcf",
        "events": events, "signed_commit": signed, "projection": projection,
        "state_proof": proof, "absence_proof": absence,
        "commit_signing_bytes": canonicalize(&signed.body).unwrap(),
        "event_bytes": canonicalize(&events).unwrap(),
        "state_proof_bytes": canonicalize(&proof).unwrap(),
        "absence_proof_bytes": canonicalize(&absence).unwrap()
    });
    let destination = std::env::args().nth(1).expect("explicit output path required");
    std::fs::write(destination, serde_json::to_string_pretty(&corpus).unwrap() + "\n").unwrap();
}
