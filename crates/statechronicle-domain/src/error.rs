//! Domain error type.
//!
//! Errors raised by domain type construction and validation, built with
//! `thiserror`.

use statechronicle_core::error::StateChronicleError;
use statechronicle_core::limits::MAX_ID_LENGTH;

/// The crate's root error type.
///
/// Every fallible public constructor and parser in `statechronicle-domain`
/// returns this type. Variants are typed and carry structured context so
/// callers can fail closed without string matching.
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    /// A prefixed identifier string is malformed.
    ///
    /// Raised when an id does not start with its required prefix, has an empty
    /// remainder, or exceeds [`MAX_ID_LENGTH`] characters (protocol §9–15).
    #[error(
        "invalid {kind} id: `{value}` must start with `{expected_prefix}` and be ≤ {MAX_ID_LENGTH} chars"
    )]
    InvalidId {
        /// The kind of id being constructed, e.g. `state`, `intent`, or `event`.
        kind: &'static str,
        /// The rejected input string.
        value: String,
        /// The required prefix for this id kind.
        expected_prefix: String,
    },

    /// An operation string is malformed.
    ///
    /// Raised when an operation name is empty or exceeds [`MAX_ID_LENGTH`]
    /// characters (protocol §11.1).
    #[error("invalid operation: {0}")]
    InvalidOperation(String),

    /// A signing key id is malformed.
    ///
    /// Raised when a `KeyId` is empty or exceeds [`MAX_ID_LENGTH`] characters
    /// (protocol §11.1 signature block).
    #[error("invalid key id: {0}")]
    InvalidKeyId(String),

    /// A profile registry id is malformed.
    ///
    /// Raised when a `ProfileId` is empty or exceeds [`MAX_ID_LENGTH`]
    /// characters (protocol §13.1).
    #[error("invalid profile id: {0}")]
    InvalidProfile(String),

    /// A nonce string is malformed.
    ///
    /// Raised when a nonce does not use the `b64u:` base64url-unpadded form or
    /// exceeds [`crate::intent::MAX_NONCE_BYTES`] decoded bytes (protocol §11.1).
    #[error("invalid nonce: {0}")]
    InvalidNonce(String),

    /// A failure from the shared core crate.
    ///
    /// Raised when canonicalization, digest, or signature primitives fail
    /// (ADR-004).
    #[error(transparent)]
    Core(#[from] StateChronicleError),
}
