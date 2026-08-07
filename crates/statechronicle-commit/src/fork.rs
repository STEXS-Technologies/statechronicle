//! Fork and failure semantics (protocol §31).
//!
//! A fork occurs when two different commits claim the same parent and
//! sequence under the same canonical scope. Baseline behavior: verifiers must
//! reject ambiguous forks unless a configured fork-resolution policy exists;
//! implementations maintain append-only evidence of rejected or superseded
//! commits; and recovery never rewrites accepted event objects without
//! preserving audit history.
//!
//! This module supplies the pure, fail-closed predicates — [`detect_fork`],
//! [`check_chain_continuity`], and [`validate_no_event_rewrite`] — plus the
//! [`ForkEvidence`] value record an implementation persists append-only.
//! Nothing here performs persistence or policy; those are the platform's job.

use core::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use statechronicle_core::canonicalize::canonicalize_and_digest;

use statechronicle_domain::commit::Commit;
use statechronicle_domain::event::Event;
use statechronicle_domain::ids::CommitId;

use crate::error::CommitError;

/// Detects a fork under `previous`: two candidates claiming the same parent
/// and sequence with different commit ids (protocol §31).
///
/// Returns `Ok` when there is no ambiguity — different parents, different
/// sequences, or identical commit ids.
///
/// # Errors
///
/// Returns [`CommitError::ForkDetected`] when both `candidate_a` and
/// `candidate_b` declare `previous.commit_id` as their parent, share the same
/// `sequence`, and have different commit ids.
pub fn detect_fork(
    previous: &Commit,
    candidate_a: &Commit,
    candidate_b: &Commit,
) -> Result<(), CommitError> {
    if candidate_a.parent_commit_id.as_ref() == Some(&previous.commit_id)
        && candidate_b.parent_commit_id.as_ref() == Some(&previous.commit_id)
        && candidate_a.sequence == candidate_b.sequence
        && candidate_a.commit_id != candidate_b.commit_id
    {
        return Err(CommitError::ForkDetected {
            parent: String::from(previous.commit_id.as_str()),
            sequence: candidate_a.sequence,
        });
    }
    Ok(())
}

/// Verifies that `next` continues the chain from `previous` (protocol §31).
///
/// `next.parent_commit_id` must equal `previous.commit_id` and
/// `next.sequence` must equal `previous.sequence + 1` (checked arithmetic,
/// overflow fails closed).
///
/// # Errors
///
/// Returns [`CommitError::ChainGap`] when `next` does not declare
/// `previous.commit_id` as its parent, and [`CommitError::SequenceMismatch`]
/// when the parent link is correct but `next.sequence` does not continue the
/// previous sequence.
pub fn check_chain_continuity(previous: &Commit, next: &Commit) -> Result<(), CommitError> {
    if next.parent_commit_id.as_ref() != Some(&previous.commit_id) {
        return Err(CommitError::ChainGap {
            expected_parent: String::from(previous.commit_id.as_str()),
            actual_parent: next
                .parent_commit_id
                .as_ref()
                .map(|id| String::from(id.as_str())),
        });
    }
    let expected_sequence =
        previous
            .sequence
            .checked_add(1)
            .ok_or(CommitError::SequenceMismatch {
                expected: u64::MAX,
                actual: next.sequence,
            })?;
    if next.sequence != expected_sequence {
        return Err(CommitError::SequenceMismatch {
            expected: expected_sequence,
            actual: next.sequence,
        });
    }
    Ok(())
}

/// Verifies that `candidate` is not a rewrite of the accepted event `accepted`
/// (protocol §31 recovery).
///
/// Recovery must never rewrite accepted event objects: an event resubmitted
/// under the same id with a different payload fails closed. Comparison is over
/// the BCS canonical digest of the full event. Events with different ids are
/// distinct and always `Ok`.
///
/// # Errors
///
/// Returns [`CommitError::Core`] when an event cannot be BCS canonicalized,
/// and [`CommitError::EventRewrite`] when both events share an id but their
/// canonical payloads differ.
pub fn validate_no_event_rewrite(accepted: &Event, candidate: &Event) -> Result<(), CommitError> {
    if accepted.event_id != candidate.event_id {
        return Ok(());
    }
    let accepted_digest = canonicalize_and_digest(accepted)?;
    let candidate_digest = canonicalize_and_digest(candidate)?;
    if accepted_digest != candidate_digest {
        return Err(CommitError::EventRewrite {
            event_id: String::from(accepted.event_id.as_str()),
        });
    }
    Ok(())
}

