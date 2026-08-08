//! Integration test: the cross-tenant trade settlement with a declared linkage
//! manifest (Phase 3, protocol §8.2, §18.3).
//!
//! Runs a `trade.lock` -> cross-tenant `trade.settle` lifecycle through the REAL
//! cross-crate executor (in-memory port fakes). An asset in tenant alpha is
//! settled to the buyer while the fungible value leg (asset-for-gold) moves
//! inside tenant beta. The legs have DISTINCT intent ids, so the linkage is
//! DECLARED in a [`TradeManifest`] naming the settle intent and the value leg,
//! and the transaction runs through `execute_cross_tenant_trade` in ONE atomic
//! commit: one `trade.settle` group in alpha plus one net-zero `balance.transfer`
//! pair in beta.
//!
//! Asserted: (a) the two-tenant asset-for-gold settle commits with two groups
//! and one `commit`; (b) a settle whose value leg is missing from the manifest
//! fails closed and rolls back with nothing escaping.
//!
//! 3-tenant deferral: a trade that spans THREE tenants (asset A in alpha, asset
//! B in beta, value in gamma) needs TWO settle legs, but the declared-linkage
//! path validates exactly ONE settle leg plus one optional value leg. The
//! executor's `execute_cross_tenant` (inferred-linkage) path already covers
//! multi-tenant batches that share one intent id; the manifest-driven trade path
//! covers the two-tenant asset-for-value case and fails closed on an undeclared
//! second asset leg (see `cross_tenant_trade_extra_settle_leg_rejected` in the
//! executor's atomicity unit tests). Genuine three-tenant trade settlement is
//! deferred to a later phase.

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
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::ports::state_index::StateIndex;

use common::{Harness, beta};

const ALICE: &str = "account:example:player_123"; // seller
const BOB: &str = "account:example:player_456"; // buyer
const ASSET: &str = "asset:relic_001";
const WALLET: &str = "wallet:gold";
const TRADE: &str = "trade_001";
const PRICE: u64 = 100;

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

/// Seeds alpha (asset minted + locked into a trade) and beta (buyer wallet).
async fn seed(harness: &Harness, alpha: &TenantId, beta: &TenantId) {
    harness
        .run(
            &signed(
                harness,
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
                harness,
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
    harness
        .run(
            &signed(
                harness,
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
}

/// Builds the settle (alpha) + value leg (beta) intent pair for a trade.
fn settle_intents(harness: &Harness, alpha: &TenantId, beta: &TenantId) -> Vec<ValidatedIntent> {
    vec![
        signed(
            harness,
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
        ),
        signed(
            harness,
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
        ),
    ]
}

/// Builds the declaring value-leg manifest for the seeded trade.
fn manifest(harness: &Harness, value_leg: Option<ValueLeg>) -> TradeManifest {
    let _ = harness;
    TradeManifest {
        trade_id: String::from(TRADE),
        settle_intent_id: IntentId::new(String::from("int_xct_settle")).unwrap(),
        value_leg,
        settle_assets: Vec::new(),
    }
}

#[tokio::test]
async fn two_tenant_asset_for_gold_settles_in_one_commit() {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    harness.tenant_store.register(beta.clone());
    seed(&harness, &alpha, &beta).await;

    let intents = settle_intents(&harness, &alpha, &beta);
    let manifest = manifest(
        &harness,
        Some(ValueLeg {
            resource: ResourceId(String::from(WALLET)),
            amount: PRICE.to_string(),
            to_subject: SubjectId(String::from(ALICE)),
        }),
    );

    let groups = harness
        .executor
        .execute_cross_tenant_trade(&intents, &manifest)
        .await
        .unwrap();

    // Two groups (alpha, beta), one commit.
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].tenant, alpha);
    assert_eq!(groups[0].events.len(), 1);
    assert_eq!(groups[1].tenant, beta);
    assert_eq!(groups[1].events.len(), 2);
    assert_eq!(
        harness.transactions.log(),
        vec!["begin_multi:acme.game.alpha,acme.game.beta", "commit"]
    );

    for event in &groups[0].events {
        harness.index.apply(event, StateType::UniqueAsset);
    }
    for event in &groups[1].events {
        harness.index.apply(event, StateType::FungibleBalance);
    }

    let wallet = ResourceId(String::from(WALLET));
    let asset = ResourceId(String::from(ASSET));
    let held = harness
        .index
        .get_state(&alpha, &asset)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.state["owner"], json!(BOB));
    assert_eq!(held.state["status"], json!("active"));

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
}

#[tokio::test]
async fn value_leg_missing_from_manifest_fails_closed_and_rolls_back() {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    harness.tenant_store.register(beta.clone());
    seed(&harness, &alpha, &beta).await;

    // The batch carries the value leg, but the manifest declares NO value leg:
    // the linkage is incomplete and the transaction must fail closed.
    let intents = settle_intents(&harness, &alpha, &beta);
    let manifest = manifest(&harness, None);

    let err = harness
        .executor
        .execute_cross_tenant_trade(&intents, &manifest)
        .await
        .unwrap_err();
    assert!(matches!(err, ExecutorError::AtomicityViolation(_)));

    // Rolled back atomically: nothing escapes.
    assert_eq!(
        harness.transactions.log(),
        vec!["begin_multi:acme.game.alpha,acme.game.beta", "rollback",]
    );

    let wallet = ResourceId(String::from(WALLET));
    let asset = ResourceId(String::from(ASSET));
    // The asset is still owned by ALICE and still trade_held in alpha.
    let held = harness
        .index
        .get_state(&alpha, &asset)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.state["owner"], json!(ALICE));
    assert_eq!(held.state["status"], json!("trade_held"));
    // The buyer's beta balance is unchanged.
    let bob_wallet = harness
        .index
        .get_subject_state(&beta, &SubjectId(String::from(BOB)), &wallet)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bob_wallet.state["balance"], json!("1000"));
    // The seller was never credited in beta.
    assert!(
        harness
            .index
            .get_subject_state(&beta, &SubjectId(String::from(ALICE)), &wallet)
            .await
            .unwrap()
            .is_none()
    );
}
