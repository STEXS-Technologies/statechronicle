//! Commit persistence (protocol §18.1 step 15).
//!
//! [`persist`] writes a signed commit and its events to the stores and
//! publishes them. Writes are validated fail-closed against the commit body
//! before anything is persisted: the supplied events must match the declared
//! `event_count` and recompute the declared `event_merkle_root`, otherwise
//! nothing is written.
//!
//! The v0 [`StateIndex`] port is read-only (§27): it exposes `get_state` and
//! `get_subject_state` but no write operation. [`persist`] therefore derives
//! the current-state projections via [`projections_for`] and leaves applying
//! them to the composition root's index adapter (e.g. inside its
//! `TransactionManager`) — the documented integration point for projection
//! writes in this workspace.
//!
//! The `Commit` body carries only `event_count` and `event_merkle_root`
//! (protocol §13.1), not the events themselves, so [`persist`] takes the
//! committed events (with their state types) from the caller — the executor
//! that assembled the batch.

use statechronicle_domain::commit::{Commit, ScopeKind};
use statechronicle_domain::event::Event;
use statechronicle_domain::ids::CommitId;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::tenant::TenantId;

use statechronicle_ports::commit_store::CommitStore;
use statechronicle_ports::event_publisher::EventPublisher;
use statechronicle_ports::event_store::EventStore;
use statechronicle_ports::state_index::StateIndex;

use crate::error::CommitError;
use crate::roots::event_root;

/// Backend-agnostic store set used by [`persist`].
///
/// Holds the driven ports as `Send + Sync` trait objects so the persistence
/// path composes in any runtime. Kept distinct from the executor crate's own
/// ports struct to avoid coupling the two lanes.
pub struct CommitPorts {
    /// Append-only commit store.
    pub commit_store: Box<dyn CommitStore>,
    /// Append-only event store.
    pub event_store: Box<dyn EventStore>,
    /// Read-only current-state index (§27).
    pub state_index: Box<dyn StateIndex>,
    /// Optional event/commit publisher, used when present.
    pub event_publisher: Option<Box<dyn EventPublisher>>,
}

/// A committed event paired with the state type that shaped its projection.
#[derive(Debug, Clone, Copy)]
pub struct CommittedEvent<'event> {
    /// The committed event.
    pub event: &'event Event,
    /// The state type of the event's resource, needed to derive the index
    /// projection (the event body itself does not carry `StateType`).
    pub state_type: StateType,
}

/// Persists a signed commit and its events, fail-closed on validation.
///
/// The commit is stored via [`CommitStore`], the events are appended via
/// [`EventStore`], and both are published through [`EventPublisher`] when one
/// is present. Current-state projections are derived via [`projections_for`]
/// for the composition root's index adapter (see module docs).
///
/// # Errors
///
/// Returns [`CommitError::Store`] when the commit is not tenant-scoped (global
/// checkpoints carry tenant roots, not direct events), or a store/publisher
/// rejects the write; [`CommitError::EventRootMismatch`] when `entries` does
/// not match the commit's declared event count or Merkle root;
/// [`CommitError::InvalidEvent`] when the event count does not fit the `u64`
/// field; and [`CommitError::Core`] or [`CommitError::EmptyBatch`] when the
/// event root cannot be recomputed.
pub async fn persist(
    ports: &CommitPorts,
    commit: &Signed<Commit>,
    entries: &[CommittedEvent<'_>],
) -> Result<(), CommitError> {
    let tenant = commit_tenant(&commit.body)?;
    let events: Vec<Event> = entries.iter().map(|entry| entry.event.clone()).collect();
    let event_count = u64::try_from(events.len()).map_err(|err| {
        CommitError::InvalidEvent(format!("event count does not fit in u64: {err}"))
    })?;
    if event_count != commit.body.event_count {
        return Err(CommitError::EventRootMismatch);
    }
    let computed_root = event_root(&events)?;
    if computed_root != commit.body.event_merkle_root {
        return Err(CommitError::EventRootMismatch);
    }

    ports
        .commit_store
        .put_commit(tenant, commit)
        .await
        .map_err(|err| CommitError::Store(err.to_string()))?;
    ports
        .event_store
        .append_events(tenant, &events)
        .await
        .map_err(|err| CommitError::Store(err.to_string()))?;
    if let Some(publisher) = &ports.event_publisher {
        publisher
            .publish_events(tenant, &events)
            .await
            .map_err(|err| CommitError::Store(err.to_string()))?;
        publisher
            .publish_commit(tenant, commit)
            .await
            .map_err(|err| CommitError::Store(err.to_string()))?;
    }
    // Projection derivation for the composition root's index adapter.
    let _projections = projections_for(&commit.body.commit_id, entries);
    Ok(())
}

/// Derives the current-state projections of a commit's events (protocol §9).
///
/// Each projection is a deterministic function of its event's after-state:
/// version, state hash, state payload, last event id, and last commit id all
/// come from the event/commit; the state type comes from the caller.
pub fn projections_for<'event>(
    commit_id: &CommitId,
    entries: &[CommittedEvent<'event>],
) -> Vec<StateProjection> {
    entries
        .iter()
        .map(|entry| {
            let event = entry.event;
            StateProjection {
                tenant_id: event.tenant_id.clone(),
                resource_id: event.resource_id.clone(),
                state_type: entry.state_type,
                version: event.after.version,
                last_event_id: event.event_id.clone(),
                last_commit_id: commit_id.clone(),
                state_hash: event.after.state_hash.clone(),
                state: event.after.state.clone(),
            }
        })
        .collect()
}

/// Resolves the tenant scope of a commit for persistence.
///
/// # Errors
///
/// Returns [`CommitError::Store`] when the commit is not tenant-scoped or its
/// tenant id is missing.
fn commit_tenant(body: &Commit) -> Result<&TenantId, CommitError> {
    if body.scope.kind != ScopeKind::Tenant {
        return Err(CommitError::Store(String::from(
            "commit persistence requires a tenant-scoped commit; global checkpoint commits contain tenant roots, not direct events",
        )));
    }
    body.scope.tenant_id.as_ref().ok_or_else(|| {
        CommitError::Store(String::from(
            "tenant-scoped commit is missing its tenant id",
        ))
    })
}
