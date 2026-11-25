//! Signed historical rebuild sources and source-bound checkpoint namespaces.
//!
//! This verifies one complete tenant history from genesis against an independent
//! historical tip pin. It does not establish live canonicality, reexecute domain
//! authorization, or authenticate a physical database. Use a dedicated projection
//! sink for the selected history; checkpoint binding does not erase stale rows.
use super::{CheckpointStore, ProjectionSink, RebuildError, validate_rebuild_stream};
use chrono::{DateTime, Utc};
use statechronicle_accumulator::sparse_merkle::StateUpdate;
use statechronicle_commit::{
    fork::check_chain_continuity,
    persist::{
        CommittedEvent, SignedCommitVerifier, validate_committed_events, validate_tenant_commit,
    },
    roots::{compute_state_root, event_root, state_root_updates, verify_state_root_continuity},
    sign_v1::verify_commit_v1_with_policy,
};
use statechronicle_core::{
    canonicalize::{canonicalize, canonicalize_and_digest},
    digest::{ContentDigest, hash_bytes},
    limits::{
        MAX_COMMIT_BYTES, MAX_EVENT_BATCH_BYTES, MAX_EVENT_BYTES, MAX_EVENTS_PER_COMMIT, check_size,
    },
    signature_wire::MAX_V1_ENVELOPE_BYTES,
};
use statechronicle_domain::{
    commit::Commit, event::Event, ids::CommitId, signed::Signed, subject::SubjectId,
    tenant::TenantId,
};
use statechronicle_ports::signature_policy::{SignatureAuthority, SignaturePolicy, SignatureUse};
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
    let mut replay = HistoryReplay::default();
    let mut previous = None;
    for witness in witnesses {
        let body = &witness.commit.body;
        replay.check_identity(body, &pin.tenant)?;
        check_size(
            "commit",
            MAX_COMMIT_BYTES,
            canonicalize(witness.commit).map_err(invalid)?.len(),
        )
        .map_err(invalid)?;
        verifier.verify(witness.commit).map_err(invalid)?;
        replay.append(body, previous, witness.events, &pin.tenant)?;
        previous = Some(body);
    }
    validate_rebuild_stream(&replay.events)?;
    let source: Vec<_> = witnesses.iter().map(|w| (w.commit, w.events)).collect();
    let digest =
        canonicalize_and_digest(&("statechronicle.verified-history.v0", &pin.tenant, source))
            .map_err(invalid)?;
    Ok(VerifiedHistory {
        events: replay.events,
        digest,
    })
}
/// Explicit format and exact signed-record digest, obtained independently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryRecordPin {
    /// Digest of the canonical BCS legacy signed commit.
    Legacy(ContentDigest),
    /// Digest of the original v1 signed envelope.
    V1(ContentDigest),
}

/// Trusted tenant, ordered signed records, and v1 signer for a complete history.
pub struct MixedHistoryPin<'pin> {
    /// Independently selected tenant.
    pub tenant: &'pin TenantId,
    /// One exact signed-record pin per history entry, in order from genesis.
    pub records: &'pin [HistoryRecordPin],
    /// Independently configured v1 tenant commit authority.
    pub authority: &'pin SubjectId,
}

/// An untrusted historical batch with its original signed record.
pub enum MixedHistoryWitness<'history> {
    /// Legacy commit with its full ordered event batch.
    Legacy(HistoryWitness<'history>),
    /// Exact v1 commit record with its full ordered event batch.
    V1 {
        /// Complete canonical v1 commit envelope bytes.
        record: &'history [u8],
        /// All events in the order committed by the event root.
        events: &'history [Event],
    },
}

