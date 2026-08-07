//! Property tests (proptest) for the profile rule sets.
//!
//! The profile rule gate is fail-closed and total: for any operation name, any
//! state payload, and any input map, `check` must never panic. When it does
//! return `Ok`, the operation must be in the profile's allow list, and the
//! quantity-bounded operations must respect the current quantity.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;

use proptest::prelude::*;

use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::ids::{CommitId, EventId};
use statechronicle_domain::intent::Operation;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::tenant::TenantId;

use statechronicle_profiles::registry::{ProfileRegistry, ProfileRules};

fn registry() -> ProfileRegistry {
    ProfileRegistry::baseline()
}

fn projection(state_type: StateType, state: serde_json::Value) -> StateProjection {
    StateProjection {
        tenant_id: TenantId(String::from("tenant.test")),
        resource_id: ResourceId(String::from("res:prop_001")),
        state_type,
        version: 1,
        last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
        last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
        state_hash: ContentDigest::new([0u8; 32]),
        state,
    }
}

fn op(name: &str) -> Operation {
    Operation::new(String::from(name)).unwrap()
}

/// An arbitrary JSON document of bounded depth and size.
fn json_value() -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::Bool),
        any::<i64>().prop_map(serde_json::Value::from),
        any::<f64>().prop_map(serde_json::Value::from),
        any::<String>().prop_map(serde_json::Value::String),
    ];
    leaf.prop_recursive(4, 16, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(serde_json::Value::Array),
            prop::collection::btree_map(any::<String>(), inner, 0..4)
                .prop_map(|entries| serde_json::Value::Object(entries.into_iter().collect())),
        ]
    })
}

/// An arbitrary input map for a rule check.
fn input_map() -> impl Strategy<Value = BTreeMap<String, serde_json::Value>> {
    prop::collection::btree_map(any::<String>(), json_value(), 0..4)
}

/// Asserts the fail-closed and allow-list properties for one rule set.
fn check_property(
    rules: &dyn ProfileRules,
    op_name: &str,
    payload: serde_json::Value,
    input_map: &BTreeMap<String, serde_json::Value>,
) {
    let operation = Operation(String::from(op_name));
    let current = Some(projection(rules.state_type(), payload));

    let with_resource = rules.check(&operation, current.as_ref(), input_map);
    if with_resource.is_ok() {
        assert!(
            rules.allowed_operations().contains(&op_name),
            "Ok must imply `{op_name}` is allowed"
        );
    }

    let without_resource = rules.check(&operation, None, input_map);
    if without_resource.is_ok() {
        assert!(
            rules.allowed_operations().contains(&op_name),
            "Ok (no prior state) must imply `{op_name}` is allowed"
        );
    }
}

