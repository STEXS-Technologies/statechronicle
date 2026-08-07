//! Commits: signed batches of events (protocol §13).
//!
//! A commit groups ordered events, computes the event Merkle root and state
//! roots, and is signed with an Ed25519 commit key. The `Commit` struct
//! deliberately excludes the `signature` field: per the ADR-004 structural
//! envelope rule (§2) the signature lives in [`crate::signed::Signed<Commit>`]
//! and covers only the body.

use core::fmt;
use core::str::FromStr;

use chrono::{DateTime, Utc};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use statechronicle_core::digest::ContentDigest;
use statechronicle_core::limits::MAX_ID_LENGTH;

use crate::error::DomainError;
use crate::ids::CommitId;
use crate::subject::SubjectId;
use crate::tenant::TenantId;

/// Schema identifier for v0 commits (protocol §13.1).
pub const COMMIT_SCHEMA: &str = "statechronicle.commit.v0";

/// The scope of a commit: a tenant history or the global checkpoint chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopeKind {
    /// A tenant-scoped commit (§13.1).
    #[serde(rename = "tenant")]
    Tenant,
    /// A global checkpoint commit over tenant roots (§13.4).
    #[serde(rename = "global_checkpoint")]
    GlobalCheckpoint,
}

/// The declared scope of a commit.
///
/// The typed constructors enforce the invariant that a tenant scope carries
/// its [`TenantId`] while a global checkpoint scope carries none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitScope {
    /// Whether this is a tenant or global checkpoint commit.
    pub kind: ScopeKind,
    /// The tenant id, present iff `kind` is [`ScopeKind::Tenant`].
    pub tenant_id: Option<TenantId>,
}

impl CommitScope {
    /// Constructs a tenant-scoped commit scope.
    pub const fn tenant(tenant_id: TenantId) -> Self {
        Self {
            kind: ScopeKind::Tenant,
            tenant_id: Some(tenant_id),
        }
    }

    /// Constructs a global checkpoint commit scope.
    pub const fn global_checkpoint() -> Self {
        Self {
            kind: ScopeKind::GlobalCheckpoint,
            tenant_id: None,
        }
    }
}

/// A profile registry id (protocol §13.1 `profile` field).
///
/// A registry-open dotted name, e.g. `statechronicle.profile.resource.v0`.
/// Only non-emptiness and length are validated at the domain layer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProfileId(pub String);

impl ProfileId {
    /// Constructs a validated profile registry id.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidProfile`] when `value` is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    pub fn new(value: String) -> Result<Self, DomainError> {
        validate_profile_id(&value)?;
        Ok(Self(value))
    }

    /// Returns the profile id as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ProfileId {
    type Err = DomainError;

    /// Parses a profile registry id, validating it.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidProfile`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(String::from(s))
    }
}

impl TryFrom<String> for ProfileId {
    type Error = DomainError;

    /// Converts an owned string into a validated profile registry id.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidProfile`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for ProfileId {
    type Error = DomainError;

    /// Converts a borrowed string into a validated profile registry id.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidProfile`] when the string is empty or
    /// exceeds [`MAX_ID_LENGTH`] characters.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(String::from(value))
    }
}

