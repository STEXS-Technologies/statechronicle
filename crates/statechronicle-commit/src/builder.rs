//! Commit body assembly (protocol §13.1).
//!
//! [`CommitBuilder`] turns a validated [`CommitBatch`] into a [`Commit`]: it
//! computes the event Merkle root, derives the batch's state updates, computes
//! the next state root on top of the caller's prior state, and assembles the
//! body. The commit id is supplied per build through an injected generator.
//! Production ids come from the platform sequencer, and this crate never
//! invents randomness in non-test code.

use chrono::{DateTime, Utc};

use statechronicle_accumulator::sparse_merkle::StateUpdate;
use statechronicle_core::digest::ContentDigest;

use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
use statechronicle_domain::ids::CommitId;
use statechronicle_domain::subject::SubjectId;

use crate::batch::CommitBatch;
use crate::error::CommitError;
use crate::roots::{compute_state_root, event_root, state_root_updates};

/// Static identity and metadata for commit body assembly.
#[derive(Debug, Clone)]
pub struct CommitBuilder {
    /// The tenant or global checkpoint scope of the commit.
    scope: CommitScope,
    /// Monotonic commit sequence number.
    sequence: u64,
    /// The authorized commit executor that will sign the body.
    executor: SubjectId,
    /// The profile whose rules produced the state roots.
    profile: ProfileId,
    /// When the commit was created (UTC wall-clock metadata).
    created_at: DateTime<Utc>,
    /// The previous canonical commit, absent for a genesis commit.
    parent_commit_id: Option<CommitId>,
}

impl CommitBuilder {
    /// Creates a builder for a commit with fixed identity fields.
    pub const fn new(
        scope: CommitScope,
        sequence: u64,
        executor: SubjectId,
        profile: ProfileId,
        created_at: DateTime<Utc>,
        parent_commit_id: Option<CommitId>,
    ) -> Self {
        Self {
            scope,
            sequence,
            executor,
            profile,
            created_at,
            parent_commit_id,
        }
    }

