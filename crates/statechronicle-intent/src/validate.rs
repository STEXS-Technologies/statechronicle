//! Validation of parsed intents.
//!
//! Schema validation, canonicalization, and intent-id idempotency checks that
//! produce a `ValidatedIntent`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use statechronicle_core::signature::Signature;
use statechronicle_domain::authority::AuthorityProof;
use statechronicle_domain::ids::IntentId;
use statechronicle_domain::intent::{
    INTENT_SCHEMA, Intent, KeyId, Nonce, Operation, SignatureAlg, SignatureBlock,
};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use crate::error::IntentError;
use crate::raw::{RawIntent, RawSignature};
use crate::validated::{IdempotencyKey, ValidatedIntent};

/// Validates a parsed [`RawIntent`] into a [`ValidatedIntent`].
///
/// The validation stage checks, in order:
///
/// 1. the `schema` field equals [`INTENT_SCHEMA`];
/// 2. every id, operation, key id, and nonce constructs through its validated
///    newtype;
/// 3. `state_type` and `authority` parse via `serde_json` from their raw
///    values;
/// 4. `expires_at`, when present, is strictly after `created_at` (§11.2);
/// 5. the optional signature block parses into a [`SignatureBlock`].
///
/// # Errors
///
/// Returns [`IntentError::InvalidSchema`] when `raw.schema` is not
/// [`INTENT_SCHEMA`], [`IntentError::InvalidExpiry`] when `expires_at` is not
/// after `created_at`, [`IntentError::InvalidField`] when a field fails its
/// type or protocol check, or [`IntentError::Domain`] when a domain newtype
/// constructor rejects a value.
pub fn validate(raw: &RawIntent) -> Result<ValidatedIntent, IntentError> {
    if raw.schema != INTENT_SCHEMA {
        return Err(IntentError::InvalidSchema {
            found: raw.schema.clone(),
            expected: String::from(INTENT_SCHEMA),
        });
    }

    let tenant_id = TenantId(raw.tenant_id.clone());
    let intent_id = IntentId::new(raw.intent_id.clone())?;
    let operation = Operation::new(raw.operation.clone())?;
    let actor = SubjectId(raw.actor.clone());
    let resource_id = ResourceId(raw.resource_id.clone());

    let state_type = raw
        .state_type
        .as_ref()
        .map(|value| {
            serde_json::from_value::<StateType>(value.clone()).map_err(|source| {
                IntentError::InvalidField(format!("invalid state_type: {source}"))
            })
        })
        .transpose()?;

    let authority = raw
        .authority
        .as_ref()
        .map(|value| {
            serde_json::from_value::<AuthorityProof>(value.clone()).map_err(|source| {
                IntentError::InvalidField(format!("invalid authority proof: {source}"))
            })
        })
        .transpose()?;

    let created_at = parse_timestamp("created_at", &raw.created_at)?;
    let expires_at = raw
        .expires_at
        .as_ref()
        .map(|value| parse_timestamp("expires_at", value))
        .transpose()?;

    if let Some(expiry) = &expires_at
        && *expiry <= created_at
    {
        return Err(IntentError::InvalidExpiry(format!(
            "expires_at `{expiry}` must be after created_at `{created_at}`"
        )));
    }

    let nonce = Nonce::from_b64u_str(&raw.nonce)?;
    let signature = raw.signature.as_ref().map(parse_signature).transpose()?;

    let inputs: BTreeMap<String, serde_json::Value> = raw.inputs.clone();
    let intent = Intent::new(
        tenant_id.clone(),
        intent_id.clone(),
        operation.clone(),
        actor.clone(),
        resource_id.clone(),
        state_type,
        raw.expected_version,
        inputs,
        authority,
        created_at,
        expires_at,
        nonce,
    );

    let idempotency_key = IdempotencyKey::new(tenant_id, intent_id, actor, resource_id, operation);

    tracing::debug!(intent_id = %idempotency_key.intent_id, "validated intent");
    Ok(ValidatedIntent {
        intent,
        idempotency_key,
        signature,
    })
}

/// Parses an RFC 3339 timestamp string into a UTC [`DateTime`].
///
/// # Errors
///
/// Returns [`IntentError::InvalidField`] when `value` is not a valid RFC 3339
/// timestamp.
fn parse_timestamp(field: &str, value: &str) -> Result<DateTime<Utc>, IntentError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|source| IntentError::InvalidField(format!("invalid {field} `{value}`: {source}")))
}

