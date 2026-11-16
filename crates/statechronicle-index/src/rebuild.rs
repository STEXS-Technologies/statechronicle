//! Deterministic current-state projection rebuild from canonical events.

#![allow(
    clippy::double_must_use,
    reason = "async_trait emits redundant must_use attributes"
)]

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::ContentDigest;
use statechronicle_domain::event::EVENT_SCHEMA;
use statechronicle_domain::event::Event;
use statechronicle_domain::ids::CommitId;
use statechronicle_domain::state::StateProjection;
use thiserror::Error;

/// Errors raised while rebuilding projections.
#[derive(Debug, Error)]
pub enum RebuildError {
    /// An event carries an invalid or conflicting scope.
    #[error("projection rebuild invariant violation: {0}")]
    Invariant(String),
    /// The projection sink could not be updated.
    #[error("projection sink unavailable: {0}")]
    Sink(String),
}

/// Write-side sink used only by a controlled rebuild operation.
#[async_trait]
pub trait ProjectionSink: Send + Sync {
    /// Replaces/upserts one projection while preserving monotonic versions.
    async fn upsert_projection(&self, projection: &StateProjection) -> Result<(), String>;
}

/// Durable checkpoint store for restartable projection rebuilds.
#[async_trait]
pub trait CheckpointStore: Send + Sync {
    /// Loads the next event offset for a rebuild key.
    async fn load(&self, key: &str) -> Result<Option<usize>, String>;
    /// Persists the next event offset after a successful chunk.
    async fn save(&self, key: &str, next_event: usize) -> Result<(), String>;
}

/// Durable progress marker for a chunked projection rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebuildCheckpoint {
    /// Offset of the next event to process.
    pub next_event: usize,
    /// Number of projections emitted by the completed chunk.
    pub projections_written: u64,
}

/// Operational progress for a resumable projection rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebuildProgress {
    /// Number of canonical events in the verified source stream.
    pub total_events: usize,
    /// Next event offset recorded by the durable checkpoint.
    pub next_event: usize,
    /// Events still pending before the rebuild is caught up.
    pub lag_events: usize,
    /// Whether the checkpoint has reached the end of the source stream.
    pub caught_up: bool,
}

/// Computes bounded projection-rebuild lag from a source length and checkpoint.
///
/// A checkpoint beyond the source stream is rejected instead of clamped: that
/// condition indicates stale/corrupt recovery metadata and must page an
/// operator rather than falsely reporting a healthy projection.
///
/// # Errors
///
/// Returns [`RebuildError::Invariant`] when `next_event` exceeds
/// `total_events`.
pub fn rebuild_progress(
    total_events: usize,
    next_event: usize,
) -> Result<RebuildProgress, RebuildError> {
    if next_event > total_events {
        return Err(RebuildError::Invariant(String::from(
            "rebuild checkpoint exceeds canonical event stream",
        )));
    }
    let lag_events = total_events.saturating_sub(next_event);
    Ok(RebuildProgress {
        total_events,
        next_event,
        lag_events,
        caught_up: lag_events == 0,
    })
}

// Owner-based state addresses exclude its mutable owner; subject-held state
// addresses include the holder, matching ResourceState and canonical state keys.
type ProjectionAddress = (String, String, Option<String>);
fn projection_address(event: &Event) -> ProjectionAddress {
    (
        event.tenant_id.0.clone(),
        event.resource_id.0.clone(),
        event.after.state.subject().map(|subject| subject.0.clone()),
    )
}

