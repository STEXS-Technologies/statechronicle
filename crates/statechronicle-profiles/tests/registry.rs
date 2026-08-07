//! Integration tests for the baseline profile registry.
//!
//! Covers registry coverage of all seven baseline state types, the paid
//! unique asset overlay, and end-to-end lifecycle checks that each rule set
//! accepts its happy path and fails closed on invalid transitions.

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use statechronicle_core::amount::Amount;
use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::ids::{CommitId, EventId};
use statechronicle_domain::intent::Operation;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::tenant::TenantId;

use statechronicle_profiles::error::ProfileError;
use statechronicle_profiles::registry::ProfileRegistry;

/// The seven baseline state types in protocol order.
const ALL_STATE_TYPES: [StateType; 7] = [
    StateType::UniqueAsset,
    StateType::ConsumableStack,
    StateType::FungibleBalance,
    StateType::Entitlement,
    StateType::MeteredResource,
    StateType::Listing,
    StateType::Escrow,
];

fn projection(state_type: StateType, state: serde_json::Value) -> StateProjection {
    StateProjection {
        tenant_id: TenantId(String::from("tenant.test")),
        resource_id: ResourceId(String::from("res:test_001")),
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

fn inputs(entries: &[(&str, serde_json::Value)]) -> BTreeMap<String, serde_json::Value> {
    entries
        .iter()
        .map(|(key, value)| (String::from(*key), value.clone()))
        .collect()
}

#[test]
fn baseline_registers_all_seven_state_types() {
    let registry = ProfileRegistry::baseline();
    for state_type in ALL_STATE_TYPES {
        let rules = registry.get(state_type);
        assert!(
            rules.is_some(),
            "state type `{state_type:?}` is not registered"
        );
        let rules = rules.unwrap();
        assert_eq!(rules.state_type(), state_type);
        assert!(!rules.allowed_operations().is_empty());
        // Every rule set is stateless and shareable across resources.
        assert_eq!(rules.profile_id(), rules.profile_id());
    }
}

#[test]
fn registry_profiles_map_to_expected_rule_sets() {
    let registry = ProfileRegistry::baseline();
    let expected = [
        (StateType::UniqueAsset, "unique_asset"),
        (StateType::ConsumableStack, "consumable_stack"),
        (StateType::FungibleBalance, "fungible_balance"),
        (StateType::Entitlement, "entitlement"),
        (StateType::MeteredResource, "meter"),
        (StateType::Listing, "listing"),
        (StateType::Escrow, "escrow"),
    ];
    for (state_type, profile_id) in expected {
        assert_eq!(registry.get(state_type).unwrap().profile_id(), profile_id);
    }
}

#[test]
fn paid_unique_asset_overlay_is_separately_registered() {
    let registry = ProfileRegistry::baseline();
    let paid = registry.paid_unique_asset();
    assert_eq!(paid.profile_id(), "paid_unique_asset");
    assert_eq!(paid.state_type(), StateType::UniqueAsset);
    // The plain and paid rules are distinct instances over the same state type.
    let plain = registry.get(StateType::UniqueAsset).unwrap();
    assert_eq!(plain.profile_id(), "unique_asset");
    assert_ne!(plain.profile_id(), paid.profile_id());
}

#[test]
fn unique_asset_lifecycle_mint_transfer_lock_unlock_burn() {
    let registry = ProfileRegistry::baseline();
    let rules = registry.get(StateType::UniqueAsset).unwrap();

    // mint: no prior state, to_owner seeds the owner.
    rules
        .check(
            &op("asset.mint"),
            None,
            &inputs(&[("to_owner", serde_json::json!("alice"))]),
        )
        .unwrap();
    let owned_by_alice = projection(
        StateType::UniqueAsset,
        serde_json::json!({ "status": "active", "owner": "alice" }),
    );

    // transfer: alice -> bob.
    rules
        .check(
            &op("asset.transfer"),
            Some(&owned_by_alice),
            &inputs(&[
                ("from_owner", serde_json::json!("alice")),
                ("to_owner", serde_json::json!("bob")),
            ]),
        )
        .unwrap();
    let owned_by_bob = projection(
        StateType::UniqueAsset,
        serde_json::json!({ "status": "active", "owner": "bob" }),
    );

    // lock then unlock.
    rules
        .check(&op("asset.lock"), Some(&owned_by_bob), &BTreeMap::new())
        .unwrap();
    let locked = projection(
        StateType::UniqueAsset,
        serde_json::json!({ "status": "locked", "owner": "bob" }),
    );
    rules
        .check(&op("asset.unlock"), Some(&locked), &BTreeMap::new())
        .unwrap();
    let unlocked = projection(
        StateType::UniqueAsset,
        serde_json::json!({ "status": "active", "owner": "bob" }),
    );

    // burn by the current owner.
    rules
        .check(
            &op("asset.burn"),
            Some(&unlocked),
            &inputs(&[("from_owner", serde_json::json!("bob"))]),
        )
        .unwrap();

    // A wrong-owner burn fails closed with a structured error.
    assert!(matches!(
        rules.check(
            &op("asset.burn"),
            Some(&unlocked),
            &inputs(&[("from_owner", serde_json::json!("alice"))])
        ),
        Err(ProfileError::OwnershipMismatch { expected, actual })
        if expected == "bob" && actual == "alice"
    ));
}

#[test]
fn fungible_balance_lifecycle_create_credit_debit() {
    let registry = ProfileRegistry::baseline();
    let rules = registry.get(StateType::FungibleBalance).unwrap();

    rules
        .check(
            &op("balance.create"),
            None,
            &inputs(&[
                ("subject", serde_json::json!("alice")),
                ("balance", serde_json::json!("100")),
                ("unit", serde_json::json!("gold_minor")),
            ]),
        )
        .unwrap();
    let balance = projection(
        StateType::FungibleBalance,
        serde_json::json!({
            "subject": "alice",
            "balance": "100",
            "unit": "gold_minor"
        }),
    );

    rules
        .check(
            &op("balance.credit"),
            Some(&balance),
            &inputs(&[("amount", serde_json::json!("50"))]),
        )
        .unwrap();
    let credited = projection(
        StateType::FungibleBalance,
        serde_json::json!({
            "subject": "alice",
            "balance": "150",
            "unit": "gold_minor"
        }),
    );

    rules
        .check(
            &op("balance.debit"),
            Some(&credited),
            &inputs(&[("amount", serde_json::json!("30"))]),
        )
        .unwrap();

    // Over-debiting fails closed.
    assert!(matches!(
        rules.check(
            &op("balance.debit"),
            Some(&credited),
            &inputs(&[("amount", serde_json::json!("151"))])
        ),
        Err(ProfileError::InsufficientQuantity { available, requested })
        if available == Amount::from_u64(150) && requested == Amount::from_u64(151)
    ));
}

#[test]
fn consumable_stack_lifecycle_create_consume() {
    let registry = ProfileRegistry::baseline();
    let rules = registry.get(StateType::ConsumableStack).unwrap();

    rules
        .check(
            &op("stack.create"),
            None,
            &inputs(&[
                ("subject", serde_json::json!("alice")),
                ("quantity", serde_json::json!("10")),
                ("unit", serde_json::json!("arrows")),
            ]),
        )
        .unwrap();
    let stack = projection(
        StateType::ConsumableStack,
        serde_json::json!({
            "subject": "alice",
            "quantity": "10",
            "unit": "arrows"
        }),
    );

    rules
        .check(
            &op("stack.consume"),
            Some(&stack),
            &inputs(&[("quantity", serde_json::json!("3"))]),
        )
        .unwrap();

    // Consuming more than available fails closed.
    assert!(matches!(
        rules.check(
            &op("stack.consume"),
            Some(&stack),
            &inputs(&[("quantity", serde_json::json!("11"))])
        ),
        Err(ProfileError::InsufficientQuantity { .. })
    ));
    // Float-formatted input is rejected even before the bound check.
    assert!(matches!(
        rules.check(
            &op("stack.consume"),
            Some(&stack),
            &inputs(&[("quantity", serde_json::json!("1.5"))])
        ),
        Err(ProfileError::FloatForbidden)
    ));
}

#[test]
fn listing_and_escrow_lifecycle_create_buy() {
    let registry = ProfileRegistry::baseline();
    let listing_rules = registry.get(StateType::Listing).unwrap();
    let escrow_rules = registry.get(StateType::Escrow).unwrap();

    listing_rules
        .check(
            &op("listing.create"),
            None,
            &inputs(&[("seller", serde_json::json!("alice"))]),
        )
        .unwrap();
    let listed = projection(
        StateType::Listing,
        serde_json::json!({ "status": "listed", "seller": "alice" }),
    );

    escrow_rules
        .check(
            &op("escrow.lock"),
            None,
            &inputs(&[
                ("buyer", serde_json::json!("bob")),
                ("seller", serde_json::json!("alice")),
            ]),
        )
        .unwrap();
    let locked = projection(
        StateType::Escrow,
        serde_json::json!({
            "status": "locked",
            "buyer": "bob",
            "seller": "alice"
        }),
    );

    // Purchase settlement advances both resources; each side validates alone.
    listing_rules
        .check(
            &op("listing.buy"),
            Some(&listed),
            &inputs(&[("buyer", serde_json::json!("bob"))]),
        )
        .unwrap();
    escrow_rules
        .check(&op("escrow.release"), Some(&locked), &BTreeMap::new())
        .unwrap();

    // A listing that is already sold cannot be bought again.
    let sold = projection(
        StateType::Listing,
        serde_json::json!({ "status": "sold", "seller": "alice" }),
    );
    assert!(matches!(
        listing_rules.check(&op("listing.buy"), Some(&sold), &inputs(&[("buyer", serde_json::json!("bob"))])),
        Err(ProfileError::InvalidTransition { from, .. }) if from == "sold"
    ));
}

#[test]
fn entitlement_and_meter_lifecycle() {
    let registry = ProfileRegistry::baseline();
    let entitlement_rules = registry.get(StateType::Entitlement).unwrap();
    let meter_rules = registry.get(StateType::MeteredResource).unwrap();

    entitlement_rules
        .check(
            &op("entitlement.grant"),
            None,
            &inputs(&[
                ("subject", serde_json::json!("alice")),
                ("transferable", serde_json::json!(true)),
            ]),
        )
        .unwrap();
    let granted = projection(
        StateType::Entitlement,
        serde_json::json!({
            "subject": "alice",
            "status": "granted",
            "transferable": true
        }),
    );
    entitlement_rules
        .check(
            &op("entitlement.activate"),
            Some(&granted),
            &BTreeMap::new(),
        )
        .unwrap();
    let active = projection(
        StateType::Entitlement,
        serde_json::json!({
            "subject": "alice",
            "status": "active",
            "transferable": true
        }),
    );
    entitlement_rules
        .check(&op("entitlement.expire"), Some(&active), &BTreeMap::new())
        .unwrap();

    meter_rules
        .check(
            &op("meter.create"),
            None,
            &inputs(&[
                ("subject", serde_json::json!("alice")),
                ("remaining", serde_json::json!("40")),
                ("maximum", serde_json::json!("100")),
            ]),
        )
        .unwrap();
    let meter = projection(
        StateType::MeteredResource,
        serde_json::json!({
            "subject": "alice",
            "remaining": "40",
            "maximum": "100"
        }),
    );
    meter_rules
        .check(
            &op("meter.consume"),
            Some(&meter),
            &inputs(&[("amount", serde_json::json!("5"))]),
        )
        .unwrap();
    meter_rules
        .check(&op("meter.refill"), Some(&meter), &BTreeMap::new())
        .unwrap();
}

#[test]
fn check_never_panics_on_garbage_payloads() {
    // Rule evaluation is fail-closed: calling it with malformed payloads and
    // arbitrary inputs yields a `Result` (never a panic) for every
    // registered rule set. Ops that only gate on resource existence (for
    // example `stack.expire`) may still return `Ok`; they never read the
    // payload.
    let registry = ProfileRegistry::baseline();
    let garbage = projection(
        StateType::UniqueAsset,
        serde_json::json!({ "not": ["a", "valid", 1.5, null, true] }),
    );
    for state_type in ALL_STATE_TYPES {
        let rules = registry.get(state_type).unwrap();
        for name in rules.allowed_operations() {
            // Panics here are the failure mode under test.
            let _with_resource = rules.check(&op(name), Some(&garbage), &BTreeMap::new());
            let _without_resource = rules.check(&op(name), None, &BTreeMap::new());
        }
    }
    // An unknown operation is rejected even on a well-formed resource.
    let rules = registry.get(StateType::UniqueAsset).unwrap();
    assert!(matches!(
        rules.check(&op("asset.hard_delete"), None, &BTreeMap::new()),
        Err(ProfileError::UnknownOperation(_))
    ));
}
