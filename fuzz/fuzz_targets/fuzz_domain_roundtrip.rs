#![no_main]

use std::str::FromStr;

use libfuzzer_sys::fuzz_target;

use statechronicle_core::canonicalize::canonicalize;

use statechronicle_domain::ids::{EventId, IntentId};
use statechronicle_domain::intent::{Nonce, Operation};
use statechronicle_domain::status::Status;

// A small domain struct built from fuzz input must canonicalize and round-trip
// through BCS without panicking. Every field is constructed via its
// Result-returning constructor, which fails closed on malformed input; inputs
// that do not validate are skipped.
#[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
struct DomainRecord {
    intent_id: IntentId,
    event_id: EventId,
    nonce: Nonce,
    operation: Operation,
    version: u64,
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }

    let mut version_bytes = [0u8; 8];
    version_bytes.copy_from_slice(&data[..8]);
    let version = u64::from_le_bytes(version_bytes);

    let nonce_bytes: Vec<u8> = data[8..].iter().take(64).copied().collect();
    let body: String = data[8..]
        .iter()
        .map(|&byte| char::from(byte % 128))
        .collect();

    let Ok(nonce) = Nonce::from_bytes(nonce_bytes) else {
        return;
    };
    let Ok(intent_id) = IntentId::new(format!("int_{body}")) else {
        return;
    };
    let Ok(event_id) = EventId::new(format!("evt_{body}")) else {
        return;
    };
    let Ok(operation) = Operation::new(body) else {
        return;
    };

    let record = DomainRecord {
        intent_id,
        event_id,
        nonce,
        operation,
        version,
    };

    if let Ok(bytes) = canonicalize(&record) {
        // Determinism: a second canonicalization yields identical bytes.
        let again = canonicalize(&record);
        assert!(again.is_ok());
        assert_eq!(bytes, again.unwrap());

        // Roundtrip: BCS decoding recovers the exact original value.
        let decoded = bcs::from_bytes::<DomainRecord>(&bytes);
        assert!(decoded.is_ok());
        assert_eq!(decoded.unwrap(), record);
    }

    // Operation/Status serde round-trip on arbitrary bytes: parse via the
    // validated newtype constructors, serialize to JSON, and assert the decoded
    // value equals the original. Never panics on non-UTF-8/empty/oversized data.
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(op) = Operation::from_str(text)
        && let Ok(json) = serde_json::to_string(&op)
        && let Ok(decoded) = serde_json::from_str::<Operation>(&json)
    {
        assert_eq!(decoded, op);
    }
    if let Ok(status) = Status::try_from_str(text)
        && let Ok(json) = serde_json::to_string(&status)
        && let Ok(decoded) = serde_json::from_str::<Status>(&json)
    {
        assert_eq!(decoded, status);
    }
});