/// Parses a raw signature block into a validated [`SignatureBlock`].
///
/// # Errors
///
/// Returns [`IntentError::InvalidField`] when the algorithm is unsupported,
/// the key id is malformed, or the signature is not a valid `b64u:` Ed25519
/// signature.
fn parse_signature(raw: &RawSignature) -> Result<SignatureBlock, IntentError> {
    let alg = match raw.alg.as_str() {
        "ed25519" => SignatureAlg::Ed25519,
        other => {
            return Err(IntentError::InvalidField(format!(
                "unsupported signature alg `{other}`"
            )));
        }
    };
    let key_id = KeyId::new(raw.key_id.clone())?;
    let sig = serde_json::from_value::<Signature>(serde_json::Value::String(raw.sig.clone()))
        .map_err(|source| {
            IntentError::InvalidField(format!("invalid signature `{}`: {source}", raw.sig))
        })?;
    Ok(SignatureBlock { alg, key_id, sig })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_domain::intent::Nonce;

    fn sample_raw() -> RawIntent {
        RawIntent {
            schema: String::from(INTENT_SCHEMA),
            tenant_id: String::from("acme.game.alpha"),
            intent_id: String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2"),
            operation: String::from("asset.transfer"),
            actor: String::from("account:example:player_123"),
            resource_id: String::from("asset:sword_001"),
            state_type: Some(serde_json::json!("unique_asset")),
            expected_version: 41,
            inputs: BTreeMap::from([(
                String::from("to_owner"),
                serde_json::json!("account:example:player_456"),
            )]),
            authority: None,
            created_at: String::from("2026-07-14T00:00:00Z"),
            expires_at: Some(String::from("2026-07-14T00:05:00Z")),
            nonce: String::from("b64u:AAME"),
            signature: None,
        }
    }

    #[test]
    fn validate_accepts_valid_raw_intent() {
        let validated = validate(&sample_raw()).unwrap();
        assert_eq!(validated.intent.schema, INTENT_SCHEMA);
        assert_eq!(validated.intent.expected_version, 41);
        assert_eq!(
            validated.idempotency_key.intent_id.as_str(),
            "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2"
        );
        assert_eq!(
            validated.idempotency_key.operation.as_str(),
            "asset.transfer"
        );
        assert_eq!(validated.idempotency_key.tenant_id.0, "acme.game.alpha");
        assert!(validated.signature.is_none());
    }

    #[test]
    fn validate_rejects_wrong_schema() {
        let mut raw = sample_raw();
        raw.schema = String::from("statechronicle.intent.v999");
        let error = validate(&raw).unwrap_err();
        assert!(matches!(
            error,
            IntentError::InvalidSchema { found, expected }
            if found == "statechronicle.intent.v999" && expected == INTENT_SCHEMA
        ));
    }

    #[test]
    fn validate_rejects_invalid_state_type() {
        let mut raw = sample_raw();
        raw.state_type = Some(serde_json::json!("profile_custom"));
        assert!(matches!(
            validate(&raw),
            Err(IntentError::InvalidField(message))
            if message.contains("state_type")
        ));
    }

    #[test]
    fn validate_accepts_missing_state_type() {
        let mut raw = sample_raw();
        raw.state_type = None;
        let validated = validate(&raw).unwrap();
        assert!(validated.intent.state_type.is_none());
    }

    #[test]
    fn validate_rejects_expiry_at_creation_time() {
        let mut raw = sample_raw();
        raw.expires_at = Some(String::from("2026-07-14T00:00:00Z"));
        assert!(matches!(validate(&raw), Err(IntentError::InvalidExpiry(_))));
    }

    #[test]
    fn validate_rejects_expiry_before_creation_time() {
        let mut raw = sample_raw();
        raw.expires_at = Some(String::from("2026-07-13T23:59:59Z"));
        assert!(matches!(validate(&raw), Err(IntentError::InvalidExpiry(_))));
    }

    #[test]
    fn validate_accepts_expiry_after_creation_time() {
        let validated = validate(&sample_raw()).unwrap();
        let expires_at = validated.intent.expires_at.unwrap();
        let created_at = validated.intent.created_at;
        assert!(expires_at > created_at);
    }

    #[test]
    fn validate_rejects_invalid_intent_id() {
        let mut raw = sample_raw();
        raw.intent_id = String::from("no_prefix");
        assert!(matches!(validate(&raw), Err(IntentError::Domain(_))));
    }

    #[test]
    fn validate_rejects_invalid_nonce() {
        let mut raw = sample_raw();
        raw.nonce = String::from("not-a-b64u-nonce");
        assert!(matches!(
            validate(&raw),
            Err(IntentError::Domain(
                statechronicle_domain::error::DomainError::InvalidNonce(_)
            ))
        ));
    }

    #[test]
    fn validate_rejects_invalid_timestamp() {
        let mut raw = sample_raw();
        raw.created_at = String::from("yesterday");
        assert!(matches!(
            validate(&raw),
            Err(IntentError::InvalidField(message))
            if message.contains("created_at")
        ));
    }

    #[test]
    fn validate_rejects_unsupported_signature_alg() {
        let mut raw = sample_raw();
        raw.signature = Some(RawSignature {
            alg: String::from("rsa"),
            key_id: String::from("did:key:z6Mk...#key-1"),
            sig: String::from("b64u:AAAA"),
        });
        assert!(matches!(
            validate(&raw),
            Err(IntentError::InvalidField(message))
            if message.contains("signature alg")
        ));
    }

    #[test]
    fn validate_rejects_malformed_signature_bytes() {
        let mut raw = sample_raw();
        raw.signature = Some(RawSignature {
            alg: String::from("ed25519"),
            key_id: String::from("did:key:z6Mk...#key-1"),
            sig: String::from("b64u:AAAA"),
        });
        assert!(matches!(
            validate(&raw),
            Err(IntentError::InvalidField(message))
            if message.contains("invalid signature")
        ));
    }

    #[test]
    fn is_expired_checks_expiry_against_now() {
        let validated = validate(&sample_raw()).unwrap();
        let before = DateTime::parse_from_rfc3339("2026-07-14T00:04:59Z")
            .unwrap()
            .with_timezone(&Utc);
        let at = DateTime::parse_from_rfc3339("2026-07-14T00:05:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let after = DateTime::parse_from_rfc3339("2026-07-14T00:05:01Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!validated.is_expired(before));
        assert!(validated.is_expired(at));
        assert!(validated.is_expired(after));
    }

    #[test]
    fn intent_without_expiry_never_expires() {
        let mut raw = sample_raw();
        raw.expires_at = None;
        let validated = validate(&raw).unwrap();
        let far_future = DateTime::parse_from_rfc3339("2999-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!validated.is_expired(far_future));
    }

    #[test]
    fn validate_preserves_nonce_and_authority() {
        let validated = validate(&sample_raw()).unwrap();
        assert_eq!(
            validated.intent.nonce,
            Nonce::from_bytes(vec![0, 3, 4]).unwrap()
        );
        assert!(validated.intent.authority.is_none());
    }
}
