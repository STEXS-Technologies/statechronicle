//! Property tests for the execution pipeline (protocol §18).
//!
//! Proptest strategies over arbitrary operations, state payloads, inputs, and
//! `u64` quantities assert that:
//!
//! * `transition::apply` is deterministic, so identical inputs always produce
//!   identical after-state JSON (or identical errors);
//! * after-state arithmetic is checked and never introduces floats;
//! * the full executor never panics on arbitrary intents (always `Ok` or
//!   `Err`).

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::type_complexity
)]

mod common;

use std::collections::BTreeMap;

use proptest::prelude::*;
use serde_json::json;

use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::authority::{AggregationPolicy, aggregate_evaluation_digest};
use statechronicle_domain::ids::{CommitId, EventId, IntentId};
use statechronicle_domain::intent::{Intent, Nonce, Operation};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_executor::atomicity;
use statechronicle_executor::error::ExecutorError;
use statechronicle_executor::transition;
use statechronicle_intent::validated::{IdempotencyKey, ValidatedIntent};

use common::{
    FakeTrustGrant, Harness, balance_create, balance_transfer, cross_balance_create,
    cross_balance_transfer, fixed_now, tenant,
};

/// Builds a projection over a state payload for a given state type.
fn projection(state_type: StateType, state: serde_json::Value) -> StateProjection {
    StateProjection {
        tenant_id: tenant(),
        resource_id: ResourceId(String::from("res:property")),
        state_type,
        version: 1,
        last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
        last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
        state_hash: ContentDigest::new([0u8; 32]),
        state,
    }
}

fn op(name: &str) -> Operation {
    Operation(name.to_owned())
}

