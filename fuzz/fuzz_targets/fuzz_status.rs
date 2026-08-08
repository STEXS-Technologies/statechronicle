#![no_main]

use libfuzzer_sys::fuzz_target;

use statechronicle_core::limits::MAX_ID_LENGTH;
use statechronicle_domain::status::Status;

use statechronicle_profiles::entitlement::status as entitlement_status;
use statechronicle_profiles::marketplace::escrow_status;
use statechronicle_profiles::marketplace::listing_status;
use statechronicle_profiles::paid_unique_asset::exceptional_status;
use statechronicle_profiles::unique_asset::status as asset_status;

/// The per-profile status accessors shipped by the newtype migration. Holding
/// them as a static array of accessor functions lets a single loop assert that
/// parsing each const's canonical name back through the validator yields the
/// const itself.
const STATUS_ACCESSORS: &[fn() -> &'static Status] = &[
    // unique_asset (asset_status).
    asset_status::active,
    asset_status::locked,
    asset_status::listed,
    asset_status::escrowed,
    asset_status::redeemed,
    asset_status::burned,
    asset_status::trade_held,
    asset_status::restricted,
    asset_status::quarantined,
    asset_status::unsupported,
    asset_status::tombstoned,
    // entitlement (entitlement_status).
    entitlement_status::granted,
    entitlement_status::active,
    entitlement_status::suspended,
    entitlement_status::expired,
    entitlement_status::revoked,
    // marketplace listing (listing_status).
    listing_status::listed,
    listing_status::cancelled,
    listing_status::sold,
    listing_status::expired,
    // marketplace escrow (escrow_status).
    escrow_status::locked,
    escrow_status::released,
    escrow_status::refunded,
    // paid_unique_asset exceptional statuses (exceptional_status).
    exceptional_status::legal_hold,
    exceptional_status::fraud_lock,
    exceptional_status::policy_restricted,
];

// `Status::try_from_str`/`Status::new` must never panic on arbitrary bytes: the
// input may be non-UTF-8, empty, or oversized. Empty and oversized inputs must
// fail closed. When a value parses, its canonical form must round-trip
// losslessly, and every known const's name must parse back to the const itself.
fuzz_target!(|data: &[u8]| {
    // Empty status names always fail closed.
    assert!(Status::try_from_str("").is_err());

    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    // Oversized status names (exceeding the id length bound) fail closed.
    if text.len() > MAX_ID_LENGTH {
        assert!(Status::try_from_str(text).is_err());
        return;
    }

    if let Ok(status) = Status::try_from_str(text) {
        // Round-trip: parsing the canonical form yields the identical value.
        assert!(matches!(
            Status::try_from_str(status.as_str()),
            Ok(ref r) if r == &status
        ));
    }

    // Every known status const's name must parse back to the const itself.
    for accessor in STATUS_ACCESSORS {
        let status = accessor();
        assert!(matches!(
            Status::try_from_str(status.as_str()),
            Ok(ref r) if r == status
        ));
    }
});
