//! Prefixed newtype identifiers.
//!
//! Canonical identity for the protocol's core objects, hand-written here as
//! validated newtypes (the shared crate's `macros.rs` is a placeholder):
//!
//! | Prefix | Id | Protocol |
//! |---|---|---|
//! | `stc_` | `StateId` | §9 |
//! | `int_` | `IntentId` | §11.1 |
//! | `evt_` | `EventId` | §12.1 |
//! | `cmt_` | `CommitId` | §13.1 |
//! | `snp_` | `SnapshotId` | §15 |
//!
//! Protocol examples show ULID-style string bodies, but the protocol does not
//! mandate ULID — only the exact prefix and the total length (≤
//! [`statechronicle_core::limits::MAX_ID_LENGTH`]) are validated. Construction,
//! parsing, and deserialization all fail closed on malformed input.

use core::fmt;
use core::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use statechronicle_core::limits::MAX_ID_LENGTH;

use crate::error::DomainError;

/// Prefix for state record ids (protocol §9).
const STATE_PREFIX: &str = "stc_";
/// Prefix for intent ids (protocol §11.1).
const INTENT_PREFIX: &str = "int_";
/// Prefix for event ids (protocol §12.1).
const EVENT_PREFIX: &str = "evt_";
/// Prefix for commit ids (protocol §13.1).
const COMMIT_PREFIX: &str = "cmt_";
/// Prefix for snapshot ids (protocol §15).
const SNAPSHOT_PREFIX: &str = "snp_";

/// Validates a prefixed id string: exact prefix, non-empty remainder, and
/// total length within the protocol limit.
///
/// # Errors
///
/// Returns [`DomainError::InvalidId`] when `value` does not start with
/// `prefix`, has an empty remainder, or exceeds
/// [`MAX_ID_LENGTH`](statechronicle_core::limits::MAX_ID_LENGTH) characters.
fn validate_id(kind: &'static str, prefix: &str, value: &str) -> Result<(), DomainError> {
    let remainder = value
        .strip_prefix(prefix)
        .ok_or_else(|| DomainError::InvalidId {
            kind,
            value: String::from(value),
            expected_prefix: String::from(prefix),
        })?;
    if remainder.is_empty() {
        return Err(DomainError::InvalidId {
            kind,
            value: String::from(value),
            expected_prefix: String::from(prefix),
        });
    }
    if value.len() > MAX_ID_LENGTH {
        return Err(DomainError::InvalidId {
            kind,
            value: String::from(value),
            expected_prefix: String::from(prefix),
        });
    }
    Ok(())
}

/// Identifies a state record within a tenant (protocol §9).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StateId(pub String);

impl StateId {
    /// Constructs a validated `StateId` from its `stc_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when `value` does not start with the
    /// `stc_` prefix, has an empty remainder, or exceeds [`MAX_ID_LENGTH`]
    /// characters.
    pub fn new(value: String) -> Result<Self, DomainError> {
        validate_id("state", STATE_PREFIX, &value)?;
        Ok(Self(value))
    }

    /// Returns the id as its string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for StateId {
    type Err = DomainError;

    /// Parses a `StateId` from its `stc_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `StateId`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(String::from(s))
    }
}

impl TryFrom<String> for StateId {
    type Error = DomainError;

    /// Converts an owned string into a validated `StateId`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `StateId`.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for StateId {
    type Error = DomainError;

    /// Converts a borrowed string into a validated `StateId`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `StateId`.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(String::from(value))
    }
}

impl Serialize for StateId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for StateId {
    /// Deserializes from the canonical `stc_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns a serde error when the string is not a valid `StateId`.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// Identifies an intent (protocol §11.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IntentId(pub String);

impl IntentId {
    /// Constructs a validated `IntentId` from its `int_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when `value` does not start with the
    /// `int_` prefix, has an empty remainder, or exceeds [`MAX_ID_LENGTH`]
    /// characters.
    pub fn new(value: String) -> Result<Self, DomainError> {
        validate_id("intent", INTENT_PREFIX, &value)?;
        Ok(Self(value))
    }

    /// Returns the id as its string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IntentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for IntentId {
    type Err = DomainError;