fn inputs_map(entries: &[(&str, serde_json::Value)]) -> BTreeMap<String, serde_json::Value> {
    entries
        .iter()
        .map(|(key, value)| (String::from(*key), value.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Strategies.
// ---------------------------------------------------------------------------

fn arbitrary_state_type() -> impl Strategy<Value = StateType> {
    prop_oneof![
        Just(StateType::UniqueAsset),
        Just(StateType::ConsumableStack),
        Just(StateType::FungibleBalance),
        Just(StateType::Entitlement),
        Just(StateType::MeteredResource),
        Just(StateType::Listing),
        Just(StateType::Escrow),
    ]
}

fn arbitrary_operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        any::<String>().prop_map(Operation),
        Just(op("asset.mint")),
        Just(op("asset.transfer")),
        Just(op("asset.lock")),
        Just(op("stack.credit")),
        Just(op("stack.debit")),
        Just(op("stack.consume")),
        Just(op("balance.credit")),
        Just(op("balance.debit")),
        Just(op("balance.spend")),
        Just(op("meter.consume")),
        Just(op("meter.refill")),
    ]
}

fn arbitrary_state() -> impl Strategy<Value = serde_json::Value> {
    prop::collection::btree_map(
        "[a-z_]{1,12}",
        prop_oneof![
            any::<u64>().prop_map(|value| json!(value.to_string())),
            any::<String>().prop_map(|value| json!(value)),
            Just(json!(true)),
            Just(json!("active")),
        ],
        0..12,
    )
    .prop_map(|map: BTreeMap<String, serde_json::Value>| {
        serde_json::Value::Object(map.into_iter().collect())
    })
}

/// Arbitrary inputs, including raw JSON numbers, floats, and arbitrary
/// strings, to stress fail-closed parsing and canonicalization.
fn arbitrary_inputs() -> impl Strategy<Value = BTreeMap<String, serde_json::Value>> {
    prop::collection::btree_map(
        "[a-z_]{1,16}",
        prop_oneof![
            any::<u64>().prop_map(|value| json!(value.to_string())),
            any::<u64>().prop_map(|value| json!(value)),
            any::<f64>().prop_map(|value| json!(value)),
            any::<String>().prop_map(|value| json!(value)),
            Just(json!(true)),
        ],
        0..8,
    )
}

fn build_validated(
    operation: &str,
    state_type: Option<StateType>,
    expected_version: u64,
    inputs: BTreeMap<String, serde_json::Value>,
) -> ValidatedIntent {
    let body = Intent::new(
        tenant(),
        IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
        Operation(operation.to_owned()),
        SubjectId(String::from("account:stexs:player_123")),
        ResourceId(String::from("asset:property")),
        state_type,
        expected_version,
        inputs,
        None,
        fixed_now(),
        None,
        Nonce::from_bytes(vec![1]).unwrap(),
    );
    let idempotency_key = IdempotencyKey::new(
        body.tenant_id.clone(),
        body.intent_id.clone(),
        body.actor.clone(),
        body.resource_id.clone(),
        body.operation.clone(),
    );
    ValidatedIntent {
        intent: body,
        idempotency_key,
        signature: None,
    }
}

fn arbitrary_intent() -> impl Strategy<Value = ValidatedIntent> {
    (
        any::<String>(),
        prop_oneof![
            Just(Some(StateType::UniqueAsset)),
            Just(Some(StateType::ConsumableStack)),
            Just(Some(StateType::FungibleBalance)),
            Just(Some(StateType::Entitlement)),
            Just(Some(StateType::MeteredResource)),
            Just(None),
        ],
        any::<u64>(),
        arbitrary_inputs(),
    )
        .prop_map(|(operation, state_type, expected_version, mut inputs)| {
            inputs.insert(String::from("quantity"), json!(42_u64.to_string()));
            build_validated(&operation, state_type, expected_version, inputs)
        })
}

// ---------------------------------------------------------------------------
// Properties.
// ---------------------------------------------------------------------------

proptest! {
    /// `transition::apply` is a pure function: identical inputs produce
    /// identical after-state JSON (or identical errors) for every state type.
    #[test]
    fn transition_apply_is_deterministic(
        state_type in arbitrary_state_type(),
        state in arbitrary_state(),
        operation in arbitrary_operation(),
        inputs in arbitrary_inputs(),
    ) {
        let before = projection(state_type, state);
        let first = transition::apply(Some(&before), &operation, &inputs);
        let second = transition::apply(Some(&before), &operation, &inputs);
        match (first, second) {
            (Ok(first_state), Ok(second_state)) => prop_assert_eq!(first_state, second_state),
            (Err(first_error), Err(second_error)) => {
                prop_assert_eq!(first_error.to_string(), second_error.to_string())
            }
            _ => prop_assert!(false, "apply is not deterministic"),
        }
    }

    /// After-state arithmetic is checked and always yields canonical integer
    /// strings (never floats, never wraparound).
    #[test]
    fn stack_credit_is_checked_and_deterministic(
        current in any::<u64>(),
        amount in any::<u64>(),
    ) {
        let before = projection(
            StateType::ConsumableStack,
            json!({
                "subject": "account:stexs:player_123",
                "quantity": current.to_string(),
                "unit": "arrows",
            }),
        );
        let inputs = inputs_map(&[("quantity", json!(amount.to_string()))]);
        let result = transition::apply(Some(&before), &op("stack.credit"), &inputs);
        match result {
            Ok(after) => {
                let quantity = after
                    .get("quantity")
                    .and_then(serde_json::Value::as_str)
                    .expect("after-state carries a quantity string");
                prop_assert!(!quantity.contains(['.', 'e', 'E']));
                let parsed = quantity
                    .parse::<u128>()
                    .expect("quantity is a canonical u128");
                prop_assert_eq!(parsed, u128::from(current) + u128::from(amount));
            }
            Err(error) => {
                // Overflow fails closed; the error must be deterministic.
                prop_assert!(error.to_string().contains("overflow"));
            }
        }
    }
}

/// The full executor never panics on arbitrary intents: every execution is
/// either `Ok` (event emitted or idempotent replay) or `Err` (a fail-closed
/// gate), for arbitrary operation names, state types, versions, and inputs.
#[test]
fn execute_never_panics_on_arbitrary_intents() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let harness = Harness::new(FakeTrustGrant::allow());
    proptest!(|(validated in arbitrary_intent())| {
        let result = runtime.block_on(harness.executor.execute(&validated));
        prop_assert!(result.is_ok() || result.is_err());
    });
}

