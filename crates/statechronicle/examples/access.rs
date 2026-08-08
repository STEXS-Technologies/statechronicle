//! Run: `cargo run -p statechronicle --example access`
//!
//! Two subject-held lifecycles under the baseline profiles: an entitlement
//! (grant → activate → suspend → restore → revoke, with a fail-closed
//! `NotTransferable` because the grant is non-transferable) and a meter
//! (create → consume → refill → set_maximum → reset → expire).

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::type_complexity
)]

mod common;

use serde_json::json;

use statechronicle::domain::event::Event;
use statechronicle::domain::ids::IntentId;
use statechronicle::domain::intent::{Intent, Nonce, Operation};
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::state_type::StateType;
use statechronicle::domain::subject::SubjectId;
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::profiles::error::ProfileError;

use common::Harness;

const BOB: &str = "account:example:player_456";
const ENTITLEMENT: &str = "entitlement:membership";
const METER: &str = "meter:bandwidth";

/// Builds a signed subject-held intent via `Intent::builder()` + `harness.sign`.
#[allow(clippy::too_many_arguments)]
fn signed(
    harness: &Harness,
    id: &str,
    op: &'static str,
    resource: &str,
    state_type: StateType,
    version: u64,
    inputs: &[(&str, serde_json::Value)],
) -> ValidatedIntent {
    let mut b = Intent::builder()
        .tenant(harness.tenant())
        .intent_id(IntentId::new(format!("int_{id}")).unwrap())
        .operation(Operation::from_static(op))
        .actor(SubjectId(String::from(BOB)))
        .resource(ResourceId(String::from(resource)))
        .state_type(state_type)
        .expected_version(version)
        .created_at(harness.now())
        .nonce(Nonce::from_bytes(vec![0]).unwrap());
    for (k, v) in inputs {
        b = b.input(k, v.clone());
    }
    harness.sign(b.build().unwrap(), None)
}

async fn run_entitlement(harness: &Harness, events: &mut Vec<Event>, intent: &ValidatedIntent) {
    let event = harness.run(intent, StateType::Entitlement).await;
    events.push(event);
}

