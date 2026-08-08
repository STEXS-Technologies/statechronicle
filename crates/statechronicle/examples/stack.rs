//! Run: `cargo run -p statechronicle --example stack`
//!
//! A consumable stack lifecycle under the baseline `consumable_stack` profile:
//! create → credit → consume → debit → reserve → release → adjust → expire, with
//! a fail-closed over-consume (`InsufficientQuantity`).

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
const RESOURCE: &str = "stack:arrows";
const UNIT: &str = "arrows";

/// Builds a signed `stack.*` intent via `Intent::builder()` + `harness.sign`.
#[allow(clippy::too_many_arguments)]
fn signed(
    harness: &Harness,
    id: &str,
    op: &'static str,
    version: u64,
    inputs: &[(&str, serde_json::Value)],
) -> ValidatedIntent {
    let mut b = Intent::builder()
        .tenant(harness.tenant())
        .intent_id(IntentId::new(format!("int_{id}")).unwrap())
        .operation(Operation::from_static(op))
        .actor(SubjectId(String::from(BOB)))
        .resource(ResourceId(String::from(RESOURCE)))
        .state_type(StateType::ConsumableStack)
        .expected_version(version)
        .created_at(harness.now())
        .nonce(Nonce::from_bytes(vec![0]).unwrap());
    for (k, v) in inputs {
        b = b.input(k, v.clone());
    }
    harness.sign(b.build().unwrap(), None)
}

async fn apply(harness: &Harness, events: &mut Vec<Event>, intent: &ValidatedIntent) {
    let event = harness.run(intent, StateType::ConsumableStack).await;
    events.push(event);
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let harness = Harness::new();
    let mut events: Vec<Event> = Vec::new();

    println!("== stack: a consumable stack under the baseline profile ==");

    // stack.create(BOB, 10, arrows): version 0 -> 1.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "stk_create",
            "stack.create",
            0,
            &[
                ("subject", json!(BOB)),
                ("quantity", json!("10")),
                ("unit", json!(UNIT)),
            ],
        ),
    )
    .await;
    println!("stack.create(BOB, 10)  -> {}", events[0].after.state);

    // credit(5): 10 -> 15.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "stk_credit",
            "stack.credit",
            1,
            &[("quantity", json!("5"))],
        ),
    )
    .await;
    println!("credit(5)              -> {}", events[1].after.state);

    // consume(3): 15 -> 12.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "stk_consume",
            "stack.consume",
            2,
            &[("quantity", json!("3"))],
        ),
    )
    .await;
    println!("consume(3)             -> {}", events[2].after.state);

    // debit(2): 12 -> 10.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "stk_debit",
            "stack.debit",
            3,
            &[("quantity", json!("2"))],
        ),
    )
    .await;
    println!("debit(2)               -> {}", events[3].after.state);

    // reserve(2): 10 -> 8.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "stk_reserve",
            "stack.reserve",
            4,
            &[("quantity", json!("2"))],
        ),
    )
    .await;
    println!("reserve(2)             -> {}", events[4].after.state);

    // release(1): 8 -> 9.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "stk_release",
            "stack.release",
            5,
            &[("quantity", json!("1"))],
        ),
    )
    .await;
    println!("release(1)             -> {}", events[5].after.state);

    // Fail-closed: consuming more than available is rejected.
    let over = signed(
        &harness,
        "stk_bad_consume",
        "stack.consume",
        6,
        &[("quantity", json!("99"))],
    );
    let err = harness.executor.execute(&over).await.unwrap_err();
    assert!(matches!(
        err,
        ExecutorError::Profile(ProfileError::InsufficientQuantity { .. })
    ));
    println!("consume(99)            -> rejected (InsufficientQuantity)");

    // adjust(0): 9 -> 0.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "stk_adjust",
            "stack.adjust",
            6,
            &[("quantity", json!("0"))],
        ),
    )
    .await;
    assert_eq!(events[6].after.state["quantity"], json!("0"));
    println!("adjust(0)              -> {}", events[6].after.state);

    // expire: terminal, quantity set to 0.
    apply(
        &harness,
        &mut events,
        &signed(&harness, "stk_expire", "stack.expire", 7, &[]),
    )
    .await;
    println!("expire                 -> {}", events[7].after.state);

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

    println!("stack: OK");
}