impl Serialize for ProfileId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ProfileId {
    /// Deserializes a profile registry id, validating it.
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

/// Validates a profile registry id: non-empty and within the id length bound.
///
/// # Errors
///
/// Returns [`DomainError::InvalidProfile`] when `value` is empty or exceeds
/// [`MAX_ID_LENGTH`] characters.
fn validate_profile_id(value: &str) -> Result<(), DomainError> {
    if value.is_empty() {
        return Err(DomainError::InvalidProfile(String::from(
            "profile id must not be empty",
        )));
    }
    if value.len() > MAX_ID_LENGTH {
        return Err(DomainError::InvalidProfile(format!(
            "profile id must be at most {MAX_ID_LENGTH} chars, got {}",
            value.len()
        )));
    }
    Ok(())
}

/// An ordered batch of validated events (protocol §13.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    /// Schema identifier, always [`COMMIT_SCHEMA`] for v0.
    pub schema: String,
    /// The tenant or global checkpoint scope.
    pub scope: CommitScope,
    /// Unique commit id.
    pub commit_id: CommitId,
    /// The previous canonical commit, absent for a genesis commit.
    pub parent_commit_id: Option<CommitId>,
    /// Monotonic commit sequence number.
    pub sequence: u64,
    /// Number of events in the commit batch.
    pub event_count: u64,
    /// Merkle root over the included events.
    pub event_merkle_root: ContentDigest,
    /// State root before applying the batch.
    pub previous_state_root: ContentDigest,
    /// State root after applying the batch.
    pub next_state_root: ContentDigest,
    /// When the commit was created (UTC).
    pub created_at: DateTime<Utc>,
    /// The authorized commit executor that signed the body.
    pub executor: SubjectId,
    /// The profile whose rules produced the state roots.
    pub profile: ProfileId,
}

impl Commit {
    /// Constructs a commit with the v0 schema identifier set.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope: CommitScope,
        commit_id: CommitId,
        parent_commit_id: Option<CommitId>,
        sequence: u64,
        event_count: u64,
        event_merkle_root: ContentDigest,
        previous_state_root: ContentDigest,
        next_state_root: ContentDigest,
        created_at: DateTime<Utc>,
        executor: SubjectId,
        profile: ProfileId,
    ) -> Self {
        Self {
            schema: String::from(COMMIT_SCHEMA),
            scope,
            commit_id,
            parent_commit_id,
            sequence,
            event_count,
            event_merkle_root,
            previous_state_root,
            next_state_root,
            created_at,
            executor,
            profile,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_core::digest::hash_bytes;

    fn sample_digest() -> ContentDigest {
        hash_bytes(b"state-root")
    }

    fn sample_commit() -> Commit {
        Commit::new(
            CommitScope::tenant(TenantId(String::from("stexs.game.alpha"))),
            CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            Some(CommitId::new(String::from("cmt_01JZ8WZ0QH93JK8J19VVD3QXSC")).unwrap()),
            918273,
            180000,
            sample_digest(),
            sample_digest(),
            sample_digest(),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
                .unwrap()
                .with_timezone(&Utc),
            SubjectId(String::from("service:statechronicle.stexs.net")),
            ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
        )
    }

    #[test]
    fn constructor_sets_schema() {
        let commit = sample_commit();
        assert_eq!(commit.schema, COMMIT_SCHEMA);
    }

    #[test]
    fn scope_constructors_enforce_invariant() {
        let tenant = CommitScope::tenant(TenantId(String::from("stexs.game.alpha")));
        assert_eq!(tenant.kind, ScopeKind::Tenant);
        assert_eq!(
            tenant.tenant_id,
            Some(TenantId(String::from("stexs.game.alpha")))
        );

        let global = CommitScope::global_checkpoint();
        assert_eq!(global.kind, ScopeKind::GlobalCheckpoint);
        assert_eq!(global.tenant_id, None);
    }

    #[test]
    fn genesis_commit_has_no_parent() {
        let mut commit = sample_commit();
        commit.parent_commit_id = None;
        assert_eq!(commit.parent_commit_id, None);
        assert_eq!(commit.sequence, 918273);
    }

    #[test]
    fn serde_json_roundtrips() {
        let commit = sample_commit();
        let json = serde_json::to_string(&commit).unwrap();
        let decoded: Commit = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, commit);
    }

    #[test]
    fn bcs_roundtrips() {
        let commit = sample_commit();
        let bytes = bcs::to_bytes(&commit).unwrap();
        let decoded: Commit = bcs::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, commit);
    }

    #[test]
    fn profile_id_validates() {
        assert!(ProfileId::new(String::new()).is_err());
        assert!(ProfileId::new(String::from("statechronicle.profile.resource.v0")).is_ok());
        assert!(ProfileId::from_str("statechronicle.profile.resource.v0").is_ok());
    }
}
