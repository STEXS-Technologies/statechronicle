//! Run: `cargo run -p statechronicle --example inventory`
//!
//! A unique asset's full lifecycle under the baseline `unique_asset` profile:
//! mint → transfer → lock → unlock → restrict → restore → burn, with fail-closed
//! rejections for a non-owner transfer and the (base-profile-unknown)
//! `asset.hard_delete`. Ends with a signed commit + state accumulator.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::type_complexity
)]

mod common;

use statechronicle::domain::authority::AuthorityProof;
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

const ALICE: &str = "account:example:player_123";
const BOB: &str = "account:example:player_456";
const MALLORY: &str = "account:example:player_mallory";
const RESOURCE: &str = "asset:sword_001";

/// Builds a signed `asset.*` intent via `Intent::builder()` + `harness.sign`.
#[allow(clippy::too_many_arguments)]
fn signed(
    harness: &Harness,
    id: &str,
    op: &'static str,
    actor: &str,
    version: u64,
    inputs: &[(&str, serde_json::Value)],
    authority: Option<AuthorityProof>,
) -> ValidatedIntent {
    let mut b = Intent::builder()
        .tenant(harness.tenant())
        .intent_id(IntentId::new(format!("int_{id}")).unwrap())
        .operation(Operation::from_static(op))
        .actor(SubjectId(String::from(actor)))
        .resource(ResourceId(String::from(RESOURCE)))
        .state_type(StateType::UniqueAsset)
        .expected_version(version)
        .created_at(harness.now())
        .nonce(Nonce::from_bytes(vec![0]).unwrap());
    for (k, v) in inputs {
        b = b.input(k, v.clone());
    }
    harness.sign(b.build().unwrap(), authority)
}

async fn apply(harness: &Harness, events: &mut Vec<Event>, intent: &ValidatedIntent) {
    let event = harness.run(intent, StateType::UniqueAsset).await;
    events.push(event);
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let harness = Harness::new();
    let mut events: Vec<Event> = Vec::new();

    println!("== inventory: a unique asset under the baseline profile ==");
    println!("fixed tenant: {}", harness.tenant().0);

    // mint(ALICE): version 0 -> 1, owner ALICE, active.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "inv_mint",
            "asset.mint",
            ALICE,
            0,
            &[("to_owner", serde_json::json!(ALICE))],
            None,
        ),
    )
    .await;
    assert_eq!(
        events[0].after.state,
        serde_json::json!({ "owner": ALICE, "status": "active" })
    );
    println!("mint(ALICE)          -> {}", events[0].after.state);

    // Fail-closed: a non-owner cannot transfer the asset.
    let mallory_transfer = signed(
        &harness,
        "inv_bad_transfer",
        "asset.transfer",
        MALLORY,
        1,
        &[
            ("from_owner", serde_json::json!(MALLORY)),
            ("to_owner", serde_json::json!(BOB)),
        ],
        Some(harness.authority()),
    );
    let transfer_err = harness
        .executor
        .execute(&mallory_transfer)
        .await
        .unwrap_err();
    assert!(matches!(transfer_err, ExecutorError::ActorMismatch { .. }));
    println!("transfer(MALLORY->BOB) -> rejected (ActorMismatch)");

    // Fail-closed: the base profile does not know asset.hard_delete.
    let hard_delete = signed(
        &harness,
        "inv_hard_delete",
        "asset.hard_delete",
        ALICE,
        1,
        &[("actor", serde_json::json!(ALICE))],
        Some(harness.authority()),
    );
    let hard_delete_err = harness.executor.execute(&hard_delete).await.unwrap_err();
    assert!(matches!(
        hard_delete_err,
        ExecutorError::Profile(ProfileError::UnknownOperation(_))
    ));
    println!("hard_delete(ALICE)    -> rejected (UnknownOperation)");

    // transfer(ALICE -> BOB, authority): 1 -> 2, owner BOB.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "inv_transfer",
            "asset.transfer",
            ALICE,
            1,
            &[
                ("from_owner", serde_json::json!(ALICE)),
                ("to_owner", serde_json::json!(BOB)),
            ],
            Some(harness.authority()),
        ),
    )
    .await;
    println!("transfer(ALICE->BOB)  -> {}", events[1].after.state);

    // lock: 2 -> 3, locked.
    apply(
        &harness,
        &mut events,
        &signed(&harness, "inv_lock", "asset.lock", BOB, 2, &[], None),
    )
    .await;
    println!("lock(BOB)             -> {}", events[2].after.state);

    // unlock: 3 -> 4, active.
    apply(
        &harness,
        &mut events,
        &signed(&harness, "inv_unlock", "asset.unlock", BOB, 3, &[], None),
    )
    .await;
    println!("unlock(BOB)           -> {}", events[3].after.state);

    // restrict: 4 -> 5, restricted.
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "inv_restrict",
            "asset.restrict",
            BOB,
            4,
            &[("status", serde_json::json!("restricted"))],
            None,
        ),
    )
    .await;
    println!("restrict(BOB)         -> {}", events[4].after.state);

    // restore: 5 -> 6, active.
    apply(
        &harness,
        &mut events,
        &signed(&harness, "inv_restore", "asset.restore", BOB, 5, &[], None),
    )
    .await;
    println!("restore(BOB)          -> {}", events[5].after.state);

    // burn(BOB, authority): 6 -> 7, burned (terminal).
    apply(
        &harness,
        &mut events,
        &signed(
            &harness,
            "inv_burn",
            "asset.burn",
            BOB,
            6,
            &[("from_owner", serde_json::json!(BOB))],
            Some(harness.authority()),
        ),
    )
    .await;
    assert_eq!(
        events[6].after.state,
        serde_json::json!({ "owner": BOB, "status": "burned" })
    );
    println!("burn(BOB, authority)  -> {}", events[6].after.state);

    // Build the signed commit + accumulator over every emitted event.
    let (signed, accumulator) = harness.commit_events(&events);
    assert_eq!(signed.body.event_count as usize, events.len());
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

    println!("inventory: OK");
}
