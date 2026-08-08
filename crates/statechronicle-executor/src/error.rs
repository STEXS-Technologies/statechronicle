//! Execution pipeline error type.
//!
//! Failures from validation, transition, conflict, and atomicity stages, built
//! with `thiserror`. Every variant carries structured context so callers can
//! fail closed without string matching (protocol §18).

use statechronicle_core::error::StateChronicleError;
use statechronicle_domain::error::DomainError;
use statechronicle_intent::error::IntentError;
use statechronicle_profiles::error::ProfileError;

/// The crate's root error type.
///
/// Every fallible public function in `statechronicle-executor` returns this
/// type. Variants are typed and carry structured context so callers can fail
/// closed without string matching.
#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    /// An upstream intent-processing failure (parse, schema, or size).
    ///
    /// Raised when the intent crate rejected a payload that reached the
    /// executor, or when an intent field fails its domain newtype checks.
    #[error(transparent)]
    Intent(#[from] IntentError),

    /// A domain type construction or parsing failure.
    #[error(transparent)]
    Domain(#[from] DomainError),

    /// A profile rule rejected the operation.
    ///
    /// Raised by [`crate::pipeline::Executor::execute`] when
    /// [`statechronicle_profiles::registry::ProfileRules::check`] rejects the
    /// operation (unknown operation, invalid transition, malformed input,
    /// insufficient quantity, ownership mismatch).
    #[error(transparent)]
    Profile(#[from] ProfileError),

    /// A protocol object exceeded a size bound (protocol §30).
    ///
    /// Mirrors [`IntentError::SizeLimitExceeded`]: a
    /// [`StateChronicleError::SizeLimitExceeded`] is remapped onto this variant
    /// instead of being wrapped in `Domain`.
    #[error("size limit exceeded for `{name}`: length {actual} exceeds limit {limit}")]
    SizeLimitExceeded {
        /// The name of the bounded value (e.g. `"intent"`).
        name: String,
        /// The protocol limit in bytes.
        limit: usize,
        /// The actual length in bytes.
        actual: usize,
    },

    /// The intent expired before acceptance (protocol §18.2).
    #[error("intent `{intent_id}` expired before acceptance")]
    Expired {
        /// The rejected intent's id.
        intent_id: String,
    },

    /// A duplicate `intent_id` was submitted with a different payload
    /// (protocol §18.2).
    #[error("duplicate intent id `{intent_id}` with different payload")]
    DuplicateIntent {
        /// The colliding intent id.
        intent_id: String,
    },

    /// The resource's current version does not match the intent's
    /// `expected_version` (protocol §18.2).
    #[error("expected version mismatch for `{resource}`: expected {expected}, actual {actual}")]
    ExpectedVersionMismatch {
        /// The resource whose version did not match.
        resource: String,
        /// The version declared by the intent.
        expected: u64,
        /// The resource's current version.
        actual: u64,
    },

    /// The intent carries no tenant scope (protocol §18.2).
    #[error("intent has no tenant scope")]
    TenantScopeMissing,

    /// The intent's tenant scope does not exist (protocol §8).
    #[error("tenant `{tenant}` not found")]
    TenantNotFound {
        /// The unknown tenant id.
        tenant: String,
    },

    /// The resource the intent targets does not exist.
    ///
    /// Raised when an intent with a positive `expected_version` targets a
    /// resource with no projection (protocol §18.2).
    #[error("resource `{resource}` not found")]
    ResourceNotFound {
        /// The missing resource id.
        resource: String,
    },

    /// The TrustGrant authority evaluation denied the operation
    /// (protocol §18.2).
    #[error("authority evaluation denied")]
    AuthorityDenied,

    /// The TrustGrant authority proof is stale or revoked under the verifier
    /// policy (protocol §18.2).
    #[error("authority proof stale or revoked")]
    AuthorityStale,

    /// The TrustGrant authority source could not be reached.
    #[error("authority source unavailable: {0}")]
    AuthorityUnavailable(String),

    /// The active profile requires an authority binding for the operation but
    /// the intent carried none (protocol §11.2, ADR-006 §36 Q5 / deferral item
    /// 4).
    #[error("authority binding required for operation `{operation}`")]
    AuthorityMissing {
        /// The operation that required an authority binding.
        operation: String,
    },

    /// The acting actor or `from_owner` input does not match the resource's
    /// current owner or holder (protocol §18.2).
    #[error("actor mismatch: expected `{expected}`, got `{actual}`")]
    ActorMismatch {
        /// The owner or holder recorded in the current state.
        expected: String,
        /// The owner or holder declared by the intent inputs.
        actual: String,
    },

    /// The resource is locked, burned, revoked, or escrowed in a way that
    /// blocks the operation (protocol §18.2).
    #[error("resource `{resource}` is locked, burned, revoked, or escrowed")]
    ResourceLocked {
        /// The blocked resource id.
        resource: String,
    },

    /// A deterministic transition could not be computed.
    ///
    /// Raised for unknown operations, unknown state types, missing or
    /// malformed inputs, integer overflow, or underflow in after-state
    /// arithmetic. The executor fails closed rather than producing an
    /// ambiguous transition.
    #[error("invalid transition: {0}")]
    TransitionInvalid(String),

    /// The intent's detached signature did not verify over its canonical body
    /// (protocol §18.1 step 4, ADR-004 §5).
    ///
    /// Raised when the injected intent verifier rejects a present
    /// [`SignatureBlock`](statechronicle_domain::intent::SignatureBlock)
    /// against the BCS canonical bytes of the intent body.
    #[error("actor authentication failed: {0}")]
    ActorAuthenticationFailed(String),

    /// A transfer pair is not internally consistent (protocol §20.5).
    ///
    /// Raised when the two events sharing a transfer intent id do not form an
    /// atomic debit + credit pair: the source debit does not equal the
    /// destination credit (net-zero violated), or the pair's payloads are
    /// malformed.
    #[error("transfer mismatch: {0}")]
    TransferMismatch(String),

    /// A multi-resource batch could not commit atomically (protocol §18.3).
    #[error("atomicity violation: {0}")]
    AtomicityViolation(String),

    /// A backing port failed.
    #[error("store error: {0}")]
    Store(String),

    /// The intent carries no `state_type` but the active profile requires one.
    #[error("intent requires a state_type")]
    StateTypeRequired,
}

/// A builder failure while assembling a [`crate::pipeline::Ports`] bundle.
///
/// Raised by [`crate::pipeline::PortsBuilder::build`] when a required port was
/// not injected. The `trustgrant` set is optional (defaults to empty), so it is
/// never a build error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[allow(clippy::enum_variant_names)]
pub enum PortsBuildError {
    /// The intent store port was not injected.
    #[error("missing required port `intent_store`")]
    MissingIntentStore,
    /// The state index port was not injected.
    #[error("missing required port `state_index`")]
    MissingStateIndex,
    /// The tenant store port was not injected.
    #[error("missing required port `tenant_store`")]
    MissingTenantStore,
    /// The transaction manager port was not injected.
    #[error("missing required port `transaction_manager`")]
    MissingTransactionManager,
}

/// A builder failure while assembling an [`crate::pipeline::Executor`].
///
/// Raised by [`crate::pipeline::ExecutorBuilder::build`] when a required
/// component was not injected. `profiles` is optional (defaults to the baseline
/// registry), so it is never a build error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[allow(clippy::enum_variant_names)]
pub enum ExecutorBuildError {
    /// The port bundle was not injected.
    #[error("missing required executor component `ports`")]
    MissingPorts,
    /// The executor identity was not injected.
    #[error("missing required executor component `executor`")]
    MissingExecutor,
    /// The wall clock was not injected.
    #[error("missing required executor component `clock`")]
    MissingClock,
    /// The event-id generator was not injected.
    #[error("missing required executor component `event_id_gen`")]
    MissingEventIdGen,
    /// The intent verifier was not injected.
    #[error("missing required executor component `intent_verifier`")]
    MissingIntentVerifier,
}

impl From<StateChronicleError> for ExecutorError {
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
        let error = ExecutorError::SizeLimitExceeded {
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
        let converted = ExecutorError::from(source);
        assert!(matches!(
            converted,
            ExecutorError::SizeLimitExceeded { name, limit, actual }
            if name == "intent" && limit == 8 && actual == 9
        ));
    }

    #[test]
    fn other_state_chronicle_errors_map_to_domain() {
        let source = StateChronicleError::SignatureVerification(String::from("boom"));
        let converted = ExecutorError::from(source);
        assert!(matches!(
            converted,
            ExecutorError::Domain(DomainError::Core(..))
        ));
    }

    #[test]
    fn error_display_messages() {
        assert_eq!(
            ExecutorError::Expired {
                intent_id: String::from("int_abc"),
            }
            .to_string(),
            "intent `int_abc` expired before acceptance"
        );
        assert_eq!(
            ExecutorError::DuplicateIntent {
                intent_id: String::from("int_abc"),
            }
            .to_string(),
            "duplicate intent id `int_abc` with different payload"
        );
        assert_eq!(
            ExecutorError::ExpectedVersionMismatch {
                resource: String::from("asset:sword_001"),
                expected: 5,
                actual: 6,
            }
            .to_string(),
            "expected version mismatch for `asset:sword_001`: expected 5, actual 6"
        );
        assert_eq!(
            ExecutorError::TenantScopeMissing.to_string(),
            "intent has no tenant scope"
        );
        assert_eq!(
            ExecutorError::TenantNotFound {
                tenant: String::from("acme.game.alpha"),
            }
            .to_string(),
            "tenant `acme.game.alpha` not found"
        );
        assert_eq!(
            ExecutorError::ResourceNotFound {
                resource: String::from("asset:sword_001"),
            }
            .to_string(),
            "resource `asset:sword_001` not found"
        );
        assert_eq!(
            ExecutorError::AuthorityDenied.to_string(),
            "authority evaluation denied"
        );
        assert_eq!(
            ExecutorError::AuthorityStale.to_string(),
            "authority proof stale or revoked"
        );
        assert_eq!(
            ExecutorError::AuthorityUnavailable(String::from("down")).to_string(),
            "authority source unavailable: down"
        );
        assert_eq!(
            ExecutorError::AuthorityMissing {
                operation: String::from("asset.transfer"),
            }
            .to_string(),
            "authority binding required for operation `asset.transfer`"
        );
        assert_eq!(
            ExecutorError::ActorMismatch {
                expected: String::from("alice"),
                actual: String::from("mallory"),
            }
            .to_string(),
            "actor mismatch: expected `alice`, got `mallory`"
        );
        assert_eq!(
            ExecutorError::ResourceLocked {
                resource: String::from("asset:sword_001"),
            }
            .to_string(),
            "resource `asset:sword_001` is locked, burned, revoked, or escrowed"
        );
        assert_eq!(
            ExecutorError::TransitionInvalid(String::from("unknown operation")).to_string(),
            "invalid transition: unknown operation"
        );
        assert_eq!(
            ExecutorError::AtomicityViolation(String::from("partial commit")).to_string(),
            "atomicity violation: partial commit"
        );
        assert_eq!(
            ExecutorError::ActorAuthenticationFailed(String::from("bad signature")).to_string(),
            "actor authentication failed: bad signature"
        );
        assert_eq!(
            ExecutorError::TransferMismatch(String::from("net-zero violated")).to_string(),
            "transfer mismatch: net-zero violated"
        );
        assert_eq!(
            ExecutorError::Store(String::from("db down")).to_string(),
            "store error: db down"
        );
        assert_eq!(
            ExecutorError::StateTypeRequired.to_string(),
            "intent requires a state_type"
        );
    }
}
