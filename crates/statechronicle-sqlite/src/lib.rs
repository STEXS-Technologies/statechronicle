//! SQLite durable ledger adapter.
//!
//! This adapter provides one local transactional database for StateChronicle
//! mutations. It is suitable for a single game-backend writer cluster when the
//! SQLite file is on durable storage; deployments needing horizontal writes
//! should use the same [`statechronicle_ports::ledger_store`] contract backed
//! by a server database.

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};
use statechronicle_commit::roots::event_root;
use statechronicle_core::digest::{ContentDigest, hash_bytes};
use statechronicle_core::limits::{
    MAX_EVENT_BATCH_BYTES, MAX_EVENTS_PER_COMMIT, MAX_OUTBOX_CLAIM, MAX_OUTBOX_PAYLOAD_BYTES,
    check_size,
};
use statechronicle_domain::commit::Commit;
use statechronicle_domain::event::Event;
use statechronicle_domain::ids::{CommitId, IntentId};
use statechronicle_domain::intent::{Intent, Operation};
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_ports::commit_store::{
    CanonicalHead as PortCanonicalHead, CommitStore, CommitStoreError,
};
use statechronicle_ports::event_store::{EventStore, EventStoreError};
use statechronicle_ports::intent_store::{IntentStore, IntentStoreError};
use statechronicle_ports::ledger_store::{
    IdempotencyClaim, LedgerStore, LedgerStoreError, LedgerTransaction, OutboxRecord,
};
use statechronicle_ports::outbox::{
    ConsumerDedupStore, ConsumerDeliveryClaim, OutboxError, OutboxPayload, OutboxStore,
};
use statechronicle_ports::state_index::{StateIndex, StateIndexError};

const MAX_OUTBOX_ERROR_CHARS: usize = 1024;

fn bounded_outbox_error(error: &str) -> String {
    error.chars().take(MAX_OUTBOX_ERROR_CHARS).collect()
}

/// A SQLite-backed durable ledger store.
#[derive(Clone)]
pub struct SqliteLedgerStore {
    connection: Arc<Mutex<Connection>>,
}

/// Current on-disk schema version understood by this adapter.
pub const SQLITE_SCHEMA_VERSION: i64 = 2;

/// Results of a startup/recovery integrity scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityReport {
    /// Tenant that was scanned.
    pub tenant: TenantId,
    /// Number of commits validated in sequence order.
    pub commit_count: u64,
    /// Number of events belonging to the tenant.
    pub event_count: u64,
    /// Canonical head commit, when the tenant has accepted commits.
    pub head_commit_id: Option<CommitId>,
}

/// Canonical tenant-head metadata returned by the adapter.
pub use statechronicle_ports::commit_store::CanonicalHead;

