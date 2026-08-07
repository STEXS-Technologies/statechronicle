//! ADR-004 structural signed envelope.
//!
//! The protocol signs an explicit body type (`body` + `signature`) so the
//! signature covers only the BCS canonical bytes of `body`, never a
//! `signature` field (ADR-004 §2). [`Signed`] is the generic envelope used for
//! intents, commits, and snapshots.

use serde::{Deserialize, Serialize};

use crate::intent::SignatureBlock;

/// A signed protocol body with its detached signature block (ADR-004 §2).
///
/// The signature covers only the BCS canonical bytes of `body`; a verifier
/// recomputes `bcs::to_bytes(&body)` and checks it against `signature.sig`
/// under `signature.key_id`. The envelope itself also serializes through BCS
/// (and JSON for the HTTP API logical view).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signed<T> {
    /// The signed protocol body (intent, commit, or snapshot).
    pub body: T,
    /// The detached signature block over the body's canonical bytes.
    pub signature: SignatureBlock,
}

impl<T> Signed<T> {
    /// Constructs a signed envelope from a body and its signature block.
    pub const fn new(body: T, signature: SignatureBlock) -> Self {
        Self { body, signature }
    }
}
