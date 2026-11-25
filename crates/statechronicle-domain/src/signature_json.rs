//! JSON framing for v1 signed records. Typed protocol codecs own body validation.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use statechronicle_core::{
    limits::MAX_COMMIT_BYTES,
    signature::B64U_PREFIX,
    signature_provider::{KeySetId, MAX_SIGNATURE_BYTES, SignatureError, SignatureSuiteId},
    signature_wire::V1SignedEnvelope,
};

const SCHEMA: &str = "statechronicle.signed.v1";
const MAX_BASE64_BYTES: usize = (MAX_SIGNATURE_BYTES * 4).div_ceil(3);

/// Full JSON envelope cap, including worst-case JSON escaping of a binary body.
pub const MAX_V1_JSON_ENVELOPE_BYTES: usize = 6 * MAX_COMMIT_BYTES + 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEnvelope<'input> {
    envelope_schema: String,
    #[serde(borrow)]
    body: &'input RawValue,
    #[serde(borrow)]
    signature: &'input RawValue,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawSignature {
    suite_id: String,
    key_set_id: String,
    signature_bytes: String,
}

#[derive(Serialize)]
struct OutputEnvelope<'input> {
    envelope_schema: &'static str,
    body: &'input RawValue,
    signature: RawSignature,
}

/// Parsed JSON envelope with original body text for its typed codec.
pub struct JsonSignedEnvelopeV1<'input> {
    body: &'input RawValue,
    suite_id: SignatureSuiteId,
    key_set_id: KeySetId,
    signature: Vec<u8>,
}

impl<'input> JsonSignedEnvelopeV1<'input> {
    /// The original JSON body object.
    pub fn body_json(&self) -> &'input str {
        self.body.get()
    }

    /// Combines parsed signature metadata with canonical bytes from the typed body codec.
    pub fn with_canonical_body<'body>(&'body self, body: &'body [u8]) -> V1SignedEnvelope<'body> {
        V1SignedEnvelope {
            body,
            suite_id: self.suite_id.as_str(),
            key_set_id: &self.key_set_id.0,
            signature: &self.signature,
        }
    }
}

/// Decodes a v1 JSON envelope with explicit schema and canonical signature strings.
///
/// The returned body is still untrusted. Its typed decoder constructs the
/// canonical binary body for signature verification.
///
/// # Errors
/// Rejects oversized input, malformed framing, duplicate or unknown envelope
/// fields, noncanonical encodings, and the wrong envelope schema.
pub fn decode_v1_json_envelope(input: &[u8]) -> Result<JsonSignedEnvelopeV1<'_>, SignatureError> {
    if input.len() > MAX_V1_JSON_ENVELOPE_BYTES {
        return Err(SignatureError::CapacityExceeded);
    }
    if input.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err(SignatureError::MalformedEncoding);
    }
    let envelope: RawEnvelope<'_> =
        serde_json::from_slice(input).map_err(|_error| SignatureError::MalformedEncoding)?;
    if envelope.envelope_schema != SCHEMA
        || !envelope.body.get().starts_with('{')
        || !envelope.signature.get().starts_with('{')
    {
        return Err(SignatureError::MalformedEncoding);
    }
    let raw: RawSignature = serde_json::from_str(envelope.signature.get())
        .map_err(|_error| SignatureError::MalformedEncoding)?;
    let suite_id = SignatureSuiteId::new(&raw.suite_id)?;
    if !raw
        .key_set_id
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(SignatureError::MalformedEncoding);
    }
    let mut key = [0; 32];
    hex::decode_to_slice(raw.key_set_id, &mut key)
        .map_err(|_error| SignatureError::MalformedEncoding)?;
    let encoded = raw
        .signature_bytes
        .strip_prefix(B64U_PREFIX)
        .ok_or(SignatureError::MalformedEncoding)?;
    if encoded.len() > MAX_BASE64_BYTES {
        return Err(SignatureError::CapacityExceeded);
    }
    let signature = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_error| SignatureError::MalformedEncoding)?;
    if signature.len() > MAX_SIGNATURE_BYTES {
        return Err(SignatureError::CapacityExceeded);
    }
    Ok(JsonSignedEnvelopeV1 {
        body: envelope.body,
        suite_id,
        key_set_id: KeySetId(key),
        signature,
    })
}