/// A transfer pair is always net-zero for arbitrary `u128` source balances and
/// amounts, and never panics: the source debit equals the destination credit
/// (the atomic unit of protocol §20.5 / §18.3), and the pair passes
/// `validate_batch_consistency`.
///
/// Amounts are sampled across the full `u128` range, so (unlike the former
/// `u64` ceiling in `atomicity`) values **above** `u64::MAX` (whose canonical
/// strings are far longer than `u64`) genuinely flow through the money path and
/// the widened fixed-point `Amount` accumulation.
#[test]
fn transfer_executes_atomically_net_zero_for_arbitrary_amounts() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    proptest!(|(source in 1u128..=u128::MAX, raw_amount in 1u128..=u128::MAX)| {
        let harness = Harness::new(FakeTrustGrant::allow());
        let created = runtime
            .block_on(harness.executor.execute(&balance_create(
                "create_001",
                "currency:gold",
                "alice",
                &source.to_string(),
            )))
            .unwrap();
        harness.index.apply(&created[0], StateType::FungibleBalance);

        // The profile requires 0 < amount <= source balance.
        let amount = raw_amount.min(source);
        let events = runtime.block_on(harness.executor.execute(&balance_transfer(
            "transfer_001",
            "currency:gold",
            "alice",
            "bob",
            1,
            &amount.to_string(),
        )));
        let events = match events {
            Ok(events) => events,
            Err(error) => {
                // A valid in-range transfer must succeed; surface the error.
                prop_assert!(false, "transfer failed: {error}");
                return Ok(());
            }
        };
        prop_assert_eq!(events.len(), 2);
        let source_before = events[0].before.state["balance"]
            .as_str()
            .unwrap()
            .parse::<u128>()
            .unwrap();
        let source_after = events[0].after.state["balance"]
            .as_str()
            .unwrap()
            .parse::<u128>()
            .unwrap();
        let dest_after = events[1].after.state["balance"]
            .as_str()
            .unwrap()
            .parse::<u128>()
            .unwrap();
        prop_assert_eq!(source_before.saturating_sub(source_after), dest_after);
        prop_assert_eq!(source_before.saturating_sub(source_after), amount);
        prop_assert!(atomicity::validate_batch_consistency(&events).is_ok());
    });
}

/// `transition::transfer_after_state` is deterministic and total over arbitrary
/// amounts: identical inputs always yield identical (or identically erroring)
/// destination after-state, and never panics.
#[test]
fn transfer_after_state_is_deterministic_for_arbitrary_amounts() {
    proptest!(|(source in any::<u64>(), existing in any::<u64>(), amount in any::<u64>())| {
        let source_projection = projection(
            StateType::FungibleBalance,
            json!({
                "subject": "account:stexs:player_123",
                "balance": source.to_string(),
                "unit": "gold_minor",
            }),
        );
        let destination = projection(
            StateType::FungibleBalance,
            json!({
                "subject": "account:stexs:player_456",
                "balance": existing.to_string(),
                "unit": "gold_minor",
            }),
        );
        let inputs = inputs_map(&[
            ("to_subject", json!("account:stexs:player_456")),
            ("amount", json!(amount.to_string())),
        ]);
        let first = transition::transfer_after_state(
            &source_projection,
            Some(&destination),
            &op("balance.transfer"),
            &inputs,
        );
        let second = transition::transfer_after_state(
            &source_projection,
            Some(&destination),
            &op("balance.transfer"),
            &inputs,
        );
        match (first, second) {
            (Ok(a), Ok(b)) => prop_assert_eq!(a, b),
            (Err(a), Err(b)) => prop_assert_eq!(a.to_string(), b.to_string()),
            _ => prop_assert!(false, "transfer_after_state is not deterministic"),
        }
    });
}

