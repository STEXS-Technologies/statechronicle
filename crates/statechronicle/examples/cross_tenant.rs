//! Run: `cargo run -p statechronicle --example cross_tenant`
//!
//! Cross-tenant atomicity (protocol §8.2, §18.3): one transaction spans two
//! tenants (alpha transfers an asset, beta transfers a balance), sharing one
//! intent id, and commits one tenant-scoped event group per tenant. A failing
//! variant (beta declares a stale expected version) rolls the whole transaction
//! back atomically.

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
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;

use common::{Harness, beta};

const ALICE: &str = "account:example:player_123";
const BOB: &str = "account:example:player_456";
const CAROL: &str = "account:example:player_carol";
const DAVE: &str = "account:example:player_dave";
const ASSET: &str = "asset:sword_001";
const CURRENCY: &str = "currency:gold";

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

    println!("== cross_tenant: atomic transaction across alpha + beta ==");

    // Seed alpha: mint an asset for ALICE (version 0 -> 1).
    let minted = harness
        .run(
            &signed(
                &harness,
                alpha.clone(),
                "ct_mint",
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
    println!("alpha mint(ALICE) -> {}", minted.after.state);

    // Seed beta: create a balance for CAROL (version 0 -> 1).
    let created = harness
        .run(
            &signed(
                &harness,
                beta.clone(),
                "ct_create",
                "balance.create",
                CAROL,
                CURRENCY,
                StateType::FungibleBalance,
                0,
                &[
                    ("subject", json!(CAROL)),
                    ("unit", json!("gold_minor")),
                    ("balance", json!("100")),
                ],
                None,
            ),
            StateType::FungibleBalance,
        )
        .await;
    println!("beta  balance.create(CAROL) -> {}", created.after.state);

    // The cross-tenant transaction: alpha transfers an asset, beta transfers a
    // balance, sharing one intent id "xt_001" (the cross-tenant linkage).
    let intents = vec![
        signed(
            &harness,
            alpha.clone(),
            "xt_001",
            "asset.transfer",
            ALICE,
            ASSET,
            StateType::UniqueAsset,
            1,
            &[("from_owner", json!(ALICE)), ("to_owner", json!(BOB))],
            Some(harness.authority()),
        ),
        signed(
            &harness,
            beta.clone(),
            "xt_001",
            "balance.transfer",
            CAROL,
            CURRENCY,
            StateType::FungibleBalance,
            1,
            &[("to_subject", json!(DAVE)), ("amount", json!("40"))],
            None,
        ),
    ];
    let groups = harness
        .executor
        .execute_cross_tenant(&intents)
        .await
        .unwrap();

    assert_eq!(
        groups.len(),
        2,
        "one tenant event group per affected tenant"
    );
    // Groups come back sorted by tenant name (alpha, then beta).
    assert_eq!(groups[0].tenant, alpha);
    assert_eq!(groups[0].events.len(), 1, "alpha: one asset-transfer event");
    assert_eq!(groups[1].tenant, beta);
    assert_eq!(groups[1].events.len(), 2, "beta: debit + credit pair");

    assert_eq!(
        harness.transactions.log(),
        vec!["begin_multi:acme.game.alpha,acme.game.beta", "commit"]
    );
    println!(
        "cross-tenant commit -> 2 groups; tx log: begin_multi:acme.game.alpha,acme.game.beta, commit"
    );
    println!("alpha group: {} event(s)", groups[0].events.len());
    println!("beta  group: {} event(s)", groups[1].events.len());

    // Commit each tenant's group separately into its own signed commit.
    let (alpha_signed, alpha_acc) = harness.commit_events(&groups[0].events);
    let (beta_signed, beta_acc) = harness.commit_events(&groups[1].events);
    println!(
        "alpha commit {} signs {} event; state root {}",
        alpha_signed.body.commit_id.as_str(),
        alpha_signed.body.event_count,
        alpha_signed.body.next_state_root.as_str()
    );
    println!(
        "beta  commit {} signs {} events; state root {}",
        beta_signed.body.commit_id.as_str(),
        beta_signed.body.event_count,
        beta_signed.body.next_state_root.as_str()
    );
    assert_eq!(
        alpha_acc.root().as_bytes(),
        alpha_signed.body.next_state_root.as_bytes()
    );
    assert_eq!(
        beta_acc.root().as_bytes(),
        beta_signed.body.next_state_root.as_bytes()
    );

    // Failing variant: beta declares a stale expected version (99). Seed a
    // fresh asset in alpha so alpha's leg is valid and only beta fails, then
    // confirm the whole transaction rolls back atomically.
    const DAGGER: &str = "asset:dagger_002";
    let dagger_mint = harness
        .run(
            &signed(
                &harness,
                alpha.clone(),
                "ct_dagger_mint",
                "asset.mint",
                ALICE,
                DAGGER,
                StateType::UniqueAsset,
                0,
                &[("to_owner", json!(ALICE))],
                None,
            ),
            StateType::UniqueAsset,
        )
        .await;
    println!("alpha mint(DAGGER) -> {}", dagger_mint.after.state);

    let stale = vec![
        signed(
            &harness,
            alpha.clone(),
            "xt_002",
            "asset.transfer",
            ALICE,
            DAGGER,
            StateType::UniqueAsset,
            1,
            &[("from_owner", json!(ALICE)), ("to_owner", json!(BOB))],
            Some(harness.authority()),
        ),
        signed(
            &harness,
            beta.clone(),
            "xt_002",
            "balance.transfer",
            CAROL,
            CURRENCY,
            StateType::FungibleBalance,
            99,
            &[("to_subject", json!(DAVE)), ("amount", json!("40"))],
            None,
        ),
    ];
    let err = harness
        .executor
        .execute_cross_tenant(&stale)
        .await
        .unwrap_err();
    assert!(matches!(err, ExecutorError::AtomicityViolation(_)));
    assert_eq!(
        harness.transactions.log(),
        vec![
            "begin_multi:acme.game.alpha,acme.game.beta",
            "commit",
            "begin_multi:acme.game.alpha,acme.game.beta",
            "rollback",
        ]
    );
    println!("stale expected_version -> rejected (AtomicityViolation), rollback");

    println!("cross_tenant: OK");
}