async fn run_meter(harness: &Harness, events: &mut Vec<Event>, intent: &ValidatedIntent) {
    let event = harness.run(intent, StateType::MeteredResource).await;
    events.push(event);
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let harness = Harness::new();
    let mut events: Vec<Event> = Vec::new();

    println!("== access: entitlement and meter lifecycles ==");

    // --- Entitlement: non-transferable by default. ---
    run_entitlement(
        &harness,
        &mut events,
        &signed(
            &harness,
            "acc_grant",
            "entitlement.grant",
            ENTITLEMENT,
            StateType::Entitlement,
            0,
            &[("subject", json!(BOB)), ("transferable", json!(false))],
        ),
    )
    .await;
    assert_eq!(events[0].after.state["status"], json!("granted"));
    println!("entitlement.grant(BOB)  -> {}", events[0].after.state);

    run_entitlement(
        &harness,
        &mut events,
        &signed(
            &harness,
            "acc_activate",
            "entitlement.activate",
            ENTITLEMENT,
            StateType::Entitlement,
            1,
            &[],
        ),
    )
    .await;
    assert_eq!(events[1].after.state["status"], json!("active"));
    println!("entitlement.activate    -> {}", events[1].after.state);

    run_entitlement(
        &harness,
        &mut events,
        &signed(
            &harness,
            "acc_suspend",
            "entitlement.suspend",
            ENTITLEMENT,
            StateType::Entitlement,
            2,
            &[],
        ),
    )
    .await;
    assert_eq!(events[2].after.state["status"], json!("suspended"));
    println!("entitlement.suspend     -> {}", events[2].after.state);

    run_entitlement(
        &harness,
        &mut events,
        &signed(
            &harness,
            "acc_restore",
            "entitlement.restore",
            ENTITLEMENT,
            StateType::Entitlement,
            3,
            &[],
        ),
    )
    .await;
    assert_eq!(events[3].after.state["status"], json!("active"));
    println!("entitlement.restore     -> {}", events[3].after.state);

    // Fail-closed: the grant is non-transferable, so a transfer is rejected.
    let transfer = signed(
        &harness,
        "acc_bad_transfer",
        "entitlement.transfer",
        ENTITLEMENT,
        StateType::Entitlement,
        4,
        &[("to_subject", json!("account:example:player_123"))],
    );
    let err = harness.executor.execute(&transfer).await.unwrap_err();
    assert!(matches!(
        err,
        ExecutorError::Profile(ProfileError::NotTransferable)
    ));
    println!("entitlement.transfer    -> rejected (NotTransferable)");

    run_entitlement(
        &harness,
        &mut events,
        &signed(
            &harness,
            "acc_revoke",
            "entitlement.revoke",
            ENTITLEMENT,
            StateType::Entitlement,
            4,
            &[],
        ),
    )
    .await;
    assert_eq!(events[4].after.state["status"], json!("revoked"));
    println!(
        "entitlement.revoke      -> {} (terminal)",
        events[4].after.state["status"]
    );

    // --- Meter: refill is deterministic; set_maximum clamps. ---
    run_meter(
        &harness,
        &mut events,
        &signed(
            &harness,
            "meter_create",
            "meter.create",
            METER,
            StateType::MeteredResource,
            0,
            &[
                ("subject", json!(BOB)),
                ("remaining", json!("40")),
                ("maximum", json!("100")),
            ],
        ),
    )
    .await;
    println!("meter.create(40/100)    -> {}", events[5].after.state);

    run_meter(
        &harness,
        &mut events,
        &signed(
            &harness,
            "meter_consume",
            "meter.consume",
            METER,
            StateType::MeteredResource,
            1,
            &[("amount", json!("5"))],
        ),
    )
    .await;
    assert_eq!(events[6].after.state["remaining"], json!("35"));
    println!("meter.consume(5)        -> {}", events[6].after.state);

    run_meter(
        &harness,
        &mut events,
        &signed(
            &harness,
            "meter_refill",
            "meter.refill",
            METER,
            StateType::MeteredResource,
            2,
            &[],
        ),
    )
    .await;
    assert_eq!(events[7].after.state["remaining"], json!("100"));
    assert_eq!(events[7].after.state["maximum"], json!("100"));
    println!(
        "meter.refill            -> {} (remaining == maximum)",
        events[7].after.state
    );

    // set_maximum(60) clamps remaining down to 60.
    run_meter(
        &harness,
        &mut events,
        &signed(
            &harness,
            "meter_max",
            "meter.set_maximum",
            METER,
            StateType::MeteredResource,
            3,
            &[("maximum", json!("60"))],
        ),
    )
    .await;
    assert_eq!(events[8].after.state["remaining"], json!("60"));
    assert_eq!(events[8].after.state["maximum"], json!("60"));
    println!(
        "meter.set_maximum(60)   -> {} (clamped)",
        events[8].after.state
    );

    run_meter(
        &harness,
        &mut events,
        &signed(
            &harness,
            "meter_reset",
            "meter.reset",
            METER,
            StateType::MeteredResource,
            4,
            &[],
        ),
    )
    .await;
    assert_eq!(events[9].after.state["remaining"], json!("0"));
    println!("meter.reset             -> {}", events[9].after.state);

    run_meter(
        &harness,
        &mut events,
        &signed(
            &harness,
            "meter_expire",
            "meter.expire",
            METER,
            StateType::MeteredResource,
            5,
            &[],
        ),
    )
    .await;
    println!(
        "meter.expire            -> {} (terminal)",
        events[10].after.state["remaining"]
    );

    // Build the signed commit + accumulator over every emitted event.
    let (signed, accumulator) = harness.commit_events(&events);
    println!(
        "commit {} signs {} events; state root {}",
        signed.body.commit_id.as_str(),
        events.len(),
        signed.body.next_state_root.as_str()
    );
    assert_eq!(
        accumulator.root().as_bytes(),
        signed.body.next_state_root.as_bytes()
    );

    println!("access: OK");
}
