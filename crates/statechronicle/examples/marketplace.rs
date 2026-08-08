//! Run: `cargo run -p statechronicle --example marketplace`
//!
//! An atomic purchase settlement under the baseline profiles (protocol §20.9):
//! seed buyer + seller wallets, mint an asset, create a listing and lock an
//! escrow, then settle ownership and payment in one all-or-nothing
//! `execute_batch`: `listing.buy`, `escrow.release`, `asset.transfer`, buyer
//! debit, and seller credit. A stale expected-version batch rolls back
//! atomically (nothing escapes).

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
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::state_type::StateType;
use statechronicle::domain::subject::SubjectId;
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::ports::state_index::StateIndex;

use common::{Harness, run, sample_authority, tenant, typed_intent};

const ALICE: &str = "account:example:player_123"; // seller
const BOB: &str = "account:example:player_456"; // buyer
const ASSET: &str = "asset:sword_001";
const LISTING: &str = "listing:001";
const ESCROW: &str = "escrow:001";
const WALLET: &str = "wallet:gold";
const PRICE: u64 = 100;

/// Applies a returned event to the index, choosing the state type by operation.
async fn apply_batch_event(harness: &Harness, event: &Event) {
    let state_type = if event.operation.as_str().starts_with("asset.") {
        StateType::UniqueAsset
    } else if event.operation.as_str().starts_with("listing.") {
        StateType::Listing
    } else if event.operation.as_str().starts_with("escrow.") {
        StateType::Escrow
    } else {
        StateType::FungibleBalance
    };
    harness.index.apply(event, state_type);
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let harness = Harness::new();
    let wallet = ResourceId(String::from(WALLET));

    println!("== marketplace: atomic purchase settlement ==");

    // Seed the buyer's and seller's wallets, mint the asset, create a listing,
    // and lock an escrow: all single transitions.
    run(
        &harness,
        &typed_intent(
            tenant(),
            "mkt_bob_wallet",
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
    run(
        &harness,
        &typed_intent(
            tenant(),
            "mkt_alice_wallet",
            "balance.create",
            ALICE,
            WALLET,
            StateType::FungibleBalance,
            0,
            &[
                ("subject", json!(ALICE)),
                ("unit", json!("gold_minor")),
                ("balance", json!("500")),
            ],
            None,
        ),
        StateType::FungibleBalance,
    )
    .await;
    run(
        &harness,
        &typed_intent(
            tenant(),
            "mkt_mint",
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
    run(
        &harness,
        &typed_intent(
            tenant(),
            "mkt_list",
            "listing.create",
            ALICE,
            LISTING,
            StateType::Listing,
            0,
            &[("seller", json!(ALICE))],
            None,
        ),
        StateType::Listing,
    )
    .await;
    run(
        &harness,
        &typed_intent(
            tenant(),
            "mkt_escrow",
            "escrow.lock",
            BOB,
            ESCROW,
            StateType::Escrow,
            0,
            &[("buyer", json!(BOB)), ("seller", json!(ALICE))],
            None,
        ),
        StateType::Escrow,
    )
    .await;
    println!(
        "seeded wallets, minted asset, created listing {}, locked escrow {}",
        LISTING, ESCROW
    );

    // A stale batch rolls back atomically (nothing escapes): one leg carries a
    // version that no longer matches, so the whole batch is rejected.
    let stale = settlement_intents("bad", 99);
    let stale_err = harness.executor.execute_batch(&stale).await.unwrap_err();
    assert!(matches!(stale_err, ExecutorError::AtomicityViolation(_)));
    assert_eq!(
        harness.transactions.log(),
        vec!["begin:acme.game.alpha", "rollback"]
    );
    println!("stale expected_version -> rejected (AtomicityViolation), rollback");

    // The successful atomic settlement: listing sold, escrow released, asset
    // transferred, buyer debited, seller credited. One tenant, all-or-nothing.
    let ok = settlement_intents("ok", 1);
    let events = harness.executor.execute_batch(&ok).await.unwrap();
    assert_eq!(events.len(), 5);
    assert_eq!(
        harness.transactions.log(),
        vec![
            "begin:acme.game.alpha",
            "rollback",
            "begin:acme.game.alpha",
            "commit"
        ]
    );
    for event in &events {
        apply_batch_event(&harness, event).await;
    }
    println!("settlement batch -> 5 events, tx log begin...commit");

    let bob_wallet = harness
        .index
        .get_subject_state(&tenant(), &SubjectId(String::from(BOB)), &wallet)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bob_wallet.state["balance"],
        json!((1000 - PRICE).to_string())
    );
    let alice_wallet = harness
        .index
        .get_subject_state(&tenant(), &SubjectId(String::from(ALICE)), &wallet)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        alice_wallet.state["balance"],
        json!((500 + PRICE).to_string())
    );
    let asset = harness
        .index
        .get_state(&tenant(), &ResourceId(String::from(ASSET)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(asset.state["owner"], json!(BOB));
    let listing = harness
        .index
        .get_state(&tenant(), &ResourceId(String::from(LISTING)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(listing.state["status"], json!("sold"));
    let escrow = harness
        .index
        .get_state(&tenant(), &ResourceId(String::from(ESCROW)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(escrow.state["status"], json!("released"));
    println!(
        "buyer debited to {}; seller credited to {}; asset owned by BOB; \
         listing sold; escrow released",
        bob_wallet.state["balance"], alice_wallet.state["balance"]
    );

    println!("marketplace: OK");
}

/// Builds the settlement batch: listing.buy, escrow.release, asset transfer,
/// buyer debit, and seller credit. `suffix` disambiguates intent ids;
/// `version` sets the expected version on every leg (use 99 to go stale).
fn settlement_intents(suffix: &str, version: u64) -> Vec<ValidatedIntent> {
    vec![
        typed_intent(
            tenant(),
            &format!("mkt_{suffix}_buy"),
            "listing.buy",
            BOB,
            LISTING,
            StateType::Listing,
            version,
            &[("buyer", json!(BOB))],
            None,
        ),
        typed_intent(
            tenant(),
            &format!("mkt_{suffix}_release"),
            "escrow.release",
            BOB,
            ESCROW,
            StateType::Escrow,
            version,
            &[],
            None,
        ),
        typed_intent(
            tenant(),
            &format!("mkt_{suffix}_transfer"),
            "asset.transfer",
            ALICE,
            ASSET,
            StateType::UniqueAsset,
            version,
            &[("from_owner", json!(ALICE)), ("to_owner", json!(BOB))],
            Some(sample_authority()),
        ),
        typed_intent(
            tenant(),
            &format!("mkt_{suffix}_debit"),
            "balance.debit",
            BOB,
            WALLET,
            StateType::FungibleBalance,
            version,
            &[("amount", json!(PRICE.to_string()))],
            None,
        ),
        typed_intent(
            tenant(),
            &format!("mkt_{suffix}_credit"),
            "balance.credit",
            ALICE,
            WALLET,
            StateType::FungibleBalance,
            version,
            &[("amount", json!(PRICE.to_string()))],
            None,
        ),
    ]
}