impl SqliteLedgerStore {
    /// Opens or creates a durable SQLite database at `path` and initializes
    /// the invariant-enforcing schema.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerStoreError::Unavailable`] when the database cannot be
    /// opened or initialized.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LedgerStoreError> {
        let connection = Connection::open(path).map_err(sqlite_error)?;
        initialize(&connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Opens a durable database and verifies every tenant before returning it.
    ///
    /// This is the fail-closed startup path for services that accept player
    /// mutations: callers must not begin writes until all persisted canonical
    /// histories, projections, and outbox rows have passed integrity checks.
    ///
    /// # Errors
    ///
    /// Returns the same database or integrity error as [`Self::open`] and
    /// [`Self::verify_all_integrity`].
    pub fn open_verified(path: impl AsRef<Path>) -> Result<Self, LedgerStoreError> {
        let store = Self::open(path)?;
        store.verify_all_integrity()?;
        Ok(store)
    }

    /// Creates an in-memory SQLite ledger. This is intended for tests and
    /// local development; it is not durable across process restart.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerStoreError::Unavailable`] when schema initialization
    /// fails.
    pub fn in_memory() -> Result<Self, LedgerStoreError> {
        let connection = Connection::open_in_memory().map_err(sqlite_error)?;
        initialize(&connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Creates a consistent, integrity-checked SQLite snapshot at
    /// `destination`.
    ///
    /// The destination must not already exist. The source is scanned across
    /// every tenant before the snapshot is created, and the resulting file is
    /// reopened and scanned again before this method returns. SQLite's
    /// transactional `VACUUM INTO` operation provides a point-in-time copy
    /// that includes WAL-backed changes without requiring callers to stop
    /// readers.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerStoreError::Invariant`] when the source fails
    /// integrity verification, or [`LedgerStoreError::Unavailable`] when the
    /// snapshot cannot be written or reopened.
    pub fn backup_to(&self, destination: impl AsRef<Path>) -> Result<(), LedgerStoreError> {
        self.verify_all_integrity()?;
        let destination = destination.as_ref();
        if destination.as_os_str().is_empty() {
            return Err(LedgerStoreError::Unavailable(String::from(
                "backup destination must not be empty",
            )));
        }
        if destination.exists() {
            return Err(LedgerStoreError::Unavailable(format!(
                "backup destination already exists: {}",
                destination.display()
            )));
        }
        let destination_text = destination.to_string_lossy().into_owned();
        {
            let connection = self.connection.lock().map_err(lock_error)?;
            connection
                .execute("VACUUM INTO ?1", params![destination_text])
                .map_err(sqlite_error)?;
        }
        let snapshot = Self::open(destination)?;
        snapshot.verify_all_integrity()?;
        Ok(())
    }

    /// Verifies persisted commit-chain continuity and event-to-commit
    /// references for one tenant.  Call this during startup and after restore;
    /// callers should block writes if it returns an error.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerStoreError::Invariant`] when serialized rows, chain
    /// continuity, canonical head, or event references are inconsistent, and
    /// [`LedgerStoreError::Unavailable`] when the database cannot be read.
    pub fn verify_integrity(&self, tenant: &TenantId) -> Result<IntegrityReport, LedgerStoreError> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare(
                "SELECT commit_id, sequence, payload FROM commits
                 WHERE tenant_id=?1 ORDER BY sequence ASC",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map(params![tenant.0], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(sqlite_error)?;
        let mut previous: Option<(CommitId, u64, Vec<u8>)> = None;
        let mut commit_count = 0u64;
        for row in rows {
            let (commit_id, sequence, payload) = row.map_err(sqlite_error)?;
            let signed: Signed<Commit> = bcs::from_bytes(&payload).map_err(|error| {
                LedgerStoreError::Invariant(format!("invalid commit payload: {error}"))
            })?;
            if signed.body.commit_id.0 != commit_id || signed.body.sequence != sequence {
                return Err(LedgerStoreError::Invariant(String::from(
                    "commit index does not match signed commit body",
                )));
            }
            if let Some((parent_id, parent_sequence, parent_root)) = &previous {
                if signed.body.parent_commit_id.as_ref() != Some(parent_id)
                    || signed.body.sequence != parent_sequence.saturating_add(1)
                    || signed.body.previous_state_root.as_bytes() != parent_root.as_slice()
                {
                    return Err(LedgerStoreError::Invariant(String::from(
                        "canonical commit chain continuity check failed",
                    )));
                }
            } else if signed.body.parent_commit_id.is_some() {
                return Err(LedgerStoreError::Invariant(String::from(
                    "genesis commit unexpectedly declares a parent",
                )));
            }
            let mut event_statement = connection
                .prepare(
                    "SELECT event_id, payload FROM events
                     WHERE tenant_id=?1 AND commit_id=?2
                     ORDER BY event_index ASC, event_id ASC",
                )
                .map_err(sqlite_error)?;
            let event_rows = event_statement
                .query_map(params![tenant.0, commit_id], |event_row| {
                    Ok((
                        event_row.get::<_, String>(0)?,
                        event_row.get::<_, Vec<u8>>(1)?,
                    ))
                })
                .map_err(sqlite_error)?;
            let mut commit_events = Vec::new();
            for event_row in event_rows {
                let (indexed_event_id, event_payload) = event_row.map_err(sqlite_error)?;
                let event: Event = bcs::from_bytes(&event_payload).map_err(|error| {
                    LedgerStoreError::Invariant(format!("invalid event payload: {error}"))
                })?;
                if event.event_id.0 != indexed_event_id {
                    return Err(LedgerStoreError::Invariant(String::from(
                        "event payload id does not match event index",
                    )));
                }
                if event.tenant_id != *tenant {
                    return Err(LedgerStoreError::Invariant(String::from(
                        "event payload tenant does not match event index",
                    )));
                }
                commit_events.push(event);
            }
            let stored_event_count = u64::try_from(commit_events.len()).map_err(|error| {
                LedgerStoreError::Invariant(format!("event count overflow: {error}"))
            })?;
            let stored_event_root = event_root(&commit_events)
                .map_err(|error| LedgerStoreError::Invariant(error.to_string()))?;
            if stored_event_count != signed.body.event_count
                || stored_event_root != signed.body.event_merkle_root
            {
                return Err(LedgerStoreError::Invariant(format!(
                    "event set does not match commit `{}`",
                    signed.body.commit_id.0
                )));
            }
            previous = Some((
                signed.body.commit_id.clone(),
                signed.body.sequence,
                signed.body.next_state_root.as_bytes().to_vec(),
            ));
            commit_count = commit_count.saturating_add(1);
        }
        let head: Option<(String, u64)> = connection
            .query_row(
                "SELECT commit_id, sequence FROM heads WHERE tenant_id=?1",
                params![tenant.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(sqlite_error)?;
        let head_commit_id = match (previous.as_ref(), head) {
            (None, None) => None,
            (Some((commit_id, sequence, _)), Some((head_id, head_sequence)))
                if commit_id.0 == head_id && *sequence == head_sequence =>
            {
                Some(commit_id.clone())
            }
            _ => {
                return Err(LedgerStoreError::Invariant(String::from(
                    "canonical head does not match commit chain",
                )));
            }
        };
        let event_count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM events WHERE tenant_id=?1",
                params![tenant.0],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        let orphan_events: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM events e LEFT JOIN commits c
                 ON c.tenant_id=e.tenant_id AND c.commit_id=e.commit_id
                 WHERE e.tenant_id=?1 AND e.commit_id IS NOT NULL AND c.commit_id IS NULL",
                params![tenant.0],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        let unbound_events: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM events WHERE tenant_id=?1 AND commit_id IS NULL",
                params![tenant.0],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        if orphan_events != 0 || unbound_events != 0 {
            return Err(LedgerStoreError::Invariant(String::from(
                "event is not bound to an existing commit",
            )));
        }
        let duplicate_event_indexes: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM (
                   SELECT commit_id, event_index FROM events
                   WHERE tenant_id=?1 AND commit_id IS NOT NULL AND event_index IS NOT NULL
                   GROUP BY commit_id, event_index HAVING COUNT(*) > 1
                 )",
                params![tenant.0],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        if duplicate_event_indexes != 0 {
            return Err(LedgerStoreError::Invariant(String::from(
                "commit contains duplicate event indexes",
            )));
        }
        let mut outbox_statement = connection
            .prepare(
                "SELECT o.commit_id, o.payload_digest, o.payload FROM outbox o
                 WHERE o.tenant_id=?1",
            )
            .map_err(sqlite_error)?;
        let outbox_rows = outbox_statement
            .query_map(params![tenant.0], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(sqlite_error)?;
        for row in outbox_rows {
            let (outbox_commit, digest, payload) = row.map_err(sqlite_error)?;
            if hash_bytes(&payload).as_bytes() != digest.as_slice() {
                return Err(LedgerStoreError::Invariant(format!(
                    "outbox payload digest mismatch for commit `{outbox_commit}`"
                )));
            }
        }
        let orphan_outbox: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM outbox o LEFT JOIN commits c
                 ON c.tenant_id=o.tenant_id AND c.commit_id=o.commit_id
                 WHERE o.tenant_id=?1 AND c.commit_id IS NULL",
                params![tenant.0],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        if orphan_outbox != 0 {
            return Err(LedgerStoreError::Invariant(String::from(
                "outbox record is not bound to an existing commit",
            )));
        }
        let invalid_quarantine_rows: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM outbox
                 WHERE quarantined_at IS NOT NULL
                   AND (delivered_at IS NOT NULL OR quarantine_error IS NULL)",
                [],
                |row| row.get(0),
            )
            .map_err(sqlite_error)?;
        if invalid_quarantine_rows != 0 {
            return Err(LedgerStoreError::Invariant(String::from(
                "outbox quarantine metadata is inconsistent",
            )));
        }
        let mut projection_statement = connection
            .prepare("SELECT resource_id, payload FROM projections WHERE tenant_id=?1")
            .map_err(sqlite_error)?;
        let projection_rows = projection_statement
            .query_map(params![tenant.0], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(sqlite_error)?;
        for row in projection_rows {
            let (indexed_resource_id, payload) = row.map_err(sqlite_error)?;
            let projection: StateProjection = bcs::from_bytes(&payload).map_err(|error| {
                LedgerStoreError::Invariant(format!("invalid projection payload: {error}"))
            })?;
            if projection.tenant_id != *tenant || projection.resource_id.0 != indexed_resource_id {
                return Err(LedgerStoreError::Invariant(String::from(
                    "projection payload does not match projection index",
                )));
            }
        }
        Ok(IntegrityReport {
            tenant: tenant.clone(),
            commit_count,
            event_count,
            head_commit_id,
        })
    }

    /// Reads the canonical head for one tenant after validating its stored
    /// identifiers and root length.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerStoreError::Invariant`] when persisted head metadata is
    /// malformed, or [`LedgerStoreError::Unavailable`] when it cannot be read.
    pub fn canonical_head(
        &self,
        tenant: &TenantId,
    ) -> Result<Option<CanonicalHead>, LedgerStoreError> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let row: Option<(String, u64, Vec<u8>)> = connection
            .query_row(
                "SELECT commit_id, sequence, state_root FROM heads WHERE tenant_id=?1",
                params![tenant.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(sqlite_error)?;
        row.map(|(commit_id, sequence, root)| {
            let commit_id = CommitId::new(commit_id).map_err(domain_error)?;
            let root: [u8; 32] = root.try_into().map_err(|_invalid_root| {
                LedgerStoreError::Invariant(String::from(
                    "canonical head state root must contain 32 bytes",
                ))
            })?;
            Ok(CanonicalHead {
                commit_id,
                sequence,
                state_root: ContentDigest::new(root),
            })
        })
        .transpose()
    }

    /// Returns the durable key used for tenant-scoped rebuild metadata.
    fn rebuild_checkpoint_key(tenant: &TenantId, key: &str) -> Result<String, LedgerStoreError> {
        if tenant.0.is_empty() || key.is_empty() {
            return Err(LedgerStoreError::Invariant(String::from(
                "tenant and rebuild checkpoint key must not be empty",
            )));
        }
        // Length-prefix the tenant so a pair such as ("ab", "c:d") cannot
        // collide with ("ab:c", "d"). The key is metadata, never client SQL.
        let mut scoped = format!("{}:{}:", tenant.0.len(), tenant.0);
        scoped.push_str(key);
        Ok(scoped)
    }

    /// Deletes a completed tenant-scoped projection-rebuild checkpoint.
    ///
    /// Call this only after the rebuilt projection store has been verified and
    /// promoted. Keeping checkpoints bounded avoids unbounded metadata growth.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerStoreError::Unavailable`] when the checkpoint row
    /// cannot be removed.
    pub fn clear_rebuild_checkpoint_for_tenant(
        &self,
        tenant: &TenantId,
        key: &str,
    ) -> Result<(), LedgerStoreError> {
        let scoped_key = Self::rebuild_checkpoint_key(tenant, key)?;
        let connection = self.connection.lock().map_err(lock_error)?;
        connection
            .execute(
                "DELETE FROM projection_rebuild_checkpoints WHERE checkpoint_key=?1",
                params![scoped_key],
            )
            .map_err(sqlite_error)?;
        Ok(())
    }

    /// Deletes a legacy unscoped checkpoint by exact key.
    ///
    /// New recovery tooling should use [`Self::clear_rebuild_checkpoint_for_tenant`]
    /// so one tenant cannot clear another tenant's rebuild metadata.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerStoreError::Invariant`] for an empty key or
    /// [`LedgerStoreError::Unavailable`] when the checkpoint row cannot be
    /// removed.
    pub fn clear_rebuild_checkpoint(&self, key: &str) -> Result<(), LedgerStoreError> {
        if key.is_empty() {
            return Err(LedgerStoreError::Invariant(String::from(
                "rebuild checkpoint key must not be empty",
            )));
        }
        let connection = self.connection.lock().map_err(lock_error)?;
        connection
            .execute(
                "DELETE FROM projection_rebuild_checkpoints WHERE checkpoint_key=?1",
                params![key],
            )
            .map_err(sqlite_error)?;
        Ok(())
    }

    /// Reports projection rebuild lag for one tenant and checkpoint key.
    ///
    /// The canonical stream is loaded only after callers have a verified
    /// database; a checkpoint beyond that stream fails closed instead of being
    /// clamped into a false healthy state.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerStoreError::Invariant`] for an empty key or corrupt
    /// checkpoint, and a database/integrity error when the stream cannot be
    /// read.
    pub fn projection_rebuild_progress(
        &self,
        tenant: &TenantId,
        key: &str,
    ) -> Result<statechronicle_index::rebuild::RebuildProgress, LedgerStoreError> {
        if key.is_empty() {
            return Err(LedgerStoreError::Invariant(String::from(
                "rebuild checkpoint key must not be empty",
            )));
        }
        let scoped_key = Self::rebuild_checkpoint_key(tenant, key)?;
        self.verify_integrity(tenant)?;
        let total_events = self.canonical_events(tenant)?.len();
        let next_event: Option<usize> = {
            let connection = self.connection.lock().map_err(lock_error)?;
            connection
                .query_row(
                    "SELECT next_event FROM projection_rebuild_checkpoints WHERE checkpoint_key=?1",
                    params![scoped_key],
                    |row| row.get::<_, u64>(0),
                )
                .optional()
                .map_err(sqlite_error)?
                .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
        };
        statechronicle_index::rebuild::rebuild_progress(total_events, next_event.unwrap_or(0))
            .map_err(|error| LedgerStoreError::Invariant(error.to_string()))
    }

    /// Lists tenant scopes present in the durable ledger.
    ///
    /// Recovery tooling can call this and run [`Self::verify_integrity`] for
    /// every returned tenant before unfreezing mutations.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerStoreError::Unavailable`] when the database cannot be
    /// queried or a stored tenant identifier is invalid.
    pub fn tenants(&self) -> Result<Vec<TenantId>, LedgerStoreError> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare(
                "SELECT tenant_id FROM (
                   SELECT tenant_id FROM idempotency
                   UNION SELECT tenant_id FROM events
                   UNION SELECT tenant_id FROM commits
                   UNION SELECT tenant_id FROM projections
                   UNION SELECT tenant_id FROM outbox
                 ) ORDER BY tenant_id",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sqlite_error)?;
        rows.map(|row| {
            let tenant = row.map_err(sqlite_error)?;
            if tenant.is_empty() {
                return Err(LedgerStoreError::Invariant(String::from(
                    "database contains an empty tenant identifier",
                )));
            }
            Ok(TenantId(tenant))
        })
        .collect()
    }

    /// Verifies every tenant currently present in the database.
    ///
    /// Reports are returned in stable tenant-ID order and the operation fails
    /// on the first integrity violation so callers can keep writes frozen.
    ///
    /// # Errors
    ///
    /// Returns the first database or integrity error encountered while listing
    /// or scanning tenants.
    pub fn verify_all_integrity(&self) -> Result<Vec<IntegrityReport>, LedgerStoreError> {
        {
            let connection = self.connection.lock().map_err(lock_error)?;
            let invalid_consumer_rows: u64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM consumer_deliveries
                     WHERE status NOT IN ('in_progress','applied')
                        OR length(delivery_key) = 0
                        OR length(attempt_id) = 0
                        OR lease_until < 0
                        OR (status='applied' AND applied_at IS NULL)",
                    [],
                    |row| row.get(0),
                )
                .map_err(sqlite_error)?;
            if invalid_consumer_rows != 0 {
                return Err(LedgerStoreError::Invariant(String::from(
                    "consumer delivery metadata is inconsistent",
                )));
            }
        }
        self.tenants()?
            .iter()
            .map(|tenant| self.verify_integrity(tenant))
            .collect()
    }

    /// Loads the verified canonical event stream for projection rebuilding.
    ///
    /// The stream is ordered by commit sequence and the event order recorded
    /// inside each commit. Call [`Self::verify_integrity`] first; this method
    /// repeats payload/scope decoding but deliberately does not silently skip
    /// malformed rows.
    ///
    /// # Errors
    ///
    /// Returns an invariant or availability error when the database cannot be
    /// read, identifiers are invalid, or an event payload cannot be decoded.
    pub fn canonical_events(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<(Event, CommitId)>, LedgerStoreError> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut statement = connection
            .prepare(
                "SELECT e.commit_id, e.event_id, e.payload FROM events e
                 JOIN commits c ON c.tenant_id=e.tenant_id AND c.commit_id=e.commit_id
                 WHERE e.tenant_id=?1
                 ORDER BY c.sequence ASC, e.event_index ASC, e.event_id ASC",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map(params![tenant.0], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            })
            .map_err(sqlite_error)?;
        rows.map(|row| {
            let (commit_id, indexed_event_id, payload) = row.map_err(sqlite_error)?;
            let commit_id = CommitId::new(commit_id).map_err(domain_error)?;
            let event: Event = bcs::from_bytes(&payload)
                .map_err(|error| LedgerStoreError::Invariant(error.to_string()))?;
            if event.event_id.0 != indexed_event_id {
                return Err(LedgerStoreError::Invariant(String::from(
                    "event payload id does not match event index",
                )));
            }
            Ok((event, commit_id))
        })
        .collect()
    }

    /// Verifies a tenant and rebuilds its projections from the canonical
    /// event stream using restartable checkpoints.
    ///
    /// # Errors
    ///
    /// Returns a stringified integrity, database, checkpoint, or projection
    /// rebuild error. No rebuild is attempted when integrity verification
    /// fails.
    pub async fn rebuild_projections_from_canonical(
        &self,
        tenant: &TenantId,
        checkpoint_key: &str,
        chunk_size: usize,
    ) -> Result<u64, String> {
        self.verify_integrity(tenant)
            .map_err(|error| error.to_string())?;
        let events = self
            .canonical_events(tenant)
            .map_err(|error| error.to_string())?;
        let scoped_key = Self::rebuild_checkpoint_key(tenant, checkpoint_key)
            .map_err(|error| error.to_string())?;
        statechronicle_index::rebuild::rebuild_projections_resumable(
            &events,
            &scoped_key,
            chunk_size,
            self,
            self,
        )
        .await
        .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl LedgerStore for SqliteLedgerStore {
    async fn begin(
        &self,
        tenant: &TenantId,
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        if tenant.0.is_empty() {
            return Err(LedgerStoreError::Invariant(String::from(
                "tenant identifier must not be empty",
            )));
        }
        Ok(Box::new(SqliteLedgerTransaction {
            connection: Arc::clone(&self.connection),
            tenant: tenant.clone(),
            attempt_id: None,
            claimed_intent_id: None,
            claimed_intent_actor: None,
            claimed_intent_operation: None,
            finalized_commit_id: None,
            finalized: false,
            events: Vec::new(),
            commit: None,
            projections: Vec::new(),
            outbox: Vec::new(),
        }))
    }

    async fn begin_multi(
        &self,
        tenants: &[TenantId],
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        if tenants.is_empty() {
            return Err(LedgerStoreError::Invariant(String::from(
                "multi-tenant transaction requires at least one tenant",
            )));
        }
        if tenants.iter().any(|tenant| tenant.0.is_empty()) {
            return Err(LedgerStoreError::Invariant(String::from(
                "tenant identifiers must not be empty",
            )));
        }
        let mut sorted = tenants.to_vec();
        sorted.sort_by(|left, right| left.0.cmp(&right.0));
        sorted.dedup_by(|left, right| left == right);
        if sorted.len() != tenants.len() {
            return Err(LedgerStoreError::Invariant(String::from(
                "multi-tenant transaction contains duplicate tenants",
            )));
        }
        // The current transaction object carries one canonical tenant for
        // idempotency. Multi-tenant callers must use one shared intent scope;
        // reject ambiguous ownership rather than pretending to provide 2PC.
        if sorted.len() != 1 {
            return Err(LedgerStoreError::Unavailable(String::from(
                "cross-tenant SQLite transaction requires an application-level manifest adapter",
            )));
        }
        let Some(first) = sorted.first() else {
            return Err(LedgerStoreError::Invariant(String::from(
                "multi-tenant transaction has no tenant",
            )));
        };
        self.begin(first).await
    }
}

#[async_trait]
impl IntentStore for SqliteLedgerStore {
    async fn put_intent(&self, tenant: &TenantId, intent: &Intent) -> Result<(), IntentStoreError> {
        let digest = statechronicle_core::canonicalize::canonicalize_and_digest(intent)
            .map_err(|error| IntentStoreError::Unavailable(error.to_string()))?;
        let payload = bcs::to_bytes(intent)
            .map_err(|error| IntentStoreError::Unavailable(error.to_string()))?;
        let connection = self
            .connection
            .lock()
            .map_err(|error| IntentStoreError::Unavailable(format!("mutex poisoned: {error}")))?;
        match connection.execute(
            "INSERT INTO idempotency (tenant_id, intent_id, payload_digest, status, attempt_id, lease_expires, intent_payload)
             VALUES (?1, ?2, ?3, 'in_progress', 'legacy', 0, ?4)",
            params![tenant.0, intent.intent_id.0, digest.as_bytes(), payload],
        ) {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(_, _)) => {
                let existing: Option<Vec<u8>> = connection
                    .query_row(
                        "SELECT payload_digest FROM idempotency WHERE tenant_id=?1 AND intent_id=?2",
                        params![tenant.0, intent.intent_id.0],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|error| IntentStoreError::Unavailable(error.to_string()))?;
                if existing.as_deref() == Some(digest.as_bytes()) { Ok(()) } else { Err(IntentStoreError::Duplicate) }
            }
            Err(error) => Err(IntentStoreError::Unavailable(error.to_string())),
        }
    }

    async fn get_intent(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
    ) -> Result<Option<Intent>, IntentStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| IntentStoreError::Unavailable(format!("mutex poisoned: {error}")))?;
        let payload: Option<Vec<u8>> = connection
            .query_row(
                "SELECT intent_payload FROM idempotency WHERE tenant_id=?1 AND intent_id=?2",
                params![tenant.0, intent_id.0],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| IntentStoreError::Unavailable(error.to_string()))?;
        payload
            .map(|bytes| {
                bcs::from_bytes(&bytes)
                    .map_err(|error| IntentStoreError::Unavailable(error.to_string()))
            })
            .transpose()
    }
}

#[async_trait]
impl EventStore for SqliteLedgerStore {
    async fn append_events(
        &self,
        tenant: &TenantId,
        events: &[Event],
    ) -> Result<(), EventStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| EventStoreError::Unavailable(format!("mutex poisoned: {error}")))?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        for event in events {
            let payload = bcs::to_bytes(event)
                .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
            transaction
                .execute(
                    "INSERT INTO events (tenant_id, event_id, payload) VALUES (?1, ?2, ?3)",
                    params![tenant.0, event.event_id.0, payload],
                )
                .map_err(|error| {
                    if matches!(error, rusqlite::Error::SqliteFailure(_, _)) {
                        EventStoreError::DuplicateEventId
                    } else {
                        EventStoreError::Unavailable(error.to_string())
                    }
                })?;
        }
        transaction
            .commit()
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))
    }

    async fn events_for_resource(
        &self,
        tenant: &TenantId,
        resource_id: &statechronicle_domain::resource::ResourceId,
    ) -> Result<Vec<Event>, EventStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| EventStoreError::Unavailable(format!("mutex poisoned: {error}")))?;
        let mut statement = connection
            .prepare("SELECT payload FROM events WHERE tenant_id=?1 ORDER BY rowid")
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        let rows = statement
            .query_map(params![tenant.0], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        let mut events = Vec::new();
        for row in rows {
            let event: Event = bcs::from_bytes(
                &row.map_err(|error| EventStoreError::Unavailable(error.to_string()))?,
            )
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
            if event.resource_id == *resource_id {
                events.push(event);
            }
        }
        Ok(events)
    }

