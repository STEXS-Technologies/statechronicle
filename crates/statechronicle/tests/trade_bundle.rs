//! Integration test: the BUNDLE trade settlement (Phase 4, protocol §18.3).
//!
//! Runs a `trade.lock` -> bundle `trade.settle` lifecycle through the REAL
//! cross-crate executor (in-memory port fakes). A trade bundles N assets per
//! side (player A offers sword + shield for player B's helm + gauntlets, 2-for-2):
//! each `trade.lock` freezes one asset into a pending trade (N locks commit
//! atomically via `execute_batch`), then all four settle in ONE atomic
//! `execute_settle` carrying four bundle-declaring `trade.settle` intents
//! (each declares the shared `trade_id` and `bundle_size`).
//!
//! Asserted: (a) a full 2-for-2 bundle settles in one commit with four owner
//! changes; (b) a bundle settle where one asset was never trade_held (its lock
//! was skipped) fails closed and rolls back with nothing escaping; (c) a
//! duplicate-asset bundle is rejected atomically.

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
use statechronicle::executor::error::ExecutorError;
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

/// Mints `asset` to `owner` and locks it into the trade (as its own lock
/// batch). Returns the lock outcome when `lock` is true, so a test can skip
/// locking one asset to build a settle that is missing a trade_held leg.
async fn mint_and_lock(harness: &Harness, id: &str, asset: &str, owner: &str, lock: bool) {
    harness
        .run(
            &signed(
                harness,
                &format!("{id}_mint"),
                "asset.mint",
                owner,
                asset,
                StateType::UniqueAsset,
                0,
                &[("to_owner", json!(owner))],
                None,
            ),
            StateType::UniqueAsset,
        )
        .await;
    if lock {
        harness
            .run(
                &signed(
                    harness,
                    &format!("{id}_lock"),
                    "trade.lock",
                    owner,
                    asset,
                    StateType::UniqueAsset,
                    1,
                    &[("from_owner", json!(owner)), ("trade_id", json!(TRADE))],
                    None,
                ),
                StateType::UniqueAsset,
            )
            .await;
    }
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

/// Seeds the full 2-for-2 bundle: A's sword + shield and B's helm + gauntlets,
/// all locked into the trade.
async fn seed_full_bundle(harness: &Harness) {
    mint_and_lock(harness, "sword", SWORD, ALICE, true).await;
    mint_and_lock(harness, "shield", SHIELD, ALICE, true).await;
    mint_and_lock(harness, "helm", HELM, BOB, true).await;
    mint_and_lock(harness, "gauntlets", GAUNTLETS, BOB, true).await;
}

/// The four bundle-declaring settle intents for the full 2-for-2 bundle.
fn full_settle_intents(harness: &Harness) -> Vec<ValidatedIntent> {
    vec![
        settle_intent(harness, "tb_settle_sword", SWORD, ALICE, BOB),
        settle_intent(harness, "tb_settle_shield", SHIELD, ALICE, BOB),
        settle_intent(harness, "tb_settle_helm", HELM, BOB, ALICE),
        settle_intent(harness, "tb_settle_gauntlets", GAUNTLETS, BOB, ALICE),
    ]
}

#[tokio::test]
async fn two_for_two_bundle_settles_in_one_commit() {
    let harness = Harness::new();
    seed_full_bundle(&harness).await;

    let events = harness
        .executor
        .execute_settle(&full_settle_intents(&harness))
        .await
        .unwrap();

    // Four trade.settle events in one commit.
    assert_eq!(events.len(), 4);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.operation.as_str() == "trade.settle")
            .count(),
        4
    );
    assert_eq!(
        harness.transactions.log(),
        vec![
            format!("begin:{}", harness.tenant().0),
            String::from("commit")
        ]
    );

    for event in &events {
        apply_settle_event(&harness, event).await;
    }

    // All four owners flipped in the one commit.
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
}

#[tokio::test]
async fn bundle_settle_with_unlocked_asset_fails_closed_and_rolls_back() {
    let harness = Harness::new();
    // Seed three of the four assets locked; HELM is minted but its lock is
    // SKIPPED, so it is still `active` (never trade_held).
    mint_and_lock(&harness, "sword", SWORD, ALICE, true).await;
    mint_and_lock(&harness, "shield", SHIELD, ALICE, true).await;
    mint_and_lock(&harness, "helm", HELM, BOB, false).await;
    mint_and_lock(&harness, "gauntlets", GAUNTLETS, BOB, true).await;

    let err = harness
        .executor
        .execute_settle(&full_settle_intents(&harness))
        .await
        .unwrap_err();
    assert!(matches!(err, ExecutorError::AtomicityViolation(_)));

    // Rolled back atomically: nothing escapes.
    assert_eq!(
        harness.transactions.log(),
        vec![
            format!("begin:{}", harness.tenant().0),
            String::from("rollback")
        ]
    );

    // The locked assets stay trade_held with their original owners; the
    // unlocked asset stays active and owned by BOB. No settle side effects.
    let sword = harness
        .index
        .get_state(&harness.tenant(), &ResourceId(String::from(SWORD)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sword.state["owner"], json!(ALICE));
    assert_eq!(sword.state["status"], json!("trade_held"));

    let helm = harness
        .index
        .get_state(&harness.tenant(), &ResourceId(String::from(HELM)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(helm.state["owner"], json!(BOB));
    assert_eq!(helm.state["status"], json!("active"));
}

#[tokio::test]
async fn duplicate_asset_bundle_rejected_atomically() {
    let harness = Harness::new();
    // A bundle that nominally settles two assets, but both settle intents
    // target the SAME asset (SWORD): a duplicate asset cannot settle atomically.
    mint_and_lock(&harness, "sword", SWORD, ALICE, true).await;
    mint_and_lock(&harness, "shield", SHIELD, ALICE, true).await;

    let duplicate = vec![
        settle_intent(&harness, "tb_dup_1", SWORD, ALICE, BOB),
        settle_intent(&harness, "tb_dup_2", SWORD, ALICE, BOB),
    ];
    let err = harness
        .executor
        .execute_settle(&duplicate)
        .await
        .unwrap_err();
    assert!(matches!(err, ExecutorError::AtomicityViolation(_)));

    // Rolled back atomically: nothing escapes.
    assert_eq!(
        harness.transactions.log(),
        vec![
            format!("begin:{}", harness.tenant().0),
            String::from("rollback")
        ]
    );

    let sword = harness
        .index
        .get_state(&harness.tenant(), &ResourceId(String::from(SWORD)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sword.state["owner"], json!(ALICE));
    assert_eq!(sword.state["status"], json!("trade_held"));
}
