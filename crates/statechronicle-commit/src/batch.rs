//! Commit batching.
//!
//! [`CommitBatch`] accumulates validated events for one commit (protocol
//! §13.1). It is pure and deterministic: events are appended in call order,
//! every event in a tenant-scoped batch must share the batch's tenant, and
//! duplicate event ids are rejected fail-closed (§18.2). Batching is separated
//! from ordering. The executor's parallel lane orders the batch
//! deterministically before this crate consumes it.

use statechronicle_core::canonicalize::canonicalize;
use statechronicle_core::limits::MAX_COMMIT_BYTES;

use statechronicle_domain::commit::CommitScope;
use statechronicle_domain::event::Event;

use crate::error::CommitError;

/// An ordered batch of validated events destined for a single commit.
#[derive(Debug, Clone)]
pub struct CommitBatch {
    /// The tenant or global checkpoint scope of the enclosing commit.
    scope: CommitScope,
    /// The ordered events, in batch (commit) order.
    events: Vec<Event>,
}

impl CommitBatch {
    /// Creates an empty batch for the given commit scope.
    pub const fn new(scope: CommitScope) -> Self {
        Self {
            scope,
            events: Vec::new(),
        }
    }

    /// Returns the commit scope.
    pub const fn scope(&self) -> &CommitScope {
        &self.scope
    }

    /// Returns the events in batch order.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Returns the number of events in the batch.
    pub const fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Returns whether the batch holds no events.
    pub const fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Appends a validated event, fail-closed on scope or id conflicts.
    ///
    /// For a tenant-scoped batch, every event must belong to the batch's
    /// tenant (protocol §13.1). A global-checkpoint-scoped batch holds no
    /// direct events (§13.4), so no tenant constraint is applied there.
    ///
    /// # Errors
    ///
    /// Returns [`CommitError::MixedTenant`] when the batch is tenant-scoped
    /// and `event.tenant_id` differs from the batch tenant, and
    /// [`CommitError::DuplicateEventId`] when the batch already contains an
    /// event with the same id.
    pub fn add_event(&mut self, event: Event) -> Result<(), CommitError> {
        if let Some(tenant) = &self.scope.tenant_id
            && tenant != &event.tenant_id
        {
            return Err(CommitError::MixedTenant);
        }
        if self
            .events
            .iter()
            .any(|existing| existing.event_id == event.event_id)
        {
            return Err(CommitError::DuplicateEventId {
                event_id: String::from(event.event_id.as_str()),
            });
        }
        self.events.push(event);
        Ok(())
    }