    async fn event_by_id(
        &self,
        tenant: &TenantId,
        event_id: &statechronicle_domain::ids::EventId,
    ) -> Result<Option<Event>, EventStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| EventStoreError::Unavailable(format!("mutex poisoned: {error}")))?;
        let payload: Option<Vec<u8>> = connection
            .query_row(
                "SELECT payload FROM events WHERE tenant_id=?1 AND event_id=?2",
                params![tenant.0, event_id.0],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| EventStoreError::Unavailable(error.to_string()))?;
        payload
            .map(|bytes| {
                bcs::from_bytes(&bytes)
                    .map_err(|error| EventStoreError::Unavailable(error.to_string()))
            })
            .transpose()
    }
}

#[async_trait]
impl CommitStore for SqliteLedgerStore {
    async fn put_commit(
        &self,
        tenant: &TenantId,
        commit: &Signed<Commit>,
    ) -> Result<(), CommitStoreError> {
        let bytes = bcs::to_bytes(commit)
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        let connection = self
            .connection
            .lock()
            .map_err(|error| CommitStoreError::Unavailable(format!("mutex poisoned: {error}")))?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        let head: Option<(String, u64, Vec<u8>)> = transaction
            .query_row(
                "SELECT commit_id, sequence, state_root FROM heads WHERE tenant_id=?1",
                params![tenant.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        match head {
            Some((head_id, sequence, root))
                if commit.body.parent_commit_id.as_ref().map(|id| &id.0) != Some(&head_id)
                    || commit.body.sequence != sequence.saturating_add(1)
                    || commit.body.previous_state_root.as_bytes() != root.as_slice() =>
            {
                return Err(CommitStoreError::Unavailable(String::from(
                    "commit does not extend canonical head",
                )));
            }
            None if commit.body.parent_commit_id.is_some() => {
                return Err(CommitStoreError::Unavailable(String::from(
                    "commit declares a parent but no canonical head exists",
                )));
            }
            _ => {}
        }
        transaction
            .execute(
                "INSERT INTO commits (tenant_id, commit_id, sequence, payload) VALUES (?1, ?2, ?3, ?4)",
                params![tenant.0, commit.body.commit_id.0, commit.body.sequence, bytes],
            )
            .map_err(|error| {
                if matches!(error, rusqlite::Error::SqliteFailure(_, _)) {
                    CommitStoreError::Duplicate
                } else {
                    CommitStoreError::Unavailable(error.to_string())
                }
            })?;
        transaction
            .execute(
                "INSERT INTO heads (tenant_id, commit_id, sequence, state_root) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(tenant_id) DO UPDATE SET commit_id=excluded.commit_id,
                 sequence=excluded.sequence, state_root=excluded.state_root",
                params![tenant.0, commit.body.commit_id.0, commit.body.sequence, commit.body.next_state_root.as_bytes()],
            )
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))
    }

    async fn commit_by_id(
        &self,
        tenant: &TenantId,
        commit_id: &CommitId,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| CommitStoreError::Unavailable(format!("mutex poisoned: {error}")))?;
        let payload: Option<Vec<u8>> = connection
            .query_row(
                "SELECT payload FROM commits WHERE tenant_id=?1 AND commit_id=?2",
                params![tenant.0, commit_id.0],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        payload
            .map(|bytes| {
                bcs::from_bytes(&bytes)
                    .map_err(|error| CommitStoreError::Unavailable(error.to_string()))
            })
            .transpose()
    }

    async fn commit_by_sequence(
        &self,
        tenant: &TenantId,
        sequence: u64,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| CommitStoreError::Unavailable(format!("mutex poisoned: {error}")))?;
        let payload: Option<Vec<u8>> = connection
            .query_row(
                "SELECT payload FROM commits WHERE tenant_id=?1 AND sequence=?2",
                params![tenant.0, sequence],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        payload
            .map(|bytes| {
                bcs::from_bytes(&bytes)
                    .map_err(|error| CommitStoreError::Unavailable(error.to_string()))
            })
            .transpose()
    }

    async fn canonical_head(
        &self,
        tenant: &TenantId,
    ) -> Result<Option<PortCanonicalHead>, CommitStoreError> {
        SqliteLedgerStore::canonical_head(self, tenant)
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))
    }
}

#[async_trait]
impl StateIndex for SqliteLedgerStore {
    async fn get_state(
        &self,
        tenant: &TenantId,
        resource_id: &statechronicle_domain::resource::ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| StateIndexError::Unavailable(format!("mutex poisoned: {error}")))?;
        let payload: Option<Vec<u8>> = connection
            .query_row(
                "SELECT payload FROM projections WHERE tenant_id=?1 AND resource_id=?2",
                params![tenant.0, resource_id.0],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| StateIndexError::Unavailable(error.to_string()))?;
        payload
            .map(|bytes| {
                bcs::from_bytes(&bytes)
                    .map_err(|error| StateIndexError::Inconsistent(error.to_string()))
            })
            .transpose()
    }

    async fn get_subject_state(
        &self,
        tenant: &TenantId,
        subject: &statechronicle_domain::subject::SubjectId,
        resource_id: &statechronicle_domain::resource::ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        let projection = self.get_state(tenant, resource_id).await?;
        Ok(projection.filter(|projection| {
            projection
                .state
                .subject()
                .is_some_and(|current| current == subject)
        }))
    }
}

#[async_trait]
impl statechronicle_index::rebuild::ProjectionSink for SqliteLedgerStore {
    async fn upsert_projection(&self, projection: &StateProjection) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| format!("projection database mutex poisoned: {error}"))?;
        let payload = bcs::to_bytes(projection).map_err(|error| error.to_string())?;
        let existing: Option<(u64, Vec<u8>)> = connection
            .query_row(
                "SELECT version, payload FROM projections
                 WHERE tenant_id=?1 AND resource_id=?2",
                params![projection.tenant_id.0, projection.resource_id.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if let Some((version, existing_payload)) = existing {
            if version > projection.version {
                return Ok(());
            }
            if version == projection.version && existing_payload != payload {
                return Err(format!(
                    "conflicting projection version {} for resource `{}`",
                    projection.version, projection.resource_id.0
                ));
            }
        }
        connection
            .execute(
                "INSERT INTO projections (tenant_id, resource_id, version, payload)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(tenant_id, resource_id) DO UPDATE SET
                   version=excluded.version, payload=excluded.payload
                 WHERE excluded.version > projections.version",
                params![
                    projection.tenant_id.0,
                    projection.resource_id.0,
                    projection.version,
                    payload
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

#[async_trait]
impl statechronicle_index::rebuild::CheckpointStore for SqliteLedgerStore {
    async fn load(&self, key: &str) -> Result<Option<usize>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| format!("checkpoint database mutex poisoned: {error}"))?;
        connection
            .query_row(
                "SELECT next_event FROM projection_rebuild_checkpoints WHERE checkpoint_key=?1",
                params![key],
                |row| row.get::<_, u64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .map(|value| usize::try_from(value).map_err(|error| error.to_string()))
            .transpose()
    }

    async fn save(&self, key: &str, next_event: usize) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| format!("checkpoint database mutex poisoned: {error}"))?;
        let next_event = u64::try_from(next_event).map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO projection_rebuild_checkpoints (checkpoint_key, next_event)
                 VALUES (?1, ?2)
                 ON CONFLICT(checkpoint_key) DO UPDATE SET next_event=excluded.next_event
                 WHERE excluded.next_event >= projection_rebuild_checkpoints.next_event",
                params![key, next_event],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

#[async_trait]
impl OutboxStore for SqliteLedgerStore {
    async fn claim(
        &self,
        worker_id: &str,
        limit: usize,
        lease_until_unix: i64,
    ) -> Result<Vec<(String, OutboxPayload)>, OutboxError> {
        if worker_id.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        if lease_until_unix <= chrono::Utc::now().timestamp() {
            return Err(OutboxError::Unavailable(String::from(
                "outbox lease expiration must be in the future",
            )));
        }
        if limit > MAX_OUTBOX_CLAIM {
            return Err(OutboxError::Unavailable(format!(
                "outbox claim limit {limit} exceeds maximum {MAX_OUTBOX_CLAIM}"
            )));
        }
        let connection = self
            .connection
            .lock()
            .map_err(|poisoned| OutboxError::Unavailable(format!("mutex poisoned: {poisoned}")))?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        let now = chrono::Utc::now().timestamp();
        let mut statement = transaction
            .prepare(
                "SELECT delivery_key, tenant_id, commit_id, payload_digest, payload
             FROM outbox WHERE delivered_at IS NULL AND quarantined_at IS NULL
               AND (lease_until IS NULL OR lease_until <= ?1)
             ORDER BY delivery_key LIMIT ?2",
            )
            .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        let rows = statement
            .query_map(
                params![now, i64::try_from(limit).unwrap_or(i64::MAX)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                    ))
                },
            )
            .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        let mut claimed = Vec::new();
        for row in rows {
            let (key, tenant, commit_id, digest, payload) =
                row.map_err(|error| OutboxError::Unavailable(error.to_string()))?;
            // Keep the claimed row available to the dispatch policy even when
            // its stored digest is malformed. `dispatch_once_with_policy`
            // performs the authoritative check and can quarantine a poison
            // row instead of allowing an endless retry loop. Invalid digest
            // lengths use a deterministic placeholder that is guaranteed to
            // fail that check.
            let payload_digest = digest
                .as_slice()
                .try_into()
                .map(ContentDigest::new)
                .unwrap_or_else(|_| hash_bytes(&digest));
            let changed = transaction
                .execute(
                    "UPDATE outbox SET lease_owner=?1, lease_until=?2
                 WHERE delivery_key=?3 AND delivered_at IS NULL AND quarantined_at IS NULL
                   AND (lease_until IS NULL OR lease_until <= ?4)",
                    params![worker_id, lease_until_unix, key, now],
                )
                .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
            // Another worker may have claimed this row after the SELECT but
            // before our conditional UPDATE. Only return rows whose lease was
            // actually acquired; otherwise two processes could publish the
            // same item while believing they own it.
            if changed != 1 {
                continue;
            }
            claimed.push((
                key,
                OutboxPayload::Opaque {
                    tenant: TenantId(tenant),
                    commit_id: CommitId::new(commit_id)
                        .map_err(|error| OutboxError::Corrupt(error.to_string()))?,
                    payload_digest,
                    payload,
                },
            ));
        }
        drop(statement);
        transaction
            .commit()
            .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        Ok(claimed)
    }

    async fn mark_delivered(&self, delivery_key: &str) -> Result<(), OutboxError> {
        let _ = delivery_key;
        Err(OutboxError::Unavailable(String::from(
            "ownerless delivery completion is disabled; use mark_delivered_by",
        )))
    }

    async fn mark_delivered_by(
        &self,
        delivery_key: &str,
        worker_id: &str,
    ) -> Result<(), OutboxError> {
        if delivery_key.is_empty() || worker_id.is_empty() {
            return Err(OutboxError::Unavailable(String::from(
                "outbox delivery key and worker must not be empty",
            )));
        }
        let connection = self
            .connection
            .lock()
            .map_err(|poisoned| OutboxError::Unavailable(format!("mutex poisoned: {poisoned}")))?;
        let changed = connection
            .execute(
                "UPDATE outbox SET delivered_at=?1, lease_owner=NULL, lease_until=NULL
                 WHERE delivery_key=?2 AND delivered_at IS NULL AND quarantined_at IS NULL
                   AND lease_owner=?3",
                params![chrono::Utc::now().timestamp(), delivery_key, worker_id],
            )
            .map_err(|database_error| OutboxError::Unavailable(database_error.to_string()))?;
        if changed == 1 {
            return Ok(());
        }
        let state: Option<(Option<i64>, Option<String>)> = connection
            .query_row(
                "SELECT delivered_at, lease_owner FROM outbox WHERE delivery_key=?1",
                params![delivery_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|database_error| OutboxError::Unavailable(database_error.to_string()))?;
        if state.is_some_and(|(delivered, _)| delivered.is_some()) {
            Ok(())
        } else {
            Err(OutboxError::Unavailable(String::from(
                "outbox delivery lease is no longer owned by worker",
            )))
        }
    }

    async fn release(&self, delivery_key: &str, error: &str) -> Result<(), OutboxError> {
        let _ = (delivery_key, error);
        Err(OutboxError::Unavailable(String::from(
            "ownerless delivery release is disabled; use release_by",
        )))
    }

    async fn release_by(
        &self,
        delivery_key: &str,
        worker_id: &str,
        error: &str,
    ) -> Result<(), OutboxError> {
        if delivery_key.is_empty() || worker_id.is_empty() {
            return Err(OutboxError::Unavailable(String::from(
                "outbox delivery key and worker must not be empty",
            )));
        }
        let connection = self
            .connection
            .lock()
            .map_err(|poisoned| OutboxError::Unavailable(format!("mutex poisoned: {poisoned}")))?;
        let changed = connection
            .execute(
                "UPDATE outbox SET lease_owner=NULL, lease_until=NULL, last_error=?3
                 WHERE delivery_key=?1 AND delivered_at IS NULL AND quarantined_at IS NULL
                   AND lease_owner=?2",
                params![delivery_key, worker_id, bounded_outbox_error(error)],
            )
            .map_err(|database_error| OutboxError::Unavailable(database_error.to_string()))?;
        if changed != 1 {
            return Err(OutboxError::Unavailable(String::from(
                "outbox delivery lease is no longer owned by worker",
            )));
        }
        Ok(())
    }

    async fn quarantine(&self, delivery_key: &str, error: &str) -> Result<(), OutboxError> {
        let _ = (delivery_key, error);
        Err(OutboxError::Unavailable(String::from(
            "ownerless delivery quarantine is disabled; use quarantine_by",
        )))
    }

    async fn quarantine_by(
        &self,
        delivery_key: &str,
        worker_id: &str,
        error: &str,
    ) -> Result<(), OutboxError> {
        if delivery_key.is_empty() || worker_id.is_empty() {
            return Err(OutboxError::Unavailable(String::from(
                "outbox delivery key and worker must not be empty",
            )));
        }
        let connection = self
            .connection
            .lock()
            .map_err(|poisoned| OutboxError::Unavailable(format!("mutex poisoned: {poisoned}")))?;
        let changed = connection
            .execute(
                "UPDATE outbox SET quarantined_at=?1, quarantine_error=?2,
                 lease_owner=NULL, lease_until=NULL
                 WHERE delivery_key=?3 AND delivered_at IS NULL AND quarantined_at IS NULL
                   AND lease_owner=?4",
                params![
                    chrono::Utc::now().timestamp(),
                    bounded_outbox_error(error),
                    delivery_key,
                    worker_id
                ],
            )
            .map_err(|database_error| OutboxError::Unavailable(database_error.to_string()))?;
        if changed == 0 {
            return Err(OutboxError::Unavailable(String::from(
                "outbox delivery lease is no longer owned by worker",
            )));
        }
        Ok(())
    }

    async fn pending_count(&self, tenant: Option<&TenantId>) -> Result<u64, OutboxError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| OutboxError::Unavailable(format!("mutex poisoned: {error}")))?;
        let count: i64 = tenant
            .map_or_else(
                || {
                    connection.query_row(
                        "SELECT COUNT(*) FROM outbox WHERE delivered_at IS NULL AND quarantined_at IS NULL",
                        [],
                        |row| row.get(0),
                    )
                },
                |tenant| {
                    connection.query_row(
                        "SELECT COUNT(*) FROM outbox WHERE tenant_id=?1 AND delivered_at IS NULL AND quarantined_at IS NULL",
                        params![tenant.0],
                        |row| row.get(0),
                    )
                },
            )
            .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        u64::try_from(count).map_err(|error| OutboxError::Unavailable(error.to_string()))
    }
}