/// Rebuilds latest projections from events in canonical commit order.
///
/// Events are grouped by tenant/resource and, for subject-held state, subject.
/// The highest version is selected. Equal versions with different event/state hashes are rejected;
/// this prevents a reordered or forked stream from silently overwriting a
/// projection. The caller must supply events in verified canonical order.
///
/// # Errors
///
/// Returns [`RebuildError::Invariant`] for invalid scopes or conflicting
/// equal-version events, and [`RebuildError::Sink`] when the projection store
/// cannot be updated.
pub async fn rebuild_projections(
    events: &[(Event, CommitId)],
    sink: &dyn ProjectionSink,
) -> Result<u64, RebuildError> {
    validate_rebuild_stream(events)?;
    let mut latest: BTreeMap<ProjectionAddress, StateProjection> = BTreeMap::new();
    for (event, commit_id) in events {
        if event.tenant_id.0.is_empty() || event.resource_id.0.is_empty() {
            return Err(RebuildError::Invariant(String::from(
                "event tenant/resource scope must not be empty",
            )));
        }
        let projection = StateProjection {
            tenant_id: event.tenant_id.clone(),
            resource_id: event.resource_id.clone(),
            state_type: event.after.state.state_type(),
            version: event.after.version,
            last_event_id: event.event_id.clone(),
            last_commit_id: commit_id.clone(),
            state_hash: event.after.state_hash.clone(),
            state: event.after.state.clone(),
        };
        let key = projection_address(event);
        match latest.get(&key) {
            Some(existing) if projection.version < existing.version => {}
            Some(existing)
                if projection.version == existing.version
                    && (projection.state_hash != existing.state_hash
                        || projection.last_event_id != existing.last_event_id) =>
            {
                return Err(RebuildError::Invariant(format!(
                    "conflicting projection version {} for resource `{}`",
                    projection.version, projection.resource_id.0
                )));
            }
            _ => {
                latest.insert(key, projection);
            }
        }
    }
    for projection in latest.values() {
        sink.upsert_projection(projection)
            .await
            .map_err(RebuildError::Sink)?;
    }
    u64::try_from(latest.len()).map_err(|error| RebuildError::Invariant(error.to_string()))
}

/// Validates the complete source stream before any projection is written.
/// Continuity uses the full projection address because canonical commits may
/// interleave independent resources and holders. This check is intentionally
/// separate from chunk execution so a duplicate or fork split across chunks cannot be
/// hidden by resumable processing.
fn validate_rebuild_stream(events: &[(Event, CommitId)]) -> Result<(), RebuildError> {
    let mut event_ids = BTreeSet::new();
    let mut previous: BTreeMap<ProjectionAddress, (u64, ContentDigest)> = BTreeMap::new();
    for (event, commit_id) in events {
        validate_event_for_rebuild(event, commit_id)?;
        if event.tenant_id.0.is_empty() || event.resource_id.0.is_empty() {
            return Err(RebuildError::Invariant(String::from(
                "event tenant/resource scope must not be empty",
            )));
        }
        if !event_ids.insert((event.tenant_id.0.clone(), event.event_id.clone())) {
            return Err(RebuildError::Invariant(format!(
                "duplicate event id `{}` in rebuild stream",
                event.event_id
            )));
        }
        let key = projection_address(event);
        if previous.get(&key).is_some_and(|(version, state_hash)| {
            event.before.version != *version || event.before.state_hash != *state_hash
        }) {
            return Err(RebuildError::Invariant(format!(
                "event continuity mismatch for resource `{}`",
                event.resource_id.0
            )));
        }
        previous.insert(key, (event.after.version, event.after.state_hash.clone()));
    }
    Ok(())
}

/// Validates the integrity fields that a rebuild must not trust from a
/// deserialized or externally supplied event stream. Durable adapters should
/// additionally verify the enclosing commit signature and canonical chain;
/// this local check prevents a malformed event from poisoning a projection
/// even when a caller accidentally skips that higher-level validation.
fn validate_event_for_rebuild(event: &Event, commit_id: &CommitId) -> Result<(), RebuildError> {
    if event.schema != EVENT_SCHEMA {
        return Err(RebuildError::Invariant(String::from(
            "event schema is not the supported v0 schema",
        )));
    }
    CommitId::new(commit_id.0.clone()).map_err(|error| {
        RebuildError::Invariant(format!("invalid projection commit id: {error}"))
    })?;
    let expected_after = event.before.version.checked_add(1).ok_or_else(|| {
        RebuildError::Invariant(String::from("event before-state version overflows"))
    })?;
    if event.after.version != expected_after {
        return Err(RebuildError::Invariant(String::from(
            "event versions must advance by exactly one",
        )));
    }
    let before_digest = canonicalize_and_digest(&event.before.state)
        .map_err(|error| RebuildError::Invariant(error.to_string()))?;
    if before_digest != event.before.state_hash {
        return Err(RebuildError::Invariant(String::from(
            "event before-state digest does not match state",
        )));
    }
    let after_digest = canonicalize_and_digest(&event.after.state)
        .map_err(|error| RebuildError::Invariant(error.to_string()))?;
    if after_digest != event.after.state_hash {
        return Err(RebuildError::Invariant(String::from(
            "event after-state digest does not match state",
        )));
    }
    if event.before.state.state_type() != event.after.state.state_type() {
        return Err(RebuildError::Invariant(String::from(
            "event before/after state types must match",
        )));
    }
    Ok(())
}

