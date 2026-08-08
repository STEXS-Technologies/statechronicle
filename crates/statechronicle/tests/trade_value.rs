//! Integration test: the full value-leg settlement (Phase 2, protocol §18.3).
//!
//! Runs a `trade.lock` -> value-leg `trade.settle` lifecycle through the REAL
//! cross-crate executor (in-memory port fakes): an asset is frozen into a
//! pending trade, then settled for a fungible value leg (asset-for-gold) in ONE
//! atomic `execute_settle` transaction. Asserts the settle batch grows to three
//! events (one `trade.settle` + one net-zero `balance.transfer` debit+credit
//! pair) and that the owner, buyer balance, and seller balance all move in a
//! single all-or-nothing commit. A mismatched value amount rolls back
//! atomically.

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

use statechronicle::domain::ids::IntentId;
use statechronicle::domain::intent::{Intent, Nonce, Operation};
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::state_type::StateType;
use statechronicle::domain::subject::SubjectId;
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::ports::state_index::StateIndex;

use common::Harness;

const ALICE: &str = "account:example:player_123"; // seller
const BOB: &str = "account:example:player_456"; // buyer
const ASSET: &str = "asset:relic_001";
const WALLET: &str = "wallet:gold";
const TRADE: &str = "trade_001";
const PRICE: u64 = 100;

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
    authority: Option<statechronicle::domain::authority::AuthorityProof>,
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

/// Seeds the buyer's wallet, mints + locks the asset, and runs the value-leg
/// settle with the given declared/transfer amounts, returning the outcome.
async fn settle_for(
    harness: &Harness,
    declared_amount: u64,
    transfer_amount: u64,
) -> Result<Vec<statechronicle::domain::event::Event>, ExecutorError> {
    harness
        .run(
            &signed(
                harness,
                "tv_bob_wallet",
                "balance.create",
                BOB,
                WALLET,
                StateType::FungibleBalance,
                0,
                &[
                    ("subject", json!(BOB)),
                    ("unit", json!("gold_minor")),
                    ("balance", json!("1000")),
                ],
                None,
            ),
            StateType::FungibleBalance,
        )
        .await;
    harness
        .run(
            &signed(
                harness,
                "tv_mint",
                "asset.mint",
                ALICE,
                ASSET,
                StateType::UniqueAsset,
                0,
                &[("to_owner", json!(ALICE))],
                None,
            ),
            StateType::UniqueAsset,
        )
        .await;
    harness
        .run(
            &signed(
                harness,
                "tv_lock",
                "trade.lock",
                ALICE,
                ASSET,
                StateType::UniqueAsset,
                1,
                &[("from_owner", json!(ALICE)), ("trade_id", json!(TRADE))],
                None,
            ),
            StateType::UniqueAsset,
        )
        .await;

    let settle = signed(
        harness,
        "tv_settle",
        "trade.settle",
        ALICE,
        ASSET,
        StateType::UniqueAsset,
        2,
        &[
            ("from_owner", json!(ALICE)),
            ("to_owner", json!(BOB)),
            ("trade_id", json!(TRADE)),
            ("value_resource", json!(WALLET)),
            ("value_amount", json!(declared_amount.to_string())),
            ("value_to_subject", json!(ALICE)),
        ],
        Some(harness.authority()),
    );
    let value_leg = signed(
        harness,
        "tv_value",
        "balance.transfer",
        BOB,
        WALLET,
        StateType::FungibleBalance,
        1,
        &[
            ("to_subject", json!(ALICE)),
            ("amount", json!(transfer_amount.to_string())),
        ],
        None,
    );
    harness.executor.execute_settle(&[settle, value_leg]).await
}

