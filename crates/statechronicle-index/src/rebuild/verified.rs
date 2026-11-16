//! Signed historical rebuild sources and source-bound checkpoint namespaces.
//!
//! This verifies one complete tenant history from genesis against an independent
//! historical tip pin. It does not establish live canonicality, reexecute domain
//! authorization, or authenticate a physical database. Use a dedicated projection
//! sink for the selected history; checkpoint binding does not erase stale rows.
use super::{CheckpointStore, ProjectionSink, RebuildError, validate_rebuild_stream};
use statechronicle_commit::{
    fork::check_chain_continuity,
    persist::{
        CommittedEvent, SignedCommitVerifier, validate_committed_events, validate_tenant_commit,
    },
    roots::{compute_state_root, event_root, state_root_updates, verify_state_root_continuity},
};
use statechronicle_core::{
    canonicalize::{canonicalize, canonicalize_and_digest},
    digest::ContentDigest,
    limits::{
        MAX_COMMIT_BYTES, MAX_EVENT_BATCH_BYTES, MAX_EVENT_BYTES, MAX_EVENTS_PER_COMMIT, check_size,
    },
};
use statechronicle_domain::{
    commit::Commit, event::Event, ids::CommitId, signed::Signed, tenant::TenantId,
};
use std::collections::BTreeSet;

