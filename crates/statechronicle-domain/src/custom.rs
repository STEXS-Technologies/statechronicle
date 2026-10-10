//! Domain-separated consumer-defined state payloads.
use crate::commit::ProfileId;
use serde::{Deserialize, Serialize};

/// A canonical BCS payload whose domain and schema are included in every hash.
/// Payload interpretation must check both tags before decoding. Empty payloads
/// at event version zero are the unborn-state commitment, not a decoded state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomState {
    /// Stable consumer domain identity.
    pub domain: ProfileId,
    /// Explicit consumer schema version.
    pub schema_version: u32,
    /// Canonical BCS encoding of the consumer's native state.
    pub payload: Vec<u8>,
}