    /// Assembles the commit body for `batch`.
    ///
    /// `previous_state_root` is the declared state root before this batch;
    /// `prior_updates` is the full accumulated leaf set committed before it
    /// (the union of every earlier commit's [`StateUpdate`]s, empty for a
    /// genesis commit). The next state root is computed by inserting
    /// `prior_updates` and the batch's updates into an empty accumulator, so
    /// the result is a pure function of the total state set (ADR-005) and
    /// chains correctly across commits.
    ///
    /// `next_commit_id` supplies the commit id fail-closed: production ids are
    /// minted by the platform sequencer, never invented here.
    ///
    /// # Errors
    ///
    /// Returns [`CommitError::EmptyBatch`] or [`CommitError::SizeLimitExceeded`]
    /// when the batch is invalid, [`CommitError::Core`] when an event cannot be
    /// BCS canonicalized, [`CommitError::InvalidEvent`] when an after-state
    /// cannot be keyed, [`CommitError::Accumulator`] when the accumulator
    /// rejects the updates, [`CommitError::Domain`] when the id generator
    /// yields an invalid commit id, or [`CommitError::InvalidEvent`] when the
    /// event count does not fit the `u64` field.
    pub fn build(
        &self,
        batch: &CommitBatch,
        previous_state_root: ContentDigest,
        prior_updates: &[StateUpdate],
        next_commit_id: impl FnOnce() -> Result<CommitId, CommitError>,
    ) -> Result<Commit, CommitError> {
        batch.validate()?;
        let events = batch.events();
        let event_merkle_root = event_root(events)?;
        let current_updates = state_root_updates(events)?;
        let mut all_updates = prior_updates.to_vec();
        all_updates.extend_from_slice(&current_updates);
        all_updates.sort_by_key(|a| a.key);
        let next_state_root = compute_state_root(&all_updates)?;
        let commit_id = next_commit_id()?;
        let event_count = u64::try_from(events.len()).map_err(|err| {
            CommitError::InvalidEvent(format!("event count does not fit in u64: {err}"))
        })?;
        Ok(Commit::new(
            self.scope.clone(),
            commit_id,
            self.parent_commit_id.clone(),
            self.sequence,
            event_count,
            event_merkle_root,
            previous_state_root,
            ContentDigest::new(*next_state_root.as_bytes()),
            self.created_at,
            self.executor.clone(),
            self.profile.clone(),
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use statechronicle_core::digest::hash_bytes;
    use statechronicle_domain::commit::{COMMIT_SCHEMA, ScopeKind};
    use statechronicle_domain::event::{Event, StateCommitment};
    use statechronicle_domain::ids::{EventId, IntentId};
    use statechronicle_domain::intent::Operation;
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::tenant::TenantId;

    fn tenant_scope() -> CommitScope {
        CommitScope::tenant(TenantId(String::from("stexs.game.alpha")))
    }

    fn profile() -> ProfileId {
        ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap()
    }

    fn executor() -> SubjectId {
        SubjectId(String::from("service:statechronicle.stexs.net"))
    }

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn sample_event(id: &str) -> Event {
        let state = serde_json::json!({ "owner": "account:stexs:player_456", "status": "active" });
        Event::new(
            TenantId(String::from("stexs.game.alpha")),
            EventId::new(format!("evt_{id}")).unwrap(),
            IntentId::new(format!("int_{id}")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            ResourceId(format!("asset:{id}")),
            SubjectId(String::from("account:stexs:player_123")),
            StateCommitment {
                version: 41,
                state_hash: hash_bytes(b"before"),
                state: serde_json::json!({}),
            },
            StateCommitment {
                version: 42,
                state_hash: hash_bytes(b"after"),
                state,
            },
            None,
            executor(),
            timestamp(),
        )
    }

    fn fixed_commit_id() -> Result<CommitId, CommitError> {
        CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).map_err(CommitError::from)
    }

    fn sample_batch() -> CommitBatch {
        let mut batch = CommitBatch::new(tenant_scope());
        batch.add_event(sample_event("a")).unwrap();
        batch.add_event(sample_event("b")).unwrap();
        batch
    }

    #[test]
    fn build_assembles_commit_body() {
        let batch = sample_batch();
        let builder = CommitBuilder::new(
            tenant_scope(),
            7,
            executor(),
            profile(),
            timestamp(),
            Some(CommitId::new(String::from("cmt_parent")).unwrap()),
        );
        let genesis_root = hash_bytes(b"genesis");
        let commit = builder
            .build(&batch, genesis_root.clone(), &[], fixed_commit_id)
            .unwrap();

        assert_eq!(commit.schema, COMMIT_SCHEMA);
        assert_eq!(commit.scope.kind, ScopeKind::Tenant);
        assert_eq!(commit.commit_id.as_str(), "cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W");
        assert_eq!(commit.parent_commit_id.unwrap().as_str(), "cmt_parent");
        assert_eq!(commit.sequence, 7);
        assert_eq!(commit.event_count, 2);
        assert_eq!(commit.previous_state_root, genesis_root);
        assert_eq!(commit.executor, executor());
        assert_eq!(commit.profile, profile());
        assert_eq!(commit.created_at, timestamp());
        // The event root is derived from the batch.
        assert_eq!(
            commit.event_merkle_root,
            event_root(batch.events()).unwrap()
        );
        // The next state root is derived from the batch updates on an empty
        // accumulator for a genesis commit.
        let updates = state_root_updates(batch.events()).unwrap();
        let expected = compute_state_root(&updates).unwrap();
        assert_eq!(commit.next_state_root.as_bytes(), expected.as_bytes());
    }

    #[test]
    fn build_rejects_empty_batch() {
        let batch = CommitBatch::new(tenant_scope());
        let builder =
            CommitBuilder::new(tenant_scope(), 1, executor(), profile(), timestamp(), None);
        let error = builder
            .build(&batch, hash_bytes(b"genesis"), &[], fixed_commit_id)
            .unwrap_err();
        assert!(matches!(error, CommitError::EmptyBatch));
    }

    #[test]
    fn build_chain_derives_next_root_on_prior_state() {
        // Commit 1 mutates resources a and b; commit 2 mutates resource c.
        let mut first = CommitBatch::new(tenant_scope());
        first.add_event(sample_event("a")).unwrap();
        first.add_event(sample_event("b")).unwrap();
        let mut second = CommitBatch::new(tenant_scope());
        second.add_event(sample_event("c")).unwrap();

        let builder =
            CommitBuilder::new(tenant_scope(), 1, executor(), profile(), timestamp(), None);
        let first_updates = state_root_updates(first.events()).unwrap();
        let first_commit = builder
            .build(&first, hash_bytes(b"genesis"), &[], fixed_commit_id)
            .unwrap();
        assert_eq!(
            first_commit.next_state_root.as_bytes(),
            compute_state_root(&first_updates).unwrap().as_bytes()
        );

        // Commit 2's next root must account for commit 1's leaves.
        let second_updates = state_root_updates(second.events()).unwrap();
        let mut combined = first_updates.clone();
        combined.extend_from_slice(&second_updates);
        combined.sort_by_key(|a| a.key);
        let expected = compute_state_root(&combined).unwrap();

        let second_builder = CommitBuilder::new(
            tenant_scope(),
            2,
            executor(),
            profile(),
            timestamp(),
            Some(CommitId::new(String::from("cmt_parent_1")).unwrap()),
        );
        let second_commit = second_builder
            .build(
                &second,
                first_commit.next_state_root.clone(),
                &first_updates,
                || CommitId::new(String::from("cmt_second")).map_err(CommitError::from),
            )
            .unwrap();
        assert_eq!(
            second_commit.previous_state_root,
            first_commit.next_state_root
        );
        assert_eq!(
            second_commit.next_state_root.as_bytes(),
            expected.as_bytes()
        );
    }
}
