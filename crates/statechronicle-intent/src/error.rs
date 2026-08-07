//! Intent processing error type.
//!
//! Errors for parse and validation failures, built with `thiserror`.

use statechronicle_core::error::StateChronicleError;
use statechronicle_domain::error::DomainError;

/// The crate's root error type.
///
/// Every fallible public function in `statechronicle-intent` returns this
/// type. Variants are typed and carry structured context so callers can fail
/// closed without string matching.
#[derive(Debug, thiserror::Error)]
pub enum IntentError {
    /// An intent payload exceeded [`statechronicle_core::limits::MAX_INTENT_BYTES`].
    ///
    /// Raised by [`crate::parse::parse_intent`] and
    /// [`crate::parse::parse_intent_str`] when the input length exceeds the
    /// protocol size bound (protocol §30), so parsing fails closed instead of
    /// exhausting resources.
    #[error("size limit exceeded for `{name}`: length {actual} exceeds limit {limit}")]
    SizeLimitExceeded {
        /// The name of the bounded value (always `"intent"`).
        name: String,
        /// The protocol limit in bytes.
        limit: usize,
        /// The actual length in bytes.
        actual: usize,
    },

    /// The payload is not well-formed JSON.
    ///
    /// Raised when `serde_json` cannot deserialize the payload into a
    /// [`crate::raw::RawIntent`].
    #[error("intent payload is not valid JSON: {source}")]
    InvalidJson {
        /// The underlying JSON parsing error.
        #[from]
        source: serde_json::Error,
    },

    /// The `schema` field is not the supported intent schema.
    ///
    /// Raised when `schema` does not equal
    /// [`statechronicle_domain::intent::INTENT_SCHEMA`].
    #[error("unsupported intent schema `{found}`; expected `{expected}`")]
    InvalidSchema {
        /// The schema identifier found in the payload.
        found: String,
        /// The supported schema identifier.
        expected: String,
    },

    /// A typed field failed validation or type conversion.
    ///
    /// Raised when a newtype constructor rejects a value (malformed state
    /// type, timestamp, authority proof, or signature block).
    #[error("invalid intent field: {0}")]
    InvalidField(String),

    /// The intent expiry is not after its creation time.
    ///
    /// Raised when `expires_at` is present but not strictly after
    /// `created_at` (protocol §11.2).
    #[error("invalid intent expiry: {0}")]
    InvalidExpiry(String),

    /// A domain type construction failed.
    ///
    /// Raised when a `statechronicle-domain` newtype (id, operation, key id,
    /// or nonce) rejects a value.
    #[error(transparent)]
    Domain(#[from] DomainError),
}

impl From<StateChronicleError> for IntentError {
    fn from(source: StateChronicleError) -> Self {
        match source {
            StateChronicleError::SizeLimitExceeded {
                name,
                limit,
                actual,
            } => Self::SizeLimitExceeded {
                name,
                limit,
                actual,
            },
            other @ (StateChronicleError::Canonicalization { .. }
            | StateChronicleError::InvalidDigest(_)
            | StateChronicleError::SignatureVerification(_)) => {
                Self::Domain(DomainError::Core(other))
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_core::limits::MAX_INTENT_BYTES;

    #[test]
    fn size_limit_exceeded_display_mentions_name_and_bounds() {
        let error = IntentError::SizeLimitExceeded {
            name: String::from("intent"),
            limit: MAX_INTENT_BYTES,
            actual: MAX_INTENT_BYTES.saturating_add(1),
        };
        let rendered = error.to_string();
        assert!(rendered.contains("intent"));
        assert!(rendered.contains("size limit exceeded"));
        assert!(rendered.contains(&MAX_INTENT_BYTES.to_string()));
    }

    #[test]
    fn state_chronicle_size_limit_converts_to_size_limit() {
        let source = StateChronicleError::SizeLimitExceeded {
            name: String::from("intent"),
            limit: 8,
            actual: 9,
        };
        let converted = IntentError::from(source);
        assert!(matches!(
            converted,
            IntentError::SizeLimitExceeded { name, limit, actual }
            if name == "intent" && limit == 8 && actual == 9
        ));
    }

    #[test]
    fn other_state_chronicle_errors_map_to_domain() {
        let source = StateChronicleError::SignatureVerification(String::from("boom"));
        let converted = IntentError::from(source);
        assert!(matches!(
            converted,
            IntentError::Domain(DomainError::Core(..))
        ));
    }
}
