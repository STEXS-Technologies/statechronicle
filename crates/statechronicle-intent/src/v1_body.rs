//! Versioned BCS body codec for actor intents (ADR-009).
//!
//! The codec preserves the public legacy [`Intent`] model while encoding its
//! JSON inputs through a tagged, integer-only v1 value representation. It
//! performs no signature verification or executor dispatch.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use statechronicle_core::limits::{MAX_INTENT_BYTES, MAX_JSON_DEPTH};
use statechronicle_domain::authority::AuthorityProof;
use statechronicle_domain::ids::IntentId;
use statechronicle_domain::intent::{INTENT_SCHEMA, Intent, Nonce, Operation};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use crate::error::IntentError;

/// Schema placed in the transcript for v1 actor intent bodies.
pub const INTENT_V1_SCHEMA: &str = "statechronicle.intent.v1";

/// Converts an existing typed intent into the v1 canonical BCS body.
///
/// The returned body uses the v1 schema but retains the legacy typed `Intent`
/// API and its v0 serialization behavior.
///
/// # Errors
/// Rejects a non-v0 input schema, floating-point inputs, excessive nesting,
/// and bodies larger than [`MAX_INTENT_BYTES`].
pub fn encode_v1_intent_body(intent: &Intent) -> Result<Vec<u8>, IntentError> {
    if intent.schema != INTENT_SCHEMA {
        return Err(invalid_v1_body(
            "input intent must use the legacy typed schema",
        ));
    }
    let inputs = encode_inputs(&intent.inputs)?;
    let wire = V1IntentBody {
        schema: String::from(INTENT_V1_SCHEMA),
        tenant_id: intent.tenant_id.clone(),
        intent_id: intent.intent_id.clone(),
        operation: intent.operation.clone(),
        actor: intent.actor.clone(),
        resource_id: intent.resource_id.clone(),
        state_type: intent.state_type,
        expected_version: intent.expected_version,
        inputs,
        authority: intent.authority.clone(),
        created_at: intent.created_at,
        expires_at: intent.expires_at,
        nonce: intent.nonce.clone(),
    };
    let encoded = bcs::to_bytes(&wire).map_err(|error| invalid_v1_body(error.to_string()))?;
    check_size(encoded.len())?;
    Ok(encoded)
}