/// Verify a complete mixed-format history under independently pinned records.
/// Legacy keys are checked by the legacy verifier and v1 authority by Historical
/// signature policy. Both formats share the genesis, continuity and replay checks.
///
/// # Errors
/// Returns an invariant error on incomplete, mismatched or untrusted evidence.
pub async fn verify_mixed_history(
    witnesses: &[MixedHistoryWitness<'_>],
    pin: &MixedHistoryPin<'_>,
    legacy: &dyn SignedCommitVerifier,
    policy: &dyn SignaturePolicy,
    now: DateTime<Utc>,
) -> Result<VerifiedHistory, RebuildError> {
    if witnesses.is_empty() || witnesses.len() != pin.records.len() {
        return Err(invalid("history does not match ordered record pins"));
    }
    let mut replay = HistoryReplay::default();
    let mut previous = None;
    let mut record_digests = Vec::with_capacity(witnesses.len());
    for (witness, pinned) in witnesses.iter().zip(pin.records) {
        let (body, batch, format, digest) = match (witness, pinned) {
            (MixedHistoryWitness::Legacy(witness), HistoryRecordPin::Legacy(expected)) => {
                let record = canonicalize(witness.commit).map_err(invalid)?;
                check_size("commit", MAX_COMMIT_BYTES, record.len()).map_err(invalid)?;
                let digest = hash_bytes(&record);
                if &digest != expected {
                    return Err(invalid("legacy history record does not match exact pin"));
                }
                legacy.verify(witness.commit).map_err(invalid)?;
                (
                    std::borrow::Cow::Borrowed(&witness.commit.body),
                    witness.events,
                    0u8,
                    digest,
                )
            }
            (MixedHistoryWitness::V1 { record, events }, HistoryRecordPin::V1(expected)) => {
                check_size("v1 commit", MAX_V1_ENVELOPE_BYTES, record.len()).map_err(invalid)?;
                let digest = hash_bytes(record);
                if &digest != expected {
                    return Err(invalid("v1 history record does not match exact pin"));
                }
                let body = verify_commit_v1_with_policy(
                    record,
                    SignatureAuthority::TenantCommit {
                        tenant: pin.tenant,
                        authority: pin.authority,
                    },
                    SignatureUse::Historical,
                    now,
                    policy,
                )
                .await
                .map_err(invalid)?;
                (std::borrow::Cow::Owned(body), *events, 1u8, digest)
            }
            _ => return Err(invalid("history does not match pinned record format")),
        };
        replay.check_identity(&body, pin.tenant)?;
        replay.append(&body, previous.as_deref(), batch, pin.tenant)?;
        previous = Some(body);
        record_digests.push((format, digest));
    }
    validate_rebuild_stream(&replay.events)?;
    // Each exact signed record binds its ordered event batch by its validated root.
    let digest = canonicalize_and_digest(&(
        "statechronicle.verified-history.v1",
        pin.tenant,
        record_digests,
    ))
    .map_err(invalid)?;
    Ok(VerifiedHistory {
        events: replay.events,
        digest,
    })
}

#[derive(Default)]
struct HistoryReplay {
    events: Vec<(Event, CommitId)>,
    updates: Vec<StateUpdate>,
    commits: BTreeSet<CommitId>,
    addresses: BTreeSet<super::ProjectionAddress>,
}

impl HistoryReplay {
    fn check_identity(&mut self, body: &Commit, tenant: &TenantId) -> Result<(), RebuildError> {
        if validate_tenant_commit(body).map_err(invalid)? != tenant
            || !self.commits.insert(body.commit_id.clone())
        {
            return Err(invalid("history commit scope or identity mismatch"));
        }
        Ok(())
    }

    fn append(
        &mut self,
        body: &Commit,
        previous: Option<&Commit>,
        batch: &[Event],
        tenant: &TenantId,
    ) -> Result<(), RebuildError> {
        if let Some(previous) = previous {
            check_chain_continuity(previous, body).map_err(invalid)?;
        } else if body.sequence != 1
            || body.parent_commit_id.is_some()
            || body.previous_state_root
                != ContentDigest::new(*compute_state_root(&[]).map_err(invalid)?.as_bytes())
        {
            return Err(invalid("history must start at genesis"));
        }
        if batch.is_empty()
            || batch.len() > MAX_EVENTS_PER_COMMIT
            || u64::try_from(batch.len()).map_err(invalid)? != body.event_count
        {
            return Err(invalid("history event count mismatch"));
        }
        let mut bytes = 0usize;
        for event in batch {
            let size = canonicalize(event).map_err(invalid)?.len();
            check_size("event", MAX_EVENT_BYTES, size).map_err(invalid)?;
            bytes = bytes.saturating_add(size);
            if event.tenant_id != *tenant {
                return Err(invalid("history event tenant mismatch"));
            }
            if self.addresses.insert(super::projection_address(event)) && event.before.version != 0
            {
                return Err(invalid(
                    "history omits initial resource or holder transition",
                ));
            }
        }
        check_size("event_batch", MAX_EVENT_BATCH_BYTES, bytes).map_err(invalid)?;
        if event_root(batch).map_err(invalid)? != body.event_merkle_root {
            return Err(invalid("history event root mismatch"));
        }
        let entries: Vec<_> = batch
            .iter()
            .map(|event| CommittedEvent {
                event,
                state_type: event.after.state.state_type(),
            })
            .collect();
        validate_committed_events(&entries).map_err(invalid)?;
        let current = state_root_updates(batch).map_err(invalid)?;
        verify_state_root_continuity(
            &body.previous_state_root,
            &self.updates,
            &current,
            &body.next_state_root,
        )
        .map_err(invalid)?;
        self.updates.extend(current);
        self.events.extend(
            batch
                .iter()
                .cloned()
                .map(|event| (event, body.commit_id.clone())),
        );
        Ok(())
    }
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