/// `aggregate_evaluation_digest` is total and order-independent over arbitrary
/// sub-digest sets: permutations of the same multiset yield the identical
/// digest, and the function never panics on any input (including empty sets).
#[test]
fn aggregate_digest_is_order_independent_and_total() {
    proptest!(
        |(
            digests in prop::collection::vec(any::<[u8; 32]>(), 0..16),
            policy in prop_oneof![
                Just(AggregationPolicy::RequireAll),
                Just(AggregationPolicy::AnyOf),
            ],
        )| {
            let set: Vec<ContentDigest> = digests.iter().map(|b| ContentDigest::new(*b)).collect();
            let baseline = aggregate_evaluation_digest(policy, &set);
            // Total: the digest is a valid 32-byte digest and never panics.
            prop_assert_eq!(baseline.as_bytes().len(), 32);
            let mut permutation = set.clone();
            for _ in 0..set.len() {
                permutation.rotate_left(1);
                let rerun = aggregate_evaluation_digest(policy, &permutation);
                prop_assert_eq!(
                    rerun.as_str(),
                    baseline.as_str(),
                    "aggregate digest must be independent of evaluator order"
                );
            }
        }
    );
}

/// Cross-tenant batches are atomic and deterministically grouped (protocol
/// §8.2): an `Ok` result returns one tenant-scoped group per affected tenant,
/// groups sorted by tenant, with every event scoped to its group's tenant; an
/// `Err` result is an `AtomicityViolation` with no partial groups escaping.
///
/// Arbitrary (including zero) transfer amounts are exercised so both the
/// success and fail-closed paths are covered over the full `u128` range.
#[test]
fn cross_tenant_batch_is_atomic_and_deterministically_grouped() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    proptest!(|(raw_amount in 0u128..=u128::MAX)| {
        let harness = Harness::new(FakeTrustGrant::allow());
        let alpha = TenantId(String::from("stexs.game.alpha"));
        let beta = TenantId(String::from("stexs.game.beta"));
        harness.tenant_store.register(beta.clone());

        let a = runtime
            .block_on(harness.executor.execute(&cross_balance_create(
                alpha.clone(),
                "create_a",
                "currency:gold",
                "alice",
                "100",
            )))
            .unwrap();
        harness.index.apply(&a[0], StateType::FungibleBalance);
        let b = runtime
            .block_on(harness.executor.execute(&cross_balance_create(
                beta.clone(),
                "create_b",
                "currency:gold",
                "carol",
                "100",
            )))
            .unwrap();
        harness.index.apply(&b[0], StateType::FungibleBalance);

        let amount = raw_amount.min(100);
        let intents = vec![
            cross_balance_transfer(
                alpha.clone(),
                "xt",
                "currency:gold",
                "alice",
                "bob",
                1,
                &amount.to_string(),
            ),
            cross_balance_transfer(
                beta.clone(),
                "xt",
                "currency:gold",
                "carol",
                "dave",
                1,
                &amount.to_string(),
            ),
        ];
        let result = runtime.block_on(harness.executor.execute_cross_tenant(&intents));
        match result {
            Ok(groups) => {
                prop_assert_eq!(groups.len(), 2);
                prop_assert_eq!(&groups[0].tenant, &alpha);
                prop_assert_eq!(&groups[1].tenant, &beta);
                for group in &groups {
                    // Every leg produced events for its tenant, correctly scoped.
                    prop_assert!(!group.events.is_empty());
                    for event in &group.events {
                        prop_assert_eq!(&event.tenant_id, &group.tenant);
                    }
                }
            }
            Err(error) => {
                // Fail-closed atomicity: no partial groups escape.
                prop_assert!(matches!(error, ExecutorError::AtomicityViolation(_)));
            }
        }
    });
}