#[async_trait]
impl ConsumerDedupStore for SqliteLedgerStore {
    async fn claim_delivery(
        &self,
        delivery_key: &str,
        lease_until_unix: i64,
    ) -> Result<ConsumerDeliveryClaim, OutboxError> {
        if delivery_key.is_empty() {
            return Err(OutboxError::Unavailable(String::from(
                "consumer delivery key must not be empty",
            )));
        }
        if lease_until_unix <= chrono::Utc::now().timestamp() {
            return Err(OutboxError::Unavailable(String::from(
                "consumer lease expiration must be in the future",
            )));
        }
        let connection = self
            .connection
            .lock()
            .map_err(|error| OutboxError::Unavailable(format!("mutex poisoned: {error}")))?;
        let now = chrono::Utc::now().timestamp();
        let existing: Option<(String, String, i64)> = connection
            .query_row(
                "SELECT status, attempt_id, lease_until FROM consumer_deliveries
                 WHERE delivery_key=?1",
                params![delivery_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        if let Some((status, attempt_id, lease_until)) = existing {
            if status == "applied" {
                return Ok(ConsumerDeliveryClaim::AlreadyApplied);
            }
            if lease_until <= now {
                let replacement = format!("consumer-attempt-{}", uuid::Uuid::new_v4());
                let changed = connection
                    .execute(
                        "UPDATE consumer_deliveries
                         SET attempt_id=?1, lease_until=?2, last_error=NULL
                         WHERE delivery_key=?3 AND status='in_progress'
                           AND attempt_id=?4 AND lease_until<=?5",
                        params![replacement, lease_until_unix, delivery_key, attempt_id, now],
                    )
                    .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
                if changed == 1 {
                    return Ok(ConsumerDeliveryClaim::New {
                        attempt_id: replacement,
                    });
                }
            }
            return Ok(ConsumerDeliveryClaim::InProgress {
                attempt_id,
                lease_expires_at_unix: lease_until,
            });
        }
        let attempt_id = format!("consumer-attempt-{}", uuid::Uuid::new_v4());
        match connection.execute(
            "INSERT INTO consumer_deliveries
             (delivery_key, status, attempt_id, lease_until)
             VALUES (?1, 'in_progress', ?2, ?3)",
            params![delivery_key, attempt_id, lease_until_unix],
        ) {
            Ok(_) => Ok(ConsumerDeliveryClaim::New { attempt_id }),
            Err(error) if error.to_string().contains("UNIQUE constraint failed") => {
                let row: (String, String, i64) = connection
                    .query_row(
                        "SELECT status, attempt_id, lease_until FROM consumer_deliveries
                         WHERE delivery_key=?1",
                        params![delivery_key],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .map_err(|query_error| OutboxError::Unavailable(query_error.to_string()))?;
                if row.0 == "applied" {
                    Ok(ConsumerDeliveryClaim::AlreadyApplied)
                } else {
                    Ok(ConsumerDeliveryClaim::InProgress {
                        attempt_id: row.1,
                        lease_expires_at_unix: row.2,
                    })
                }
            }
            Err(error) => Err(OutboxError::Unavailable(error.to_string())),
        }
    }

    async fn mark_delivery_applied(
        &self,
        delivery_key: &str,
        attempt_id: &str,
    ) -> Result<(), OutboxError> {
        if delivery_key.is_empty() || attempt_id.is_empty() {
            return Err(OutboxError::Unavailable(String::from(
                "consumer delivery identifiers must not be empty",
            )));
        }
        let connection = self
            .connection
            .lock()
            .map_err(|error| OutboxError::Unavailable(format!("mutex poisoned: {error}")))?;
        let changed = connection
            .execute(
                "UPDATE consumer_deliveries SET status='applied', applied_at=?1,
                 lease_until=0 WHERE delivery_key=?2 AND status='in_progress'
                 AND attempt_id=?3",
                params![chrono::Utc::now().timestamp(), delivery_key, attempt_id],
            )
            .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        if changed == 1 {
            return Ok(());
        }
        let status: Option<String> = connection
            .query_row(
                "SELECT status FROM consumer_deliveries WHERE delivery_key=?1",
                params![delivery_key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        if status.as_deref() == Some("applied") {
            Ok(())
        } else {
            Err(OutboxError::Unavailable(String::from(
                "consumer delivery claim is not owned by attempt",
            )))
        }
    }

    async fn release_delivery(
        &self,
        delivery_key: &str,
        attempt_id: &str,
        error: &str,
    ) -> Result<(), OutboxError> {
        if delivery_key.is_empty() || attempt_id.is_empty() {
            return Err(OutboxError::Unavailable(String::from(
                "consumer delivery identifiers must not be empty",
            )));
        }
        let connection = self
            .connection
            .lock()
            .map_err(|poisoned| OutboxError::Unavailable(format!("mutex poisoned: {poisoned}")))?;
        connection
            .execute(
                "UPDATE consumer_deliveries SET lease_until=0, last_error=?1
                 WHERE delivery_key=?2 AND status='in_progress' AND attempt_id=?3",
                params![error, delivery_key, attempt_id],
            )
            .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        Ok(())
    }
}

struct SqliteLedgerTransaction {
    connection: Arc<Mutex<Connection>>,
    tenant: TenantId,
    attempt_id: Option<String>,
    claimed_intent_id: Option<IntentId>,
    claimed_intent_actor: Option<SubjectId>,
    claimed_intent_operation: Option<Operation>,
    finalized_commit_id: Option<CommitId>,
    finalized: bool,
    events: Vec<Event>,
    commit: Option<Signed<Commit>>,
    projections: Vec<StateProjection>,
    outbox: Vec<OutboxRecord>,
}

#[async_trait]
impl LedgerTransaction for SqliteLedgerTransaction {
    async fn claim_idempotency(
        &mut self,
        tenant: &TenantId,
        intent: &Intent,
        payload_digest: &ContentDigest,
    ) -> Result<IdempotencyClaim, LedgerStoreError> {
        if self.tenant != *tenant || self.attempt_id.is_some() {
            return Err(LedgerStoreError::Closed);
        }
        let connection = self.connection.lock().map_err(lock_error)?;
        let existing = connection
            .query_row(
                "SELECT payload_digest, status, attempt_id, lease_expires, commit_id
                 FROM idempotency WHERE tenant_id = ?1 AND intent_id = ?2",
                params![tenant.0, intent.intent_id.0],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(sqlite_error)?;
        if let Some((digest, status, attempt_id, lease, commit_id)) = existing {
            if digest != payload_digest.as_bytes() {
                return Ok(IdempotencyClaim::ConflictDifferentPayload);
            }
            if status == "committed" {
                let commit_id = commit_id.ok_or_else(|| {
                    LedgerStoreError::Invariant(String::from(
                        "committed idempotency row has no commit id",
                    ))
                })?;
                return Ok(IdempotencyClaim::Committed {
                    commit_id: CommitId::new(commit_id).map_err(domain_error)?,
                });
            }
            let now = chrono::Utc::now().timestamp();
            if lease <= now {
                let replacement = format!("attempt-{}", uuid::Uuid::new_v4());
                let replacement_lease = now.saturating_add(60);
                let changed = connection
                    .execute(
                        "UPDATE idempotency SET attempt_id=?1, lease_expires=?2
                         WHERE tenant_id=?3 AND intent_id=?4 AND status='in_progress'
                           AND attempt_id=?5 AND lease_expires<=?6",
                        params![
                            replacement,
                            replacement_lease,
                            tenant.0,
                            intent.intent_id.0,
                            attempt_id,
                            now
                        ],
                    )
                    .map_err(sqlite_error)?;
                if changed == 1 {
                    self.attempt_id = Some(replacement.clone());
                    self.claimed_intent_id = Some(intent.intent_id.clone());
                    self.claimed_intent_actor = Some(intent.actor.clone());
                    self.claimed_intent_operation = Some(intent.operation.clone());
                    return Ok(IdempotencyClaim::NewReservation {
                        attempt_id: replacement,
                    });
                }
            }
            return Ok(IdempotencyClaim::InProgress {
                attempt_id,
                lease_expires_at_unix: lease,
            });
        }
        let attempt_id = format!("attempt-{}", uuid::Uuid::new_v4());
        let lease = chrono::Utc::now().timestamp().saturating_add(60);
        let intent_payload = bcs::to_bytes(intent)
            .map_err(|error| LedgerStoreError::Invariant(error.to_string()))?;
        let insert_result = connection.execute(
                "INSERT INTO idempotency
                 (tenant_id, intent_id, payload_digest, status, attempt_id, lease_expires, intent_payload)
                 VALUES (?1, ?2, ?3, 'in_progress', ?4, ?5, ?6)",
                params![
                    tenant.0,
                    intent.intent_id.0,
                    payload_digest.as_bytes(),
                    attempt_id,
                    lease,
                    intent_payload
                ],
            );
        if let Err(error) = insert_result {
            // Independent database connections can race between the SELECT
            // above and this INSERT. Convert the uniqueness loser into the
            // same deterministic claim result as a row observed normally.
            if !error
                .to_string()
                .contains("UNIQUE constraint failed: idempotency")
            {
                return Err(sqlite_error(error));
            }
            let (stored_digest, status, owner, expires, committed): (
                Vec<u8>,
                String,
                String,
                i64,
                Option<String>,
            ) = connection
                .query_row(
                    "SELECT payload_digest, status, attempt_id, lease_expires, commit_id
                     FROM idempotency WHERE tenant_id=?1 AND intent_id=?2",
                    params![tenant.0, intent.intent_id.0],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .map_err(sqlite_error)?;
            if stored_digest != payload_digest.as_bytes() {
                return Ok(IdempotencyClaim::ConflictDifferentPayload);
            }
            if status == "committed" {
                let commit_id = committed.ok_or_else(|| {
                    LedgerStoreError::Invariant(String::from(
                        "committed idempotency row has no commit id",
                    ))
                })?;
                return Ok(IdempotencyClaim::Committed {
                    commit_id: CommitId::new(commit_id).map_err(domain_error)?,
                });
            }
            return Ok(IdempotencyClaim::InProgress {
                attempt_id: owner,
                lease_expires_at_unix: expires,
            });
        }
        self.attempt_id = Some(attempt_id.clone());
        self.claimed_intent_id = Some(intent.intent_id.clone());
        self.claimed_intent_actor = Some(intent.actor.clone());
        self.claimed_intent_operation = Some(intent.operation.clone());
        Ok(IdempotencyClaim::NewReservation { attempt_id })
    }

    async fn append_events(&mut self, events: &[Event]) -> Result<(), LedgerStoreError> {
        if self.attempt_id.is_none() || self.finalized {
            return Err(LedgerStoreError::Closed);
        }
        if self.events.len().saturating_add(events.len()) > MAX_EVENTS_PER_COMMIT {
            return Err(LedgerStoreError::Invariant(format!(
                "event count exceeds durable limit {MAX_EVENTS_PER_COMMIT}"
            )));
        }
        for event in events {
            if event.tenant_id != self.tenant
                || self.claimed_intent_id.as_ref() != Some(&event.intent_id)
                || self.claimed_intent_actor.as_ref() != Some(&event.actor)
                || self.claimed_intent_operation.as_ref() != Some(&event.operation)
            {
                return Err(LedgerStoreError::Invariant(String::from(
                    "event scope does not match claimed intent",
                )));
            }
            if event.event_id.0.is_empty() || event.intent_id.0.is_empty() {
                return Err(LedgerStoreError::Invariant(String::from(
                    "event and intent identifiers must not be empty",
                )));
            }
        }
        let mut total_bytes = 0usize;
        for event in self.events.iter().chain(events) {
            let bytes = bcs::to_bytes(event)
                .map_err(|error| LedgerStoreError::Invariant(error.to_string()))?;
            total_bytes = total_bytes.checked_add(bytes.len()).ok_or_else(|| {
                LedgerStoreError::Invariant(String::from("event batch size overflow"))
            })?;
        }
        if total_bytes > MAX_EVENT_BATCH_BYTES {
            return Err(LedgerStoreError::Invariant(format!(
                "event batch bytes exceed durable limit {MAX_EVENT_BATCH_BYTES}"
            )));
        }
        self.events.extend_from_slice(events);
        Ok(())
    }

    async fn append_commit(&mut self, commit: &Signed<Commit>) -> Result<(), LedgerStoreError> {
        if self.attempt_id.is_none() || self.finalized {
            return Err(LedgerStoreError::Closed);
        }
        if commit.body.commit_id.0.is_empty()
            || commit.body.executor.0.is_empty()
            || commit.body.profile.0.is_empty()
            || commit.signature.key_id.as_str().is_empty()
        {
            return Err(LedgerStoreError::Invariant(String::from(
                "commit identity and signature key identifiers must not be empty",
            )));
        }
        if commit.body.scope.tenant_id.as_ref() != Some(&self.tenant) {
            return Err(LedgerStoreError::Invariant(String::from(
                "commit scope does not match transaction tenant",
            )));
        }
        if self.commit.is_some() {
            return Err(LedgerStoreError::Invariant(String::from(
                "transaction may append only one commit",
            )));
        }
        if self.events.is_empty() {
            return Err(LedgerStoreError::Invariant(String::from(
                "commit must include at least one staged event",
            )));
        }
        self.commit = Some(commit.clone());
        Ok(())
    }

    async fn upsert_projection(
        &mut self,
        projection: &StateProjection,
    ) -> Result<(), LedgerStoreError> {
        if self.attempt_id.is_none() || self.finalized {
            return Err(LedgerStoreError::Closed);
        }
        if projection.tenant_id != self.tenant
            || projection.resource_id.0.is_empty()
            || projection.last_event_id.0.is_empty()
            || projection.last_commit_id.0.is_empty()
        {
            return Err(LedgerStoreError::Invariant(String::from(
                "projection scope or identifiers are invalid",
            )));
        }
        self.projections.push(projection.clone());
        Ok(())
    }

    async fn enqueue_outbox(&mut self, record: &OutboxRecord) -> Result<(), LedgerStoreError> {
        if self.attempt_id.is_none() || self.finalized {
            return Err(LedgerStoreError::Closed);
        }
        if record.tenant != self.tenant
            || record.delivery_key.is_empty()
            || record.commit_id.0.is_empty()
        {
            return Err(LedgerStoreError::Invariant(String::from(
                "outbox scope or identifiers are invalid",
            )));
        }
        if hash_bytes(&record.payload) != record.payload_digest {
            return Err(LedgerStoreError::Invariant(String::from(
                "outbox payload digest does not match payload",
            )));
        }
        check_size(
            "outbox_payload",
            MAX_OUTBOX_PAYLOAD_BYTES,
            record.payload.len(),
        )
        .map_err(|error| LedgerStoreError::Invariant(error.to_string()))?;
        self.outbox.push(record.clone());
        Ok(())
    }

    async fn finalize_idempotency(
        &mut self,
        tenant: &TenantId,
        intent_id: &IntentId,
        attempt_id: &str,
        commit_id: &CommitId,
    ) -> Result<(), LedgerStoreError> {
        if self.attempt_id.as_deref() != Some(attempt_id)
            || self.tenant != *tenant
            || self.claimed_intent_id.as_ref() != Some(intent_id)
        {
            return Err(LedgerStoreError::Idempotency(String::from(
                "attempt does not own idempotency reservation",
            )));
        }
        if intent_id.0.is_empty() || commit_id.0.is_empty() {
            return Err(LedgerStoreError::Idempotency(String::from(
                "idempotency identifiers must not be empty",
            )));
        }
        let Some(staged_commit) = self.commit.as_ref() else {
            return Err(LedgerStoreError::Invariant(String::from(
                "idempotency cannot finalize before a commit is appended",
            )));
        };
        if staged_commit.body.scope.tenant_id.as_ref() != Some(tenant)
            || staged_commit.body.commit_id != *commit_id
        {
            return Err(LedgerStoreError::Invariant(String::from(
                "idempotency finalization must target the staged tenant commit",
            )));
        }
        // Stage this update. It is written by `commit()` together with every
        // other ledger effect, closing the crash window between finalization
        // and event/commit persistence.
        self.finalized_commit_id = Some(commit_id.clone());
        self.finalized = true;
        Ok(())
    }

    #[allow(clippy::collapsible_if)]
    async fn commit(mut self: Box<Self>) -> Result<(), LedgerStoreError> {
        if self.attempt_id.is_none() || !self.finalized {
            return Err(LedgerStoreError::Invariant(String::from(
                "transaction must finalize idempotency before commit",
            )));
        }
        let signed_commit = self.commit.as_ref().ok_or_else(|| {
            LedgerStoreError::Invariant(String::from("transaction has no commit"))
        })?;
        let event_count = u64::try_from(self.events.len()).map_err(|error| {
            LedgerStoreError::Invariant(format!("event count overflow: {error}"))
        })?;
        if event_count != signed_commit.body.event_count
            || event_root(&self.events)
                .map_err(|error| LedgerStoreError::Invariant(error.to_string()))?
                != signed_commit.body.event_merkle_root
        {
            return Err(LedgerStoreError::Invariant(String::from(
                "commit event count or Merkle root does not match staged events",
            )));
        }
        let connection = self.connection.lock().map_err(lock_error)?;
        let sql_transaction = connection.unchecked_transaction().map_err(sqlite_error)?;
        let attempt_id = self.attempt_id.as_deref().ok_or(LedgerStoreError::Closed)?;
        let finalized_commit_id = self.finalized_commit_id.as_ref().ok_or_else(|| {
            LedgerStoreError::Invariant(String::from("missing finalized commit id"))
        })?;
        let claimed_intent_id = self.claimed_intent_id.as_ref().ok_or_else(|| {
            LedgerStoreError::Invariant(String::from("missing claimed intent id"))
        })?;
        let head = sql_transaction
            .query_row(
                "SELECT commit_id, sequence, state_root FROM heads WHERE tenant_id=?1",
                params![self.tenant.0],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(sqlite_error)?;
        match head {
            Some((head_id, head_sequence, head_root)) => {
                if signed_commit.body.parent_commit_id.as_ref().map(|id| &id.0) != Some(&head_id)
                    || signed_commit.body.sequence != head_sequence.saturating_add(1)
                    || signed_commit.body.previous_state_root.as_bytes() != head_root.as_slice()
                {
                    return Err(LedgerStoreError::Conflict(String::from(
                        "commit does not extend the canonical tenant head",
                    )));
                }
            }
            None if signed_commit.body.parent_commit_id.is_some() => {
                return Err(LedgerStoreError::Conflict(String::from(
                    "commit declares a parent but the tenant has no canonical head",
                )));
            }
            None => {}
        }
        let commit_bytes = bcs::to_bytes(signed_commit)
            .map_err(|error| LedgerStoreError::Invariant(error.to_string()))?;
        sql_transaction
            .execute(
                "INSERT INTO commits (tenant_id, commit_id, sequence, payload) VALUES (?1, ?2, ?3, ?4)",
                params![self.tenant.0, signed_commit.body.commit_id.0, signed_commit.body.sequence, commit_bytes],
            )
            .map_err(sqlite_error)?;
        for (event_index, event) in self.events.iter().enumerate() {
            let bytes = bcs::to_bytes(event)
                .map_err(|error| LedgerStoreError::Invariant(error.to_string()))?;
            sql_transaction
                .execute(
                    "INSERT INTO events (tenant_id, event_id, commit_id, event_index, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        event.tenant_id.0,
                        event.event_id.0,
                        signed_commit.body.commit_id.0,
                        i64::try_from(event_index).map_err(|error| {
                            LedgerStoreError::Invariant(format!("event index overflow: {error}"))
                        })?,
                        bytes
                    ],
                )
                .map_err(sqlite_error)?;
        }
        sql_transaction
            .execute(
                "INSERT INTO heads (tenant_id, commit_id, sequence, state_root) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(tenant_id) DO UPDATE SET commit_id=excluded.commit_id,
                   sequence=excluded.sequence, state_root=excluded.state_root",
                params![
                    self.tenant.0,
                    signed_commit.body.commit_id.0,
                    signed_commit.body.sequence,
                    signed_commit.body.next_state_root.as_bytes()
                ],
            )
            .map_err(sqlite_error)?;
        for projection in &self.projections {
            if projection.tenant_id != self.tenant
                || projection.last_commit_id != signed_commit.body.commit_id
            {
                return Err(LedgerStoreError::Invariant(String::from(
                    "projection is not bound to the staged commit",
                )));
            }
            let bytes = bcs::to_bytes(projection)
                .map_err(|error| LedgerStoreError::Invariant(error.to_string()))?;
            let existing: Option<(u64, Vec<u8>)> = sql_transaction
                .query_row(
                    "SELECT version, payload FROM projections
                     WHERE tenant_id=?1 AND resource_id=?2",
                    params![projection.tenant_id.0, projection.resource_id.0],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(sqlite_error)?;
            if let Some((version, existing_payload)) = existing {
                if version == projection.version && existing_payload != bytes {
                    return Err(LedgerStoreError::Conflict(format!(
                        "conflicting projection version {} for resource `{}`",
                        projection.version, projection.resource_id.0
                    )));
                }
            }
            sql_transaction
                .execute(
                    "INSERT INTO projections (tenant_id, resource_id, version, payload)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(tenant_id, resource_id) DO UPDATE SET
                       version=excluded.version, payload=excluded.payload
                     WHERE excluded.version > projections.version",
                    params![
                        projection.tenant_id.0,
                        projection.resource_id.0,
                        projection.version,
                        bytes
                    ],
                )
                .map_err(sqlite_error)?;
        }
        for record in &self.outbox {
            if record.tenant != self.tenant || record.commit_id != signed_commit.body.commit_id {
                return Err(LedgerStoreError::Invariant(String::from(
                    "outbox record is not bound to the staged commit",
                )));
            }
            sql_transaction
                .execute(
                    "INSERT INTO outbox (delivery_key, tenant_id, commit_id, payload_digest, payload)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        record.delivery_key,
                        record.tenant.0,
                        record.commit_id.0,
                        record.payload_digest.as_bytes(),
                        &record.payload
                    ],
                )
                .map_err(sqlite_error)?;
        }
        let changed = sql_transaction
            .execute(
                "UPDATE idempotency SET status='committed', commit_id=?1
                 WHERE tenant_id=?2 AND intent_id=?3 AND attempt_id=?4 AND status='in_progress'",
                params![
                    finalized_commit_id.0,
                    self.tenant.0,
                    claimed_intent_id.0,
                    attempt_id
                ],
            )
            .map_err(sqlite_error)?;
        if changed != 1 {
            return Err(LedgerStoreError::Idempotency(String::from(
                "idempotency reservation was lost",
            )));
        }
        sql_transaction.commit().map_err(sqlite_error)
    }

    async fn rollback(self: Box<Self>) -> Result<(), LedgerStoreError> {
        if let Some(attempt_id) = self.attempt_id {
            let connection = self.connection.lock().map_err(lock_error)?;
            connection
                .execute(
                    "DELETE FROM idempotency WHERE tenant_id=?1 AND attempt_id=?2 AND status='in_progress'",
                    params![self.tenant.0, attempt_id],
                )
                .map_err(sqlite_error)?;
        }
        Ok(())
    }
}

fn initialize(connection: &Connection) -> Result<(), LedgerStoreError> {
    let existing_version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sqlite_error)?;
    if existing_version > SQLITE_SCHEMA_VERSION {
        return Err(LedgerStoreError::Unavailable(format!(
            "database schema version {existing_version} is newer than supported version {SQLITE_SCHEMA_VERSION}"
        )));
    }
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS idempotency (
               tenant_id TEXT NOT NULL CHECK(length(tenant_id) > 0),
               intent_id TEXT NOT NULL CHECK(length(intent_id) > 0),
               payload_digest BLOB NOT NULL,
               status TEXT NOT NULL CHECK(status IN ('in_progress','committed')),
               attempt_id TEXT NOT NULL,
               lease_expires INTEGER NOT NULL,
               commit_id TEXT,
               intent_payload BLOB,
               PRIMARY KEY (tenant_id, intent_id)
             );
             CREATE TABLE IF NOT EXISTS events (
               tenant_id TEXT NOT NULL CHECK(length(tenant_id) > 0),
               event_id TEXT NOT NULL CHECK(length(event_id) > 0),
               commit_id TEXT,
               event_index INTEGER,
               payload BLOB NOT NULL,
               PRIMARY KEY (tenant_id, event_id)
             );
             CREATE TABLE IF NOT EXISTS commits (
               tenant_id TEXT NOT NULL CHECK(length(tenant_id) > 0),
               commit_id TEXT NOT NULL CHECK(length(commit_id) > 0),
               sequence INTEGER NOT NULL CHECK(sequence >= 0),
               payload BLOB NOT NULL,
               PRIMARY KEY (tenant_id, commit_id),
               UNIQUE (tenant_id, sequence)
             );
             CREATE TABLE IF NOT EXISTS heads (
               tenant_id TEXT PRIMARY KEY CHECK(length(tenant_id) > 0),
               commit_id TEXT NOT NULL CHECK(length(commit_id) > 0),
               sequence INTEGER NOT NULL CHECK(sequence >= 0),
               state_root BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS projections (
               tenant_id TEXT NOT NULL CHECK(length(tenant_id) > 0),
               resource_id TEXT NOT NULL CHECK(length(resource_id) > 0),
               version INTEGER NOT NULL CHECK(version >= 0),
               payload BLOB NOT NULL,
               PRIMARY KEY (tenant_id, resource_id)
             );
             CREATE TABLE IF NOT EXISTS outbox (
               delivery_key TEXT PRIMARY KEY CHECK(length(delivery_key) > 0),
               tenant_id TEXT NOT NULL CHECK(length(tenant_id) > 0),
               commit_id TEXT NOT NULL CHECK(length(commit_id) > 0),
               payload_digest BLOB NOT NULL,
               payload BLOB NOT NULL DEFAULT X'',
               lease_owner TEXT,
               lease_until INTEGER,
               delivered_at INTEGER,
               quarantined_at INTEGER,
               quarantine_error TEXT CHECK(quarantine_error IS NULL OR length(quarantine_error) <= 4096),
               last_error TEXT CHECK(last_error IS NULL OR length(last_error) <= 4096)
             );
             CREATE TABLE IF NOT EXISTS projection_rebuild_checkpoints (
               checkpoint_key TEXT PRIMARY KEY CHECK(length(checkpoint_key) > 0),
               next_event INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS consumer_deliveries (
               delivery_key TEXT PRIMARY KEY CHECK(length(delivery_key) > 0),
               status TEXT NOT NULL CHECK(status IN ('in_progress','applied')),
               attempt_id TEXT NOT NULL,
               lease_until INTEGER NOT NULL,
               applied_at INTEGER,
               last_error TEXT
             );",
        )
        .map_err(sqlite_error)?;
    // Keep upgrades from the initial adapter schema recoverable. Existing
    // databases may predate intent payload storage, which is required for
    // legacy IntentStore replay; add the nullable column without rewriting
    // any immutable ledger rows.
    let has_intent_payload: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('idempotency') WHERE name='intent_payload'",
            [],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    if has_intent_payload == 0 {
        connection
            .execute("ALTER TABLE idempotency ADD COLUMN intent_payload BLOB", [])
            .map_err(sqlite_error)?;
    }
    ensure_column(
        connection,
        "outbox",
        "payload",
        "ALTER TABLE outbox ADD COLUMN payload BLOB NOT NULL DEFAULT X''",
    )?;
    ensure_column(
        connection,
        "events",
        "commit_id",
        "ALTER TABLE events ADD COLUMN commit_id TEXT",
    )?;
    ensure_column(
        connection,
        "events",
        "event_index",
        "ALTER TABLE events ADD COLUMN event_index INTEGER",
    )?;
    connection
        .execute_batch(&format!("PRAGMA user_version = {SQLITE_SCHEMA_VERSION};"))
        .map_err(sqlite_error)?;
    ensure_column(
        connection,
        "outbox",
        "lease_owner",
        "ALTER TABLE outbox ADD COLUMN lease_owner TEXT",
    )?;
    ensure_column(
        connection,
        "outbox",
        "lease_until",
        "ALTER TABLE outbox ADD COLUMN lease_until INTEGER",
    )?;
    ensure_column(
        connection,
        "outbox",
        "quarantined_at",
        "ALTER TABLE outbox ADD COLUMN quarantined_at INTEGER",
    )?;
    ensure_column(
        connection,
        "outbox",
        "quarantine_error",
        "ALTER TABLE outbox ADD COLUMN quarantine_error TEXT",
    )?;
    ensure_column(
        connection,
        "outbox",
        "last_error",
        "ALTER TABLE outbox ADD COLUMN last_error TEXT",
    )?;
    Ok(())
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    alter_statement: &str,
) -> Result<(), LedgerStoreError> {
    let present: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name=?2",
            params![table, column],
            |row| row.get(0),
        )
        .map_err(sqlite_error)?;
    if present == 0 {
        connection
            .execute(alter_statement, [])
            .map_err(sqlite_error)?;
    }
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn sqlite_error(error: rusqlite::Error) -> LedgerStoreError {
    LedgerStoreError::Unavailable(error.to_string())
}

fn lock_error<T>(_error: std::sync::PoisonError<T>) -> LedgerStoreError {
    LedgerStoreError::Unavailable(String::from("ledger database mutex poisoned"))
}

#[allow(clippy::needless_pass_by_value)]
fn domain_error(error: statechronicle_domain::error::DomainError) -> LedgerStoreError {
    LedgerStoreError::Invariant(error.to_string())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::wildcard_enum_match_arm
)]
mod tests {
    use super::*;
    use statechronicle_core::canonicalize::canonicalize_and_digest;
    use statechronicle_core::digest::hash_bytes;
    use statechronicle_domain::commit::{Commit, CommitScope, ProfileId};
    use statechronicle_domain::event::{Event, StateCommitment};
    use statechronicle_domain::ids::{CommitId, EventId, IntentId};
    use statechronicle_domain::intent::{KeyId, Nonce, Operation, SignatureAlg, SignatureBlock};
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::resource_state::{ResourceState, UniqueAssetState};
    use statechronicle_domain::signed::Signed;
    use statechronicle_domain::state::StateProjection;
    use statechronicle_domain::status::Status;
    use statechronicle_domain::subject::SubjectId;
    use statechronicle_ports::outbox::{
        OutboxConsumer, OutboxPublisher, OutboxStore, PoisonPolicy, consume_once, dispatch_once,
        dispatch_once_with_policy, dispatch_until_idle,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestPublisher {
        fail: bool,
    }

    #[tokio::test]
    async fn outbox_claim_limit_is_bounded() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        assert!(
            store
                .claim("worker", MAX_OUTBOX_CLAIM + 1, 1)
                .await
                .is_err()
        );
    }

    struct CountingConsumer {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    #[async_trait]
    impl OutboxConsumer for CountingConsumer {
        async fn apply(&self, _delivery_key: &str, _payload: &OutboxPayload) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(String::from("consumer unavailable"))
            } else {
                Ok(())
            }
        }
    }

    #[async_trait]
    impl OutboxPublisher for TestPublisher {
        async fn publish(
            &self,
            _delivery_key: &str,
            _payload: &OutboxPayload,
        ) -> Result<(), String> {
            if self.fail {
                Err(String::from("broker unavailable"))
            } else {
                Ok(())
            }
        }
    }

    fn intent() -> Intent {
        Intent::new(
            TenantId(String::from("game")),
            IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
            Operation::from_static("asset.transfer"),
            statechronicle_domain::subject::SubjectId(String::from("account:alice")),
            ResourceId(String::from("asset:sword")),
            Some(statechronicle_domain::state_type::StateType::UniqueAsset),
            1,
            std::collections::BTreeMap::new(),
            None,
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            None,
            Nonce::from_bytes(vec![1]).unwrap(),
        )
    }

    #[tokio::test]
    async fn reservation_rolls_back_and_can_be_retried() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let tenant = TenantId(String::from("game"));
        let request = intent();
        let digest = canonicalize_and_digest(&request).unwrap();
        let mut first = store.begin(&tenant).await.unwrap();
        assert!(matches!(
            first
                .claim_idempotency(&tenant, &request, &digest)
                .await
                .unwrap(),
            IdempotencyClaim::NewReservation { .. }
        ));
        first.rollback().await.unwrap();

        let mut retry = store.begin(&tenant).await.unwrap();
        assert!(matches!(
            retry
                .claim_idempotency(&tenant, &request, &digest)
                .await
                .unwrap(),
            IdempotencyClaim::NewReservation { .. }
        ));
        retry.rollback().await.unwrap();
    }

    #[test]
    fn process_crash_before_commit_releases_reservation() {
        const CHILD_ENV: &str = "STATECHRONICLE_CRASH_CHILD_DB";
        if let Ok(path) = std::env::var(CHILD_ENV) {
            let store = SqliteLedgerStore::open(path).unwrap();
            let tenant = TenantId(String::from("game"));
            let request = intent();
            let digest = canonicalize_and_digest(&request).unwrap();
            let mut transaction = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(store.begin(&tenant))
                .unwrap();
            let claim = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(transaction.claim_idempotency(&tenant, &request, &digest))
                .unwrap();
            assert!(matches!(claim, IdempotencyClaim::NewReservation { .. }));
            // Exit without dropping/rolling back the transaction explicitly;
            // the OS closes the SQLite connection and recovery must discard the
            // uncommitted reservation.
            std::process::exit(137);
        }

        let path = std::env::temp_dir().join(format!(
            "statechronicle-process-crash-{}.db",
            uuid::Uuid::new_v4()
        ));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("tests::process_crash_before_commit_releases_reservation")
            .arg("--nocapture")
            .env(CHILD_ENV, &path)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(137));

        let store = SqliteLedgerStore::open(&path).unwrap();
        let tenant = TenantId(String::from("game"));
        let request = intent();
        let digest = canonicalize_and_digest(&request).unwrap();
        {
            let connection = store.connection.lock().unwrap();
            connection
                .execute(
                    "UPDATE idempotency SET lease_expires=0 WHERE tenant_id=?1 AND intent_id=?2",
                    params![tenant.0, request.intent_id.0],
                )
                .unwrap();
        }
        let mut retry = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(store.begin(&tenant))
            .unwrap();
        assert!(matches!(
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(retry.claim_idempotency(&tenant, &request, &digest))
                .unwrap(),
            IdempotencyClaim::NewReservation { .. }
        ));
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(retry.rollback())
            .unwrap();
    }

    #[tokio::test]
    async fn intent_store_roundtrips_canonical_payload() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let tenant = TenantId(String::from("game"));
        let request = intent();
        IntentStore::put_intent(&store, &tenant, &request)
            .await
            .unwrap();
        let loaded = IntentStore::get_intent(&store, &tenant, &request.intent_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded, request);
    }

    #[tokio::test]
    async fn file_backed_store_survives_reopen() {
        let path = std::env::temp_dir().join(format!(
            "statechronicle-restart-{}.db",
            uuid::Uuid::new_v4()
        ));
        let tenant = TenantId(String::from("game"));
        let request = intent();
        {
            let store = SqliteLedgerStore::open(&path).unwrap();
            IntentStore::put_intent(&store, &tenant, &request)
                .await
                .unwrap();
        }
        let reopened = SqliteLedgerStore::open(&path).unwrap();
        {
            let connection = reopened.connection.lock().unwrap();
            let synchronous: i64 = connection
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .unwrap();
            assert_eq!(synchronous, 2);
        }
        let loaded = IntentStore::get_intent(&reopened, &tenant, &request.intent_id)
            .await
            .unwrap();
        assert_eq!(loaded, Some(request));
        let verified_open = SqliteLedgerStore::open_verified(&path).unwrap();
        assert_eq!(verified_open.verify_all_integrity().unwrap().len(), 1);
        let backup_path =
            std::env::temp_dir().join(format!("statechronicle-backup-{}.db", uuid::Uuid::new_v4()));
        reopened.backup_to(&backup_path).unwrap();
        let backup = SqliteLedgerStore::open(&backup_path).unwrap();
        assert_eq!(backup.verify_all_integrity().unwrap().len(), 1);
        assert!(matches!(
            reopened.backup_to(&backup_path),
            Err(LedgerStoreError::Unavailable(message))
                if message.contains("destination already exists")
        ));
        assert_eq!(
            IntentStore::get_intent(&backup, &tenant, &intent().intent_id)
                .await
                .unwrap(),
            Some(intent())
        );
        drop(backup);
        std::fs::remove_file(backup_path).unwrap();
        statechronicle_index::rebuild::CheckpointStore::save(&reopened, "projection-v1", 42)
            .await
            .unwrap();
        drop(reopened);
        let reopened_again = SqliteLedgerStore::open(&path).unwrap();
        assert_eq!(
            statechronicle_index::rebuild::CheckpointStore::load(&reopened_again, "projection-v1")
                .await
                .unwrap(),
            Some(42)
        );
        reopened_again
            .clear_rebuild_checkpoint("projection-v1")
            .unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn backup_is_point_in_time_and_survives_source_mutation() {
        let source_path = std::env::temp_dir().join(format!(
            "statechronicle-backup-source-{}.db",
            uuid::Uuid::new_v4()
        ));
        let backup_path = std::env::temp_dir().join(format!(
            "statechronicle-backup-restore-{}.db",
            uuid::Uuid::new_v4()
        ));
        let tenant = TenantId(String::from("game"));
        let request = intent();
        {
            let source = SqliteLedgerStore::open(&source_path).unwrap();
            IntentStore::put_intent(&source, &tenant, &request)
                .await
                .unwrap();
            source.backup_to(&backup_path).unwrap();
            source
                .connection
                .lock()
                .unwrap()
                .execute(
                    "DELETE FROM idempotency WHERE tenant_id=?1 AND intent_id=?2",
                    params![tenant.0, request.intent_id.0],
                )
                .unwrap();
        }

        let source = SqliteLedgerStore::open(&source_path).unwrap();
        assert_eq!(
            IntentStore::get_intent(&source, &tenant, &request.intent_id)
                .await
                .unwrap(),
            None
        );
        let restored = SqliteLedgerStore::open_verified(&backup_path).unwrap();
        assert_eq!(
            IntentStore::get_intent(&restored, &tenant, &request.intent_id)
                .await
                .unwrap(),
            Some(request)
        );
        assert_eq!(restored.verify_all_integrity().unwrap().len(), 1);

        drop(source);
        drop(restored);
        std::fs::remove_file(source_path).unwrap();
        std::fs::remove_file(backup_path).unwrap();
    }

    #[test]
    fn integrity_scan_rejects_orphan_event_rows() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let connection = store.connection.lock().unwrap();
        connection
            .execute(
                "INSERT INTO events (tenant_id, event_id, commit_id, payload)
                 VALUES ('game', 'evt_orphan', 'cmt_missing', X'00')",
                [],
            )
            .unwrap();
        drop(connection);
        assert_eq!(
            store.tenants().unwrap(),
            vec![TenantId(String::from("game"))]
        );
        let error = store
            .verify_integrity(&TenantId(String::from("game")))
            .unwrap_err();
        assert!(error.to_string().contains("event"));
    }

    #[test]
    fn integrity_scan_rejects_orphan_outbox_rows() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let connection = store.connection.lock().unwrap();
        connection
            .execute(
                "INSERT INTO outbox
                 (delivery_key, tenant_id, commit_id, payload_digest, payload)
                 VALUES ('delivery:orphan', 'game', 'cmt_missing', ?1, ?2)",
                params![hash_bytes(b"payload").as_bytes(), b"payload".as_slice()],
            )
            .unwrap();
        drop(connection);
        let error = store
            .verify_integrity(&TenantId(String::from("game")))
            .unwrap_err();
        assert!(error.to_string().contains("outbox"));
    }

    #[test]
    fn integrity_scan_rejects_malformed_projection_rows() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let connection = store.connection.lock().unwrap();
        connection
            .execute(
                "INSERT INTO projections (tenant_id, resource_id, version, payload)
                 VALUES ('game', 'asset:sword', 1, X'00')",
                [],
            )
            .unwrap();
        drop(connection);
        let error = store
            .verify_integrity(&TenantId(String::from("game")))
            .unwrap_err();
        assert!(error.to_string().contains("projection"));
        assert!(
            store
                .projection_rebuild_progress(&TenantId(String::from("game")), "rebuild")
                .is_err()
        );
    }

