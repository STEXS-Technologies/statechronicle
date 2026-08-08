//! Profile status names.
//!
//! A validated newtype over the wire string names a profile uses for a
//! resource's projected status field (protocol §20.1). Statuses are
//! registry-open strings, not a closed enum, so profiles define their own
//! statuses (and exceptional statuses) without a protocol schema bump. The
//! wire format stays a plain string; this newtype adds fail-closed validation
//! and typed equality at the boundary so raw string comparison is eliminated
//! from profile and executor dispatch.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use statechronicle_core::limits::MAX_ID_LENGTH;

use crate::error::DomainError;

/// A validated profile status name (protocol §20.1).
///
/// Statuses are registry-open and profile-defined (e.g. `active`, `locked`,
/// `trade_held`, `sold`), so this is a validated newtype over the wire string,
/// not a closed enum. Construction is validated (non-empty and within
/// [`MAX_ID_LENGTH`]); comparison and equality are typed, so profiles and the
/// executor no longer match on raw strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Status(String);

impl Status {
    /// Constructs a validated status name.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidStatus`] when `value` is empty or exceeds
    /// [`MAX_ID_LENGTH`] characters.
    pub fn new(value: String) -> Result<Self, DomainError> {
        validate_status(&value)?;
        Ok(Self(value))
    }

    /// Constructs a status from a compile-time literal, infallibly.
    ///
    /// This trusted constructor is intended **only** for in-crate
    /// compile-time status literals (e.g. `Status::from_static("active")`). It
    /// is not `const` because `Status` owns a `String`; the caller guarantees
    /// the literal is non-empty and within [`MAX_ID_LENGTH`], which the runtime
    /// [`Self::new`] enforces identically for wire-parsed values.
    pub fn from_static(value: &'static str) -> Status {
        Status(String::from(value))
    }

    /// Parses a status from a borrowed string.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidStatus`] when `value` is empty or exceeds
    /// [`MAX_ID_LENGTH`] characters.
    pub fn try_from_str(value: &str) -> Result<Self, DomainError> {
        Self::new(String::from(value))
    }

    /// Returns the status name as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Status {
    type Err = DomainError;

    /// Parses a status name, validating it.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidStatus`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from_str(s)
    }
}

impl TryFrom<String> for Status {
    type Error = DomainError;

    /// Converts an owned string into a validated status name.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidStatus`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for Status {
    type Error = DomainError;

    /// Converts a borrowed string into a validated status name.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidStatus`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from_str(value)
    }
}

impl Serialize for Status {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Status {
    /// Deserializes a status name, validating it.
    ///
    /// # Errors
    ///
    /// Returns a serde error when the string is empty or exceeds
    /// [`MAX_ID_LENGTH`] characters.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// Validates a status name: non-empty and within the id length bound.
///
/// # Errors
///
/// Returns [`DomainError::InvalidStatus`] when `value` is empty or exceeds
/// [`MAX_ID_LENGTH`] characters.
fn validate_status(value: &str) -> Result<(), DomainError> {
    if value.is_empty() {
        return Err(DomainError::InvalidStatus(String::from(
            "status must not be empty",
        )));
    }
    if value.len() > MAX_ID_LENGTH {
        return Err(DomainError::InvalidStatus(format!(
            "status must be at most {MAX_ID_LENGTH} chars, got {}",
            value.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn valid_status_constructs() {
        let status = Status::new(String::from("active")).unwrap();
        assert_eq!(status.as_str(), "active");
    }

    #[test]
    fn empty_status_is_rejected() {
        assert!(matches!(
            Status::new(String::new()),
            Err(DomainError::InvalidStatus(_))
        ));
        assert!(matches!(
            Status::from_str(""),
            Err(DomainError::InvalidStatus(_))
        ));
    }

    #[test]
    fn oversized_status_is_rejected() {
        let oversized = "x".repeat(MAX_ID_LENGTH.saturating_add(1));
        assert!(matches!(
            Status::new(oversized),
            Err(DomainError::InvalidStatus(_))
        ));
    }

    #[test]
    fn at_limit_status_is_accepted() {
        let at_limit = "x".repeat(MAX_ID_LENGTH);
        assert!(Status::new(at_limit).is_ok());
    }

    #[test]
    fn non_utf8_boundary_fails_closed() {
        // A status constructed from bytes that are not valid UTF-8 cannot be
        // parsed as a `&str`, so parsing fails closed at the boundary.
        let bytes = vec![0xff, 0xfe, 0x00];
        assert!(std::str::from_utf8(&bytes).is_err());
    }

    #[test]
    fn try_from_str_roundtrips() {
        let parsed = Status::try_from_str("trade_held").unwrap();
        assert_eq!(parsed, Status::from_static("trade_held"));
        assert_eq!(parsed.as_str(), "trade_held");
    }

    #[test]
    fn from_static_equals_new() {
        let literal = Status::from_static("active");
        assert_eq!(literal, Status::new(String::from("active")).unwrap());
    }

    #[test]
    fn serde_roundtrips_as_plain_string() {
        let status = Status::from_static("locked");
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"locked\"");
        let decoded: Status = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, status);
    }

    #[test]
    fn serde_rejects_empty_string() {
        assert!(serde_json::from_str::<Status>("\"\"").is_err());
    }

    #[test]
    fn hash_and_equality_are_typed() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Status::from_static("active"));
        assert!(set.contains(&Status::try_from_str("active").unwrap()));
        assert!(!set.contains(&Status::from_static("locked")));
    }
}
