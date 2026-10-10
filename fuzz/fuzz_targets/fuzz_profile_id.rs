#![no_main]

use libfuzzer_sys::fuzz_target;
use statechronicle_core::limits::MAX_ID_LENGTH;
use statechronicle_domain::commit::ProfileId;
use std::str::{FromStr, from_utf8};

fuzz_target!(|data: &[u8]| {
    if data.len() > 4096 {
        return;
    }
    // Binary deserialization must enforce the same identity boundary as construction.
    if let Ok(decoded) = bcs::from_bytes::<ProfileId>(data) {
        assert_eq!(ProfileId::new(decoded.0.clone()).unwrap(), decoded);
        assert_eq!(bcs::to_bytes(&decoded).unwrap(), data);
    }
    let Ok(text) = from_utf8(data) else {
        return;
    };
    let expected = !text.is_empty()
        && text.chars().count() <= MAX_ID_LENGTH
        && !text.chars().any(char::is_control);
    let constructed = ProfileId::new(String::from(text));
    assert_eq!(constructed.is_ok(), expected);
    assert_eq!(ProfileId::from_str(text).is_ok(), expected);
    assert_eq!(ProfileId::try_from(text).is_ok(), expected);
    // Serialize the raw string to exercise both deserializers even for invalid names.
    let json = serde_json::to_string(text).unwrap();
    let binary = bcs::to_bytes(text).unwrap();
    let json_result = serde_json::from_str::<ProfileId>(&json);
    let binary_result = bcs::from_bytes::<ProfileId>(&binary);
    assert_eq!(json_result.is_ok(), expected);
    assert_eq!(binary_result.is_ok(), expected);
    if let Ok(identity) = constructed {
        assert_eq!(identity.as_str(), text);
        assert_eq!(json_result.unwrap(), identity);
        assert_eq!(binary_result.unwrap(), identity);
    }
});