/// Decodes and canonicality-checks a v1 body, returning the existing typed
/// intent model with its legacy schema identifier for downstream validation.
///
/// The caller must retain the original v1 bytes/format evidence separately;
/// this conversion does not make the result eligible for v0 signature checks.
///
/// # Errors
/// Rejects malformed/trailing/noncanonical BCS, wrong schema, floats (which
/// have no wire variant), excessive nesting, and bodies over the size limit.
pub fn decode_v1_intent_body(bytes: &[u8]) -> Result<Intent, IntentError> {
    check_size(bytes.len())?;
    let wire: V1IntentBody =
        bcs::from_bytes(bytes).map_err(|error| invalid_v1_body(error.to_string()))?;
    if wire.schema != INTENT_V1_SCHEMA {
        return Err(invalid_v1_body("wrong v1 intent body schema"));
    }
    if wire.inputs.values().map(value_depth).max().unwrap_or(0) > MAX_JSON_DEPTH {
        return Err(invalid_v1_body("intent input nesting depth exceeds limit"));
    }
    let canonical = bcs::to_bytes(&wire).map_err(|error| invalid_v1_body(error.to_string()))?;
    if canonical != bytes {
        return Err(invalid_v1_body("intent body is not canonical BCS"));
    }
    let inputs = decode_inputs(wire.inputs)?;
    Ok(Intent {
        schema: String::from(INTENT_SCHEMA),
        tenant_id: wire.tenant_id,
        intent_id: wire.intent_id,
        operation: wire.operation,
        actor: wire.actor,
        resource_id: wire.resource_id,
        state_type: wire.state_type,
        expected_version: wire.expected_version,
        inputs,
        authority: wire.authority,
        created_at: wire.created_at,
        expires_at: wire.expires_at,
        nonce: wire.nonce,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct V1IntentBody {
    schema: String,
    tenant_id: TenantId,
    intent_id: IntentId,
    operation: Operation,
    actor: SubjectId,
    resource_id: ResourceId,
    state_type: Option<StateType>,
    expected_version: u64,
    inputs: BTreeMap<String, V1JsonValue>,
    authority: Option<AuthorityProof>,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    nonce: Nonce,
}

/// Variant order is the stable BCS enum index assigned by ADR-009.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum V1JsonValue {
    Null,
    Boolean(bool),
    Negative(i64),
    Nonnegative(u64),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

fn encode_inputs(
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<BTreeMap<String, V1JsonValue>, IntentError> {
    inputs
        .iter()
        .map(|(key, child)| Ok((key.clone(), encode_value(child, MAX_JSON_DEPTH)?)))
        .collect()
}

fn encode_value(
    input_value: &serde_json::Value,
    remaining_depth: usize,
) -> Result<V1JsonValue, IntentError> {
    let nested_depth = || {
        remaining_depth
            .checked_sub(1)
            .ok_or_else(|| invalid_v1_body("intent input nesting depth exceeds limit"))
    };
    Ok(match input_value {
        serde_json::Value::Null => V1JsonValue::Null,
        serde_json::Value::Bool(value) => V1JsonValue::Boolean(*value),
        serde_json::Value::Number(number) => {
            if let Some(integer) = number.as_i64().filter(|integer| *integer < 0) {
                V1JsonValue::Negative(integer)
            } else if let Some(integer) = number.as_u64() {
                V1JsonValue::Nonnegative(integer)
            } else {
                return Err(invalid_v1_body(
                    "floating-point inputs are not supported in v1",
                ));
            }
        }
        serde_json::Value::String(value) => V1JsonValue::String(value.clone()),
        serde_json::Value::Array(values) => {
            let depth = nested_depth()?;
            V1JsonValue::Array(
                values
                    .iter()
                    .map(|child| encode_value(child, depth))
                    .collect::<Result<_, _>>()?,
            )
        }
        serde_json::Value::Object(values) => {
            let depth = nested_depth()?;
            V1JsonValue::Object(
                values
                    .iter()
                    .map(|(key, child)| Ok((key.clone(), encode_value(child, depth)?)))
                    .collect::<Result<_, IntentError>>()?,
            )
        }
    })
}

fn decode_inputs(
    inputs: BTreeMap<String, V1JsonValue>,
) -> Result<BTreeMap<String, serde_json::Value>, IntentError> {
    inputs
        .into_iter()
        .map(|(key, child)| Ok((key, decode_value(child)?)))
        .collect()
}

fn decode_value(value: V1JsonValue) -> Result<serde_json::Value, IntentError> {
    Ok(match value {
        V1JsonValue::Null => serde_json::Value::Null,
        V1JsonValue::Boolean(value) => serde_json::Value::Bool(value),
        V1JsonValue::Negative(value) if value < 0 => {
            serde_json::Value::Number(serde_json::Number::from(value))
        }
        V1JsonValue::Negative(_) => return Err(invalid_v1_body("negative variant must be < 0")),
        V1JsonValue::Nonnegative(value) => {
            serde_json::Value::Number(serde_json::Number::from(value))
        }
        V1JsonValue::String(value) => serde_json::Value::String(value),
        V1JsonValue::Array(values) => serde_json::Value::Array(
            values
                .into_iter()
                .map(decode_value)
                .collect::<Result<_, _>>()?,
        ),
        V1JsonValue::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, child)| Ok((key, decode_value(child)?)))
                .collect::<Result<_, IntentError>>()?,
        ),
    })
}

fn value_depth(value: &V1JsonValue) -> usize {
    match value {
        V1JsonValue::Array(values) => {
            1usize.saturating_add(values.iter().map(value_depth).max().unwrap_or(0))
        }
        V1JsonValue::Object(values) => {
            1usize.saturating_add(values.values().map(value_depth).max().unwrap_or(0))
        }
        V1JsonValue::Null
        | V1JsonValue::Boolean(_)
        | V1JsonValue::Negative(_)
        | V1JsonValue::Nonnegative(_)
        | V1JsonValue::String(_) => 0,
    }
}

fn check_size(actual: usize) -> Result<(), IntentError> {
    if actual > MAX_INTENT_BYTES {
        return Err(IntentError::SizeLimitExceeded {
            name: String::from("intent v1 body"),
            limit: MAX_INTENT_BYTES,
            actual,
        });
    }
    Ok(())
}

fn invalid_v1_body(message: impl Into<String>) -> IntentError {
    IntentError::InvalidField(format!("v1 intent body: {}", message.into()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_domain::intent::Nonce;

    fn sample_intent(inputs: BTreeMap<String, serde_json::Value>) -> Intent {
        Intent::new(
            TenantId(String::from("acme.game.alpha")),
            IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            SubjectId(String::from("account:example:player_123")),
            ResourceId(String::from("asset:sword_001")),
            None,
            41,
            inputs,
            None,
            Utc::now(),
            None,
            Nonce::from_bytes(vec![1, 2, 3]).unwrap(),
        )
    }

    #[test]
    fn roundtrips_nonempty_typed_json_values() {
        let mut object = serde_json::Map::new();
        object.insert(String::from("enabled"), serde_json::json!(true));
        object.insert(String::from("count"), serde_json::json!(u64::MAX));
        object.insert(
            String::from("nested"),
            serde_json::json!([null, -7, "sword"]),
        );
        let inputs = BTreeMap::from([
            (String::from("empty"), serde_json::Value::Null),
            (String::from("object"), serde_json::Value::Object(object)),
            (String::from("positive"), serde_json::json!(7)),
        ]);
        let intent = sample_intent(inputs);
        let bytes = encode_v1_intent_body(&intent).unwrap();
        let decoded = decode_v1_intent_body(&bytes).unwrap();
        assert_eq!(decoded, intent);
        assert_eq!(encode_v1_intent_body(&decoded).unwrap(), bytes);
    }

    #[test]
    fn rejects_float_and_excessive_nesting() {
        let float_intent = sample_intent(BTreeMap::from([(
            String::from("float"),
            serde_json::json!(1.25),
        )]));
        assert!(encode_v1_intent_body(&float_intent).is_err());

        let mut nested = serde_json::Value::Null;
        for _ in 0..=MAX_JSON_DEPTH {
            nested = serde_json::Value::Array(vec![nested]);
        }
        let nested_intent = sample_intent(BTreeMap::from([(String::from("nested"), nested)]));
        assert!(encode_v1_intent_body(&nested_intent).is_err());
    }

    #[test]
    fn rejects_oversized_body_before_deserialization() {
        let bytes = vec![0; MAX_INTENT_BYTES + 1];
        assert!(matches!(
            decode_v1_intent_body(&bytes),
            Err(IntentError::SizeLimitExceeded { limit, actual, .. })
                if limit == MAX_INTENT_BYTES && actual == MAX_INTENT_BYTES + 1
        ));
    }

    #[test]
    fn rejects_noncanonical_object_order() {
        let inputs = BTreeMap::from([
            (String::from("a"), serde_json::Value::Bool(true)),
            (String::from("b"), serde_json::Value::Bool(false)),
        ]);
        let mut bytes = encode_v1_intent_body(&sample_intent(inputs)).unwrap();
        let sorted = [0x01, b'a', 0x01, 0x01, 0x01, b'b', 0x01, 0x00];
        let reordered = [0x01, b'b', 0x01, 0x00, 0x01, b'a', 0x01, 0x01];
        let index = bytes
            .windows(sorted.len())
            .position(|window| window == sorted)
            .unwrap();
        bytes
            .get_mut(index..index + sorted.len())
            .unwrap()
            .copy_from_slice(&reordered);
        assert!(decode_v1_intent_body(&bytes).is_err());
    }
}
