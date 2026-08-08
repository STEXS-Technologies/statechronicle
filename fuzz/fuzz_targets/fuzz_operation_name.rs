#![no_main]

use std::str::FromStr;

use libfuzzer_sys::fuzz_target;

use statechronicle_domain::intent::Operation;

use statechronicle_profiles::consumable_stack::op as stack_op;
use statechronicle_profiles::entitlement::op as entitlement_op;
use statechronicle_profiles::fungible_balance::op as balance_op;
use statechronicle_profiles::marketplace::op as marketplace_op;
use statechronicle_profiles::meter::op as meter_op;
use statechronicle_profiles::paid_unique_asset::op as paid_op;
use statechronicle_profiles::unique_asset::op as asset_op;

/// The executor-imported and per-profile operation accessors shipped by the
/// newtype migration. Holding them as a static array of accessor functions lets
/// a single loop assert that parsing each const's canonical name back through
/// the validator yields the const itself.
const OP_ACCESSORS: &[fn() -> &'static Operation] = &[
    // unique_asset (asset_op).
    asset_op::asset_mint,
    asset_op::asset_transfer,
    asset_op::asset_burn,
    asset_op::asset_lock,
    asset_op::asset_unlock,
    asset_op::asset_redeem,
    asset_op::asset_list,
    asset_op::asset_delist,
    asset_op::asset_escrow,
    asset_op::asset_release,
    asset_op::asset_attach_content,
    asset_op::asset_detach_content,
    asset_op::asset_update_metadata,
    asset_op::asset_restrict,
    asset_op::asset_restore,
    asset_op::trade_lock,
    asset_op::trade_unlock,
    asset_op::trade_settle,
    // consumable_stack (stack_op).
    stack_op::stack_create,
    stack_op::stack_credit,
    stack_op::stack_debit,
    stack_op::stack_consume,
    stack_op::stack_transfer,
    stack_op::stack_reserve,
    stack_op::stack_release,
    stack_op::stack_expire,
    stack_op::stack_adjust,
    // entitlement (entitlement_op).
    entitlement_op::entitlement_grant,
    entitlement_op::entitlement_activate,
    entitlement_op::entitlement_suspend,
    entitlement_op::entitlement_restore,
    entitlement_op::entitlement_expire,
    entitlement_op::entitlement_revoke,
    entitlement_op::entitlement_transfer,
    // fungible_balance (balance_op).
    balance_op::balance_create,
    balance_op::balance_mint,
    balance_op::balance_credit,
    balance_op::balance_debit,
    balance_op::balance_transfer,
    balance_op::balance_reserve,
    balance_op::balance_release,
    balance_op::balance_spend,
    balance_op::balance_burn,
    balance_op::balance_convert,
    // meter (meter_op).
    meter_op::meter_create,
    meter_op::meter_consume,
    meter_op::meter_refill,
    meter_op::meter_set_maximum,
    meter_op::meter_reset,
    meter_op::meter_expire,
    // marketplace (marketplace_op).
    marketplace_op::listing_create,
    marketplace_op::listing_cancel,
    marketplace_op::listing_buy,
    marketplace_op::listing_expire,
    marketplace_op::escrow_lock,
    marketplace_op::escrow_release,
    marketplace_op::escrow_refund,
    // paid_unique_asset (paid_op).
    paid_op::asset_mint,
    paid_op::asset_transfer,
    paid_op::asset_burn,
    paid_op::asset_lock,
    paid_op::asset_unlock,
    paid_op::asset_redeem,
    paid_op::asset_list,
    paid_op::asset_delist,
    paid_op::asset_escrow,
    paid_op::asset_release,
    paid_op::asset_attach_content,
    paid_op::asset_detach_content,
    paid_op::asset_update_metadata,
    paid_op::asset_restrict,
    paid_op::asset_restore,
    paid_op::asset_hard_delete,
    paid_op::trade_lock,
    paid_op::trade_unlock,
    paid_op::trade_settle,
];

// `Operation::new`/`Operation::from_str` must never panic on arbitrary bytes:
// the input may be non-UTF-8, empty, or oversized. When a value parses, its
// canonical form must round-trip losslessly and equal the validated input, and
// every known const's name must parse back to the const itself.
fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    if let Ok(op) = Operation::from_str(text) {
        // Round-trip: parsing the canonical form yields the identical value.
        assert!(matches!(Operation::from_str(op.as_str()), Ok(ref r) if r == &op));
        // Canonical identity: the stored name is exactly the validated input.
        assert_eq!(op.as_str(), text);
    }

    // Every known op const's name must parse back to the const itself.
    for accessor in OP_ACCESSORS {
        let op = accessor();
        assert!(matches!(Operation::from_str(op.as_str()), Ok(ref r) if r == op));
    }
});