    #[test]
    fn open_verified_fails_closed_on_corrupted_file() {
        let path = std::env::temp_dir().join(format!(
            "statechronicle-open-verified-{}.db",
            uuid::Uuid::new_v4()
        ));
        {
            let store = SqliteLedgerStore::open(&path).unwrap();
            let connection = store.connection.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO projections (tenant_id, resource_id, version, payload)
                     VALUES ('game', 'asset:sword', 1, X'00')",
                    [],
                )
                .unwrap();
        }
        assert!(SqliteLedgerStore::open_verified(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rebuild_checkpoints_are_scoped_per_tenant() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let tenant_a = TenantId(String::from("game-a"));
        let tenant_b = TenantId(String::from("game-b"));
        let key = "same-operator-key";
        let scoped_a = SqliteLedgerStore::rebuild_checkpoint_key(&tenant_a, key).unwrap();
        let scoped_b = SqliteLedgerStore::rebuild_checkpoint_key(&tenant_b, key).unwrap();
        let connection = store.connection.lock().unwrap();
        connection
            .execute(
                "INSERT INTO projection_rebuild_checkpoints (checkpoint_key, next_event)
                 VALUES (?1, 0), (?2, 1)",
                params![scoped_a, scoped_b],
            )
            .unwrap();
        drop(connection);

        assert!(
            store
                .projection_rebuild_progress(&tenant_a, key)
                .unwrap()
                .caught_up
        );
        assert!(store.projection_rebuild_progress(&tenant_b, key).is_err());
        store
            .clear_rebuild_checkpoint_for_tenant(&tenant_a, key)
            .unwrap();
        let remaining_connection = store.connection.lock().unwrap();
        let remaining: i64 = remaining_connection
            .query_row(
                "SELECT COUNT(*) FROM projection_rebuild_checkpoints WHERE checkpoint_key=?1",
                params![scoped_b],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn integrity_scan_rejects_incomplete_consumer_delivery() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let connection = store.connection.lock().unwrap();
        connection
            .execute(
                "INSERT INTO consumer_deliveries
                 (delivery_key, status, attempt_id, lease_until, applied_at)
                 VALUES ('delivery:broken', 'applied', 'attempt-1', 0, NULL)",
                [],
            )
            .unwrap();
        drop(connection);
        let error = store.verify_all_integrity().unwrap_err();
        assert!(error.to_string().contains("consumer delivery"));
    }

    #[test]
    fn new_schema_rejects_empty_identifiers() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let connection = store.connection.lock().unwrap();
        let error = connection
            .execute(
                "INSERT INTO projections (tenant_id, resource_id, version, payload)
                 VALUES ('', 'asset:sword', 1, X'00')",
                [],
            )
            .unwrap_err();
        assert!(error.to_string().contains("CHECK constraint failed"));
    }

    #[test]
    fn clearing_unknown_rebuild_checkpoint_is_idempotent() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        store.clear_rebuild_checkpoint("missing").unwrap();
        store.clear_rebuild_checkpoint("missing").unwrap();
    }

