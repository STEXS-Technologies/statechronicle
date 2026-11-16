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
async fn protected_settlement_accepts_complete_declared_value_leg_without_symbolic_writes() {
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
        .prepare_player_settlement(&items, &statechronicle_test_allow::Allow, &keys(&h))
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

#[tokio::test]
async fn protected_settlement_refuses_incomplete_or_mismatched_groups_without_writes() {
    for (case, amount, recipient, bundle, omit_payment) in [
        ("missing_payment", "100", SELLER, None, true),
        ("wrong_amount", "99", SELLER, None, false),
        ("wrong_recipient", "100", "account:charlie", None, false),
        ("incomplete_bundle", "100", SELLER, Some(2), false),
        ("empty_bundle", "100", SELLER, Some(0), false),
    ] {
        let h = Harness::new();
        seed(&h).await;
        let sale = settle(&h, case, BUYER, Some("100"), bundle);
        let payment = request(
            &h,
            "payment",
            "balance.transfer",
            BUYER,
            WALLET,
            1,
            &[("to_subject", json!(recipient)), ("amount", json!(amount))],
        );
        let seller = principal(&h, SELLER);
        let buyer = principal(&h, BUYER);
        let mut items = vec![PlayerBatchItem {
            validated: &sale,
            principal: &seller,
        }];
        if !omit_payment {
            items.push(PlayerBatchItem {
                validated: &payment,
                principal: &buyer,
            });
        }
        let before = h.transactions.log();
        let result = h
            .executor
            .prepare_player_settlement(&items, &statechronicle_test_allow::Allow, &keys(&h))
            .await;
        assert!(result.is_err(), "{case} escaped: {result:?}");
        assert_eq!(h.transactions.log(), before, "{case} wrote during refusal");
    }
}

#[tokio::test]
async fn ordinary_routes_still_refuse_declared_value_legs() {
    let h = Harness::new();
    seed(&h).await;
    let sale = settle(&h, "ordinary", BUYER, Some("100"), Some(2));
    let seller = principal(&h, SELLER);
    let keys = keys(&h);
    let single = h
        .executor
        .prepare_player(&sale, &seller, &statechronicle_test_allow::Allow, &keys)
        .await;
    let batch = h
        .executor
        .prepare_player_batch(
            &[PlayerBatchItem {
                validated: &sale,
                principal: &seller,
            }],
            &statechronicle_test_allow::Allow,
            &keys,
        )
        .await;
    for error in [single.unwrap_err(), batch.unwrap_err()] {
        assert!(
            format!("{error:?}").contains("ValueLegSettleRouting"),
            "{error:?}"
        );
    }
}

fn readonly(h: &Harness) -> statechronicle::mutation::Preparer {
    use statechronicle::mutation::{PreparationConfig, PreparationPorts, Preparer};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    let counter = AtomicU64::new(1);
    Preparer::new(
        PreparationPorts {
            state_index: Box::new(h.index.clone()),
            tenant_store: Box::new(h.tenant_store.clone()),
            trustgrant: vec![Box::new(common::FakeTrustGrant::allow())],
        },
        PreparationConfig {
            profiles: statechronicle::profiles::registry::ProfileRegistry::baseline(),
            executor: common::executor_subject(),
            now: Box::new(common::fixed_now),
            event_id_fn: Box::new(move || {
                statechronicle::EventId::new(format!(
                    "evt_readonly_{}",
                    counter.fetch_add(1, Ordering::SeqCst)
                ))
                .unwrap()
            }),
            intent_verifier: Arc::new(|signature, body| {
                statechronicle::core::signature::verify(
                    body,
                    &common::fixed_key().verifying_key(),
                    &signature.sig,
                )
                .map_err(|error| {
                    statechronicle::executor::error::ExecutorError::ActorAuthenticationFailed(
                        error.to_string(),
                    )
                })
            }),
        },
    )
}

#[tokio::test]
async fn readonly_preparer_accepts_complete_bundle_and_refuses_partial_bundle() {
    let h = Harness::new();
    seed(&h).await;
    let other = "asset:second";
    for intent in [
        request(
            &h,
            "mint_second",
            "asset.mint",
            SELLER,
            other,
            0,
            &[("to_owner", json!(SELLER))],
        ),
        request(
            &h,
            "lock_second",
            "trade.lock",
            SELLER,
            other,
            1,
            &[
                ("from_owner", json!(SELLER)),
                ("trade_id", json!("trade_contested")),
            ],
        ),
    ] {
        h.run(&intent, StateType::UniqueAsset).await;
    }
    let first = settle(&h, "first_bundle", BUYER, None, Some(2));
    let second = request(
        &h,
        "second_bundle",
        "trade.settle",
        SELLER,
        other,
        2,
        &[
            ("from_owner", json!(SELLER)),
            ("to_owner", json!(BUYER)),
            ("trade_id", json!("trade_contested")),
            ("bundle_size", json!(2)),
        ],
    );
    let principal = principal(&h, SELLER);
    let items = [
        PlayerBatchItem {
            validated: &first,
            principal: &principal,
        },
        PlayerBatchItem {
            validated: &second,
            principal: &principal,
        },
    ];
    let before = h.transactions.log();
    let preparer = readonly(&h);
    assert!(
        preparer
            .prepare_player_settlement(&items[..1], &statechronicle_test_allow::Allow, &keys(&h),)
            .await
            .is_err()
    );
    let complete = preparer
        .prepare_player_settlement(&items, &statechronicle_test_allow::Allow, &keys(&h))
        .await
        .unwrap();
    assert_eq!(complete.mutations().len(), 2);
    assert_eq!(complete.events().count(), 2);
    let forwarded = h
        .executor
        .prepare_player_settlement(&items, &statechronicle_test_allow::Allow, &keys(&h))
        .await
        .unwrap();
    for (direct, forwarding) in complete.events().zip(forwarded.events()) {
        assert_eq!(direct.after, forwarding.after);
        assert_eq!(direct.intent_id, forwarding.intent_id);
        assert_eq!(direct.resource_id, forwarding.resource_id);
    }
    assert_eq!(h.transactions.log(), before);
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

#[path = "support/settlement_contenders.rs"]
mod contenders;
