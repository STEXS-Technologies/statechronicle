//! Run: `cargo run -p statechronicle --example trade_bundle`
//!
//! A BUNDLE trade settlement under the baseline profiles (Phase 4, protocol
//! §18.3): a trade that freezes N assets per side and settles them in ONE
//! atomic batch. Player A offers a sword + shield for player B's helm +
//! gauntlets (2-for-2). Each `trade.lock` freezes one `active` asset into a
//! pending trade (N locks run atomically via `execute_batch`), then all four
//! assets settle in one all-or-nothing `execute_settle`.
//!
//! The settle batch is four `trade.settle` intents, one per asset, each
//! DECLARING the bundle: a `bundle_size` input (the total 4 assets) and the
//! shared `trade_id`. The executor's `validate_settle_batch` (Phase 4) enforces
//! the bundle shape - every settle intent agrees on `bundle_size` and
//! `trade_id`, the settle-event count equals the declared bundle size, and no
//! asset appears twice - so the four owner changes land in ONE commit (4
//! events) with the transaction log `begin...commit`.
//!
//! Asserted: all four owners flip (A's two assets go to BOB, B's two assets go
//! to ALICE) in one commit of 4 events.

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

use statechronicle::domain::authority::AuthorityProof;
use statechronicle::domain::ids::IntentId;
use statechronicle::domain::intent::{Intent, Nonce, Operation};
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::state_type::StateType;
use statechronicle::domain::subject::SubjectId;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::ports::state_index::StateIndex;

use common::Harness;

const ALICE: &str = "account:example:player_123"; // offers sword + shield
const BOB: &str = "account:example:player_456"; // offers helm + gauntlets
const SWORD: &str = "asset:sword_001";
const SHIELD: &str = "asset:shield_001";
const HELM: &str = "asset:helm_001";
const GAUNTLETS: &str = "asset:gauntlets_001";
const TRADE: &str = "trade_bundle_001";
/// The total bundle size (2 assets per side = 4 assets across both sides).
const BUNDLE_SIZE: u64 = 4;

/// Builds a signed intent via `Intent::builder()` + `harness.sign`.
#[allow(clippy::too_many_arguments)]
fn signed(
    harness: &Harness,
    id: &str,
    op: &'static str,
    actor: &str,
    resource: &str,
    state_type: StateType,
    version: u64,
    inputs: &[(&str, serde_json::Value)],
    authority: Option<AuthorityProof>,
) -> ValidatedIntent {
    let mut b = Intent::builder()
        .tenant(harness.tenant())
        .intent_id(IntentId::new(format!("int_{id}")).unwrap())
        .operation(Operation::from_static(op))
        .actor(SubjectId(String::from(actor)))
        .resource(ResourceId(String::from(resource)))
        .state_type(state_type)
        .expected_version(version)
        .created_at(harness.now())
        .nonce(Nonce::from_bytes(vec![0]).unwrap());
    for (k, v) in inputs {
        b = b.input(k, v.clone());
    }
    harness.sign(b.build().unwrap(), authority)
}

/// Applies a returned settle event to the index as a unique asset.
async fn apply_settle_event(harness: &Harness, event: &statechronicle::domain::event::Event) {
    harness.index.apply(event, StateType::UniqueAsset);
}

/// Mints + locks the two assets held by `owner` into the trade in one atomic
/// lock batch per side (N-lock atomicity is free via `execute_batch`).
async fn mint_and_lock_pair(harness: &Harness, first: &str, second: &str, owner: &str, side: &str) {
    harness
        .run(
            &signed(
                harness,
                &format!("{side}_mint_1"),
                "asset.mint",
                owner,
                first,
                StateType::UniqueAsset,
                0,
                &[("to_owner", json!(owner))],
                None,
            ),
            StateType::UniqueAsset,
        )
        .await;
    harness
        .run(
            &signed(
                harness,
                &format!("{side}_mint_2"),
                "asset.mint",
                owner,
                second,
                StateType::UniqueAsset,
                0,
                &[("to_owner", json!(owner))],
                None,
            ),
            StateType::UniqueAsset,
        )
        .await;

    let locks = vec![
        signed(
            harness,
            &format!("{side}_lock_1"),
            "trade.lock",
            owner,
            first,
            StateType::UniqueAsset,
            1,
            &[("from_owner", json!(owner)), ("trade_id", json!(TRADE))],
            None,
        ),
        signed(
            harness,
            &format!("{side}_lock_2"),
            "trade.lock",
            owner,
            second,
            StateType::UniqueAsset,
            1,
            &[("from_owner", json!(owner)), ("trade_id", json!(TRADE))],
            None,
        ),
    ];
    let lock_events = harness.executor.execute_batch(&locks).await.unwrap();
    assert_eq!(lock_events.len(), 2);
    for event in &lock_events {
        harness.index.apply(event, StateType::UniqueAsset);
    }
    println!(
        "froze {side} assets {first}, {second} into trade {TRADE} (2 lock events, one atomic batch)"
    );
}