    #[test]
    fn newer_schema_version_fails_closed() {
        let path = std::env::temp_dir().join(format!(
            "statechronicle-schema-version-{}.db",
            uuid::Uuid::new_v4()
        ));
        let store = SqliteLedgerStore::open(&path).unwrap();
        {
            let connection = store.connection.lock().unwrap();
            connection
                .execute_batch("PRAGMA user_version = 999;")
                .unwrap();
        }
        drop(store);
        let error = match SqliteLedgerStore::open(&path) {
            Ok(_) => panic!("newer schema unexpectedly opened"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("newer than supported"));
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn projection_rebuild_rejects_equal_version_conflicts() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let tenant = TenantId(String::from("game"));
        let resource = ResourceId(String::from("asset:sword"));
        let make = |owner: &str, event: &str| StateProjection {
            tenant_id: tenant.clone(),
            resource_id: resource.clone(),
            state_type: statechronicle_domain::state_type::StateType::UniqueAsset,
            version: 7,
            last_event_id: EventId::new(event.to_owned()).unwrap(),
            last_commit_id: CommitId::new("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W".to_owned()).unwrap(),
            state_hash: canonicalize_and_digest(&ResourceState::UniqueAsset(UniqueAssetState {
                owner: SubjectId(owner.to_owned()),
                status: Status::from_static("active"),
                trade_id: None,
            }))
            .unwrap(),
            state: ResourceState::UniqueAsset(UniqueAssetState {
                owner: SubjectId(owner.to_owned()),
                status: Status::from_static("active"),
                trade_id: None,
            }),
        };
        statechronicle_index::rebuild::ProjectionSink::upsert_projection(
            &store,
            &make("account:alice", "evt_01JZ8X5HN3C4PXG5A9FGEWQF5W"),
        )
        .await
        .unwrap();
        let error = statechronicle_index::rebuild::ProjectionSink::upsert_projection(
            &store,
            &make("account:bob", "evt_01JZ8X5HN3C4PXG5A9FGEWQF6X"),
        )
        .await
        .unwrap_err();
        assert!(error.contains("conflicting projection version"));
    }

    #[tokio::test]
    async fn multi_tenant_transactions_fail_closed_without_2pc() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let result = store
            .begin_multi(&[
                TenantId(String::from("tenant-a")),
                TenantId(String::from("tenant-b")),
            ])
            .await;
        let error = match result {
            Ok(_) => panic!("cross-tenant SQLite transaction unexpectedly accepted"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("cross-tenant"));
    }

    #[tokio::test]
    async fn different_payload_is_a_deterministic_conflict() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let tenant = TenantId(String::from("game"));
        let request = intent();
        let digest = canonicalize_and_digest(&request).unwrap();
        let mut first = store.begin(&tenant).await.unwrap();
        first
            .claim_idempotency(&tenant, &request, &digest)
            .await
            .unwrap();
        let mut second = store.begin(&tenant).await.unwrap();
        let other_digest = ContentDigest::new([9u8; 32]);
        assert!(matches!(
            second
                .claim_idempotency(&tenant, &request, &other_digest)
                .await
                .unwrap(),
            IdempotencyClaim::ConflictDifferentPayload
        ));
        second.rollback().await.unwrap();
        first.rollback().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn thirty_two_concurrent_claims_have_one_winner() {
        let path = std::env::temp_dir().join(format!(
            "statechronicle-concurrency-{}.db",
            uuid::Uuid::new_v4()
        ));
        let stores: Vec<_> = (0..4)
            .map(|_| Arc::new(SqliteLedgerStore::open(&path).unwrap()))
            .collect();
        let tenant = TenantId(String::from("game"));
        let request = intent();
        let digest = canonicalize_and_digest(&request).unwrap();
        let mut tasks = Vec::new();
        for index in 0..32 {
            let store = Arc::clone(stores.get(index % stores.len()).unwrap());
            let tenant = tenant.clone();
            let request = request.clone();
            let digest = digest.clone();
            tasks.push(tokio::spawn(async move {
                let mut transaction = store.begin(&tenant).await.unwrap();
                let result = transaction
                    .claim_idempotency(&tenant, &request, &digest)
                    .await
                    .unwrap();
                (result, transaction)
            }));
        }
        let mut winners = 0;
        let mut transactions = Vec::new();
        for task in tasks {
            let (result, transaction) = task.await.unwrap();
            if matches!(result, IdempotencyClaim::NewReservation { .. }) {
                winners += 1;
            }
            transactions.push(transaction);
        }
        assert_eq!(winners, 1);
        for transaction in transactions {
            transaction.rollback().await.unwrap();
        }
        drop(stores);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn bounded_multi_tenant_claim_load_isolated() {
        let path = std::env::temp_dir().join(format!(
            "statechronicle-multi-tenant-load-{}.db",
            uuid::Uuid::new_v4()
        ));
        let stores: Vec<_> = (0..4)
            .map(|_| Arc::new(SqliteLedgerStore::open(&path).unwrap()))
            .collect();
        let mut tasks = Vec::new();
        for index in 0..64 {
            let store = Arc::clone(stores.get(index % stores.len()).unwrap());
            tasks.push(tokio::spawn(async move {
                let tenant = TenantId(format!("game-{}", index % 8));
                let mut request = intent();
                request.tenant_id = tenant.clone();
                request.intent_id = IntentId::new(format!("int_load_{index:03}")).unwrap();
                let digest = canonicalize_and_digest(&request).unwrap();
                let mut transaction = store.begin(&tenant).await.unwrap();
                let result = transaction
                    .claim_idempotency(&tenant, &request, &digest)
                    .await
                    .unwrap();
                assert!(matches!(result, IdempotencyClaim::NewReservation { .. }));
                transaction.rollback().await.unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        drop(stores);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_connection_payload_conflict_is_deterministic() {
        let path = std::env::temp_dir().join(format!(
            "statechronicle-payload-conflict-{}.db",
            uuid::Uuid::new_v4()
        ));
        let left = Arc::new(SqliteLedgerStore::open(&path).unwrap());
        let right = Arc::new(SqliteLedgerStore::open(&path).unwrap());
        let tenant = TenantId(String::from("game"));
        let request = intent();
        let first_store = Arc::clone(&left);
        let first_tenant = tenant.clone();
        let first_request = request.clone();
        let first = tokio::spawn(async move {
            let mut transaction = first_store.begin(&first_tenant).await.unwrap();
            let result = transaction
                .claim_idempotency(
                    &first_tenant,
                    &first_request,
                    &ContentDigest::new([1u8; 32]),
                )
                .await
                .unwrap();
            (result, transaction)
        });
        let second_store = Arc::clone(&right);
        let second_tenant = tenant.clone();
        let second_request = request.clone();
        let second = tokio::spawn(async move {
            let mut transaction = second_store.begin(&second_tenant).await.unwrap();
            let result = transaction
                .claim_idempotency(
                    &second_tenant,
                    &second_request,
                    &ContentDigest::new([2u8; 32]),
                )
                .await
                .unwrap();
            (result, transaction)
        });
        let (first_result, first_transaction) = first.await.unwrap();
        let (second_result, second_transaction) = second.await.unwrap();
        let results = [first_result, second_result];
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, IdempotencyClaim::NewReservation { .. }))
                .count(),
            1
        );
        first_transaction.rollback().await.unwrap();
        second_transaction.rollback().await.unwrap();
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, IdempotencyClaim::ConflictDifferentPayload))
                .count(),
            1
        );
        drop(left);
        drop(right);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn expired_reservation_can_be_taken_over_once() {
        let path = std::env::temp_dir().join(format!(
            "statechronicle-lease-takeover-{}.db",
            uuid::Uuid::new_v4()
        ));
        let first_store = SqliteLedgerStore::open(&path).unwrap();
        let second_store = SqliteLedgerStore::open(&path).unwrap();
        let tenant = TenantId(String::from("game"));
        let request = intent();
        let digest = canonicalize_and_digest(&request).unwrap();
        let mut first = first_store.begin(&tenant).await.unwrap();
        let original_attempt = match first
            .claim_idempotency(&tenant, &request, &digest)
            .await
            .unwrap()
        {
            IdempotencyClaim::NewReservation { attempt_id } => attempt_id,
            other => panic!("unexpected claim: {other:?}"),
        };
        let mut second = second_store.begin(&tenant).await.unwrap();
        assert!(matches!(
            second
                .claim_idempotency(&tenant, &request, &digest)
                .await
                .unwrap(),
            IdempotencyClaim::InProgress { .. }
        ));
        {
            let connection = first_store.connection.lock().unwrap();
            connection
                .execute(
                    "UPDATE idempotency SET lease_expires=0
                     WHERE tenant_id='game' AND intent_id=?1",
                    params![request.intent_id.0],
                )
                .unwrap();
        }
        let replacement = match second
            .claim_idempotency(&tenant, &request, &digest)
            .await
            .unwrap()
        {
            IdempotencyClaim::NewReservation { attempt_id } => attempt_id,
            other => panic!("unexpected takeover result: {other:?}"),
        };
        assert_ne!(replacement, original_attempt);
        first.rollback().await.unwrap();
        second.rollback().await.unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn dropped_transaction_leaves_no_partial_ledger_rows() {
        let path = std::env::temp_dir().join(format!(
            "statechronicle-crash-before-commit-{}.db",
            uuid::Uuid::new_v4()
        ));
        let store = SqliteLedgerStore::open(&path).unwrap();
        let tenant = TenantId(String::from("game"));
        let request = intent();
        let digest = canonicalize_and_digest(&request).unwrap();
        let mut transaction = store.begin(&tenant).await.unwrap();
        let attempt = match transaction
            .claim_idempotency(&tenant, &request, &digest)
            .await
            .unwrap()
        {
            IdempotencyClaim::NewReservation { attempt_id } => attempt_id,
            other => panic!("unexpected claim: {other:?}"),
        };
        let owner = SubjectId(String::from("account:alice"));
        let state = ResourceState::UniqueAsset(UniqueAssetState {
            owner: owner.clone(),
            status: Status::from_static("active"),
            trade_id: None,
        });
        let state_hash = canonicalize_and_digest(&state).unwrap();
        let event = Event::new(
            tenant.clone(),
            EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            request.intent_id.clone(),
            request.operation.clone(),
            request.resource_id.clone(),
            owner,
            StateCommitment {
                version: 0,
                state_hash: state_hash.clone(),
                state: state.clone(),
            },
            StateCommitment {
                version: 1,
                state_hash: state_hash.clone(),
                state,
            },
            None,
            SubjectId(String::from("service:ledger")),
            chrono::Utc::now(),
        );
        transaction
            .append_events(std::slice::from_ref(&event))
            .await
            .unwrap();
        transaction
            .upsert_projection(&StateProjection {
                tenant_id: tenant.clone(),
                resource_id: request.resource_id.clone(),
                state_type: statechronicle_domain::state_type::StateType::UniqueAsset,
                version: 1,
                last_event_id: event.event_id.clone(),
                last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W"))
                    .unwrap(),
                state_hash,
                state: event.after.state.clone(),
            })
            .await
            .unwrap();
        transaction
            .enqueue_outbox(&OutboxRecord {
                delivery_key: String::from("commit:game:crash-test"),
                tenant: tenant.clone(),
                commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
                payload_digest: hash_bytes(b"payload"),
                payload: b"payload".to_vec(),
            })
            .await
            .unwrap();
        drop(transaction);
        {
            let connection = store.connection.lock().unwrap();
            connection
                .execute(
                    "UPDATE idempotency SET lease_expires=0 WHERE tenant_id='game' AND intent_id=?1",
                    params![request.intent_id.0],
                )
                .unwrap();
        }
        let mut takeover = store.begin(&tenant).await.unwrap();
        assert!(matches!(
            takeover
                .claim_idempotency(&tenant, &request, &digest)
                .await
                .unwrap(),
            IdempotencyClaim::NewReservation { .. }
        ));
        takeover.rollback().await.unwrap();
        assert_eq!(
            store
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM commits", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM projections", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM outbox", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM events", [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
        assert!(!attempt.is_empty());
        drop(store);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn commit_requires_finalization_and_replays_after_commit() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let tenant = TenantId(String::from("game"));
        let request = intent();
        let digest = canonicalize_and_digest(&request).unwrap();
        let owner = SubjectId(String::from("account:alice"));
        let state = ResourceState::UniqueAsset(UniqueAssetState {
            owner: owner.clone(),
            status: Status::from_static("active"),
            trade_id: None,
        });
        let state_digest = canonicalize_and_digest(&state).unwrap();
        let event = Event::new(
            tenant.clone(),
            EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            request.intent_id.clone(),
            request.operation.clone(),
            request.resource_id.clone(),
            owner.clone(),
            StateCommitment {
                version: 0,
                state_hash: state_digest.clone(),
                state: state.clone(),
            },
            StateCommitment {
                version: 1,
                state_hash: state_digest.clone(),
                state: state.clone(),
            },
            None,
            SubjectId(String::from("service:ledger")),
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );
        let commit_id = CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap();
        let body = Commit::new(
            CommitScope::tenant(tenant.clone()),
            commit_id.clone(),
            None,
            1,
            1,
            event_root(std::slice::from_ref(&event)).unwrap(),
            hash_bytes(b"previous"),
            hash_bytes(b"next"),
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:01Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            SubjectId(String::from("service:ledger")),
            ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
        );
        let signed = Signed::new(
            body,
            SignatureBlock {
                alg: SignatureAlg::Ed25519,
                key_id: KeyId::new(String::from("did:key:z6Mk...#ledger")).unwrap(),
                sig: statechronicle_core::signature::Signature::from_bytes([0u8; 64]),
            },
        );
        let projection = StateProjection {
            tenant_id: tenant.clone(),
            resource_id: request.resource_id.clone(),
            state_type: statechronicle_domain::state_type::StateType::UniqueAsset,
            version: 1,
            last_event_id: event.event_id.clone(),
            last_commit_id: commit_id.clone(),
            state_hash: state_digest,
            state,
        };
        let outbox = OutboxRecord {
            delivery_key: String::from("commit:game:cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W"),
            tenant: tenant.clone(),
            commit_id: commit_id.clone(),
            payload_digest: hash_bytes(b"payload"),
            payload: b"payload".to_vec(),
        };
        let mut transaction = store.begin(&tenant).await.unwrap();
        let claim = transaction
            .claim_idempotency(&tenant, &request, &digest)
            .await
            .unwrap();
        let attempt = match claim {
            IdempotencyClaim::NewReservation { attempt_id } => attempt_id,
            _ => panic!("unexpected claim"),
        };
        let mut wrong_scope_event = event.clone();
        wrong_scope_event.tenant_id = TenantId(String::from("other-game"));
        assert!(matches!(
            transaction.append_events(&[wrong_scope_event]).await,
            Err(LedgerStoreError::Invariant(message)) if message.contains("scope")
        ));
        let mut wrong_actor_event = event.clone();
        wrong_actor_event.actor = SubjectId(String::from("account:mallory"));
        assert!(matches!(
            transaction.append_events(&[wrong_actor_event]).await,
            Err(LedgerStoreError::Invariant(message)) if message.contains("claimed intent")
        ));
        let mut wrong_operation_event = event.clone();
        wrong_operation_event.operation = Operation::from_static("asset.burn");
        assert!(matches!(
            transaction.append_events(&[wrong_operation_event]).await,
            Err(LedgerStoreError::Invariant(message)) if message.contains("claimed intent")
        ));
        transaction.append_events(&[event]).await.unwrap();
        transaction.append_commit(&signed).await.unwrap();
        transaction.upsert_projection(&projection).await.unwrap();
        let oversized_payload = vec![0u8; MAX_OUTBOX_PAYLOAD_BYTES + 1];
        let oversized_outbox = OutboxRecord {
            delivery_key: String::from("commit:game:oversized"),
            tenant: tenant.clone(),
            commit_id: commit_id.clone(),
            payload_digest: hash_bytes(&oversized_payload),
            payload: oversized_payload,
        };
        assert!(transaction.enqueue_outbox(&oversized_outbox).await.is_err());
        let mut corrupt_outbox = outbox.clone();
        corrupt_outbox.payload_digest = hash_bytes(b"wrong");
        assert!(matches!(
            transaction.enqueue_outbox(&corrupt_outbox).await,
            Err(LedgerStoreError::Invariant(message)) if message.contains("digest")
        ));
        transaction.enqueue_outbox(&outbox).await.unwrap();
        let wrong_commit = CommitId::new(String::from("cmt_unrelated")).unwrap();
        assert!(
            transaction
                .finalize_idempotency(&tenant, &request.intent_id, &attempt, &wrong_commit)
                .await
                .is_err()
        );
        transaction
            .finalize_idempotency(&tenant, &request.intent_id, &attempt, &commit_id)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let report = store.verify_integrity(&tenant).unwrap();
        assert_eq!(report.commit_count, 1);
        assert_eq!(report.event_count, 1);
        assert_eq!(report.head_commit_id, Some(commit_id.clone()));
        let head = store.canonical_head(&tenant).unwrap().unwrap();
        assert_eq!(head.commit_id, commit_id);
        assert_eq!(head.sequence, 1);
        assert_eq!(head.state_root, signed.body.next_state_root);

        let mut replay = store.begin(&tenant).await.unwrap();
        assert!(matches!(
            replay.claim_idempotency(&tenant, &request, &digest).await.unwrap(),
            IdempotencyClaim::Committed { commit_id: found } if found == commit_id
        ));
        replay.rollback().await.unwrap();
        assert_eq!(
            OutboxStore::pending_count(&store, Some(&tenant))
                .await
                .unwrap(),
            1
        );
        let failed = dispatch_once(
            &store,
            &TestPublisher { fail: true },
            "worker-1",
            10,
            chrono::Utc::now().timestamp().saturating_add(60),
        )
        .await
        .unwrap();
        assert_eq!(failed.retried, 1);
        assert_eq!(failed.delivered, 0);
        let last_error: String = store
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT last_error FROM outbox WHERE delivery_key=?1",
                params!["commit:game:cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(last_error, "broker unavailable");
        let delivered = dispatch_until_idle(
            &store,
            &TestPublisher { fail: false },
            "worker-1",
            10,
            chrono::Utc::now().timestamp().saturating_add(60),
            3,
        )
        .await
        .unwrap();
        assert_eq!(delivered.delivered, 1);
        assert_eq!(
            OutboxStore::pending_count(&store, Some(&tenant))
                .await
                .unwrap(),
            0
        );
        let canonical = store.canonical_events(&tenant).unwrap();
        assert_eq!(canonical.len(), 1);
        {
            let connection = store.connection.lock().unwrap();
            connection
                .execute(
                    "DELETE FROM projections WHERE tenant_id='game' AND resource_id='asset:sword'",
                    [],
                )
                .unwrap();
        }
        let rebuilt = store
            .rebuild_projections_from_canonical(&tenant, "rebuild-v1", 1)
            .await
            .unwrap();
        assert_eq!(rebuilt, 1);
        assert!(
            statechronicle_ports::state_index::StateIndex::get_state(
                &store,
                &tenant,
                &request.resource_id,
            )
            .await
            .unwrap()
            .is_some()
        );
        let connection = store.connection.lock().unwrap();
        connection
            .execute(
                "UPDATE commits SET payload=X'00' WHERE tenant_id='game'",
                [],
            )
            .unwrap();
        drop(connection);
        assert!(store.verify_integrity(&tenant).is_err());
        let restored_connection = store.connection.lock().unwrap();
        let signed_payload = bcs::to_bytes(&signed).unwrap();
        restored_connection
            .execute(
                "UPDATE commits SET payload=?1 WHERE tenant_id='game'",
                params![signed_payload],
            )
            .unwrap();
        restored_connection
            .execute("UPDATE events SET payload=X'00' WHERE tenant_id='game'", [])
            .unwrap();
        drop(restored_connection);
        assert!(store.verify_integrity(&tenant).is_err());
    }

    #[tokio::test]
    async fn consumer_dedup_applies_delivery_once_and_allows_retry() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        let tenant = TenantId(String::from("game"));
        let commit_id = CommitId::new(String::from("cmt_delivery")).unwrap();
        let payload_bytes = b"notification".to_vec();
        let payload = OutboxPayload::Opaque {
            tenant,
            commit_id,
            payload_digest: hash_bytes(&payload_bytes),
            payload: payload_bytes,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let consumer = CountingConsumer {
            calls: Arc::clone(&calls),
            fail: false,
        };
        assert!(
            consume_once(
                &store,
                &consumer,
                "delivery-1",
                &payload,
                chrono::Utc::now().timestamp().saturating_add(60),
            )
            .await
            .unwrap()
        );
        assert!(
            !consume_once(
                &store,
                &consumer,
                "delivery-1",
                &payload,
                chrono::Utc::now().timestamp().saturating_add(60),
            )
            .await
            .unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let retry_calls = Arc::new(AtomicUsize::new(0));
        let failing = CountingConsumer {
            calls: Arc::clone(&retry_calls),
            fail: true,
        };
        assert!(
            consume_once(
                &store,
                &failing,
                "delivery-2",
                &payload,
                chrono::Utc::now().timestamp().saturating_add(60),
            )
            .await
            .is_err()
        );
        let succeeding = CountingConsumer {
            calls: Arc::clone(&retry_calls),
            fail: false,
        };
        assert!(
            consume_once(
                &store,
                &succeeding,
                "delivery-2",
                &payload,
                chrono::Utc::now().timestamp().saturating_add(60),
            )
            .await
            .unwrap()
        );
        assert_eq!(retry_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn poison_outbox_payload_can_be_quarantined() {
        let store = SqliteLedgerStore::in_memory().unwrap();
        {
            let connection = store.connection.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO outbox
                     (delivery_key, tenant_id, commit_id, payload_digest, payload)
                     VALUES ('poison-1', 'game', 'cmt_missing', ?1, ?2)",
                    params![hash_bytes(b"expected").as_bytes(), b"corrupt"],
                )
                .unwrap();
        }
        let report = dispatch_once_with_policy(
            &store,
            &TestPublisher { fail: false },
            "worker-poison",
            10,
            chrono::Utc::now().timestamp().saturating_add(60),
            PoisonPolicy::Quarantine,
        )
        .await
        .unwrap();
        assert_eq!(report.quarantined, 1);
        assert_eq!(report.delivered, 0);
        assert_eq!(
            OutboxStore::pending_count(&store, Some(&TenantId(String::from("game"))))
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    #[allow(deprecated)]
    async fn stale_outbox_worker_cannot_complete_taken_over_lease() {
        let path = std::env::temp_dir().join(format!(
            "statechronicle-outbox-lease-{}.db",
            uuid::Uuid::new_v4()
        ));
        let first = SqliteLedgerStore::open(&path).unwrap();
        let second = SqliteLedgerStore::open(&path).unwrap();
        let payload = b"lease-test".to_vec();
        {
            let connection = first.connection.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO outbox
                     (delivery_key, tenant_id, commit_id, payload_digest, payload)
                     VALUES ('lease-1', 'game', 'cmt_missing', ?1, ?2)",
                    params![hash_bytes(&payload).as_bytes(), payload],
                )
                .unwrap();
        }
        let now = chrono::Utc::now().timestamp();
        let first_claim = first
            .claim("worker-a", 1, now.saturating_add(60))
            .await
            .unwrap();
        assert_eq!(first_claim.len(), 1);
        assert!(first.mark_delivered("lease-1").await.is_err());
        assert!(first.release("lease-1", "ownerless").await.is_err());
        assert!(first.quarantine("lease-1", "ownerless").await.is_err());
        {
            let connection = first.connection.lock().unwrap();
            connection
                .execute(
                    "UPDATE outbox SET lease_until=0 WHERE delivery_key='lease-1'",
                    [],
                )
                .unwrap();
        }
        let second_claim = second
            .claim("worker-b", 1, now.saturating_add(60))
            .await
            .unwrap();
        assert_eq!(second_claim.len(), 1);
        assert!(
            first
                .release_by("lease-1", "worker-a", "stale")
                .await
                .is_err()
        );
        assert!(
            second
                .mark_delivered_by("lease-1", "worker-a")
                .await
                .is_err()
        );
        second
            .mark_delivered_by("lease-1", "worker-b")
            .await
            .unwrap();
        assert_eq!(second.pending_count(None).await.unwrap(), 0);
        drop(first);
        drop(second);
        std::fs::remove_file(path).unwrap();
    }
}