proptest! {
    #[test]
    fn unique_asset_property(op_name in any::<String>(), payload in json_value(), input_map in input_map()) {
        let rules = registry().get(StateType::UniqueAsset).unwrap();
        check_property(rules, &op_name, payload, &input_map);
    }

    #[test]
    fn paid_unique_asset_property(op_name in any::<String>(), payload in json_value(), input_map in input_map()) {
        let rules = registry().paid_unique_asset();
        check_property(rules, &op_name, payload, &input_map);
    }

    #[test]
    fn consumable_stack_property(op_name in any::<String>(), payload in json_value(), input_map in input_map()) {
        let rules = registry().get(StateType::ConsumableStack).unwrap();
        check_property(rules, &op_name, payload, &input_map);
    }

    #[test]
    fn fungible_balance_property(op_name in any::<String>(), payload in json_value(), input_map in input_map()) {
        let rules = registry().get(StateType::FungibleBalance).unwrap();
        check_property(rules, &op_name, payload, &input_map);
    }

    #[test]
    fn entitlement_property(op_name in any::<String>(), payload in json_value(), input_map in input_map()) {
        let rules = registry().get(StateType::Entitlement).unwrap();
        check_property(rules, &op_name, payload, &input_map);
    }

    #[test]
    fn meter_property(op_name in any::<String>(), payload in json_value(), input_map in input_map()) {
        let rules = registry().get(StateType::MeteredResource).unwrap();
        check_property(rules, &op_name, payload, &input_map);
    }

    #[test]
    fn listing_property(op_name in any::<String>(), payload in json_value(), input_map in input_map()) {
        let rules = registry().get(StateType::Listing).unwrap();
        check_property(rules, &op_name, payload, &input_map);
    }

    #[test]
    fn escrow_property(op_name in any::<String>(), payload in json_value(), input_map in input_map()) {
        let rules = registry().get(StateType::Escrow).unwrap();
        check_property(rules, &op_name, payload, &input_map);
    }

    // Quantity invariants: a bounded operation that passes must respect the
    // current quantity.

    #[test]
    fn stack_quantity_invariants(quantity in "[0-9]{1,6}", amount in "[0-9]{1,6}") {
        let current_qty = quantity.parse::<u64>().unwrap();
        let amount_qty = amount.parse::<u64>().unwrap();
        let stack = projection(
            StateType::ConsumableStack,
            serde_json::json!({ "subject": "alice", "quantity": quantity, "unit": "arrows" }),
        );
        let inputs = BTreeMap::from([(String::from("quantity"), serde_json::json!(amount))]);
        let rules = registry().get(StateType::ConsumableStack).unwrap();
        for name in ["stack.debit", "stack.consume", "stack.reserve"] {
            if let Ok(()) = rules.check(&op(name), Some(&stack), &inputs) {
                assert!(amount_qty > 0, "{name} amount must be positive");
                assert!(amount_qty <= current_qty, "{name} amount must not exceed quantity");
            }
        }
    }

    #[test]
    fn fungible_quantity_invariants(balance in "[0-9]{1,6}", amount in "[0-9]{1,6}") {
        let current_balance = balance.parse::<u64>().unwrap();
        let amount_qty = amount.parse::<u64>().unwrap();
        let projected = projection(
            StateType::FungibleBalance,
            serde_json::json!({ "subject": "alice", "balance": balance, "unit": "gold_minor" }),
        );
        let inputs = BTreeMap::from([(String::from("amount"), serde_json::json!(amount))]);
        let rules = registry().get(StateType::FungibleBalance).unwrap();
        for name in ["balance.debit", "balance.spend", "balance.reserve"] {
            if let Ok(()) = rules.check(&op(name), Some(&projected), &inputs) {
                assert!(amount_qty > 0, "{name} amount must be positive");
                assert!(amount_qty <= current_balance, "{name} amount must not exceed balance");
            }
        }
    }

    #[test]
    fn meter_consume_invariants(remaining in "[0-9]{1,6}", maximum in "[0-9]{1,6}", amount in "[0-9]{1,6}") {
        let current_remaining = remaining.parse::<u64>().unwrap();
        let amount_qty = amount.parse::<u64>().unwrap();
        let projected = projection(
            StateType::MeteredResource,
            serde_json::json!({ "subject": "alice", "remaining": remaining, "maximum": maximum }),
        );
        let inputs = BTreeMap::from([(String::from("amount"), serde_json::json!(amount))]);
        let rules = registry().get(StateType::MeteredResource).unwrap();
        if let Ok(()) = rules.check(&op("meter.consume"), Some(&projected), &inputs) {
            assert!(amount_qty > 0, "meter.consume amount must be positive");
            assert!(amount_qty <= current_remaining, "meter.consume amount must not exceed remaining");
        }
    }

    #[test]
    fn meter_create_invariants(remaining in "[0-9]{1,6}", maximum in "[0-9]{1,6}") {
        let remaining_qty = remaining.parse::<u64>().unwrap();
        let maximum_qty = maximum.parse::<u64>().unwrap();
        let inputs = BTreeMap::from([
            (String::from("subject"), serde_json::json!("alice")),
            (String::from("remaining"), serde_json::json!(remaining)),
            (String::from("maximum"), serde_json::json!(maximum)),
        ]);
        let rules = registry().get(StateType::MeteredResource).unwrap();
        if let Ok(()) = rules.check(&op("meter.create"), None, &inputs) {
            assert!(remaining_qty <= maximum_qty, "meter.create remaining must not exceed maximum");
        }
    }
}
