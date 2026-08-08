//! Run: `cargo run -p statechronicle --example trade_cross_tenant`
//!
//! A cross-tenant trade settlement under the baseline profiles (Phase 3,
//! protocol §18.3): an asset in tenant alpha is settled to the buyer while the
//! fungible value leg (asset-for-gold) moves inside tenant beta. The two legs
//! have DISTINCT intent ids, so no id spans two tenants; instead the linkage is
//! DECLARED in a [`TradeManifest`] naming the settle intent and the value leg.
//!
//! The settle runs through `execute_cross_tenant_trade` in ONE atomic
//! transaction: alpha emits one `trade.settle` event and beta emits one
//! net-zero `balance.transfer` debit + credit pair. Asserted in one commit: the
//! asset's owner changes to the buyer in alpha, the buyer's balance is debited
//! and the seller's credited in beta, and the transaction log shows
//! `begin_multi:alpha,beta` followed by one `commit`.

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
use statechronicle::executor::atomicity::{TradeManifest, ValueLeg};
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::ports::state_index::StateIndex;

use common::{Harness, beta};

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

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    harness.tenant_store.register(beta.clone());
    let wallet = ResourceId(String::from(WALLET));
    let asset = ResourceId(String::from(ASSET));

    println!("== trade_cross_tenant: asset in alpha for gold in beta ==");

    // Seed alpha: mint the asset to the seller, then freeze it into a pending
    // trade (version 0 -> 1 -> 2).
    harness
        .run(
            &signed(
                &harness,
                alpha.clone(),
                "xct_mint",
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
                alpha.clone(),
                "xct_lock",
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
    println!("alpha: minted asset {}, froze into trade {}", ASSET, TRADE);

    // Seed beta: create the buyer's gold balance (version 0 -> 1).
    harness
        .run(
            &signed(
                &harness,
                beta.clone(),
                "xct_bob_wallet",
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
    println!("beta:  created buyer gold balance 1000");

    // The settle intent (alpha) declares the value leg; the value leg intent
    // (beta) is a balance.transfer with a DISTINCT intent id. The manifest ties
    // them together.
    let settle = signed(
        &harness,
        alpha.clone(),
        "xct_settle",
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
        beta.clone(),
        "xct_value",
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
    let manifest = TradeManifest {
        trade_id: String::from(TRADE),
        settle_intent_id: IntentId::new(String::from("int_xct_settle")).unwrap(),
        value_leg: Some(ValueLeg {
            resource: ResourceId(String::from(WALLET)),
            amount: PRICE.to_string(),
            to_subject: SubjectId(String::from(ALICE)),
        }),
        settle_assets: Vec::new(),
    };

    // Settle in ONE atomic cross-tenant transaction: one group per tenant.
    let groups = harness
        .executor
        .execute_cross_tenant_trade(&[settle, value_leg], &manifest)
        .await
        .unwrap();
    assert_eq!(
        groups.len(),
        2,
        "one tenant event group per affected tenant"
    );
    // Groups come back sorted by tenant name (alpha, then beta).
    assert_eq!(groups[0].tenant, alpha);
    assert_eq!(groups[0].events.len(), 1, "alpha: one trade.settle event");
    assert_eq!(groups[1].tenant, beta);
    assert_eq!(
        groups[1].events.len(),
        2,
        "beta: one net-zero debit + credit pair"
    );
    assert_eq!(
        harness.transactions.log(),
        vec!["begin_multi:acme.game.alpha,acme.game.beta", "commit"]
    );
    println!(
        "cross-tenant trade commit -> 2 groups; tx log: begin_multi:acme.game.alpha,acme.game.beta, commit"
    );

    // Apply each tenant's group to the index (alpha: asset; beta: balances).
    for event in &groups[0].events {
        harness.index.apply(event, StateType::UniqueAsset);
    }
    for event in &groups[1].events {
        harness.index.apply(event, StateType::FungibleBalance);
    }

    // The asset is now owned by the buyer in alpha.
    let held = harness
        .index
        .get_state(&alpha, &asset)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.state["owner"], json!(BOB));
    assert_eq!(held.state["status"], json!("active"));

    // In beta, the buyer is debited and the seller is credited (create-on-credit).
    let bob_wallet = harness
        .index
        .get_subject_state(&beta, &SubjectId(String::from(BOB)), &wallet)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bob_wallet.state["balance"],
        json!((1000 - PRICE).to_string())
    );
    let alice_wallet = harness
        .index
        .get_subject_state(&beta, &SubjectId(String::from(ALICE)), &wallet)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(alice_wallet.state["balance"], json!(PRICE.to_string()));
    println!(
        "alpha: asset owned by BOB; beta: buyer debited to {}, seller credited to {}",
        bob_wallet.state["balance"], alice_wallet.state["balance"]
    );

    println!("trade_cross_tenant: OK");
}