    /// Parses an `IntentId` from its `int_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `IntentId`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(String::from(s))
    }
}

impl TryFrom<String> for IntentId {
    type Error = DomainError;

    /// Converts an owned string into a validated `IntentId`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `IntentId`.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for IntentId {
    type Error = DomainError;

    /// Converts a borrowed string into a validated `IntentId`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `IntentId`.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(String::from(value))
    }
}

impl Serialize for IntentId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for IntentId {
    /// Deserializes from the canonical `int_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns a serde error when the string is not a valid `IntentId`.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// Identifies an event (protocol §12.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EventId(pub String);

impl EventId {
    /// Constructs a validated `EventId` from its `evt_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when `value` does not start with the
    /// `evt_` prefix, has an empty remainder, or exceeds [`MAX_ID_LENGTH`]
    /// characters.
    pub fn new(value: String) -> Result<Self, DomainError> {
        validate_id("event", EVENT_PREFIX, &value)?;
        Ok(Self(value))
    }

    /// Returns the id as its string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for EventId {
    type Err = DomainError;

    /// Parses an `EventId` from its `evt_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `EventId`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(String::from(s))
    }
}

impl TryFrom<String> for EventId {
    type Error = DomainError;

    /// Converts an owned string into a validated `EventId`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `EventId`.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for EventId {
    type Error = DomainError;

    /// Converts a borrowed string into a validated `EventId`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `EventId`.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(String::from(value))
    }
}

impl Serialize for EventId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for EventId {
    /// Deserializes from the canonical `evt_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns a serde error when the string is not a valid `EventId`.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// Identifies a commit (protocol §13.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommitId(pub String);

impl CommitId {
    /// Constructs a validated `CommitId` from its `cmt_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when `value` does not start with the
    /// `cmt_` prefix, has an empty remainder, or exceeds [`MAX_ID_LENGTH`]
    /// characters.
    pub fn new(value: String) -> Result<Self, DomainError> {
        validate_id("commit", COMMIT_PREFIX, &value)?;
        Ok(Self(value))
    }

    /// Returns the id as its string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for CommitId {
    type Err = DomainError;

    /// Parses a `CommitId` from its `cmt_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `CommitId`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(String::from(s))
    }
}

impl TryFrom<String> for CommitId {
    type Error = DomainError;

    /// Converts an owned string into a validated `CommitId`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `CommitId`.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for CommitId {
    type Error = DomainError;

    /// Converts a borrowed string into a validated `CommitId`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `CommitId`.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(String::from(value))
    }
}

impl Serialize for CommitId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CommitId {
    /// Deserializes from the canonical `cmt_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns a serde error when the string is not a valid `CommitId`.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// Identifies a snapshot (protocol §15).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SnapshotId(pub String);

impl SnapshotId {
    /// Constructs a validated `SnapshotId` from its `snp_`-prefixed string
    /// form.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when `value` does not start with the
    /// `snp_` prefix, has an empty remainder, or exceeds [`MAX_ID_LENGTH`]
    /// characters.
    pub fn new(value: String) -> Result<Self, DomainError> {
        validate_id("snapshot", SNAPSHOT_PREFIX, &value)?;
        Ok(Self(value))
    }

    /// Returns the id as its string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for SnapshotId {
    type Err = DomainError;

    /// Parses a `SnapshotId` from its `snp_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `SnapshotId`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(String::from(s))
    }
}

impl TryFrom<String> for SnapshotId {
    type Error = DomainError;

    /// Converts an owned string into a validated `SnapshotId`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `SnapshotId`.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for SnapshotId {
    type Error = DomainError;

    /// Converts a borrowed string into a validated `SnapshotId`.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidId`] when the string is not a valid
    /// `SnapshotId`.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(String::from(value))
    }
}

