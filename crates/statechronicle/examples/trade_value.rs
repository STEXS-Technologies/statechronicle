//! Run: `cargo run -p statechronicle --example trade_value`
//!
//! A value-leg settlement under the baseline profiles (Phase 2, protocol §18.3):
//! freeze an asset into a pending trade with `trade.lock`, then settle it for a
//! fungible value leg (asset-for-gold) in one all-or-nothing `execute_settle`:
//! a `trade.settle` intent declaring the value leg plus one `balance.transfer`
//! intent (the value leg), distinct intent ids. The settle batch therefore
//! grows from `[trade.settle]` to `[trade.settle, balance.transfer x2]` - one
//! settle event and one atomic debit + credit pair, all in ONE atomic
//! transaction.
//!
//! Asserted in one commit: the asset's owner changes to the buyer, the buyer's
//! balance is debited by the value amount, and the seller's balance is credited
//! by the value amount.

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
use statechronicle::domain::tenant::TenantId;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::ports::state_index::StateIndex;

use common::Harness;

const ALICE: &str = "account:example:player_123"; // seller
const BOB: &str = "account:example:player_456"; // buyer
const ASSET: &str = "asset:relic_001";
const WALLET: &str = "wallet:gold";
const TRADE: &str = "trade_001";
const PRICE: u64 = 100;

/// Builds a signed intent via `Intent::builder()` + `harness.sign`.
#[allow(clippy::too_many_arguments)]
fn signed(
    harness: &Harness,
    tenant: TenantId,
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
        .tenant(tenant)
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

/// Applies a returned event to the index, choosing the state type by operation.
async fn apply_batch_event(harness: &Harness, event: &statechronicle::domain::event::Event) {
    let state_type = match event.operation.as_str() {
        "trade.settle" => StateType::UniqueAsset,
        _ => StateType::FungibleBalance,
    };
    harness.index.apply(event, state_type);
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let harness = Harness::new();
    let wallet = ResourceId(String::from(WALLET));
    let asset = ResourceId(String::from(ASSET));

    println!("== trade_value: asset-for-gold settlement ==");

    // Seed the buyer's wallet (the value-leg source balance MUST exist before
    // the trade). The seller's wallet is created on credit by the transfer.
    harness
        .run(
            &signed(
                &harness,
                harness.tenant(),
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

    // Mint the asset to the seller, then freeze it into a pending trade.
    harness
        .run(
            &signed(
                &harness,
                harness.tenant(),
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
                harness.tenant(),
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
    println!(
        "seeded buyer wallet, minted asset {}, froze into trade {}",
        ASSET, TRADE
    );

    // Build the value-leg settle batch: one `trade.settle` intent (declaring
    // the value leg) + one `balance.transfer` intent (the value leg), distinct
    // intent ids. The settle intent is authority-required (protocol §11.2), so
    // it binds an authority proof.
    let settle = signed(
        &harness,
        harness.tenant(),
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
    let value_leg = signed(
        &harness,
        harness.tenant(),
        "tv_value",
        "balance.transfer",
        BOB,
        WALLET,
        StateType::FungibleBalance,
        1,
        &[
            ("to_subject", json!(ALICE)),
            ("amount", json!(PRICE.to_string())),
        ],
        None,
    );

    // Settle in ONE atomic transaction: the asset changes hands and the gold
    // value leg moves in the same commit. 3 events: 1 trade.settle + 2
    // balance.transfer (debit + credit).
    let events = harness
        .executor
        .execute_settle(&[settle, value_leg])
        .await
        .unwrap();
    assert_eq!(events.len(), 3);
    for event in &events {
        apply_batch_event(&harness, event).await;
    }
    println!("value-leg settle -> 3 events in one commit");

    // The asset is now owned by the buyer.
    let held = harness
        .index
        .get_state(&harness.tenant(), &asset)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.state["owner"], json!(BOB));
    assert_eq!(held.state["status"], json!("active"));

    // The buyer is debited; the seller is credited (create-on-credit).
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
    println!(
        "asset owned by BOB; buyer debited to {}; seller credited to {}",
        bob_wallet.state["balance"], alice_wallet.state["balance"]
    );

    println!("trade_value: OK");
}