/// Replays one bounded event chunk and returns a checkpoint for resumption.
///
/// The sink must provide monotonic upserts. A
/// caller may persist the returned checkpoint after each successful chunk and
/// resume after a process restart without reprocessing the prefix.
///
/// # Errors
///
/// Returns [`RebuildError::Invariant`] when `start_event` or `max_events` is
/// invalid, or any error produced by [`rebuild_projections`].
pub async fn rebuild_projections_chunk(
    events: &[(Event, CommitId)],
    start_event: usize,
    max_events: usize,
    sink: &dyn ProjectionSink,
) -> Result<RebuildCheckpoint, RebuildError> {
    if max_events == 0 || start_event > events.len() {
        return Err(RebuildError::Invariant(String::from(
            "invalid projection rebuild checkpoint",
        )));
    }
    let end = start_event.saturating_add(max_events).min(events.len());
    let chunk = events
        .get(start_event..end)
        .ok_or_else(|| RebuildError::Invariant(String::from("invalid projection rebuild range")))?;
    let written = rebuild_projections(chunk, sink).await?;
    Ok(RebuildCheckpoint {
        next_event: end,
        projections_written: written,
    })
}

/// Resumes a bounded projection rebuild from a durable checkpoint.
///
/// Projection writes happen before the checkpoint advances. If a process dies
/// between those operations, replaying the chunk is safe because sinks must
/// enforce monotonic upserts; the checkpoint is never advanced past an
/// unsuccessful write.
///
/// # Errors
///
/// Returns [`RebuildError::Invariant`] for an invalid chunk size/checkpoint,
/// [`RebuildError::Sink`] for projection or checkpoint-store failures.
pub async fn rebuild_projections_resumable(
    events: &[(Event, CommitId)],
    key: &str,
    chunk_size: usize,
    sink: &dyn ProjectionSink,
    checkpoints: &dyn CheckpointStore,
) -> Result<u64, RebuildError> {
    if key.is_empty() || chunk_size == 0 {
        return Err(RebuildError::Invariant(String::from(
            "rebuild key and chunk size must be non-empty",
        )));
    }
    validate_rebuild_stream(events)?;
    let mut offset = checkpoints
        .load(key)
        .await
        .map_err(RebuildError::Sink)?
        .unwrap_or(0);
    if offset > events.len() {
        return Err(RebuildError::Invariant(String::from(
            "stored rebuild checkpoint exceeds event stream",
        )));
    }
    let mut written = 0u64;
    while offset < events.len() {
        let checkpoint = rebuild_projections_chunk(events, offset, chunk_size, sink).await?;
        checkpoints
            .save(key, checkpoint.next_event)
            .await
            .map_err(RebuildError::Sink)?;
        written = written.saturating_add(checkpoint.projections_written);
        offset = checkpoint.next_event;
    }
    Ok(written)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use statechronicle_core::canonicalize::canonicalize_and_digest;
    use statechronicle_domain::event::StateCommitment;
    use statechronicle_domain::intent::Operation;
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::resource_state::{ResourceState, UniqueAssetState};
    use statechronicle_domain::status::Status;
    use statechronicle_domain::subject::SubjectId;
    use statechronicle_domain::tenant::TenantId;

    struct NoopSink;

    #[async_trait]
    impl ProjectionSink for NoopSink {
        async fn upsert_projection(&self, _projection: &StateProjection) -> Result<(), String> {
            Ok(())
        }
    }

    struct NoopCheckpoint;

    #[async_trait]
    impl CheckpointStore for NoopCheckpoint {
        async fn load(&self, _key: &str) -> Result<Option<usize>, String> {
            Ok(None)
        }

        async fn save(&self, _key: &str, _next_event: usize) -> Result<(), String> {
            Ok(())
        }
    }

    struct RecordingSink(std::sync::Mutex<Vec<StateProjection>>);

    #[async_trait]
    impl ProjectionSink for RecordingSink {
        async fn upsert_projection(&self, projection: &StateProjection) -> Result<(), String> {
            self.0
                .lock()
                .map_err(|error| error.to_string())?
                .push(projection.clone());
            Ok(())
        }
    }

    fn event(version: u64, owner: &str, event_id: &str) -> (Event, CommitId) {
        let state = ResourceState::UniqueAsset(UniqueAssetState {
            owner: SubjectId(String::from(owner)),
            status: Status::from_static("active"),
            trade_id: None,
        });
        let digest = canonicalize_and_digest(&state).unwrap();
        let before = StateCommitment {
            version: version.saturating_sub(1),
            state_hash: digest,
            state: state.clone(),
        };
        let after = StateCommitment {
            version,
            state_hash: canonicalize_and_digest(&state).unwrap(),
            state,
        };
        let event = Event::new(
            TenantId(String::from("game")),
            statechronicle_domain::ids::EventId::new(String::from(event_id)).unwrap(),
            statechronicle_domain::ids::IntentId::new(format!("int_{event_id}")).unwrap(),
            Operation::from_static("asset.transfer"),
            ResourceId(String::from("asset:sword")),
            SubjectId(String::from(owner)),
            before,
            after,
            None,
            SubjectId(String::from("service:ledger")),
            DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        (event, CommitId::new(format!("cmt_{event_id}")).unwrap())
    }

    fn held_event(version: u64, subject: &str, id: &str) -> (Event, CommitId) {
        use statechronicle_core::amount::Amount;
        use statechronicle_domain::resource_state::FungibleBalanceState;
        let (mut event, commit) = event(version, subject, id);
        event.resource_id = ResourceId(String::from("currency:coin"));
        event.operation = Operation::from_static("balance.credit");
        let state = |balance| {
            ResourceState::FungibleBalance(FungibleBalanceState {
                subject: SubjectId(subject.into()),
                balance: Amount::try_from_str(balance).unwrap(),
                unit: "COIN".into(),
            })
        };
        event.before.state = state(if version == 1 { "0" } else { "10" });
        event.before.state_hash = canonicalize_and_digest(&event.before.state).unwrap();
        event.after.state = state(if version == 1 { "10" } else { "20" });
        event.after.state_hash = canonicalize_and_digest(&event.after.state).unwrap();
        (event, commit)
    }
    #[tokio::test]
    async fn rebuild_keeps_independent_holders_and_their_own_continuity() {
        let events = vec![
            held_event(1, "alice", "evt_alice_1"),
            held_event(1, "bob", "evt_bob_1"),
            held_event(2, "alice", "evt_alice_2"),
        ];
        let sink = RecordingSink(std::sync::Mutex::new(Vec::new()));
        assert_eq!(rebuild_projections(&events, &sink).await.unwrap(), 2);
        let rows = sink.0.lock().unwrap();
        assert_eq!(rows.len(), 2);
        let alice = rows
            .iter()
            .find(|p| p.state.subject().unwrap().0 == "alice")
            .unwrap();
        let bob = rows
            .iter()
            .find(|p| p.state.subject().unwrap().0 == "bob")
            .unwrap();
        assert_eq!(alice.version, 2);
        assert_eq!(alice.last_event_id, events.get(2).unwrap().0.event_id);
        assert_eq!(bob.version, 1);
        assert_eq!(bob.last_event_id, events.get(1).unwrap().0.event_id);
    }
    #[tokio::test]
    async fn rebuild_event_identity_is_scoped_by_tenant() {
        let first = event(1, "alice", "evt_shared");
        let mut second = first.clone();
        second.0.tenant_id = TenantId("other".into());
        let sink = RecordingSink(std::sync::Mutex::new(Vec::new()));
        assert_eq!(
            rebuild_projections(&[first, second], &sink).await.unwrap(),
            2
        );
        assert_eq!(sink.0.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn holder_continuity_and_same_tenant_duplicate_checks_fail_before_writes() {
        let source = vec![
            held_event(1, "alice", "evt_alice_1"),
            held_event(1, "bob", "evt_bob_1"),
            held_event(2, "alice", "evt_alice_2"),
        ];
        let mut bad_before = source.clone();
        bad_before.get_mut(2).unwrap().0.before = source.get(1).unwrap().0.after.clone();
        let mut reordered = source.clone();
        reordered.swap(0, 2);
        let mut duplicate = source.clone();
        duplicate.get_mut(1).unwrap().0.event_id = source.first().unwrap().0.event_id.clone();
        for invalid in [bad_before, reordered, duplicate] {
            let sink = RecordingSink(std::sync::Mutex::new(Vec::new()));
            assert!(matches!(
                rebuild_projections(&invalid, &sink).await,
                Err(RebuildError::Invariant(_))
            ));
            assert!(sink.0.lock().unwrap().is_empty());
            assert!(matches!(
                rebuild_projections_resumable(&invalid, "test", 1, &sink, &NoopCheckpoint).await,
                Err(RebuildError::Invariant(_))
            ));
            assert!(sink.0.lock().unwrap().is_empty());
        }
    }

    #[derive(Default)]
    struct RestartableRebuild {
        rows: std::sync::Mutex<Vec<StateProjection>>,
        offset: std::sync::Mutex<Option<usize>>,
        saves: std::sync::atomic::AtomicUsize,
        fail_save: usize,
    }
    #[async_trait]
    impl ProjectionSink for RestartableRebuild {
        async fn upsert_projection(&self, projection: &StateProjection) -> Result<(), String> {
            let mut rows = self.rows.lock().unwrap();
            if let Some(old) = rows.iter_mut().find(|old| {
                old.tenant_id == projection.tenant_id
                    && old.resource_id == projection.resource_id
                    && old.state.subject() == projection.state.subject()
            }) {
                if old.version > projection.version {
                    return Ok(());
                }
                if old.version == projection.version && old != projection {
                    return Err("conflicting equal version".into());
                }
                *old = projection.clone();
            } else {
                rows.push(projection.clone());
            }
            Ok(())
        }
    }
    #[async_trait]
    impl CheckpointStore for RestartableRebuild {
        async fn load(&self, _key: &str) -> Result<Option<usize>, String> {
            Ok(*self.offset.lock().unwrap())
        }
        async fn save(&self, _key: &str, offset: usize) -> Result<(), String> {
            let count = self.saves.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if count == self.fail_save {
                return Err("injected checkpoint failure after projection writes".into());
            }
            *self.offset.lock().unwrap() = Some(offset);
            Ok(())
        }
    }
    #[tokio::test]
    async fn holder_rebuild_resumes_after_each_checkpoint_failure() {
        let source = vec![
            held_event(1, "alice", "evt_alice_1"),
            held_event(1, "bob", "evt_bob_1"),
            held_event(2, "alice", "evt_alice_2"),
            held_event(2, "bob", "evt_bob_2"),
        ];
        for width in [1, 2, 3] {
            for fail_save in 1..=source.len().div_ceil(width) {
                let durable = RestartableRebuild {
                    fail_save,
                    ..Default::default()
                };
                assert!(matches!(
                    rebuild_projections_resumable(
                        &source,
                        "pinned-history",
                        width,
                        &durable,
                        &durable
                    )
                    .await,
                    Err(RebuildError::Sink(_))
                ));
                assert_eq!(
                    durable.offset.lock().unwrap().unwrap_or(0),
                    (fail_save - 1) * width
                );
                rebuild_projections_resumable(&source, "pinned-history", width, &durable, &durable)
                    .await
                    .unwrap();
                assert_eq!(*durable.offset.lock().unwrap(), Some(4));
                let rows = durable.rows.lock().unwrap();
                assert_eq!(rows.len(), 2);
                for (subject, index) in [("alice", 2), ("bob", 3)] {
                    let row = rows
                        .iter()
                        .find(|p| p.state.subject().unwrap().0 == subject)
                        .unwrap();
                    assert_eq!(row.version, 2);
                    assert_eq!(row.last_event_id, source.get(index).unwrap().0.event_id);
                    assert_eq!(row.last_commit_id, source.get(index).unwrap().1);
                    assert_eq!(row.state, source.get(index).unwrap().0.after.state);
                }
            }
        }
    }

    #[tokio::test]
    async fn empty_event_stream_is_a_noop() {
        assert_eq!(rebuild_projections(&[], &NoopSink).await.unwrap(), 0);
        assert!(
            rebuild_projections_chunk(&[], 0, 0, &NoopSink)
                .await
                .is_err()
        );
        assert_eq!(
            rebuild_projections_resumable(&[], "test", 10, &NoopSink, &NoopCheckpoint)
                .await
                .unwrap(),
            0
        );
    }

    #[test]
    fn rebuild_progress_reports_lag_and_rejects_corrupt_checkpoint() {
        let progress = rebuild_progress(12, 7).unwrap();
        assert_eq!(progress.total_events, 12);
        assert_eq!(progress.next_event, 7);
        assert_eq!(progress.lag_events, 5);
        assert!(!progress.caught_up);

        let caught_up = rebuild_progress(12, 12).unwrap();
        assert_eq!(caught_up.lag_events, 0);
        assert!(caught_up.caught_up);

        assert!(rebuild_progress(12, 13).is_err());
    }

    #[tokio::test]
    async fn chunk_rebuild_writes_latest_projection_and_checkpoint() {
        let events = vec![
            event(1, "alice", "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4"),
            event(2, "bob", "evt_01JZ8X5HN3C4PXG5A9FGEWQF5W"),
        ];
        let sink = RecordingSink(std::sync::Mutex::new(Vec::new()));
        let checkpoint = rebuild_projections_chunk(&events, 0, 1, &sink)
            .await
            .unwrap();
        assert_eq!(checkpoint.next_event, 1);
        assert_eq!(checkpoint.projections_written, 1);
        rebuild_projections_chunk(&events, checkpoint.next_event, 1, &sink)
            .await
            .unwrap();
        let projections = sink.0.lock().unwrap();
        assert_eq!(projections.last().unwrap().version, 2);
        assert_eq!(projections.last().unwrap().state.owner().unwrap().0, "bob");
    }

    #[tokio::test]
    async fn rebuild_rejects_tampered_event_integrity_fields() {
        let (mut tampered_event, commit_id) = event(1, "alice", "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4");
        tampered_event.after.state_hash = statechronicle_core::digest::hash_bytes(b"tampered");
        assert!(
            rebuild_projections(&[(tampered_event, commit_id)], &NoopSink)
                .await
                .is_err()
        );

        let (mut schema_event, schema_commit) = event(1, "alice", "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4");
        schema_event.schema = String::from("statechronicle.event.v99");
        assert!(
            rebuild_projections(&[(schema_event, schema_commit)], &NoopSink)
                .await
                .is_err()
        );

        let (valid_event, _) = event(1, "alice", "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4");
        let invalid_commit = CommitId(String::from("not-a-commit-id"));
        assert!(
            rebuild_projections(&[(valid_event, invalid_commit)], &NoopSink)
                .await
                .is_err()
        );

        let (first_event, first_commit) = event(1, "alice", "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4");
        let (mut duplicate_event, duplicate_commit) =
            event(2, "bob", "evt_01JZ8X5HN3C4PXG5A9FGEWQF5W");
        duplicate_event.event_id = first_event.event_id.clone();
        assert!(
            rebuild_projections(
                &[
                    (first_event, first_commit),
                    (duplicate_event, duplicate_commit)
                ],
                &NoopSink
            )
            .await
            .is_err()
        );
    }
}
