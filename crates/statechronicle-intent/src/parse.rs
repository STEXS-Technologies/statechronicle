//! Parsing of raw intent payloads.
//!
//! Converts canonical JSON payloads into typed, stage-separated intent
//! documents.

use statechronicle_core::limits::{MAX_INTENT_BYTES, check_size};

use crate::error::IntentError;
use crate::raw::RawIntent;

/// Parses a canonical JSON intent payload into an unvalidated [`RawIntent`].
///
/// The payload length is first checked against [`MAX_INTENT_BYTES`];
/// oversized payloads fail closed with [`IntentError::SizeLimitExceeded`]
/// (protocol §30). The payload is then deserialized with `serde_json`; any
/// JSON failure surfaces as [`IntentError::InvalidJson`]. Field-level
/// validation happens later in [`crate::validate::validate`].
///
/// # Errors
///
/// Returns [`IntentError::SizeLimitExceeded`] when `payload` exceeds
/// [`MAX_INTENT_BYTES`] bytes, or [`IntentError::InvalidJson`] when the
/// payload is not valid JSON or does not deserialize into a [`RawIntent`].
pub fn parse_intent(payload: &[u8]) -> Result<RawIntent, IntentError> {
    check_size("intent", MAX_INTENT_BYTES, payload.len())?;
    let raw = serde_json::from_slice(payload)?;
    tracing::debug!(bytes = payload.len(), "parsed raw intent payload");
    Ok(raw)
}

/// Parses a canonical JSON intent payload string into a [`RawIntent`].
///
/// Convenience wrapper over [`parse_intent`] for UTF-8 text payloads; the
/// length check uses the string's byte length.
///
/// # Errors
///
/// Returns [`IntentError::SizeLimitExceeded`] when `payload` exceeds
/// [`MAX_INTENT_BYTES`] bytes, or [`IntentError::InvalidJson`] when the
/// payload is not valid JSON or does not deserialize into a [`RawIntent`].
pub fn parse_intent_str(payload: &str) -> Result<RawIntent, IntentError> {
    parse_intent(payload.as_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_domain::intent::INTENT_SCHEMA;

    fn sample_json() -> serde_json::Value {
        serde_json::json!({
            "schema": INTENT_SCHEMA,
            "tenant_id": "acme.game.alpha",
            "intent_id": "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2",
            "operation": "asset.transfer",
            "actor": "account:example:player_123",
            "resource_id": "asset:sword_001",
            "state_type": "unique_asset",
            "expected_version": 41,
            "inputs": {},
            "created_at": "2026-07-14T00:00:00Z",
            "expires_at": "2026-07-14T00:05:00Z",
            "nonce": "b64u:AAME",
        })
    }

    #[test]
    fn parse_intent_accepts_valid_payload() {
        let payload = serde_json::to_vec(&sample_json()).unwrap();
        let raw = parse_intent(&payload).unwrap();
        assert_eq!(raw.schema, INTENT_SCHEMA);
        assert_eq!(raw.tenant_id, "acme.game.alpha");
        assert_eq!(raw.expected_version, 41);
    }

    #[test]
    fn parse_intent_str_matches_parse_intent() {
        let text = serde_json::to_string(&sample_json()).unwrap();
        let from_bytes = parse_intent(text.as_bytes()).unwrap();
        let from_str = parse_intent_str(&text).unwrap();
        assert_eq!(from_bytes, from_str);
    }

    #[test]
    fn parse_intent_rejects_oversized_payload() {
        let payload = vec![b' '; MAX_INTENT_BYTES.saturating_add(1)];
        let error = parse_intent(&payload).unwrap_err();
        assert!(matches!(
            error,
            IntentError::SizeLimitExceeded { name, limit, actual }
            if name == "intent" && limit == MAX_INTENT_BYTES && actual == MAX_INTENT_BYTES.saturating_add(1)
        ));
    }

    #[test]
    fn parse_intent_accepts_exactly_at_limit() {
        // The size bound is inclusive; a payload exactly at the limit passes
        // the size check and then fails as JSON (b' ' is not valid JSON).
        let payload = vec![b' '; MAX_INTENT_BYTES];
        assert!(matches!(
            parse_intent(&payload),
            Err(IntentError::InvalidJson { .. })
        ));
    }

    #[test]
    fn parse_intent_rejects_invalid_json() {
        assert!(matches!(
            parse_intent(b"not json"),
            Err(IntentError::InvalidJson { .. })
        ));
        assert!(matches!(
            parse_intent(b""),
            Err(IntentError::InvalidJson { .. })
        ));
    }

    #[test]
    fn parse_intent_rejects_missing_required_fields() {
        let payload = serde_json::to_vec(&serde_json::json!({ "schema": INTENT_SCHEMA })).unwrap();
        assert!(matches!(
            parse_intent(&payload),
            Err(IntentError::InvalidJson { .. })
        ));
    }
}
