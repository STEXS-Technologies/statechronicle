//! The shared protocol error type.
//!
//! `StateChronicleError` is the crate's root error type, built with
//! `thiserror` and convertible from canonicalization, digest, and signature
//! failures. Every fallible public API in this crate surfaces one of these
//! variants.

/// The crate's root error type.
///
/// All fallible operations in `statechronicle-core` return this type. Variants
/// are typed and carry structured context so callers can fail closed without
/// string matching.
#[derive(Debug, thiserror::Error)]
pub enum StateChronicleError {
    /// BCS canonical serialization failed (ADR-004).
    ///
    /// Raised when a value cannot be serialized to its canonical BCS form,
    /// e.g. a float (structurally banned by the protocol), a non-minimal
    /// length prefix, or a nested type exceeding BCS container depth limits.
    #[error("canonicalization failed: {message}")]
    Canonicalization {
        /// Crate-local context about the failed serialization.
        message: String,
        /// The underlying BCS serialization error.
        #[source]
        source: bcs::Error,
    },

    /// A string is not a valid `sha256:<lowercase-hex>` content digest.
    ///
    /// Raised by [`crate::digest::ContentDigest::from_hex_sha256`] and
    /// [`core::str::FromStr`] when the input has a wrong prefix, is not
    /// exactly 64 lowercase hex characters, or fails to decode to 32 bytes.
    #[error("invalid content digest: {0}")]
    InvalidDigest(String),

    /// Ed25519 signature verification failed (ADR-004 §5).
    ///
    /// Raised by [`crate::signature::verify`] for malformed signatures, weak
    /// keys, malleable signatures, or a canonical payload that does not match
    /// the signature. Signatures never cover the `signature` field (structural
    /// envelope, ADR-004 §2).
    #[error("signature verification failed: {0}")]
    SignatureVerification(String),

    /// An input exceeded a protocol size bound (protocol §30).
    ///
    /// Raised by [`crate::limits::check_size`] when a payload length exceeds
    /// its configured limit, so parsers and accumulators fail closed instead
    /// of exhausting resources.
    #[error("size limit exceeded for `{name}`: length {actual} exceeds limit {limit}")]
    SizeLimitExceeded {
        /// The name of the bounded value (e.g. `"intent"`, `"event"`).
        name: String,
        /// The protocol limit in bytes.
        limit: usize,
        /// The actual length in bytes.
        actual: usize,
    },
}

impl From<bcs::Error> for StateChronicleError {
    fn from(source: bcs::Error) -> Self {
        Self::Canonicalization {
            message: String::from("BCS serialization of protocol value failed"),
            source,
        }
    }
}
