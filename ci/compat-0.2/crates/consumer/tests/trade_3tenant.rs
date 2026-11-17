//! Integration test: N-tenant trade settlement via the generalized manifest
//! (protocol §18.3, Phase 3 generalized).
//!
//! Runs a `trade.lock` -> cross-tenant `trade.settle` lifecycle through the REAL
//! cross-crate executor (in-memory port fakes) spanning THREE tenants. Each
//! asset leg runs `trade.settle` under its own tenant and the fungible value leg
//! (balance.transfer) runs under a third tenant; the legs have DISTINCT intent
//! ids, so the linkage is DECLARED in a [`TradeManifest`] naming every settle leg
//! and every value leg, and the whole trade commits in ONE atomic
//! `execute_cross_tenant_trade` call.
//!
//! Asserted: (a) a full 3-tenant lifecycle (two asset legs in two tenants plus a
//! value leg in a third) lands three [`TenantEventGroup`]s in one `commit`, with
//! all owner changes and balance movement applied; (b) a pure 3-tenant no-value
//! swap (three assets, zero value legs) also lands in one commit; (c) a
//! stale-version leg rolls the whole trade back atomically with nothing escaping.

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
use statechronicle::executor::atomicity::{SettleLeg, TradeManifest, ValueLeg};
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::ports::state_index::StateIndex;

use common::{Harness, beta};

const ALICE: &str = "account:example:player_123"; // offers ASSET_A
const BOB: &str = "account:example:player_456"; // offers ASSET_B
const CAROL: &str = "account:example:player_789"; // offers ASSET_C
const ASSET_A: &str = "asset:relic_001";
const ASSET_B: &str = "asset:sword_001";
const ASSET_C: &str = "asset:helm_001";
const WALLET: &str = "wallet:gold";
const TRADE: &str = "trade_3t_001";
const PRICE: u64 = 100;

/// A third tenant for the 3-tenant scenarios.
fn gamma() -> TenantId {
    TenantId(String::from("acme.game.gamma"))
}

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

