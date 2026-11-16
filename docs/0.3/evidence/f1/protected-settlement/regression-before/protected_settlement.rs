//! Protected settlement routing, complete declarations and canonical contention.
#![allow(clippy::unwrap_used, clippy::panic, clippy::too_many_arguments)]
mod common;
use common::Harness;
use serde_json::{Value, json};
use statechronicle::domain::intent::Nonce;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::ports::{
    authorization::AuthenticatedPrincipal,
    key_registry::{KeyRecord, KeyStatus, MemoryKeyRegistry},
};
use statechronicle::{
    Intent, IntentId, Operation, PlayerBatchItem, ResourceId, StateType, SubjectId,
};

const SELLER: &str = "account:alice";
const BUYER: &str = "account:bob";
const ASSET: &str = "asset:contested";
const WALLET: &str = "wallet:gold";

fn request(
    h: &Harness,
    id: &str,
    operation: &'static str,
    actor: &str,
    resource: &str,
    version: u64,
    inputs: &[(&str, Value)],
) -> ValidatedIntent {
    let mut signed = h.sign(
        Intent::builder()
            .tenant(h.tenant())
            .intent_id(IntentId::new(format!("int_{id}")).unwrap())
            .operation(Operation::from_static(operation))
            .actor(SubjectId(actor.into()))
            .resource(ResourceId(resource.into()))
            .state_type(if operation.starts_with("balance.") {
                StateType::FungibleBalance
            } else {
                StateType::UniqueAsset
            })
            .expected_version(version)
            .created_at(h.now())
            .nonce(Nonce::from_bytes(id.as_bytes().to_vec()).unwrap())
            .inputs(
                inputs
                    .iter()
                    .map(|(k, v)| (String::from(*k), v.clone()))
                    .collect(),
            )
            .build()
            .unwrap(),
        Some(h.authority()),
    );
    signed.signature.as_mut().unwrap().key_id = actor_key(actor);
    signed
}
fn principal(h: &Harness, actor: &str) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: SubjectId(actor.into()),
        tenant: h.tenant(),
        credential_id: "test-session".into(),
    }
}
fn actor_key(actor: &str) -> statechronicle::domain::intent::KeyId {
    statechronicle::domain::intent::KeyId::new(format!("fixture:{actor}")).unwrap()
}
fn keys(h: &Harness) -> MemoryKeyRegistry {
    let keys = MemoryKeyRegistry::new();
    // Fixed fixture public material, independently scoped key metadata per principal.
    for actor in [SELLER, BUYER] {
        keys.register(KeyRecord {
            key_id: actor_key(actor),
            tenant: h.tenant(),
            subject: Some(SubjectId(actor.into())),
            operations: vec![
                Operation::from_static("trade.settle"),
                Operation::from_static("balance.transfer"),
            ],
            valid_from: h.now(),
            valid_until: None,
            status: KeyStatus::Active,
        })
        .unwrap();
    }
    keys
}
async fn seed(h: &Harness) {
    for intent in [
        request(
            h,
            "mint",
            "asset.mint",
            SELLER,
            ASSET,
            0,
            &[("to_owner", json!(SELLER))],
        ),
        request(
            h,
            "lock",
            "trade.lock",
            SELLER,
            ASSET,
            1,
            &[
                ("from_owner", json!(SELLER)),
                ("trade_id", json!("trade_contested")),
            ],
        ),
        request(
            h,
            "wallet",
            "balance.create",
            BUYER,
            WALLET,
            0,
            &[
                ("subject", json!(BUYER)),
                ("balance", json!("1000")),
                ("unit", json!("gold")),
            ],
        ),
    ] {
        let kind = intent.intent.state_type.unwrap();
        h.run(&intent, kind).await;
    }
}
fn settle(
    h: &Harness,
    id: &str,
    buyer: &str,
    price: Option<&str>,
    bundle: Option<u64>,
) -> ValidatedIntent {
    let mut inputs = vec![
        ("from_owner", json!(SELLER)),
        ("to_owner", json!(buyer)),
        ("trade_id", json!("trade_contested")),
    ];
    if let Some(price) = price {
        inputs.extend([
            ("value_resource", json!(WALLET)),
            ("value_amount", json!(price)),
            ("value_to_subject", json!(SELLER)),
        ]);
    }
    if let Some(size) = bundle {
        inputs.push(("bundle_size", json!(size)));
    }
    request(h, id, "trade.settle", SELLER, ASSET, 2, &inputs)
}

#[tokio::test]
async fn protected_batch_accepts_complete_declared_value_leg_without_symbolic_writes() {
    let h = Harness::new();
    seed(&h).await;
    let settle = settle(&h, "sale", BUYER, Some("100"), None);
    let payment = request(
        &h,
        "payment",
        "balance.transfer",
        BUYER,
        WALLET,
        1,
        &[("to_subject", json!(SELLER)), ("amount", json!("100"))],
    );
    let seller = principal(&h, SELLER);
    let buyer = principal(&h, BUYER);
    let items = [
        PlayerBatchItem {
            validated: &settle,
            principal: &seller,
        },
        PlayerBatchItem {
            validated: &payment,
            principal: &buyer,
        },
    ];
    let before = h.transactions.log();
    let prepared = h
        .executor
        .prepare_player_batch(&items, &statechronicle_test_allow::Allow, &keys(&h))
        .await
        .unwrap();
    assert_eq!(prepared.mutations().len(), 2);
    assert_eq!(prepared.events().count(), 3);
    assert_eq!(h.transactions.log(), before);
}

#[tokio::test]
async fn incomplete_bundle_cannot_escape_single_or_batch_protected_preparation() {
    let h = Harness::new();
    seed(&h).await;
    let request = settle(&h, "incomplete", BUYER, None, Some(2));
    let principal = principal(&h, SELLER);
    let keys = keys(&h);
    let single = h
        .executor
        .prepare_player(
            &request,
            &principal,
            &statechronicle_test_allow::Allow,
            &keys,
        )
        .await;
    let batch = h
        .executor
        .prepare_player_batch(
            &[PlayerBatchItem {
                validated: &request,
                principal: &principal,
            }],
            &statechronicle_test_allow::Allow,
            &keys,
        )
        .await;
    assert!(
        single.is_err() && batch.is_err(),
        "incomplete bundle escaped: single={single:?}, batch={batch:?}"
    );
}

mod statechronicle_test_allow {
    use async_trait::async_trait;
    use statechronicle::ports::authorization::{
        AuthorizationContext, AuthorizationError, Authorizer,
    };
    pub struct Allow;
    #[async_trait]
    impl Authorizer for Allow {
        async fn authorize(&self, _: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
            Ok(())
        }
    }
}
