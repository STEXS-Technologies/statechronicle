//! Commit formation error type.
//!
//! Failures from batching, root computation, signing, and persistence, built
//! with `thiserror`. Mirrors `statechronicle-intent`'s conversion convention:
//! [`StateChronicleError::SizeLimitExceeded`] is remapped onto the typed
//! [`CommitError::SizeLimitExceeded`] variant instead of the generic
//! [`CommitError::Core`] wrapper, so callers can match the specific failure
//! without string matching (CODE_STANDARDS §5).

use statechronicle_accumulator::error::AccumulatorError;
use statechronicle_core::error::StateChronicleError;
use statechronicle_domain::error::DomainError;

/// The crate's root error type.
///
/// Every fallible public function in `statechronicle-commit` returns this
/// type. Variants are typed and carry structured context so callers can fail
/// closed without string matching.
#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    /// A commit was assembled or signed from an empty event batch.
    ///
    /// Raised by [`crate::batch::CommitBatch::validate`],
    /// [`crate::roots::event_root`], and [`crate::roots::state_root_updates`]
    /// when the batch holds no events (protocol §13.1 requires one or more
    /// events per commit).
    #[error("commit batch must contain at least one event")]
    EmptyBatch,

    /// A domain type construction failed.
    ///
    /// Raised when a `statechronicle-domain` newtype (commit id, event id,
    /// key id, ...) rejects a value.
    #[error(transparent)]
    Domain(#[from] DomainError),

    /// A core primitive (canonicalization, digest, or signature) failed.
    ///
    /// Raised when BCS canonical serialization or Ed25519 verification fails
    /// (ADR-004).
    #[error(transparent)]
    Core(StateChronicleError),

    /// The state accumulator rejected an update batch.
    #[error("state accumulator failed: {0}")]
    Accumulator(#[from] AccumulatorError),

    /// An event is structurally invalid for commit formation.
    ///
    /// Raised when an event's after-state cannot be mapped onto a state key
    /// (for example a subject-held payload without a `subject`), or its
    /// event count does not fit the protocol's `u64` field.
    #[error("invalid event: {0}")]
    InvalidEvent(String),

    /// A tenant-scoped batch contains events from more than one tenant.
    #[error("batch mixes events from multiple tenants")]
    MixedTenant,

    /// A batch contains the same event id twice.
    #[error("duplicate event id `{event_id}` in commit batch")]
    DuplicateEventId {
        /// The duplicated event id string.
        event_id: String,
    },

    /// Two events in a commit sort to the same canonical key
    /// `(resource_id, after.version)` (protocol §13.3).
    #[error("duplicate canonical key for resource `{resource_id}` at version {version}")]
    DuplicateCanonicalKey {
        /// The duplicated resource id string.
        resource_id: String,
        /// The duplicated after-state version.
        version: u64,
    },

    /// Events are not in canonical deterministic order (protocol §13.3).
    #[error(
        "event ordering violation: `{resource_id}` at version {version} is out of canonical order"
    )]
    OutOfOrder {
        /// The resource id of the out-of-order event.
        resource_id: String,
        /// The after-state version of the out-of-order event.
        version: u64,
    },

    /// Replayed state does not match a declared state root.
    #[error("state root mismatch: expected {expected}, recomputed {actual}")]
    StateRootMismatch {
        /// The declared (expected) root string.
        expected: String,
        /// The recomputed (actual) root string.
        actual: String,
    },

    /// Replayed events do not match a declared event root.
    #[error("event merkle root mismatch")]
    EventRootMismatch,

    /// Two commits claim the same parent and sequence under the same scope
    /// (protocol §31).
    #[error("fork detected: two commits claim parent `{parent}` at sequence {sequence}")]
    ForkDetected {
        /// The shared parent commit id string.
        parent: String,
        /// The contested sequence number.
        sequence: u64,
    },

    /// A commit's declared parent does not link to the preceding commit
    /// (protocol §31).
    #[error("chain gap: expected parent `{expected_parent}`, found {actual_parent:?}")]
    ChainGap {
        /// The expected parent commit id string.
        expected_parent: String,
        /// The actual declared parent commit id string, if any.
        actual_parent: Option<String>,
    },

    /// A commit's sequence does not continue its declared parent's sequence
    /// (protocol §31).
    #[error("sequence mismatch: expected {expected}, got {actual}")]
    SequenceMismatch {
        /// The expected next sequence number.
        expected: u64,
        /// The actual sequence number.
        actual: u64,
    },

    /// An accepted event was resubmitted with a different payload under the
    /// same event id (protocol §31 recovery).
    #[error("event rewrite detected for `{event_id}`")]
    EventRewrite {
        /// The rewritten event id string.
        event_id: String,
    },

    /// A payload exceeded a protocol size bound (protocol §30).
    #[error("size limit exceeded for `{name}`: length {actual} exceeds limit {limit}")]
    SizeLimitExceeded {
        /// The name of the bounded value (always `"commit"`).
        name: String,
        /// The protocol limit in bytes.
        limit: usize,
        /// The actual length in bytes.
        actual: usize,
    },

    /// A required commit-builder field was not set before `build`.
    ///
    /// Raised by [`crate::builder::CommitBuilder::build`] when a required
    /// identity field (`scope`, `executor`, `profile`, or `created_at`) was
    /// omitted.
    #[error("missing required commit builder field `{0}`")]
    BuilderFieldMissing(&'static str),

    /// Ed25519 commit signing or envelope construction failed.
    #[error("commit signing failed: {0}")]
    Signing(String),

    /// A store or publisher rejected the write.
    #[error("store operation failed: {0}")]
    Store(String),
}

impl From<StateChronicleError> for CommitError {
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
            | StateChronicleError::SignatureVerification(_)) => Self::Core(other),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_core::limits::MAX_COMMIT_BYTES;

    #[test]
    fn size_limit_exceeded_display_mentions_name_and_bounds() {
        let error = CommitError::SizeLimitExceeded {
            name: String::from("commit"),
            limit: MAX_COMMIT_BYTES,
            actual: MAX_COMMIT_BYTES.saturating_add(1),
        };
        let rendered = error.to_string();
        assert!(rendered.contains("commit"));
        assert!(rendered.contains("size limit exceeded"));
        assert!(rendered.contains(&MAX_COMMIT_BYTES.to_string()));
    }

    #[test]
    fn state_chronicle_size_limit_converts_to_typed_variant() {
        let source = StateChronicleError::SizeLimitExceeded {
            name: String::from("commit"),
            limit: 8,
            actual: 9,
        };
        let converted = CommitError::from(source);
        assert!(matches!(
            converted,
            CommitError::SizeLimitExceeded { name, limit, actual }
            if name == "commit" && limit == 8 && actual == 9
        ));
    }

    #[test]
    fn other_state_chronicle_errors_map_to_core() {
        let source = StateChronicleError::SignatureVerification(String::from("boom"));
        let converted = CommitError::from(source);
        assert!(matches!(converted, CommitError::Core(..)));
    }

    #[test]
    fn mixed_tenant_display_is_explicit() {
        assert_eq!(
            CommitError::MixedTenant.to_string(),
            "batch mixes events from multiple tenants"
        );
    }

    #[test]
    fn duplicate_event_id_display_carries_id() {
        let error = CommitError::DuplicateEventId {
            event_id: String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4"),
        };
        assert!(error.to_string().contains("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4"));
    }
}
