#![no_main]

use libfuzzer_sys::fuzz_target;

use statechronicle_core::amount::Amount;

// `Amount::try_from_str` must never panic on arbitrary bytes: the input may be
// oversized, contain non-digit bytes, or be non-UTF-8. When a value parses, its
// canonical form must round-trip losslessly (`try_from_str(to_canonical_string)
// == Ok(amount)`).
fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(amount) = Amount::try_from_str(text) {
        let canonical = amount.to_canonical_string();
        assert_eq!(Amount::try_from_str(&canonical), Ok(amount));
    }
});