#[tokio::test]
async fn value_leg_settle_moves_asset_and_balance_in_one_commit() {
    let harness = Harness::new();
    let wallet = ResourceId(String::from(WALLET));
    let asset = ResourceId(String::from(ASSET));

    let events = settle_for(&harness, PRICE, PRICE).await.unwrap();

    // 3 events: 1 trade.settle + 2 balance.transfer (debit + credit).
    assert_eq!(events.len(), 3);
    let settle_events = events
        .iter()
        .filter(|e| e.operation.as_str() == "trade.settle")
        .count();
    let transfer_events = events
        .iter()
        .filter(|e| e.operation.as_str() == "balance.transfer")
        .count();
    assert_eq!(settle_events, 1);
    assert_eq!(transfer_events, 2);
    // The transfer pair shares one value-leg intent id, distinct from settle.
    let transfer_ids: std::collections::BTreeSet<&String> = events
        .iter()
        .filter(|e| e.operation.as_str() == "balance.transfer")
        .map(|e| &e.intent_id.0)
        .collect();
    assert_eq!(transfer_ids.len(), 1);

    for event in &events {
        let state_type = if event.operation.as_str() == "trade.settle" {
            StateType::UniqueAsset
        } else {
            StateType::FungibleBalance
        };
        harness.index.apply(event, state_type);
    }

    // Asset owner changed to the buyer, status active (trade dropped).
    let held = harness
        .index
        .get_state(&harness.tenant(), &asset)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.state["owner"], json!(BOB));
    assert_eq!(held.state["status"], json!("active"));

    // Buyer debited by the amount; seller credited by the amount.
    let bob_wallet = harness
        .index
        .get_subject_state(&harness.tenant(), &SubjectId(String::from(BOB)), &wallet)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bob_wallet.state["balance"],
        json!((1000 - PRICE).to_string())
    );
    let alice_wallet = harness
        .index
        .get_subject_state(&harness.tenant(), &SubjectId(String::from(ALICE)), &wallet)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(alice_wallet.state["balance"], json!(PRICE.to_string()));
}

#[tokio::test]
async fn value_leg_mismatched_amount_rolls_back_atomically() {
    let harness = Harness::new();

    // Declare 100 in the settle but transfer only 50 -> the shape check rejects
    // the batch and rolls it back atomically (nothing escapes).
    let result = settle_for(&harness, PRICE, 50).await;
    assert!(matches!(result, Err(ExecutorError::AtomicityViolation(_))));
}

#[tokio::test]
async fn value_leg_settle_batch_shape_is_fail_closed() {
    let harness = Harness::new();
    // Seed the buyer wallet and mint + lock the asset directly.
    harness
        .run(
            &signed(
                &harness,
                "tv_bob_wallet",
                "balance.create",
                BOB,
                WALLET,
                StateType::FungibleBalance,
                0,
                &[
                    ("subject", json!(BOB)),
                    ("unit", json!("gold_minor")),
                    ("balance", json!("1000")),
                ],
                None,
            ),
            StateType::FungibleBalance,
        )
        .await;
    harness
        .run(
            &signed(
                &harness,
                "tv_mint",
                "asset.mint",
                ALICE,
                ASSET,
                StateType::UniqueAsset,
                0,
                &[("to_owner", json!(ALICE))],
                None,
            ),
            StateType::UniqueAsset,
        )
        .await;
    harness
        .run(
            &signed(
                &harness,
                "tv_lock",
                "trade.lock",
                ALICE,
                ASSET,
                StateType::UniqueAsset,
                1,
                &[("from_owner", json!(ALICE)), ("trade_id", json!(TRADE))],
                None,
            ),
            StateType::UniqueAsset,
        )
        .await;

    // A settle that declares a value leg but omits the transfer leg fails
    // closed and rolls back: the asset stays trade_held and the buyer is not
    // debited.
    let settle = signed(
        &harness,
        "tv_settle",
        "trade.settle",
        ALICE,
        ASSET,
        StateType::UniqueAsset,
        2,
        &[
            ("from_owner", json!(ALICE)),
            ("to_owner", json!(BOB)),
            ("trade_id", json!(TRADE)),
            ("value_resource", json!(WALLET)),
            ("value_amount", json!(PRICE.to_string())),
            ("value_to_subject", json!(ALICE)),
        ],
        Some(harness.authority()),
    );
    let err = harness
        .executor
        .execute_settle(&[settle])
        .await
        .unwrap_err();
    assert!(matches!(err, ExecutorError::AtomicityViolation(_)));

    // Nothing escaped: the asset is still owned by ALICE and still trade_held.
    let held = harness
        .index
        .get_state(&harness.tenant(), &ResourceId(String::from(ASSET)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.state["owner"], json!(ALICE));
    assert_eq!(held.state["status"], json!("trade_held"));
}