/// Append-only evidence that one commit superseded another (protocol §31).
///
/// An implementation persists records of rejected/superseded commits
/// append-only, preserving audit history. This is a pure value type; it does
/// not perform any persistence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkEvidence {
    /// The commit that won (was accepted) at this position.
    pub superseded_commit_id: CommitId,
    /// The commit that lost (was rejected) at this position.
    pub rejected_commit_id: CommitId,
    /// The shared parent commit of the contested pair.
    pub parent_commit_id: CommitId,
    /// The contested sequence number.
    pub sequence: u64,
    /// When the evidence was recorded (UTC).
    pub recorded_at: DateTime<Utc>,
}

impl fmt::Display for ForkEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "fork evidence: superseded {}, rejected {}, parent {}, sequence {}",
            self.superseded_commit_id.as_str(),
            self.rejected_commit_id.as_str(),
            self.parent_commit_id.as_str(),
            self.sequence
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use statechronicle_core::digest::hash_bytes;
    use statechronicle_domain::commit::{CommitScope, ProfileId};
    use statechronicle_domain::event::StateCommitment;
    use statechronicle_domain::ids::{EventId, IntentId};
    use statechronicle_domain::intent::Operation;
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::subject::SubjectId;
    use statechronicle_domain::tenant::TenantId;

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-14T00:00:06Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn profile() -> ProfileId {
        ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap()
    }

    fn executor() -> SubjectId {
        SubjectId(String::from("service:statechronicle.stexs.net"))
    }

    fn commit(id: &str, parent: Option<&str>, sequence: u64) -> Commit {
        Commit::new(
            CommitScope::tenant(TenantId(String::from("stexs.game.alpha"))),
            CommitId::new(String::from(id)).unwrap(),
            parent.map(|value| CommitId::new(String::from(value)).unwrap()),
            sequence,
            1,
            hash_bytes(b"event-root"),
            hash_bytes(b"previous-root"),
            hash_bytes(b"next-root"),
            timestamp(),
            executor(),
            profile(),
        )
    }

    fn event(id: &str, resource: &str, owner: &str) -> Event {
        let state = serde_json::json!({ "owner": owner, "status": "active" });
        Event::new(
            TenantId(String::from("stexs.game.alpha")),
            EventId::new(format!("evt_{id}")).unwrap(),
            IntentId::new(format!("int_{id}")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            ResourceId(String::from(resource)),
            SubjectId(String::from("account:stexs:player_123")),
            StateCommitment {
                version: 1,
                state_hash: hash_bytes(b"before"),
                state: serde_json::json!({}),
            },
            StateCommitment {
                version: 2,
                state_hash: hash_bytes(b"after"),
                state,
            },
            None,
            executor(),
            timestamp(),
        )
    }

    #[test]
    fn detect_fork_rejects_two_heads() {
        let previous = commit("cmt_parent", None, 1);
        let candidate_a = commit("cmt_a", Some("cmt_parent"), 2);
        let candidate_b = commit("cmt_b", Some("cmt_parent"), 2);
        let error = detect_fork(&previous, &candidate_a, &candidate_b).unwrap_err();
        assert!(matches!(
            error,
            CommitError::ForkDetected { parent, sequence }
            if parent == "cmt_parent" && sequence == 2
        ));
    }

    #[test]
    fn detect_fork_ok_same_commit_id() {
        let previous = commit("cmt_parent", None, 1);
        let candidate = commit("cmt_a", Some("cmt_parent"), 2);
        assert!(detect_fork(&previous, &candidate, &candidate).is_ok());
    }

    #[test]
    fn detect_fork_ok_different_parents() {
        let previous = commit("cmt_parent", None, 1);
        let candidate_a = commit("cmt_a", Some("cmt_parent"), 2);
        let candidate_b = commit("cmt_b", Some("cmt_other"), 2);
        assert!(detect_fork(&previous, &candidate_a, &candidate_b).is_ok());
    }

    #[test]
    fn detect_fork_ok_different_sequences() {
        let previous = commit("cmt_parent", None, 1);
        let candidate_a = commit("cmt_a", Some("cmt_parent"), 2);
        let candidate_b = commit("cmt_b", Some("cmt_parent"), 3);
        assert!(detect_fork(&previous, &candidate_a, &candidate_b).is_ok());
    }

    #[test]
    fn chain_continuity_ok() {
        let previous = commit("cmt_1", None, 1);
        let next = commit("cmt_2", Some("cmt_1"), 2);
        assert!(check_chain_continuity(&previous, &next).is_ok());
    }

    #[test]
    fn chain_continuity_rejects_wrong_parent() {
        let previous = commit("cmt_1", None, 1);
        let next = commit("cmt_2", Some("cmt_other"), 2);
        let error = check_chain_continuity(&previous, &next).unwrap_err();
        assert!(matches!(
            error,
            CommitError::ChainGap { expected_parent, actual_parent }
            if expected_parent == "cmt_1"
                && actual_parent == Some(String::from("cmt_other"))
        ));
    }

    #[test]
    fn chain_continuity_rejects_missing_parent() {
        let previous = commit("cmt_1", None, 1);
        let next = commit("cmt_2", None, 2);
        let error = check_chain_continuity(&previous, &next).unwrap_err();
        assert!(matches!(
            error,
            CommitError::ChainGap { expected_parent, actual_parent }
            if expected_parent == "cmt_1" && actual_parent.is_none()
        ));
    }

    #[test]
    fn chain_continuity_rejects_sequence_mismatch() {
        let previous = commit("cmt_1", None, 1);
        let next = commit("cmt_2", Some("cmt_1"), 3);
        let error = check_chain_continuity(&previous, &next).unwrap_err();
        assert!(matches!(
            error,
            CommitError::SequenceMismatch { expected, actual }
            if expected == 2 && actual == 3
        ));
    }

    #[test]
    fn no_event_rewrite_ok_identical_payload() {
        let accepted = event("sword", "asset:sword", "alice");
        let candidate = event("sword", "asset:sword", "alice");
        assert!(validate_no_event_rewrite(&accepted, &candidate).is_ok());
    }

    #[test]
    fn no_event_rewrite_ok_distinct_ids() {
        let accepted = event("sword", "asset:sword", "alice");
        let candidate = event("shield", "asset:shield", "bob");
        assert!(validate_no_event_rewrite(&accepted, &candidate).is_ok());
    }

    #[test]
    fn event_rewrite_is_rejected() {
        let accepted = event("sword", "asset:sword", "alice");
        let candidate = event("sword", "asset:sword", "bob");
        let error = validate_no_event_rewrite(&accepted, &candidate).unwrap_err();
        assert!(matches!(
            error,
            CommitError::EventRewrite { event_id } if event_id == "evt_sword"
        ));
    }

    #[test]
    fn fork_evidence_display_mentions_all_ids() {
        let evidence = ForkEvidence {
            superseded_commit_id: CommitId::new(String::from("cmt_a")).unwrap(),
            rejected_commit_id: CommitId::new(String::from("cmt_b")).unwrap(),
            parent_commit_id: CommitId::new(String::from("cmt_parent")).unwrap(),
            sequence: 2,
            recorded_at: timestamp(),
        };
        let rendered = evidence.to_string();
        assert!(rendered.contains("cmt_a"));
        assert!(rendered.contains("cmt_b"));
        assert!(rendered.contains("cmt_parent"));
        assert!(rendered.contains("sequence 2"));
    }
}
