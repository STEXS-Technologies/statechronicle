//! Workload adapted from the frozen original two-tenant trade integration test.
//! Measures symbolic cross-tenant planning, not durable cross-store atomicity.
use crate::common::{Harness, beta, fixed_key};
use ed25519_dalek::VerifyingKey;
use serde_json::json;
use statechronicle::domain::authority::AuthorityProof;
use statechronicle::domain::ids::IntentId;
use statechronicle::domain::intent::{Intent, Nonce};
use statechronicle::domain::proof::ResourceStateProof;
use statechronicle::domain::trade::TradeProof;
use statechronicle::executor::atomicity::{SettleLeg, TenantEventGroup, TradeManifest, ValueLeg};
use statechronicle::index::build::{IngestBatch, apply};
use statechronicle::proof::bundle::build_state_proof;
use statechronicle::proof::trade::{build_trade_proof, verify_trade_proof};
use statechronicle::{
    Commit, Event, Operation, ResourceId, Signed, StateAccumulator, StateKey, StateProjection,
    StateType, SubjectId, TenantId,
};
use std::collections::BTreeMap;
const ALICE: &str = "account:example:player_123";
const BOB: &str = "account:example:player_456";
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
) -> statechronicle::intent::validated::ValidatedIntent {
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
                "thp_mint",
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
                "thp_lock",
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
                "thp_wallet",
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

fn settle_intents(
    harness: &Harness,
    alpha: &TenantId,
    beta: &TenantId,
) -> Vec<statechronicle::intent::validated::ValidatedIntent> {
    vec![
        signed(
            harness,
            alpha.clone(),
            "thp_settle",
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
            "thp_value",
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

fn manifest() -> TradeManifest {
    TradeManifest {
        trade_id: String::from(TRADE),
        settle_legs: vec![SettleLeg {
            asset: ResourceId(String::from(ASSET)),
            settle_intent_id: IntentId::new(String::from("int_thp_settle")).unwrap(),
        }],
        value_legs: vec![ValueLeg {
            resource: ResourceId(String::from(WALLET)),
            amount: PRICE.to_string(),
            to_subject: SubjectId(String::from(ALICE)),
        }],
    }
}

/// Builds a genuine, verifiable state proof of the settled asset at the alpha
/// commit, using the accumulator returned by the harness's commit formation.
fn build_asset_proof(
    settle_event: &Event,
    signed_alpha: &Signed<Commit>,
    acc_alpha: &StateAccumulator,
) -> ResourceStateProof {
    let projection = StateProjection {
        tenant_id: settle_event.tenant_id.clone(),
        resource_id: settle_event.resource_id.clone(),
        state_type: StateType::UniqueAsset,
        version: settle_event.after.version,
        last_event_id: settle_event.event_id.clone(),
        last_commit_id: signed_alpha.body.commit_id.clone(),
        state_hash: settle_event.after.state_hash.clone(),
        state: settle_event.after.state.clone(),
    };
    let key = StateKey::for_resource(&settle_event.tenant_id.0, &settle_event.resource_id.0);
    let inclusion = acc_alpha.prove_inclusion(&key).unwrap();
    build_state_proof(
        &projection,
        signed_alpha,
        &inclusion,
        &Operation::from_static("trade.settle"),
        None,
        key,
    )
    .unwrap()
}

async fn plan() -> (
    Harness,
    Vec<TenantEventGroup>,
    Vec<statechronicle::intent::validated::ValidatedIntent>,
) {
    let harness = Harness::new();
    let alpha = harness.tenant();
    let beta = beta();
    harness.tenant_store.register(beta.clone());
    seed(&harness, &alpha, &beta).await;
    let intents = settle_intents(&harness, &alpha, &beta);
    let groups = harness
        .executor
        .execute_cross_tenant_trade(&intents, &manifest())
        .await
        .unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].events.len(), 1);
    assert_eq!(groups[1].events.len(), 2);
    (harness, groups, intents)
}

pub async fn plan_including_setup() {
    std::hint::black_box(plan().await);
}

pub async fn proof_fixture() -> (TradeProof, BTreeMap<String, (Signed<Commit>, VerifyingKey)>) {
    let (harness, groups, intents) = plan().await;
    let (alpha, tree) = harness.commit_events(&groups[0].events);
    let (beta, _) = harness.commit_events(&groups[1].events);
    let asset = build_asset_proof(&groups[0].events[0], &alpha, &tree);
    let mut records = BTreeMap::new();
    for (group, signed) in groups.iter().zip([&alpha, &beta]) {
        apply(
            &mut records,
            &IngestBatch {
                events: group.events.clone(),
                settle_intents: vec![intents[0].intent.clone()],
                commit: signed.clone(),
            },
        )
        .unwrap();
    }
    let proof = build_trade_proof(
        &records[TRADE],
        vec![(groups[0].tenant.clone(), vec![asset])],
    )
    .unwrap();
    assert_eq!(proof.summary.value_legs.len(), 1);
    let commits = BTreeMap::from([(
        alpha.body.commit_id.0.clone(),
        (alpha, fixed_key().verifying_key()),
    )]);
    verify_trade_proof(&proof, &commits).unwrap();
    (proof, commits)
}
