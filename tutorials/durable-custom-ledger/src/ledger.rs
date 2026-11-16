//! Bounded single-writer file adapter. One synced replacement owns all effects.
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use statechronicle::accumulator::{
    key::StateKey,
    sparse_merkle::{StateAccumulator, StateUpdate},
};
use statechronicle::commit::{
    persist::{
        CommittedEvent, SignedCommitVerifier, projections_for, validate_committed_events,
        validate_tenant_commit,
    },
    roots::event_root,
};
use statechronicle::core::{canonicalize::canonicalize_and_digest, digest::hash_bytes};
use statechronicle::index::rebuild::verified::{HistoryPin, HistoryWitness, verify_history};
use statechronicle::ports::{
    commit_store::{CanonicalHead, CommitReadFuture, CommitReader, CommitStoreError},
    ledger_store::{
        IdempotencyClaim, LedgerLookup, LedgerMutation, LedgerRecovery, LedgerStore,
        LedgerStoreError, LedgerTransaction, OutboxRecord,
    },
    state_index::{StateIndex, StateIndexError},
    tenant_store::{TenantStore, TenantStoreError},
};
use statechronicle::{
    Commit, CommitId, ContentDigest, Event, Intent, IntentId, Signed, StateProjection, StateType,
    SubjectId, TenantId,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_RECORDS: usize = 64;
fn invariant(error: impl ToString) -> LedgerStoreError {
    LedgerStoreError::Invariant(error.to_string())
}
fn unavailable(error: impl ToString) -> LedgerStoreError {
    LedgerStoreError::Unavailable(error.to_string())
}

// Storage DTOs are a versioned application format, not protocol wire schemas.
#[derive(Clone, Serialize, Deserialize)]
struct Notice {
    key: String,
    tenant: TenantId,
    commit: CommitId,
    digest: ContentDigest,
    payload: Vec<u8>,
}
impl From<&OutboxRecord> for Notice {
    fn from(row: &OutboxRecord) -> Self {
        Self {
            key: row.delivery_key.clone(),
            tenant: row.tenant.clone(),
            commit: row.commit_id.clone(),
            digest: row.payload_digest.clone(),
            payload: row.payload.clone(),
        }
    }
}
impl Notice {
    fn record(&self) -> OutboxRecord {
        OutboxRecord {
            delivery_key: self.key.clone(),
            tenant: self.tenant.clone(),
            commit_id: self.commit.clone(),
            payload_digest: self.digest.clone(),
            payload: self.payload.clone(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Record {
    intent: Intent,
    digest: ContentDigest,
    commit: Signed<Commit>,
    events: Vec<Event>,
    projections: Vec<StateProjection>,
    outbox: Vec<Notice>,
}
impl Record {
    fn mutation(&self) -> LedgerMutation {
        LedgerMutation {
            intent: self.intent.clone(),
            payload_digest: self.digest.clone(),
            commit: self.commit.clone(),
            events: self.events.clone(),
            projections: self.projections.clone(),
            outbox: self.outbox.iter().map(Notice::record).collect(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Journal {
    format: u32,
    tenant: TenantId,
    records: Vec<Record>,
}
impl Journal {
    fn projections(&self) -> Vec<StateProjection> {
        let mut current = BTreeMap::new();
        for record in &self.records {
            for row in &record.projections {
                current.insert(row.resource_id.clone(), row.clone());
            }
        }
        current.into_values().collect()
    }
    fn head(&self) -> Option<&Signed<Commit>> {
        self.records.last().map(|record| &record.commit)
    }
    fn accumulator(&self) -> Result<StateAccumulator, LedgerStoreError> {
        let mut accumulator = StateAccumulator::empty();
        accumulator
            .insert_batch(
                &self
                    .projections()
                    .iter()
                    .map(|row| {
                        StateUpdate::new(
                            StateKey::for_resource(&row.tenant_id.0, &row.resource_id.0),
                            *row.state_hash.as_bytes(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .map_err(invariant)?;
        Ok(accumulator)
    }
}
struct State {
    journal: Journal,
    attempts: u64,
    reservations: BTreeMap<IntentId, (ContentDigest, String)>,
    uncertain: bool,
}
struct Owner {
    path: PathBuf,
    tenant: TenantId,
    state: Mutex<State>,
    _lock: File,
}
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.path.join("writer.lock"));
    }
}
#[derive(Clone)]
pub struct FileLedger {
    owner: Arc<Owner>,
    lose_ack: bool,
    fail_outbox: bool,
}
impl FileLedger {
    /// Allowed heads are independently retained: previous known tip and pending signed plan.
    /// `None` permits an empty journal, never a nonempty provider-selected history.
    pub fn open(
        path: &Path,
        tenant: TenantId,
        allowed_heads: &[Option<ContentDigest>],
        verifier: &dyn SignedCommitVerifier,
    ) -> Result<Self, LedgerStoreError> {
        fs::create_dir_all(path).map_err(unavailable)?;
        let lock = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join("writer.lock"))
            .map_err(unavailable)?;
        // Own cleanup even if loading/validation fails. No second writer can open.
        let mut owner = Owner {
            path: path.to_owned(),
            tenant: tenant.clone(),
            state: Mutex::new(State {
                journal: Journal {
                    format: 1,
                    tenant: tenant.clone(),
                    records: Vec::new(),
                },
                attempts: 0,
                reservations: BTreeMap::new(),
                uncertain: false,
            }),
            _lock: lock,
        };
        let file = path.join("journal.json");
        let journal = match File::open(&file) {
            Ok(mut input) => {
                if input.metadata().map_err(unavailable)?.len() > MAX_FILE_BYTES {
                    return Err(invariant("journal size bound"));
                }
                let mut bytes = Vec::new();
                Read::by_ref(&mut input)
                    .take(MAX_FILE_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(unavailable)?;
                if u64::try_from(bytes.len()).map_err(invariant)? > MAX_FILE_BYTES {
                    return Err(invariant("journal size bound"));
                }
                serde_json::from_slice::<Journal>(&bytes).map_err(invariant)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Journal {
                format: 1,
                tenant: tenant.clone(),
                records: Vec::new(),
            },
            Err(error) => return Err(unavailable(error)),
        };
        if journal.format != 1 || journal.tenant != tenant || journal.records.len() > MAX_RECORDS {
            return Err(invariant("unsupported journal format/scope/length"));
        }
        if let Some(tip) = journal.head() {
            let observed = canonicalize_and_digest(&tip.body).map_err(invariant)?;
            let pin = allowed_heads
                .iter()
                .flatten()
                .find(|pin| **pin == observed)
                .ok_or_else(|| invariant("journal tip is not independently permitted"))?;
            let witnesses: Vec<_> = journal
                .records
                .iter()
                .map(|row| HistoryWitness {
                    commit: &row.commit,
                    events: &row.events,
                })
                .collect();
            verify_history(
                &witnesses,
                &HistoryPin {
                    tenant: tenant.clone(),
                    tip_digest: pin.clone(),
                },
                verifier,
            )
            .map_err(invariant)?;
            let mut prefix = Journal {
                format: 1,
                tenant: tenant.clone(),
                records: Vec::new(),
            };
            for row in &journal.records {
                validate_next(&prefix, row)?;
                prefix.records.push(row.clone());
            }
        } else if !allowed_heads.iter().any(Option::is_none) {
            return Err(invariant("unexpected empty history"));
        }
        owner.state.get_mut().map_err(unavailable)?.journal = journal;
        Ok(Self {
            owner: Arc::new(owner),
            lose_ack: false,
            fail_outbox: false,
        })
    }
    fn state(&self) -> Result<MutexGuard<'_, State>, LedgerStoreError> {
        let state = self.owner.state.lock().map_err(unavailable)?;
        if state.uncertain {
            return Err(unavailable(
                "prior file write uncertain; close all handles and reopen using retained pins",
            ));
        }
        Ok(state)
    }
    pub fn lose_next_ack(&self) -> Self {
        Self {
            lose_ack: true,
            ..self.clone()
        }
    }
    #[cfg(test)]
    pub fn fail_before_commit(&self) -> Self {
        Self {
            fail_outbox: true,
            ..self.clone()
        }
    }
    pub fn head_and_updates(
        &self,
    ) -> Result<(Option<Signed<Commit>>, Vec<StateUpdate>), LedgerStoreError> {
        let state = self.state()?;
        Ok((
            state.journal.head().cloned(),
            state
                .journal
                .projections()
                .iter()
                .map(|row| {
                    StateUpdate::new(
                        StateKey::for_resource(&row.tenant_id.0, &row.resource_id.0),
                        *row.state_hash.as_bytes(),
                    )
                })
                .collect(),
        ))
    }
    pub fn records(&self) -> Result<Vec<LedgerMutation>, LedgerStoreError> {
        Ok(self
            .state()?
            .journal
            .records
            .iter()
            .map(Record::mutation)
            .collect())
    }
    pub fn projection(
        &self,
        resource: &statechronicle::ResourceId,
    ) -> Result<Option<StateProjection>, LedgerStoreError> {
        Ok(self
            .state()?
            .journal
            .projections()
            .into_iter()
            .find(|row| &row.resource_id == resource))
    }
    pub fn state_proof(
        &self,
        resource: &statechronicle::ResourceId,
        at: &CommitId,
    ) -> Result<statechronicle::domain::proof::ResourceStateProof, LedgerStoreError> {
        let state = self.state()?;
        let mut prefix = Journal {
            format: 1,
            tenant: self.owner.tenant.clone(),
            records: Vec::new(),
        };
        for record in &state.journal.records {
            prefix.records.push(record.clone());
            if &record.commit.body.commit_id == at {
                let row = prefix
                    .projections()
                    .into_iter()
                    .find(|row| &row.resource_id == resource)
                    .ok_or_else(|| invariant("no projection at selected commit"))?;
                let key = StateKey::for_resource(&self.owner.tenant.0, &resource.0);
                let proof = prefix
                    .accumulator()?
                    .prove_inclusion(&key)
                    .ok_or_else(|| invariant("missing committed leaf"))?;
                let operation = state
                    .journal
                    .records
                    .iter()
                    .flat_map(|row| &row.events)
                    .find(|event| event.event_id == row.last_event_id)
                    .ok_or_else(|| invariant("missing original event"))?
                    .operation
                    .clone();
                return statechronicle::proof::bundle::build_state_proof(
                    &row,
                    &record.commit,
                    &proof,
                    &operation,
                    None,
                    key,
                )
                .map_err(invariant);
            }
        }
        Err(invariant("unknown commit pin"))
    }
}

fn validate_next(prior: &Journal, record: &Record) -> Result<(), LedgerStoreError> {
    if prior.records.len() >= MAX_RECORDS || record.events.len() != 1 || record.outbox.len() != 1 {
        return Err(invariant(
            "tutorial supports one event/notice per mutation and at most 64 records",
        ));
    }
    let tenant = validate_tenant_commit(&record.commit.body).map_err(invariant)?;
    if tenant != &prior.tenant
        || record.intent.tenant_id != prior.tenant
        || record.intent.state_type != Some(StateType::Custom)
        || record.digest != canonicalize_and_digest(&record.intent).map_err(invariant)?
    {
        return Err(invariant("intent/scope/digest mismatch"));
    }
    if prior.records.iter().any(|row| {
        row.intent.intent_id == record.intent.intent_id
            || row.commit.body.commit_id == record.commit.body.commit_id
    }) {
        return Err(invariant("duplicate intent or commit identity"));
    }
    let head = prior.head();
    let sequence = head.map_or(Ok(1), |row| {
        row.body
            .sequence
            .checked_add(1)
            .ok_or_else(|| invariant("sequence overflow"))
    })?;
    let root = head.map_or_else(
        || ContentDigest::new(*StateAccumulator::empty().root().as_bytes()),
        |row| row.body.next_state_root.clone(),
    );
    if record.commit.body.sequence != sequence
        || record.commit.body.parent_commit_id.as_ref() != head.map(|row| &row.body.commit_id)
        || record.commit.body.previous_state_root != root
    {
        return Err(LedgerStoreError::Conflict(String::from(
            "canonical head changed",
        )));
    }
    let entries: Vec<_> = record
        .events
        .iter()
        .map(|event| CommittedEvent {
            event,
            state_type: event.after.state.state_type(),
        })
        .collect();
    validate_committed_events(&entries).map_err(invariant)?;
    if record.commit.body.event_count != 1
        || record.commit.body.event_merkle_root != event_root(&record.events).map_err(invariant)?
        || record.projections != projections_for(&record.commit.body.commit_id, &entries)
    {
        return Err(invariant("incomplete or inconsistent effects"));
    }
    let mut event_ids: BTreeSet<_> = prior
        .records
        .iter()
        .flat_map(|row| &row.events)
        .map(|event| &event.event_id)
        .collect();
    let existing = prior.projections();
    for event in &record.events {
        if !event_ids.insert(&event.event_id)
            || event.tenant_id != prior.tenant
            || event.intent_id != record.intent.intent_id
            || event.actor != record.intent.actor
            || event.resource_id != record.intent.resource_id
            || event.operation != record.intent.operation
            || event.after.state.state_type() != StateType::Custom
        {
            return Err(invariant("event identity/scope/type mismatch"));
        }
        let current = existing
            .iter()
            .find(|row| row.resource_id == event.resource_id);
        if event.before.version != current.map_or(0, |row| row.version)
            || record.intent.expected_version != event.before.version
            || current.is_some_and(|row| row.state_hash != event.before.state_hash)
        {
            return Err(LedgerStoreError::Conflict(String::from(
                "projection changed",
            )));
        }
    }
    let notice = record
        .outbox
        .first()
        .ok_or_else(|| invariant("missing notice"))?;
    if notice.tenant != prior.tenant
        || notice.commit != record.commit.body.commit_id
        || notice.digest != hash_bytes(&notice.payload)
        || prior
            .records
            .iter()
            .flat_map(|row| &row.outbox)
            .any(|row| row.key == notice.key)
    {
        return Err(invariant("outbox identity/digest mismatch"));
    }
    let mut next = prior.clone();
    next.records.push(record.clone());
    if *next.accumulator()?.root().as_bytes() != *record.commit.body.next_state_root.as_bytes() {
        return Err(invariant("next state root mismatch"));
    }
    Ok(())
}

struct Transaction {
    ledger: FileLedger,
    tenant: TenantId,
    claim: Option<(Intent, ContentDigest, String)>,
    events: Vec<Event>,
    commit: Option<Signed<Commit>>,
    projections: Vec<StateProjection>,
    outbox: Vec<Notice>,
    finalized: bool,
}
impl Drop for Transaction {
    fn drop(&mut self) {
        if let Some((intent, _, attempt)) = &self.claim {
            if let Ok(mut state) = self.ledger.owner.state.lock() {
                if state
                    .reservations
                    .get(&intent.intent_id)
                    .is_some_and(|(_, owner)| owner == attempt)
                {
                    state.reservations.remove(&intent.intent_id);
                }
            }
        }
    }
}
#[async_trait]
impl LedgerStore for FileLedger {
    async fn begin(
        &self,
        tenant: &TenantId,
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        if tenant != &self.owner.tenant {
            return Err(invariant("unsupported tenant"));
        }
        drop(self.state()?);
        Ok(Box::new(Transaction {
            ledger: self.clone(),
            tenant: tenant.clone(),
            claim: None,
            events: Vec::new(),
            commit: None,
            projections: Vec::new(),
            outbox: Vec::new(),
            finalized: false,
        }))
    }
    async fn begin_multi(
        &self,
        _: &[TenantId],
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        Err(invariant("cross-tenant transactions unsupported"))
    }
}
#[async_trait]
impl LedgerTransaction for Transaction {
    async fn claim_idempotency(
        &mut self,
        tenant: &TenantId,
        intent: &Intent,
        digest: &ContentDigest,
    ) -> Result<IdempotencyClaim, LedgerStoreError> {
        if self.claim.is_some()
            || tenant != &self.tenant
            || &intent.tenant_id != tenant
            || *digest != canonicalize_and_digest(intent).map_err(invariant)?
        {
            return Err(invariant("unsupported or invalid claim"));
        }
        let mut state = self.ledger.state()?;
        if let Some(row) = state
            .journal
            .records
            .iter()
            .find(|row| row.intent.intent_id == intent.intent_id)
        {
            return Ok(if &row.digest == digest {
                IdempotencyClaim::Committed {
                    commit_id: row.commit.body.commit_id.clone(),
                }
            } else {
                IdempotencyClaim::ConflictDifferentPayload
            });
        }
        if let Some((existing, attempt)) = state.reservations.get(&intent.intent_id) {
            return Ok(if existing == digest {
                IdempotencyClaim::InProgress {
                    attempt_id: attempt.clone(),
                    lease_expires_at_unix: i64::MAX,
                }
            } else {
                IdempotencyClaim::ConflictDifferentPayload
            });
        }
        state.attempts = state
            .attempts
            .checked_add(1)
            .ok_or_else(|| invariant("attempt counter exhausted"))?;
        let attempt = format!("process-{}-attempt-{}", std::process::id(), state.attempts);
        state
            .reservations
            .insert(intent.intent_id.clone(), (digest.clone(), attempt.clone()));
        self.claim = Some((intent.clone(), digest.clone(), attempt.clone()));
        Ok(IdempotencyClaim::NewReservation {
            attempt_id: attempt,
        })
    }
    async fn append_events(&mut self, events: &[Event]) -> Result<(), LedgerStoreError> {
        if !self.events.is_empty() {
            return Err(invariant("events already staged"));
        }
        self.events.extend_from_slice(events);
        Ok(())
    }
    async fn append_commit(&mut self, commit: &Signed<Commit>) -> Result<(), LedgerStoreError> {
        if self.commit.is_some() {
            return Err(invariant("commit already staged"));
        }
        self.commit = Some(commit.clone());
        Ok(())
    }
    async fn upsert_projection(
        &mut self,
        projection: &StateProjection,
    ) -> Result<(), LedgerStoreError> {
        self.projections.push(projection.clone());
        Ok(())
    }
    async fn enqueue_outbox(&mut self, record: &OutboxRecord) -> Result<(), LedgerStoreError> {
        if self.ledger.fail_outbox {
            return Err(unavailable("injected before final commit"));
        }
        self.outbox.push(Notice::from(record));
        Ok(())
    }
    async fn finalize_idempotency(
        &mut self,
        tenant: &TenantId,
        intent_id: &IntentId,
        attempt_id: &str,
        commit_id: &CommitId,
    ) -> Result<(), LedgerStoreError> {
        let (intent, digest, attempt) = self
            .claim
            .as_ref()
            .ok_or_else(|| invariant("missing claim"))?;
        if tenant != &self.tenant
            || intent_id != &intent.intent_id
            || attempt_id != attempt
            || self.commit.as_ref().map(|row| &row.body.commit_id) != Some(commit_id)
            || self.ledger.state()?.reservations.get(intent_id)
                != Some(&(digest.clone(), attempt.clone()))
        {
            return Err(LedgerStoreError::Idempotency(String::from(
                "claim fence mismatch",
            )));
        }
        self.finalized = true;
        Ok(())
    }
    async fn commit(self: Box<Self>) -> Result<(), LedgerStoreError> {
        if !self.finalized {
            return Err(invariant("claim not finalized"));
        }
        let (intent, digest, attempt) = self
            .claim
            .as_ref()
            .ok_or_else(|| invariant("missing claim"))?;
        let record = Record {
            intent: intent.clone(),
            digest: digest.clone(),
            commit: self
                .commit
                .clone()
                .ok_or_else(|| invariant("missing commit"))?,
            events: self.events.clone(),
            projections: self.projections.clone(),
            outbox: self.outbox.clone(),
        };
        let mut state = self.ledger.state()?;
        if state.reservations.get(&intent.intent_id) != Some(&(digest.clone(), attempt.clone())) {
            return Err(invariant("stale claim owner"));
        }
        validate_next(&state.journal, &record)?;
        let mut next = state.journal.clone();
        next.records.push(record);
        let bytes = serde_json::to_vec(&next).map_err(invariant)?;
        if u64::try_from(bytes.len()).map_err(invariant)? > MAX_FILE_BYTES {
            return Err(invariant("journal size bound"));
        }
        // Any I/O error from here is sticky uncertainty, even if it precedes rename.
        state.uncertain = true;
        let path = &self.ledger.owner.path;
        let temporary = path.join("journal.next");
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(unavailable)?;
        output.write_all(&bytes).map_err(unavailable)?;
        output.sync_all().map_err(unavailable)?;
        drop(output);
        fs::rename(&temporary, path.join("journal.json")).map_err(unavailable)?;
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(unavailable)?;
        state.journal = next;
        state.uncertain = false;
        drop(state);
        if self.ledger.lose_ack {
            Err(unavailable(
                "injected acknowledgement loss after synced commit",
            ))
        } else {
            Ok(())
        }
    }
    async fn rollback(self: Box<Self>) -> Result<(), LedgerStoreError> {
        Ok(())
    }
}
#[async_trait]
impl LedgerRecovery for FileLedger {
    async fn lookup_mutation(
        &self,
        tenant: &TenantId,
        intent: &IntentId,
    ) -> Result<LedgerLookup, LedgerStoreError> {
        if tenant != &self.owner.tenant {
            return Err(invariant("unsupported tenant"));
        }
        let state = self.state()?;
        if let Some(row) = state
            .journal
            .records
            .iter()
            .find(|row| &row.intent.intent_id == intent)
        {
            return Ok(LedgerLookup::Committed(Box::new(row.mutation())));
        }
        if let Some((digest, attempt)) = state.reservations.get(intent) {
            return Ok(LedgerLookup::InProgress {
                payload_digest: digest.clone(),
                attempt_id: attempt.clone(),
                lease_expires_at_unix: i64::MAX,
            });
        }
        Ok(LedgerLookup::NotFound)
    }
}
impl CommitReader for FileLedger {
    fn read_commit<'read>(
        &'read self,
        tenant: &'read TenantId,
        commit: &'read CommitId,
    ) -> CommitReadFuture<'read, Option<Signed<Commit>>> {
        Box::pin(async move {
            if tenant != &self.owner.tenant {
                return Err(CommitStoreError::Unavailable(String::from(
                    "unsupported tenant",
                )));
            }
            Ok(self
                .state()
                .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?
                .journal
                .records
                .iter()
                .find(|row| &row.commit.body.commit_id == commit)
                .map(|row| row.commit.clone()))
        })
    }
    fn read_canonical_head<'read>(
        &'read self,
        tenant: &'read TenantId,
    ) -> CommitReadFuture<'read, Option<CanonicalHead>> {
        Box::pin(async move {
            if tenant != &self.owner.tenant {
                return Err(CommitStoreError::Unavailable(String::from(
                    "unsupported tenant",
                )));
            }
            Ok(self
                .state()
                .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?
                .journal
                .head()
                .map(|row| CanonicalHead {
                    commit_id: row.body.commit_id.clone(),
                    sequence: row.body.sequence,
                    state_root: row.body.next_state_root.clone(),
                }))
        })
    }
}
#[async_trait]
impl StateIndex for FileLedger {
    async fn get_state(
        &self,
        tenant: &TenantId,
        resource: &statechronicle::ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        if tenant != &self.owner.tenant {
            return Err(StateIndexError::Unavailable(String::from(
                "unsupported tenant",
            )));
        }
        self.projection(resource)
            .map_err(|error| StateIndexError::Unavailable(error.to_string()))
    }
    async fn get_subject_state(
        &self,
        _: &TenantId,
        _: &SubjectId,
        _: &statechronicle::ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        Err(StateIndexError::Inconsistent(String::from(
            "subject-held profiles unsupported",
        )))
    }
}
#[async_trait]
impl TenantStore for FileLedger {
    async fn tenant_exists(&self, tenant: &TenantId) -> Result<bool, TenantStoreError> {
        drop(
            self.state()
                .map_err(|error| TenantStoreError::Unavailable(error.to_string()))?,
        );
        Ok(tenant == &self.owner.tenant)
    }
    async fn get_tenant_root(
        &self,
        tenant: &TenantId,
    ) -> Result<Option<ContentDigest>, TenantStoreError> {
        if tenant != &self.owner.tenant {
            return Err(TenantStoreError::Unavailable(String::from(
                "unsupported tenant",
            )));
        }
        Ok(self
            .state()
            .map_err(|error| TenantStoreError::Unavailable(error.to_string()))?
            .journal
            .head()
            .map(|row| row.body.next_state_root.clone()))
    }
}