/// Builds one bundle-declaring `trade.settle` intent for `asset`, moving it
/// from `from_owner` to `to_owner`.
fn settle_intent(
    harness: &Harness,
    id: &str,
    asset: &str,
    from_owner: &str,
    to_owner: &str,
) -> ValidatedIntent {
    signed(
        harness,
        id,
        "trade.settle",
        from_owner,
        asset,
        StateType::UniqueAsset,
        2,
        &[
            ("from_owner", json!(from_owner)),
            ("to_owner", json!(to_owner)),
            ("trade_id", json!(TRADE)),
            ("bundle_size", json!(BUNDLE_SIZE)),
        ],
        Some(harness.authority()),
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let harness = Harness::new();

    println!("== trade_bundle: A's sword + shield for B's helm + gauntlets ==");

    // A mints sword + shield and locks both into the trade; B mints helm +
    // gauntlets and locks both into the trade. Each side's two locks commit
    // atomically as one batch.
    mint_and_lock_pair(&harness, SWORD, SHIELD, ALICE, "alice").await;
    mint_and_lock_pair(&harness, HELM, GAUNTLETS, BOB, "bob").await;

    // Build the four bundle-declaring settle intents, one per asset. All share
    // the same trade_id and declare the same bundle_size (4 assets total).
    let settles = vec![
        settle_intent(&harness, "tb_settle_sword", SWORD, ALICE, BOB),
        settle_intent(&harness, "tb_settle_shield", SHIELD, ALICE, BOB),
        settle_intent(&harness, "tb_settle_helm", HELM, BOB, ALICE),
        settle_intent(&harness, "tb_settle_gauntlets", GAUNTLETS, BOB, ALICE),
    ];

    // Settle all four in ONE atomic transaction: four trade.settle events in a
    // single commit, validated by the Phase 4 bundle shape check.
    let events = harness.executor.execute_settle(&settles).await.unwrap();
    assert_eq!(events.len(), 4);
    for event in &events {
        apply_settle_event(&harness, event).await;
    }
    println!("bundle settle -> 4 events in one commit");

    // Two atomic lock batches (one per side) followed by one atomic settle
    // batch: three begin...commit pairs. The settle is the last pair.
    let log = harness.transactions.log();
    assert_eq!(log.len(), 6);
    assert_eq!(log[0], format!("begin:{}", harness.tenant().0));
    assert_eq!(log[1], String::from("commit"));
    assert_eq!(log[2], format!("begin:{}", harness.tenant().0));
    assert_eq!(log[3], String::from("commit"));
    assert_eq!(log[4], format!("begin:{}", harness.tenant().0));
    assert_eq!(log[5], String::from("commit"));
    println!(
        "tx log: begin:{}, commit (lock A), begin:{}, commit (lock B), begin:{}, commit (bundle settle)",
        harness.tenant().0,
        harness.tenant().0,
        harness.tenant().0
    );

    // Assert all four owners flipped in the one commit.
    for (asset, expected_owner) in [
        (SWORD, BOB),
        (SHIELD, BOB),
        (HELM, ALICE),
        (GAUNTLETS, ALICE),
    ] {
        let held = harness
            .index
            .get_state(&harness.tenant(), &ResourceId(String::from(asset)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(held.state["owner"], json!(expected_owner));
        assert_eq!(held.state["status"], json!("active"));
    }
    println!("all four owners flipped to their new owners in one commit");

    println!("trade_bundle: OK");
}