/// Obtain this expectation independently of the provider supplying the history.
/// The digest covers the complete final commit body, not its textual ID alone.
#[derive(Debug, Clone)]
pub struct HistoryPin {
    /// Independently selected tenant scope.
    pub tenant: TenantId,
    /// Independently trusted final commit-body digest.
    pub tip_digest: ContentDigest,
}
/// Untrusted full event batch in its signed order, paired with its commit.
pub struct HistoryWitness<'history> {
    /// Signed enclosing tenant commit.
    pub commit: &'history Signed<Commit>,
    /// Complete events in their signed batch order.
    pub events: &'history [Event],
}
/// Owned signed historical source. Construction requires complete verification;
/// no deserialization or mutable event access can manufacture this authority.
/// This is historical integrity under caller-supplied signer policy, not freshness.
///
/// ```compile_fail
/// use statechronicle_index::rebuild::verified::VerifiedHistory;
/// let history = VerifiedHistory { events: Vec::new(), digest: todo!() };
/// ```
#[derive(Debug)]
pub struct VerifiedHistory {
    events: Vec<(Event, CommitId)>,
    digest: ContentDigest,
}
fn invalid(error: impl std::fmt::Display) -> RebuildError {
    RebuildError::Invariant(error.to_string())
}
/// Verify every signed batch, scope, parent/sequence, event count/root, local
/// event integrity/continuity, and replay root from genesis to the pinned tip.
/// The verifier must independently enforce tenant-scoped historical signer trust.
/// Domain authorization and the origin of the historical pin remain host policy.
///
/// # Errors
/// Returns `Invariant` on malformed, incomplete, untrusted or mispinned history.
/// No projection or checkpoint operations occur during verification.
pub fn verify_history(
    witnesses: &[HistoryWitness<'_>],
    pin: &HistoryPin,
    verifier: &dyn SignedCommitVerifier,
) -> Result<VerifiedHistory, RebuildError> {
    let last = witnesses
        .last()
        .ok_or_else(|| invalid("empty signed history"))?;
    if canonicalize_and_digest(&last.commit.body).map_err(invalid)? != pin.tip_digest {
        return Err(invalid("history does not match independent tip pin"));
    }
    let mut events = Vec::new();
    let mut updates = Vec::new();
    let mut commits = BTreeSet::new();
    let mut addresses = BTreeSet::new();
    let mut previous: Option<&Commit> = None;
    for witness in witnesses {
        let body = &witness.commit.body;
        if validate_tenant_commit(body).map_err(invalid)? != &pin.tenant
            || !commits.insert(body.commit_id.clone())
        {
            return Err(invalid("history commit scope or identity mismatch"));
        }
        check_size(
            "commit",
            MAX_COMMIT_BYTES,
            canonicalize(witness.commit).map_err(invalid)?.len(),
        )
        .map_err(invalid)?;
        verifier.verify(witness.commit).map_err(invalid)?;
        if let Some(previous) = previous {
            check_chain_continuity(previous, body).map_err(invalid)?;
        } else if body.sequence != 1
            || body.parent_commit_id.is_some()
            || body.previous_state_root
                != ContentDigest::new(*compute_state_root(&[]).map_err(invalid)?.as_bytes())
        {
            return Err(invalid("history must start at genesis"));
        }
        if witness.events.is_empty()
            || witness.events.len() > MAX_EVENTS_PER_COMMIT
            || u64::try_from(witness.events.len()).map_err(invalid)? != body.event_count
        {
            return Err(invalid("history event count mismatch"));
        }
        let mut bytes = 0usize;
        for event in witness.events {
            let size = canonicalize(event).map_err(invalid)?.len();
            check_size("event", MAX_EVENT_BYTES, size).map_err(invalid)?;
            bytes = bytes.saturating_add(size);
            if event.tenant_id != pin.tenant {
                return Err(invalid("history event tenant mismatch"));
            }
            if addresses.insert(super::projection_address(event)) && event.before.version != 0 {
                return Err(invalid(
                    "history omits initial resource or holder transition",
                ));
            }
        }
        check_size("event_batch", MAX_EVENT_BATCH_BYTES, bytes).map_err(invalid)?;
        if event_root(witness.events).map_err(invalid)? != body.event_merkle_root {
            return Err(invalid("history event root mismatch"));
        }
        let entries: Vec<_> = witness
            .events
            .iter()
            .map(|event| CommittedEvent {
                event,
                state_type: event.after.state.state_type(),
            })
            .collect();
        validate_committed_events(&entries).map_err(invalid)?;
        let current = state_root_updates(witness.events).map_err(invalid)?;
        verify_state_root_continuity(
            &body.previous_state_root,
            &updates,
            &current,
            &body.next_state_root,
        )
        .map_err(invalid)?;
        updates.extend(current);
        events.extend(
            witness
                .events
                .iter()
                .cloned()
                .map(|event| (event, body.commit_id.clone())),
        );
        previous = Some(body);
    }
    validate_rebuild_stream(&events)?;
    let source: Vec<_> = witnesses.iter().map(|w| (w.commit, w.events)).collect();
    let digest =
        canonicalize_and_digest(&("statechronicle.verified-history.v0", &pin.tenant, source))
            .map_err(invalid)?;
    Ok(VerifiedHistory { events, digest })
}
impl VerifiedHistory {
    /// Digest of the full checked signed source, including every ordered event.
    pub const fn source_digest(&self) -> &ContentDigest {
        &self.digest
    }
    /// Source-specific durable key. Distinct jobs/sources never share an offset.
    ///
    /// # Errors
    /// Returns `Invariant` for an empty job name or encoding failure.
    pub fn checkpoint_key(&self, job: &str) -> Result<String, RebuildError> {
        if job.is_empty() {
            return Err(invalid("empty verified rebuild job"));
        }
        let digest =
            canonicalize_and_digest(&("statechronicle.verified-rebuild.v0", job, &self.digest))
                .map_err(invalid)?;
        Ok(format!("verified-rebuild:{}", digest.as_str()))
    }
    /// Rebuild only this owned verified source using a source-bound checkpoint.
    /// The sink must support idempotent monotonic upserts across interrupted
    /// chunks. Use a dedicated sink namespace; this does not clear unrelated rows.
    ///
    /// # Errors
    /// Returns existing rebuild invariant/sink errors; failed writes never advance
    /// the checkpoint past the unsuccessful chunk.
    pub async fn rebuild_resumable(
        &self,
        job: &str,
        chunk_size: usize,
        sink: &dyn ProjectionSink,
        checkpoints: &dyn CheckpointStore,
    ) -> Result<u64, RebuildError> {
        super::rebuild_projections_resumable(
            &self.events,
            &self.checkpoint_key(job)?,
            chunk_size,
            sink,
            checkpoints,
        )
        .await
    }
}