    /// Validates the batch for commit formation: non-empty and within
    /// [`MAX_COMMIT_BYTES`] of BCS canonical event bytes (protocol §30).
    ///
    /// The size bound is enforced here (at build time) rather than on every
    /// [`Self::add_event`], so a batch can grow freely while it is being
    /// assembled and only fails closed when it is committed.
    ///
    /// # Errors
    ///
    /// Returns [`CommitError::EmptyBatch`] when the batch holds no events,
    /// [`CommitError::Core`] when an event cannot be BCS canonicalized, and
    /// [`CommitError::SizeLimitExceeded`] when the summed canonical event
    /// bytes exceed [`MAX_COMMIT_BYTES`].
    pub fn validate(&self) -> Result<(), CommitError> {
        if self.events.is_empty() {
            return Err(CommitError::EmptyBatch);
        }
        let mut total = 0usize;
        for event in &self.events {
            let len = canonicalize(event)?.len();
            total = total
                .checked_add(len)
                .ok_or_else(|| CommitError::SizeLimitExceeded {
                    name: String::from("commit"),
                    limit: MAX_COMMIT_BYTES,
                    actual: usize::MAX,
                })?;
        }
        if total > MAX_COMMIT_BYTES {
            return Err(CommitError::SizeLimitExceeded {
                name: String::from("commit"),
                limit: MAX_COMMIT_BYTES,
                actual: total,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use statechronicle_core::canonicalize::canonicalize_and_digest;
    use statechronicle_domain::commit::ScopeKind;
    use statechronicle_domain::event::StateCommitment;
    use statechronicle_domain::ids::{EventId, IntentId};
    use statechronicle_domain::intent::Operation;
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::subject::SubjectId;
    use statechronicle_domain::tenant::TenantId;

    fn tenant_scope() -> CommitScope {
        CommitScope::tenant(TenantId(String::from("stexs.game.alpha")))
    }

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn sample_commitment(version: u64, state: serde_json::Value) -> StateCommitment {
        StateCommitment {
            version,
            state_hash: canonicalize_and_digest(&state).unwrap(),
            state,
        }
    }

    fn sample_event(tenant: &str, id: &str) -> Event {
        let state = serde_json::json!({ "owner": "account:stexs:player_456", "status": "active" });
        Event::new(
            TenantId(String::from(tenant)),
            EventId::new(format!("evt_{id}")).unwrap(),
            IntentId::new(format!("int_{id}")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            ResourceId(format!("asset:{id}")),
            SubjectId(String::from("account:stexs:player_123")),
            sample_commitment(41, serde_json::json!({})),
            sample_commitment(42, state),
            None,
            SubjectId(String::from("service:statechronicle.stexs.net")),
            timestamp(),
        )
    }

    #[test]
    fn new_batch_is_empty_with_declared_scope() {
        let batch = CommitBatch::new(tenant_scope());
        assert!(batch.is_empty());
        assert_eq!(batch.event_count(), 0);
        assert_eq!(batch.scope().kind, ScopeKind::Tenant);
    }

    #[test]
    fn add_event_appends_in_order() {
        let mut batch = CommitBatch::new(tenant_scope());
        batch
            .add_event(sample_event("stexs.game.alpha", "one"))
            .unwrap();
        batch
            .add_event(sample_event("stexs.game.alpha", "two"))
            .unwrap();
        assert_eq!(batch.event_count(), 2);
        assert_eq!(batch.events().first().unwrap().event_id.as_str(), "evt_one");
        assert_eq!(batch.events().get(1).unwrap().event_id.as_str(), "evt_two");
    }

    #[test]
    fn mixed_tenant_event_is_rejected() {
        let mut batch = CommitBatch::new(tenant_scope());
        batch
            .add_event(sample_event("stexs.game.alpha", "one"))
            .unwrap();
        let error = batch
            .add_event(sample_event("stexs.game.beta", "two"))
            .unwrap_err();
        assert!(matches!(error, CommitError::MixedTenant));
        assert_eq!(batch.event_count(), 1);
    }

    #[test]
    fn duplicate_event_id_is_rejected() {
        let mut batch = CommitBatch::new(tenant_scope());
        batch
            .add_event(sample_event("stexs.game.alpha", "one"))
            .unwrap();
        let error = batch
            .add_event(sample_event("stexs.game.alpha", "one"))
            .unwrap_err();
        assert!(matches!(
            error,
            CommitError::DuplicateEventId { event_id } if event_id == "evt_one"
        ));
        assert_eq!(batch.event_count(), 1);
    }

    #[test]
    fn empty_batch_fails_closed_on_validate() {
        let batch = CommitBatch::new(tenant_scope());
        assert!(matches!(batch.validate(), Err(CommitError::EmptyBatch)));
    }

    #[test]
    fn validate_accepts_well_sized_batch() {
        let mut batch = CommitBatch::new(tenant_scope());
        batch
            .add_event(sample_event("stexs.game.alpha", "one"))
            .unwrap();
        assert!(batch.validate().is_ok());
    }

    #[test]
    fn validate_rejects_oversized_batch() {
        let mut batch = CommitBatch::new(tenant_scope());
        let blob = "x".repeat(MAX_COMMIT_BYTES.saturating_add(1));
        let state = serde_json::json!({ "blob": blob });
        let after = sample_commitment(1, state);
        let before = sample_commitment(0, serde_json::json!({}));
        let event = Event::new(
            TenantId(String::from("stexs.game.alpha")),
            EventId::new(String::from("evt_big")).unwrap(),
            IntentId::new(String::from("int_big")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            ResourceId(String::from("asset:big")),
            SubjectId(String::from("account:stexs:player_123")),
            before,
            after,
            None,
            SubjectId(String::from("service:statechronicle.stexs.net")),
            timestamp(),
        );
        batch.add_event(event).unwrap();
        let error = batch.validate().unwrap_err();
        assert!(matches!(error, CommitError::SizeLimitExceeded { .. }));
    }
}
