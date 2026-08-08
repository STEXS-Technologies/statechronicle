#![no_main]

use libfuzzer_sys::fuzz_target;

use statechronicle_core::digest::hash_bytes;
use statechronicle_domain::intent::INTENT_SCHEMA;
use statechronicle_intent::parse::parse_intent;
use statechronicle_intent::raw::RawIntent;
use statechronicle_intent::validate::validate;

// Intent parsing and validation must fail closed, never panic: the input is
// arbitrary bytes that may be oversized, not JSON, or a JSON document that
// does not deserialize into a RawIntent. Every accepted payload is then run
// through validation, which must also be total over valid RawIntent shapes.
//
// The target additionally feeds a valid intent carrying an aggregate-form
// authority block (an `authority` object with evaluation_digest/result/
// evaluated_at, protocol §12.1) whose fields derive from the input, so the
// multi-authority authority-parse path is exercised too.
fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = parse_intent(data) {
        let _ = validate(&raw);
    }

    let evaluation_digest = hash_bytes(data);
    let result = match data.first() {
        Some(byte) if byte.is_ascii_lowercase() => "Allow",
        _ => "Deny",
    };
    let raw_json = serde_json::json!({
        "schema": INTENT_SCHEMA,
        "tenant_id": "acme.game.alpha",
        "intent_id": "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2",
        "operation": "asset.transfer",
        "actor": "account:example:player_123",
        "resource_id": "asset:sword_001",
        "expected_version": 41,
        "inputs": {
            "from_owner": "account:example:player_123",
            "to_owner": "account:example:player_456",
        },
        "authority": {
            "kind": "trustgrant.evaluation",
            "evaluation_digest": evaluation_digest.as_str(),
            "result": result,
            "evaluated_at": "2026-07-14T00:00:00Z",
        },
        "created_at": "2026-07-14T00:00:00Z",
        "nonce": "b64u:AAME",
    });
    if let Ok(raw) = serde_json::from_value::<RawIntent>(raw_json) {
        let _ = validate(&raw);
    }
});