/// Seeds alpha with ASSET_A minted to `a_owner` and locked into the trade.
async fn mint_and_lock(
    harness: &Harness,
    tenant: TenantId,
    mint_id: &str,
    lock_id: &str,
    owner: &str,
    asset: &str,
) {
    harness
        .run(
            &signed(
                harness,
                tenant.clone(),
                mint_id,
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
    harness
        .run(
            &signed(
                harness,
                tenant,
                lock_id,
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

/// Seeds gamma with the buyer's gold balance.
async fn seed_gold(harness: &Harness, tenant: TenantId, holder: &str) {
    harness
        .run(
            &signed(
                harness,
                tenant,
                "t3t_gold",
                "balance.create",
                holder,
                WALLET,
                StateType::FungibleBalance,
                0,
                &[
                    ("subject", json!(holder)),
                    ("unit", json!("gold_minor")),
                    ("balance", json!("1000")),
                ],
                None,
            ),
            StateType::FungibleBalance,
        )
        .await;
}

/// Builds the settle intents (two asset legs in alpha/beta + a value leg in
/// gamma) for the full 3-tenant trade, with an optional override on the alpha
/// settle's expected version (for the stale-version rollback case).
#[allow(clippy::too_many_arguments)]
fn settle_intents(
    harness: &Harness,
    alpha: &TenantId,
    beta: &TenantId,
    gamma: &TenantId,
    alpha_version: u64,
) -> Vec<ValidatedIntent> {
    vec![
        signed(
            harness,
            alpha.clone(),
            "t3t_settle_a",
            "trade.settle",
            ALICE,
            ASSET_A,
            StateType::UniqueAsset,
            alpha_version,
            &[
                ("from_owner", json!(ALICE)),
                ("to_owner", json!(BOB)),
                ("trade_id", json!(TRADE)),
            ],
            Some(harness.authority()),
        ),
        signed(
            harness,
            beta.clone(),
            "t3t_settle_b",
            "trade.settle",
            BOB,
            ASSET_B,
            StateType::UniqueAsset,
            2,
            &[
                ("from_owner", json!(BOB)),
                ("to_owner", json!(ALICE)),
                ("trade_id", json!(TRADE)),
            ],
            Some(harness.authority()),
        ),
        signed(
            harness,
            gamma.clone(),
            "t3t_value",
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

/// The full 3-tenant manifest: two settle legs (A in alpha, B in beta) plus the
/// gold value leg (in gamma).
fn manifest() -> TradeManifest {
    TradeManifest {
        trade_id: String::from(TRADE),
        settle_legs: vec![
            SettleLeg {
                asset: ResourceId(String::from(ASSET_A)),
                settle_intent_id: IntentId::new(String::from("int_t3t_settle_a")).unwrap(),
            },
            SettleLeg {
                asset: ResourceId(String::from(ASSET_B)),
                settle_intent_id: IntentId::new(String::from("int_t3t_settle_b")).unwrap(),
            },
        ],
        value_legs: vec![ValueLeg {
            resource: ResourceId(String::from(WALLET)),
            amount: PRICE.to_string(),
            to_subject: SubjectId(String::from(ALICE)),
        }],
    }
}

/// Applies every event in a group to the harness index under `state_type`.
async fn apply_group(
    harness: &Harness,
    group: &statechronicle::executor::atomicity::TenantEventGroup,
    state_type: StateType,
) {
    for event in &group.events {
        harness.index.apply(event, state_type);
    }
}

#[tokio::test]
async fn three_tenant_asset_swap_with_value_lands_in_one_commit() {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    let gamma_tenant = gamma();
    harness.tenant_store.register(beta.clone());
    harness.tenant_store.register(gamma_tenant.clone());

    // Seed: A locked in alpha (ALICE), B locked in beta (BOB), gold in gamma.
    mint_and_lock(
        &harness,
        alpha.clone(),
        "t3t_mint_a",
        "t3t_lock_a",
        ALICE,
        ASSET_A,
    )
    .await;
    mint_and_lock(
        &harness,
        beta.clone(),
        "t3t_mint_b",
        "t3t_lock_b",
        BOB,
        ASSET_B,
    )
    .await;
    seed_gold(&harness, gamma_tenant.clone(), BOB).await;

    let intents = settle_intents(&harness, &alpha, &beta, &gamma_tenant, 2);
    let manifest = manifest();

    let groups = harness
        .executor
        .execute_cross_tenant_trade(&intents, &manifest)
        .await
        .unwrap();

    // Three tenant groups, one atomic begin_multi + commit.
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].tenant, alpha);
    assert_eq!(groups[0].events.len(), 1, "alpha: one trade.settle for A");
    assert_eq!(groups[1].tenant, beta);
    assert_eq!(groups[1].events.len(), 1, "beta: one trade.settle for B");
    assert_eq!(groups[2].tenant, gamma_tenant);
    assert_eq!(
        groups[2].events.len(),
        2,
        "gamma: one net-zero debit + credit value pair"
    );
    assert_eq!(
        harness.transactions.log(),
        vec![
            "begin_multi:acme.game.alpha,acme.game.beta,acme.game.gamma",
            "commit",
        ]
    );

    // Apply each tenant's events and assert all state landed.
    apply_group(&harness, &groups[0], StateType::UniqueAsset).await;
    apply_group(&harness, &groups[1], StateType::UniqueAsset).await;
    apply_group(&harness, &groups[2], StateType::FungibleBalance).await;

    let asset_a = ResourceId(String::from(ASSET_A));
    let asset_b = ResourceId(String::from(ASSET_B));
    let wallet = ResourceId(String::from(WALLET));

    // A moved ALICE -> BOB in alpha.
    let held_a = harness
        .index
        .get_state(&alpha, &asset_a)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held_a.state.get("owner").unwrap(), json!(BOB));
    assert_eq!(held_a.state.get("status").unwrap(), json!("active"));
    // B moved BOB -> ALICE in beta.
    let held_b = harness
        .index
        .get_state(&beta, &asset_b)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held_b.state.get("owner").unwrap(), json!(ALICE));
    assert_eq!(held_b.state.get("status").unwrap(), json!("active"));

    // Gamma: BOB debited, ALICE credited.
    let bob_wallet = harness
        .index
        .get_subject_state(&gamma_tenant, &SubjectId(String::from(BOB)), &wallet)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bob_wallet.state.get("balance").unwrap(),
        json!((1000 - PRICE).to_string())
    );
    let alice_wallet = harness
        .index
        .get_subject_state(&gamma_tenant, &SubjectId(String::from(ALICE)), &wallet)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        alice_wallet.state.get("balance").unwrap(),
        json!(PRICE.to_string())
    );
}

#[tokio::test]
async fn pure_three_tenant_no_value_swap_lands_in_one_commit() {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    let gamma_tenant = gamma();
    harness.tenant_store.register(beta.clone());
    harness.tenant_store.register(gamma_tenant.clone());

    // A 3-way rotation with zero value legs: A (ALICE->BOB), B (BOB->CAROL),
    // C (CAROL->ALICE), each asset in its own tenant.
    mint_and_lock(
        &harness,
        alpha.clone(),
        "t3t_mint_a",
        "t3t_lock_a",
        ALICE,
        ASSET_A,
    )
    .await;
    mint_and_lock(
        &harness,
        beta.clone(),
        "t3t_mint_b",
        "t3t_lock_b",
        BOB,
        ASSET_B,
    )
    .await;
    mint_and_lock(
        &harness,
        gamma_tenant.clone(),
        "t3t_mint_c",
        "t3t_lock_c",
        CAROL,
        ASSET_C,
    )
    .await;

    let settle_a = signed(
        &harness,
        alpha.clone(),
        "t3t_swap_a",
        "trade.settle",
        ALICE,
        ASSET_A,
        StateType::UniqueAsset,
        2,
        &[
            ("from_owner", json!(ALICE)),
            ("to_owner", json!(BOB)),
            ("trade_id", json!(TRADE)),
        ],
        Some(harness.authority()),
    );
    let settle_b = signed(
        &harness,
        beta.clone(),
        "t3t_swap_b",
        "trade.settle",
        BOB,
        ASSET_B,
        StateType::UniqueAsset,
        2,
        &[
            ("from_owner", json!(BOB)),
            ("to_owner", json!(CAROL)),
            ("trade_id", json!(TRADE)),
        ],
        Some(harness.authority()),
    );
    let settle_c = signed(
        &harness,
        gamma_tenant.clone(),
        "t3t_swap_c",
        "trade.settle",
        CAROL,
        ASSET_C,
        StateType::UniqueAsset,
        2,
        &[
            ("from_owner", json!(CAROL)),
            ("to_owner", json!(ALICE)),
            ("trade_id", json!(TRADE)),
        ],
        Some(harness.authority()),
    );

    let manifest = TradeManifest {
        trade_id: String::from(TRADE),
        settle_legs: vec![
            SettleLeg {
                asset: ResourceId(String::from(ASSET_A)),
                settle_intent_id: IntentId::new(String::from("int_t3t_swap_a")).unwrap(),
            },
            SettleLeg {
                asset: ResourceId(String::from(ASSET_B)),
                settle_intent_id: IntentId::new(String::from("int_t3t_swap_b")).unwrap(),
            },
            SettleLeg {
                asset: ResourceId(String::from(ASSET_C)),
                settle_intent_id: IntentId::new(String::from("int_t3t_swap_c")).unwrap(),
            },
        ],
        value_legs: Vec::new(),
    };

    let groups = harness
        .executor
        .execute_cross_tenant_trade(&[settle_a, settle_b, settle_c], &manifest)
        .await
        .unwrap();

    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].events.len(), 1);
    assert_eq!(groups[1].events.len(), 1);
    assert_eq!(groups[2].events.len(), 1);
    assert_eq!(
        harness.transactions.log(),
        vec![
            "begin_multi:acme.game.alpha,acme.game.beta,acme.game.gamma",
            "commit",
        ]
    );

    for (group, state_type) in [
        (&groups[0], StateType::UniqueAsset),
        (&groups[1], StateType::UniqueAsset),
        (&groups[2], StateType::UniqueAsset),
    ] {
        apply_group(&harness, group, state_type).await;
    }

    let asset_a = ResourceId(String::from(ASSET_A));
    let asset_b = ResourceId(String::from(ASSET_B));
    let asset_c = ResourceId(String::from(ASSET_C));
    assert_eq!(
        harness
            .index
            .get_state(&alpha, &asset_a)
            .await
            .unwrap()
            .unwrap()
            .state
            .get("owner")
            .unwrap(),
        json!(BOB)
    );
    assert_eq!(
        harness
            .index
            .get_state(&beta, &asset_b)
            .await
            .unwrap()
            .unwrap()
            .state
            .get("owner")
            .unwrap(),
        json!(CAROL)
    );
    assert_eq!(
        harness
            .index
            .get_state(&gamma_tenant, &asset_c)
            .await
            .unwrap()
            .unwrap()
            .state
            .get("owner")
            .unwrap(),
        json!(ALICE)
    );
}

