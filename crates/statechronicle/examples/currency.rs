//! Run: `cargo run -p statechronicle --example currency`
//!
//! A fungible balance lifecycle under the baseline `fungible_balance` profile:
//! create → mint → credit → transfer (an atomic debit + credit pair) → reserve →
//! release → spend → convert → burn, with fail-closed rejections for an
//! over-debit (`InsufficientQuantity`) and a float amount (`FloatForbidden`).
//! One step goes through the raw-wire lane (`harness.accept`) to show the
//! `parse → validate → sign` path; the rest use the typed lane (`harness.sign`).

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
use statechronicle::domain::intent::{INTENT_SCHEMA, Intent, Nonce, Operation};
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::state_type::StateType;
use statechronicle::domain::subject::SubjectId;
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::profiles::error::ProfileError;

use common::Harness;

const ALICE: &str = "account:example:player_123";
const BOB: &str = "account:example:player_456";
const RESOURCE: &str = "currency:gold";
const UNIT: &str = "gold_minor";
const TREASURY: &str = "service:treasury";

/// Builds a signed `balance.*` typed intent via `Intent::builder()` + `harness.sign`.
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
        .state_type(StateType::FungibleBalance)
        .expected_version(version)
        .created_at(harness.now())
        .nonce(Nonce::from_bytes(vec![0]).unwrap());
    for (k, v) in inputs {
        b = b.input(k, v.clone());
    }
    harness.sign(b.build().unwrap(), None)
}

/// Builds a raw `balance.mint` payload for the raw-wire `accept` lane.
fn mint_payload() -> serde_json::Value {
    json!({
        "schema": INTENT_SCHEMA,
        "tenant_id": "acme.game.alpha",
        "intent_id": "int_cur_mint",
        "operation": "balance.mint",
        "actor": BOB,
        "resource_id": RESOURCE,
        "state_type": "fungible_balance",
        "expected_version": 1,
        "inputs": { "amount": "50", "authorized_by": TREASURY },
        "created_at": "2026-07-14T00:00:00Z",
        "expires_at": "2026-07-14T00:05:00Z",
        "nonce": "b64u:AAME",
    })
}

async fn apply(harness: &Harness, events: &mut Vec<Event>, intent: &ValidatedIntent) {
    let event = harness.run(intent, StateType::FungibleBalance).await;
    events.push(event);
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let harness = Harness::new();
    let mut events: Vec<Event> = Vec::new();

    println!("== currency: a fungible balance under the baseline profile ==");

    // balance.create(BOB, gold_minor): version 0 -> 1, balance 0.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "cur_create",
            "balance.create",
            0,
            &[
                ("subject", json!(BOB)),
                ("unit", json!(UNIT)),
                ("balance", json!("0")),
            ],
        ),
    )
    .await;
    assert_eq!(events[0].after.state["balance"], json!("0"));
    println!("balance.create(BOB)    -> {}", events[0].after.state);

    // Raw-path callout: this mint "arrived over the wire", so it is parsed +
    // validated + signed from a raw JSON payload (not built as typed data).
    let mint_payload = mint_payload();
    println!("raw payload over the wire -> {}", mint_payload);
    let mint = harness.accept(&mint_payload, None);
    apply(&harness, &mut events, &mint).await;
    assert_eq!(events[1].after.state["balance"], json!("50"));
    println!("mint(50, treasury)     -> {}", events[1].after.state);

    // credit(25): 50 -> 75.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "cur_credit",
            "balance.credit",
            2,
            &[("amount", json!("25"))],
        ),
    )
    .await;
    assert_eq!(events[2].after.state["balance"], json!("75"));
    println!("credit(25)             -> {}", events[2].after.state);

    // transfer(to_subject=ALICE, 30): an atomic debit + credit pair sharing one
    // intent id. The executor emits exactly two events.
    let transfer = signed(
        &harness,
        "cur_transfer",
        "balance.transfer",
        3,
        &[("to_subject", json!(ALICE)), ("amount", json!("30"))],
    );
    let pair = harness.executor.execute(&transfer).await.unwrap();
    assert_eq!(pair.len(), 2, "a transfer is a debit + credit pair");
    assert_eq!(pair[0].intent_id, pair[1].intent_id);
    for event in &pair {
        harness.index.apply(event, StateType::FungibleBalance);
    }
    // Source (BOB) debited 75 -> 45; destination (ALICE) created at 30.
    assert_eq!(pair[0].after.state["balance"], json!("45"));
    assert_eq!(pair[1].after.state["subject"], json!(ALICE));
    assert_eq!(pair[1].after.state["balance"], json!("30"));
    println!(
        "transfer(BOB->ALICE, 30) -> {} events, one intent id",
        pair.len()
    );
    events.push(pair[0].clone());

    // reserve(10): 45 -> 35.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "cur_reserve",
            "balance.reserve",
            4,
            &[("amount", json!("10"))],
        ),
    )
    .await;
    println!("reserve(10)            -> {}", events[4].after.state);

    // release(5): 35 -> 40.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "cur_release",
            "balance.release",
            5,
            &[("amount", json!("5"))],
        ),
    )
    .await;
    println!("release(5)             -> {}", events[5].after.state);

    // spend(20): 40 -> 20.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "cur_spend",
            "balance.spend",
            6,
            &[("amount", json!("20"))],
        ),
    )
    .await;
    println!("spend(20)              -> {}", events[6].after.state);

    // convert(to_unit=gold_major): 20 -> 5, unit changes.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "cur_convert",
            "balance.convert",
            7,
            &[("amount", json!("15")), ("to_unit", json!("gold_major"))],
        ),
    )
    .await;
    assert_eq!(events[7].after.state["unit"], json!("gold_major"));
    println!("convert(gold_major)     -> {}", events[7].after.state);

    // burn(5, authorized_by): 5 -> 0.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "cur_burn",
            "balance.burn",
            8,
            &[("amount", json!("5")), ("authorized_by", json!(TREASURY))],
        ),
    )
    .await;
    assert_eq!(events[8].after.state["balance"], json!("0"));
    println!("burn(5, treasury)      -> {}", events[8].after.state);

    // Amount math: canonical integer-string arithmetic (no floats).
    use statechronicle::Amount;
    let a = Amount::from_u64(1000);
    let b = Amount::from_u64(250);
    let sum = a.checked_add(b).unwrap();
    println!(
        "amount math: 1000 + 250 = {} (canonical, scale {})",
        sum.to_canonical_string(),
        sum.scale()
    );

    // Fail-closed: an over-debit is rejected with InsufficientQuantity.
    let over_debit = signed(
        &harness,
        "cur_bad_debit",
        "balance.debit",
        9,
        &[("amount", json!("999999"))],
    );
    let debit_err = harness.executor.execute(&over_debit).await.unwrap_err();
    assert!(matches!(
        debit_err,
        ExecutorError::Profile(ProfileError::InsufficientQuantity { .. })
    ));
    println!("debit(999999)          -> rejected (InsufficientQuantity)");

    // Fail-closed: a float amount is structurally rejected (protocol §10.3).
    let float_credit = signed(
        &harness,
        "cur_float",
        "balance.credit",
        9,
        &[("amount", json!("1.0"))],
    );
    let float_err = harness.executor.execute(&float_credit).await.unwrap_err();
    assert!(matches!(
        float_err,
        ExecutorError::Profile(ProfileError::FloatForbidden)
    ));
    println!("credit(1.0)            -> rejected (FloatForbidden)");

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

    println!("currency: OK");
}