impl Serialize for SnapshotId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SnapshotId {
    /// Deserializes from the canonical `snp_`-prefixed string form.
    ///
    /// # Errors
    ///
    /// Returns a serde error when the string is not a valid `SnapshotId`.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn new_accepts_valid_prefixed_ids() {
        let state = StateId::new(String::from("stc_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap();
        assert_eq!(state.as_str(), "stc_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2");

        let intent = IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap();
        assert_eq!(intent.as_str(), "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2");

        let event = EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap();
        assert_eq!(event.as_str(), "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4");

        let commit = CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap();
        assert_eq!(commit.as_str(), "cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W");

        let snapshot = SnapshotId::new(String::from("snp_01JZ8X9P4DC6YC4K1YZEJX45E2")).unwrap();
        assert_eq!(snapshot.as_str(), "snp_01JZ8X9P4DC6YC4K1YZEJX45E2");
    }

    #[test]
    fn new_rejects_wrong_prefix() {
        assert!(matches!(
            StateId::new(String::from("int_not_a_state")),
            Err(DomainError::InvalidId { kind: "state", .. })
        ));
        assert!(matches!(
            IntentId::new(String::from("stc_not_an_intent")),
            Err(DomainError::InvalidId { kind: "intent", .. })
        ));
        assert!(matches!(
            EventId::new(String::from("cmt_not_an_event")),
            Err(DomainError::InvalidId { kind: "event", .. })
        ));
        assert!(matches!(
            CommitId::new(String::from("evt_not_a_commit")),
            Err(DomainError::InvalidId { kind: "commit", .. })
        ));
        assert!(matches!(
            SnapshotId::new(String::from("stc_not_a_snapshot")),
            Err(DomainError::InvalidId {
                kind: "snapshot",
                ..
            })
        ));
    }

    #[test]
    fn new_rejects_empty_remainder() {
        assert!(StateId::new(String::from("stc_")).is_err());
        assert!(IntentId::new(String::from("int_")).is_err());
        assert!(EventId::new(String::from("evt_")).is_err());
        assert!(CommitId::new(String::from("cmt_")).is_err());
        assert!(SnapshotId::new(String::from("snp_")).is_err());
    }

    #[test]
    fn new_enforces_length_limit() {
        // Exactly at the limit (prefix + remainder) is accepted.
        let at_limit = format!("stc_{}", "x".repeat(MAX_ID_LENGTH.saturating_sub(4)));
        assert!(StateId::new(at_limit).is_ok());

        // One character past the limit is rejected.
        let over_limit = format!("stc_{}", "x".repeat(MAX_ID_LENGTH.saturating_sub(3)));
        assert!(matches!(
            StateId::new(over_limit),
            Err(DomainError::InvalidId { kind: "state", .. })
        ));
    }

    #[test]
    fn invalid_id_error_carries_context() {
        let error = IntentId::new(String::from("evt_")).unwrap_err();
        assert!(matches!(
            error,
            DomainError::InvalidId { kind: "intent", value, expected_prefix }
            if value == "evt_" && expected_prefix == "int_"
        ));
    }

    #[test]
    fn from_str_and_display_roundtrip() {
        let id = EventId::from_str("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4").unwrap();
        assert_eq!(id.to_string(), "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4");
        assert_eq!(EventId::from_str(id.as_str()).unwrap(), id);
        assert!(EventId::from_str("stc_bad").is_err());
    }

    #[test]
    fn try_from_roundtrips_and_rejects() {
        let id = CommitId::try_from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W").unwrap();
        assert_eq!(id.as_str(), "cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W");

        let owned: CommitId = String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")
            .try_into()
            .unwrap();
        assert_eq!(owned, id);

        assert!(CommitId::try_from(String::from("int_0")).is_err());
        assert!(CommitId::try_from("cmt_").is_err());
    }

    #[test]
    fn serde_roundtrips_through_string_form() {
        let id = IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2\"");

        let decoded: IntentId = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, id);
    }

    #[test]
    fn deserialize_rejects_invalid_string() {
        let empty_remainder = serde_json::from_str::<IntentId>("\"int_\"");
        assert!(empty_remainder.is_err());

        let wrong_prefix = serde_json::from_str::<IntentId>("\"evt_x\"");
        assert!(wrong_prefix.is_err());
    }

    #[test]
    fn ids_of_different_kinds_are_distinct_types() {
        let state = StateId::new(String::from("stc_abc")).unwrap();
        let _intent = IntentId::new(String::from("int_abc")).unwrap();
        assert_eq!(state.as_str(), "stc_abc");
    }
}