#[tokio::test]
async fn stale_version_leg_rolls_back_atomically() {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    let gamma_tenant = gamma();
    harness.tenant_store.register(beta.clone());
    harness.tenant_store.register(gamma_tenant.clone());

    mint_and_lock(
        &harness,
        alpha.clone(),
        "t3t_mint_a",
        "t3t_lock_a",
        ALICE,
        ASSET_A,
    )
    .await;
    mint_and_lock(
        &harness,
        beta.clone(),
        "t3t_mint_b",
        "t3t_lock_b",
        BOB,
        ASSET_B,
    )
    .await;
    seed_gold(&harness, gamma_tenant.clone(), BOB).await;

    // The alpha settle expects a stale (never-reached) version, so its leg fails
    // the expected-version gate and the whole cross-tenant trade must roll back.
    let intents = settle_intents(&harness, &alpha, &beta, &gamma_tenant, 99);
    let manifest = manifest();

    let err = harness
        .executor
        .execute_cross_tenant_trade(&intents, &manifest)
        .await
        .unwrap_err();
    assert!(matches!(err, ExecutorError::AtomicityViolation(_)));

    // Rolled back atomically: nothing escapes.
    assert_eq!(
        harness.transactions.log(),
        vec![
            "begin_multi:acme.game.alpha,acme.game.beta,acme.game.gamma",
            "rollback",
        ]
    );

    let asset_a = ResourceId(String::from(ASSET_A));
    let asset_b = ResourceId(String::from(ASSET_B));
    let wallet = ResourceId(String::from(WALLET));

    // A still owned by ALICE and trade_held in alpha; B still trade_held in beta.
    let held_a = harness
        .index
        .get_state(&alpha, &asset_a)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held_a.state.get("owner").unwrap(), json!(ALICE));
    assert_eq!(held_a.state.get("status").unwrap(), json!("trade_held"));
    let held_b = harness
        .index
        .get_state(&beta, &asset_b)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held_b.state.get("owner").unwrap(), json!(BOB));
    assert_eq!(held_b.state.get("status").unwrap(), json!("trade_held"));

    // Gamma balance unchanged and ALICE was never credited.
    let bob_wallet = harness
        .index
        .get_subject_state(&gamma_tenant, &SubjectId(String::from(BOB)), &wallet)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bob_wallet.state.get("balance").unwrap(), json!("1000"));
    assert!(
        harness
            .index
            .get_subject_state(&gamma_tenant, &SubjectId(String::from(ALICE)), &wallet)
            .await
            .unwrap()
            .is_none()
    );
}