/// Encodes the typed JSON body and its v1 signature metadata.
///
/// # Errors
/// Rejects invalid suite identifiers, oversized records or signatures, and
/// any body that fails JSON serialization or serializes to a non-object.
pub fn encode_v1_json_envelope<T: Serialize>(
    body: &T,
    envelope: V1SignedEnvelope<'_>,
) -> Result<Vec<u8>, SignatureError> {
    if envelope.signature.len() > MAX_SIGNATURE_BYTES {
        return Err(SignatureError::CapacityExceeded);
    }
    SignatureSuiteId::new(envelope.suite_id)?;
    let raw_body = serde_json::value::to_raw_value(body)
        .map_err(|_error| SignatureError::MalformedEncoding)?;
    if !raw_body.get().starts_with('{') {
        return Err(SignatureError::MalformedEncoding);
    }
    let record = serde_json::to_vec(&OutputEnvelope {
        envelope_schema: SCHEMA,
        body: &raw_body,
        signature: RawSignature {
            suite_id: envelope.suite_id.to_owned(),
            key_set_id: hex::encode(envelope.key_set_id),
            signature_bytes: format!(
                "{B64U_PREFIX}{}",
                URL_SAFE_NO_PAD.encode(envelope.signature)
            ),
        },
    })
    .map_err(|_error| SignatureError::MalformedEncoding)?;
    if record.len() > MAX_V1_JSON_ENVELOPE_BYTES {
        return Err(SignatureError::CapacityExceeded);
    }
    Ok(record)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn json_metadata_round_trips_to_binary_verification_view() {
        let body = serde_json::json!({"operation": "test"});
        let envelope = V1SignedEnvelope {
            body: b"canonical",
            suite_id: "ed25519",
            key_set_id: &[5; 32],
            signature: &[7; 64],
        };
        let encoded = encode_v1_json_envelope(&body, envelope).unwrap();
        let decoded = decode_v1_json_envelope(&encoded).unwrap();
        assert_eq!(decoded.body_json(), r#"{"operation":"test"}"#);
        assert_eq!(decoded.with_canonical_body(envelope.body), envelope);
    }

    #[test]
    fn rejects_noncanonical_signature_strings_and_duplicate_fields() {
        let envelope = V1SignedEnvelope {
            body: b"canonical",
            suite_id: "ed25519",
            key_set_id: &[0xab; 32],
            signature: &[0],
        };
        let encoded = encode_v1_json_envelope(&serde_json::json!({}), envelope).unwrap();
        let json = String::from_utf8(encoded).unwrap();
        for invalid in [
            json.replace("b64u:AA", "b64u:AA=="),
            json.replace("b64u:AA", "b64u:AB"),
            json.replace("abab", "ABAB"),
            json.replace(r#""body":{}"#, r#""body":{},"body":{}"#),
            json.replace(
                r#""suite_id":"ed25519""#,
                r#""suite_id":"ed25519","suite_id":"ed25519""#,
            ),
            json.replace(r#""body":{}"#, r#""body":[]"#),
        ] {
            assert!(matches!(
                decode_v1_json_envelope(invalid.as_bytes()),
                Err(SignatureError::MalformedEncoding)
            ));
        }
        let too_large = json.replace(
            "b64u:AA",
            &format!("b64u:{}", "A".repeat(MAX_BASE64_BYTES + 1)),
        );
        assert!(matches!(
            decode_v1_json_envelope(too_large.as_bytes()),
            Err(SignatureError::CapacityExceeded)
        ));
    }
}
