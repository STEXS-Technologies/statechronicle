//! PostgreSQL adapter for the durable ledger contract.
//!
//! The adapter uses one PostgreSQL transaction per [`LedgerTransaction`],
//! starts it at `SERIALIZABLE`, and stages events until their owning commit is
//! inserted. Apply `docs/OPERATIONS/postgres_schema.sql` with the deployment's
//! migration tool before opening the store. Connection pooling belongs at the
//! composition root; each begin obtains a dedicated connection so transaction
//! ownership cannot accidentally outlive a pooled lease.

#![deny(unsafe_code)]

use std::collections::BTreeMap;

use async_trait::async_trait;
use statechronicle_commit::persist::SignedCommitVerifier;
use statechronicle_commit::roots::event_root;
use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::{ContentDigest, hash_bytes};
use statechronicle_core::limits::{
    MAX_COMMIT_BYTES, MAX_EVENT_BATCH_BYTES, MAX_EVENT_BYTES, MAX_EVENTS_PER_COMMIT,
    MAX_OUTBOX_CLAIM, MAX_OUTBOX_PAYLOAD_BYTES, check_size,
};
use statechronicle_domain::commit::{Commit, ScopeKind};
use statechronicle_domain::event::Event;
use statechronicle_domain::ids::{CommitId, IntentId};
use statechronicle_domain::intent::{Intent, Operation};
use statechronicle_domain::signed::Signed;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_ports::commit_store::{CanonicalHead, CommitStore, CommitStoreError};
use statechronicle_ports::ledger_store::{
    IdempotencyClaim, LedgerStore, LedgerStoreError, LedgerTransaction, OutboxRecord,
};
use statechronicle_ports::outbox::{
    ConsumerDedupStore, ConsumerDeliveryClaim, OutboxError, OutboxPayload, OutboxStore,
};
use tokio_postgres::{Client, NoTls, Row};

#[cfg(feature = "tls")]
use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};

/// Schema version understood by this adapter.
pub const SUPPORTED_SCHEMA_VERSION: i32 = 1;
const DEFAULT_LOCK_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_STATEMENT_TIMEOUT_MS: u64 = 30_000;

/// PostgreSQL durable ledger store.
#[derive(Clone, Debug)]
pub struct PostgresLedgerStore {
    connection_string: String,
    security: ConnectionSecurity,
    lock_timeout_ms: u64,
    statement_timeout_ms: u64,
}

#[derive(Clone, Debug)]
enum ConnectionSecurity {
    Plain,
    #[cfg(feature = "tls")]
    Tls(SslConnector),
}

impl PostgresLedgerStore {
    /// Creates a store from a libpq-compatible connection string.
    pub fn new(connection_string: impl Into<String>) -> Self {
        Self {
            connection_string: connection_string.into(),
            security: ConnectionSecurity::Plain,
            lock_timeout_ms: DEFAULT_LOCK_TIMEOUT_MS,
            statement_timeout_ms: DEFAULT_STATEMENT_TIMEOUT_MS,
        }
    }

    /// Creates a PostgreSQL store and verifies every discovered tenant before
    /// returning it. This is the fail-closed startup path for services that
    /// accept player mutations.
    ///
    /// # Errors
    ///
    /// Returns a connection, schema, or integrity error when startup checks
    /// cannot complete successfully.
    pub async fn new_verified(
        connection_string: impl Into<String>,
    ) -> Result<Self, LedgerStoreError> {
        let store = Self::new(connection_string);
        verify_startup_with_retries(&store).await?;
        Ok(store)
    }

    /// Creates a TLS PostgreSQL store and verifies every discovered tenant
    /// before returning it.
    ///
    /// # Errors
    ///
    /// Returns a TLS configuration, connection, schema, or integrity error
    /// when startup checks cannot complete successfully.
    #[cfg(feature = "tls")]
    pub async fn new_with_tls_verified(
        connection_string: impl Into<String>,
        ca_file: impl AsRef<std::path::Path>,
    ) -> Result<Self, LedgerStoreError> {
        let store = Self::new_with_tls(connection_string, ca_file)
            .map_err(LedgerStoreError::Unavailable)?;
        verify_startup_with_retries(&store).await?;
        Ok(store)
    }

    /// Configures bounded PostgreSQL lock and statement waits.
    ///
    /// A zero timeout is rejected because it would turn every lock attempt
    /// into an immediate failure. The values apply with `SET LOCAL` to each
    /// ledger/outbox transaction and do not alter database-wide settings.
    ///
    /// # Errors
    ///
    /// Returns an error when either timeout is zero.
    pub fn with_timeouts(
        mut self,
        lock_timeout_ms: u64,
        statement_timeout_ms: u64,
    ) -> Result<Self, String> {
        if lock_timeout_ms == 0 || statement_timeout_ms == 0 {
            return Err(String::from("PostgreSQL timeouts must be non-zero"));
        }
        self.lock_timeout_ms = lock_timeout_ms;
        self.statement_timeout_ms = statement_timeout_ms;
        Ok(self)
    }

    /// Creates a store using TLS with peer verification against `ca_file`.
    ///
    /// The connection string should include `sslmode=require` (or a stricter
    /// mode). The CA file must be provisioned as deployment configuration and
    /// is never read from untrusted request data.
    ///
    /// # Errors
    ///
    /// Returns an error when OpenSSL cannot initialize or load the CA file.
    #[cfg(feature = "tls")]
    pub fn new_with_tls(
        connection_string: impl Into<String>,
        ca_file: impl AsRef<std::path::Path>,
    ) -> Result<Self, String> {
        let mut builder = SslConnector::builder(SslMethod::tls())
            .map_err(|error| format!("TLS connector initialization failed: {error}"))?;
        builder
            .set_ca_file(ca_file)
            .map_err(|error| format!("TLS CA configuration failed: {error}"))?;
        builder.set_verify(SslVerifyMode::PEER);
        Ok(Self {
            connection_string: connection_string.into(),
            security: ConnectionSecurity::Tls(builder.build()),
            lock_timeout_ms: DEFAULT_LOCK_TIMEOUT_MS,
            statement_timeout_ms: DEFAULT_STATEMENT_TIMEOUT_MS,
        })
    }

    /// Verifies a tenant's committed history and canonical head.
    ///
    /// This is intended for startup and restore gates. It fails closed on
    /// malformed BCS rows, scope mismatches, sequence/parent/root gaps,
    /// event-count or Merkle-root mismatches, orphan/indexed events, and head
    /// divergence.
    ///
    /// # Errors
    ///
    /// Returns [`LedgerStoreError::Unavailable`] when PostgreSQL cannot be
    /// queried and [`LedgerStoreError::Invariant`] when persisted history is
    /// malformed or inconsistent.
    pub async fn verify_integrity(&self, tenant: &TenantId) -> Result<(), LedgerStoreError> {
        self.verify_integrity_with(tenant, None).await
    }

    /// Verifies a tenant history and every detached commit signature using a
    /// deployment-provided key resolver/verifier.
    ///
    /// # Errors
    ///
    /// Returns the same integrity errors as [`Self::verify_integrity`], plus a
    /// cryptographic trust error when any commit signature is invalid,
    /// revoked, expired, or unknown to the supplied verifier.
    pub async fn verify_integrity_with_verifier(
        &self,
        tenant: &TenantId,
        verifier: &dyn SignedCommitVerifier,
    ) -> Result<(), LedgerStoreError> {
        self.verify_integrity_with(tenant, Some(verifier)).await
    }

    async fn verify_integrity_with(
        &self,
        tenant: &TenantId,
        verifier: Option<&dyn SignedCommitVerifier>,
    ) -> Result<(), LedgerStoreError> {
        validate_tenant(tenant)?;
        let client = self.connect().await?;
        self.check_schema(&client).await?;
        let rows = client
            .query(
                "SELECT commit_id,sequence,payload FROM sc_commits WHERE tenant_id=$1 ORDER BY sequence",
                &[&tenant.0],
            )
            .await
            .map_err(unavailable)?;
        let mut previous: Option<Signed<Commit>> = None;
        for row in rows {
            let indexed_id: String = row.get(0);
            let payload: Vec<u8> = row.get(2);
            let signed: Signed<Commit> = bcs::from_bytes(&payload)
                .map_err(|error| invalid(format!("invalid commit payload: {error}")))?;
            if let Some(verifier) = verifier {
                verifier.verify(&signed).map_err(|error| {
                    invalid(format!("commit signature verification failed: {error}"))
                })?;
            }
            let Some(scope_tenant) = signed.body.scope.tenant_id.as_ref() else {
                return Err(invalid(String::from("tenant commit has no tenant scope")));
            };
            if signed.body.scope.kind != ScopeKind::Tenant
                || scope_tenant != tenant
                || signed.body.commit_id.0 != indexed_id
            {
                return Err(invalid(String::from(
                    "commit payload does not match tenant index",
                )));
            }
            if let Some(prior) = previous.as_ref() {
                if signed.body.parent_commit_id.as_ref() != Some(&prior.body.commit_id)
                    || signed.body.sequence != prior.body.sequence.saturating_add(1)
                    || signed.body.previous_state_root != prior.body.next_state_root
                {
                    return Err(invalid(String::from("commit chain continuity mismatch")));
                }
            } else if signed.body.parent_commit_id.is_some() || signed.body.sequence != 1 {
                return Err(invalid(String::from("invalid genesis commit")));
            }
            let event_rows = client
                .query(
                    "SELECT event_id,event_index,payload FROM sc_events WHERE tenant_id=$1 AND commit_id=$2 ORDER BY event_index",
                    &[&tenant.0, &signed.body.commit_id.0],
                )
                .await
                .map_err(unavailable)?;
            let mut events = Vec::with_capacity(event_rows.len());
            for (expected_index, event_row) in event_rows.into_iter().enumerate() {
                let event_id: String = event_row.get(0);
                let indexed_index: i32 = event_row.get(1);
                if usize::try_from(indexed_index).ok() != Some(expected_index) {
                    return Err(invalid(String::from(
                        "event index is non-contiguous or duplicated",
                    )));
                }
                let event: Event = bcs::from_bytes(&event_row.get::<_, Vec<u8>>(2))
                    .map_err(|error| invalid(format!("invalid event payload: {error}")))?;
                if event.event_id.0 != event_id || event.tenant_id != *tenant {
                    return Err(invalid(String::from("event payload does not match index")));
                }
                events.push(event);
            }
            let event_count = u64::try_from(events.len())
                .map_err(|error| invalid(format!("event count overflow: {error}")))?;
            if event_count != signed.body.event_count
                || event_root(&events).map_err(|error| invalid(error.to_string()))?
                    != signed.body.event_merkle_root
            {
                return Err(invalid(format!(
                    "event root/count mismatch for commit `{}`",
                    signed.body.commit_id.0
                )));
            }
            previous = Some(signed);
        }
        let orphan = client
            .query_opt(
                "SELECT e.event_id FROM sc_events e LEFT JOIN sc_commits c
                 ON c.tenant_id=e.tenant_id AND c.commit_id=e.commit_id
                 WHERE e.tenant_id=$1 AND c.commit_id IS NULL LIMIT 1",
                &[&tenant.0],
            )
            .await
            .map_err(unavailable)?;
        if orphan.is_some() {
            return Err(invalid(String::from(
                "event references a missing tenant commit",
            )));
        }
        let outbox_rows = client
            .query(
                "SELECT delivery_key,payload_digest,payload FROM sc_outbox WHERE tenant_id=$1",
                &[&tenant.0],
            )
            .await
            .map_err(unavailable)?;
        for row in outbox_rows {
            let delivery_key: String = row.get(0);
            let digest: Vec<u8> = row.get(1);
            let payload: Vec<u8> = row.get(2);
            if digest.len() != 32 || hash_bytes(&payload).as_bytes() != digest.as_slice() {
                return Err(invalid(format!(
                    "outbox payload digest mismatch for `{delivery_key}`"
                )));
            }
        }
        let orphan_outbox = client
            .query_opt(
                "SELECT o.delivery_key FROM sc_outbox o LEFT JOIN sc_commits c
                 ON c.tenant_id=o.tenant_id AND c.commit_id=o.commit_id
                 WHERE o.tenant_id=$1 AND c.commit_id IS NULL LIMIT 1",
                &[&tenant.0],
            )
            .await
            .map_err(unavailable)?;
        if orphan_outbox.is_some() {
            return Err(invalid(String::from(
                "outbox row references a missing tenant commit",
            )));
        }
        let idempotency_rows = client
            .query(
                "SELECT i.intent_id,i.payload_digest,i.status,i.commit_id,i.intent_payload
                 FROM sc_idempotency i WHERE i.tenant_id=$1",
                &[&tenant.0],
            )
            .await
            .map_err(unavailable)?;
        for row in idempotency_rows {
            let intent_id: String = row.get(0);
            let digest: Vec<u8> = row.get(1);
            let status: String = row.get(2);
            let commit_id: Option<String> = row.get(3);
            let payload: Option<Vec<u8>> = row.get(4);
            if digest.len() != 32 {
                return Err(invalid(format!(
                    "idempotency digest has invalid length for `{intent_id}`"
                )));
            }
            if status == "committed" {
                let Some(commit_id) = commit_id else {
                    return Err(invalid(format!(
                        "committed idempotency row has no commit for `{intent_id}`"
                    )));
                };
                let exists = client
                    .query_opt(
                        "SELECT 1 FROM sc_commits WHERE tenant_id=$1 AND commit_id=$2",
                        &[&tenant.0, &commit_id],
                    )
                    .await
                    .map_err(unavailable)?;
                if exists.is_none() {
                    return Err(invalid(format!(
                        "idempotency row references missing commit `{commit_id}`"
                    )));
                }
            }
            let Some(payload) = payload else {
                return Err(invalid(format!(
                    "idempotency row has no canonical intent payload for `{intent_id}`"
                )));
            };
            let intent: Intent = bcs::from_bytes(&payload)
                .map_err(|error| invalid(format!("invalid idempotency intent: {error}")))?;
            if intent.tenant_id != *tenant || intent.intent_id.0 != intent_id {
                return Err(invalid(String::from(
                    "idempotency intent payload does not match index",
                )));
            }
            if canonicalize_and_digest(&intent)
                .map_err(|error| invalid(error.to_string()))?
                .as_bytes()
                != digest.as_slice()
            {
                return Err(invalid(format!(
                    "idempotency payload digest mismatch for `{intent_id}`"
                )));
            }
        }
        let projection_rows = client
            .query(
                "SELECT resource_id,payload FROM sc_projections WHERE tenant_id=$1",
                &[&tenant.0],
            )
            .await
            .map_err(unavailable)?;
        for row in projection_rows {
            let resource_id: String = row.get(0);
            let projection: StateProjection = bcs::from_bytes(&row.get::<_, Vec<u8>>(1))
                .map_err(|error| invalid(format!("invalid projection payload: {error}")))?;
            if projection.tenant_id != *tenant || projection.resource_id.0 != resource_id {
                return Err(invalid(String::from(
                    "projection payload does not match index",
                )));
            }
        }
        let head = client
            .query_opt(
                "SELECT commit_id,sequence,state_root FROM sc_heads WHERE tenant_id=$1",
                &[&tenant.0],
            )
            .await
            .map_err(unavailable)?;
        match (previous, head) {
            (None, None) => Ok(()),
            (Some(commit), Some(head)) => {
                let head_id: String = head.get(0);
                let head_sequence: i64 = head.get(1);
                let head_root: Vec<u8> = head.get(2);
                if head_id != commit.body.commit_id.0
                    || u64::try_from(head_sequence).ok() != Some(commit.body.sequence)
                    || head_root.as_slice() != commit.body.next_state_root.as_bytes()
                {
                    return Err(invalid(String::from(
                        "canonical head diverges from commit chain",
                    )));
                }
                Ok(())
            }
            _ => Err(invalid(String::from(
                "canonical head exists without commits",
            ))),
        }
    }

    /// Enumerates every tenant represented by durable tables and verifies each
    /// history. This is the preferred startup/restore gate when the caller
    /// cannot safely maintain a separate tenant allow-list.
    ///
    /// # Errors
    ///
    /// Returns the first database, malformed-scope, or integrity error found.
    pub async fn verify_all_integrity(&self) -> Result<Vec<TenantId>, LedgerStoreError> {
        self.verify_all_integrity_with(None).await
    }

    /// Enumerates every tenant and verifies integrity plus detached commit
    /// signatures with a deployment-provided verifier.
    ///
    /// # Errors
    ///
    /// Returns the first database, integrity, or signature trust error.
    pub async fn verify_all_integrity_with_verifier(
        &self,
        verifier: &dyn SignedCommitVerifier,
    ) -> Result<Vec<TenantId>, LedgerStoreError> {
        self.verify_all_integrity_with(Some(verifier)).await
    }

    async fn verify_all_integrity_with(
        &self,
        verifier: Option<&dyn SignedCommitVerifier>,
    ) -> Result<Vec<TenantId>, LedgerStoreError> {
        let client = self.connect().await?;
        self.check_schema(&client).await?;
        let rows = client
            .query(
                "SELECT tenant_id FROM sc_heads
                 UNION SELECT tenant_id FROM sc_commits
                 UNION SELECT tenant_id FROM sc_events
                 UNION SELECT tenant_id FROM sc_projections
                 UNION SELECT tenant_id FROM sc_outbox
                 UNION SELECT tenant_id FROM sc_idempotency
                 ORDER BY tenant_id",
                &[],
            )
            .await
            .map_err(unavailable)?;
        let mut tenants = Vec::with_capacity(rows.len());
        for row in rows {
            let value: String = row.get(0);
            let tenant = TenantId(value);
            validate_tenant(&tenant)?;
            self.verify_integrity_with(&tenant, verifier).await?;
            tenants.push(tenant);
        }
        Ok(tenants)
    }

    /// Loads the verified canonical event stream for one tenant in commit and
    /// event-index order. Callers can feed this stream to
    /// `statechronicle_index::rebuild_projections` after clearing a derived
    /// projection store.
    ///
    /// # Errors
    ///
    /// Returns an integrity error before returning any events when the tenant
    /// history is malformed or diverges from its canonical head.
    pub async fn canonical_events(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<(Event, CommitId)>, LedgerStoreError> {
        self.verify_integrity(tenant).await?;
        let client = self.connect().await?;
        self.check_schema(&client).await?;
        self.canonical_events_from_client(&client, tenant).await
    }

    async fn canonical_events_from_client(
        &self,
        client: &Client,
        tenant: &TenantId,
    ) -> Result<Vec<(Event, CommitId)>, LedgerStoreError> {
        let rows = client
            .query(
                "SELECT e.payload,e.commit_id FROM sc_events e
                 JOIN sc_commits c ON c.tenant_id=e.tenant_id AND c.commit_id=e.commit_id
                 WHERE e.tenant_id=$1 ORDER BY c.sequence,e.event_index",
                &[&tenant.0],
            )
            .await
            .map_err(unavailable)?;
        rows.into_iter()
            .map(|row| {
                let event: Event = bcs::from_bytes(&row.get::<_, Vec<u8>>(0)).map_err(|error| {
                    invalid(format!("invalid canonical event payload: {error}"))
                })?;
                let commit_id = CommitId::new(row.get::<_, String>(1))
                    .map_err(|error| invalid(error.to_string()))?;
                Ok((event, commit_id))
            })
            .collect()
    }

    /// Rebuilds tenant projections from the verified canonical event stream.
    ///
    /// When `clear_existing` is true, existing projections for the tenant are
    /// removed before monotonic upserts. The canonical head is locked for the
    /// rebuild transaction, so a concurrent ledger commit cannot create a
    /// partially rebuilt view.
    ///
    /// # Errors
    ///
    /// Returns an integrity, serialization, database, or equal-version
    /// projection conflict error. The transaction rolls back on every error.
    #[allow(clippy::collapsible_if)]
    pub async fn rebuild_projections_from_canonical(
        &self,
        tenant: &TenantId,
        clear_existing: bool,
    ) -> Result<u64, LedgerStoreError> {
        // Verify before opening the rebuild transaction, then read the stream
        // again on the head-locked connection below. Any commit racing before
        // the lock is included in that locked snapshot; no second connection
        // can observe a stream outside the transaction being promoted.
        self.verify_integrity(tenant).await?;
        let client = self.connect().await?;
        self.check_schema(&client).await?;
        client
            .batch_execute("BEGIN ISOLATION LEVEL SERIALIZABLE")
            .await
            .map_err(unavailable)?;
        configure_transaction(&client, self.lock_timeout_ms, self.statement_timeout_ms).await?;
        client
            .query_opt(
                "SELECT commit_id FROM sc_heads WHERE tenant_id=$1 FOR UPDATE",
                &[&tenant.0],
            )
            .await
            .map_err(map_db)?;
        let result = async {
            // Read the canonical stream only after locking the head. This
            // prevents a commit that races with rebuild from being appended
            // between stream verification and projection replacement.
            let events = self.canonical_events_from_client(&client, tenant).await?;
            let mut latest: BTreeMap<String, StateProjection> = BTreeMap::new();
            for (event, commit_id) in events {
                let projection = StateProjection {
                    tenant_id: event.tenant_id.clone(),
                    resource_id: event.resource_id.clone(),
                    state_type: event.after.state.state_type(),
                    version: event.after.version,
                    last_event_id: event.event_id,
                    last_commit_id: commit_id,
                    state_hash: event.after.state_hash,
                    state: event.after.state,
                };
                let key = projection.resource_id.0.clone();
                if let Some(existing) = latest.get(&key) {
                    if projection.version < existing.version {
                        continue;
                    }
                    if projection.version == existing.version
                        && (projection.state_hash != existing.state_hash
                            || projection.last_event_id != existing.last_event_id)
                    {
                        return Err(invalid(format!(
                            "conflicting projection version {} for resource `{key}`",
                            projection.version
                        )));
                    }
                }
                latest.insert(key, projection);
            }
            if clear_existing {
                client
                    .execute(
                        "DELETE FROM sc_projections WHERE tenant_id=$1",
                        &[&tenant.0],
                    )
                    .await
                    .map_err(map_db)?;
            }
            for projection in latest.values() {
                let payload =
                    bcs::to_bytes(projection).map_err(|error| invalid(error.to_string()))?;
                let version = i64::try_from(projection.version)
                    .map_err(|error| invalid(format!("projection version overflow: {error}")))?;
                client
                    .execute(
                        "INSERT INTO sc_projections (tenant_id,resource_id,version,payload)
                         VALUES ($1,$2,$3,$4)
                         ON CONFLICT (tenant_id,resource_id) DO UPDATE
                         SET version=EXCLUDED.version,payload=EXCLUDED.payload
                         WHERE sc_projections.version < EXCLUDED.version",
                        &[&tenant.0, &projection.resource_id.0, &version, &payload],
                    )
                    .await
                    .map_err(map_db)?;
            }
            client.batch_execute("COMMIT").await.map_err(unavailable)?;
            Ok::<u64, LedgerStoreError>(latest.len() as u64)
        }
        .await;
        if result.is_err() {
            if let Err(error) = client.batch_execute("ROLLBACK").await {
                tracing::warn!(%error, "postgres projection rebuild rollback failed");
            }
        }
        result
    }

    async fn connect(&self) -> Result<Client, LedgerStoreError> {
        match &self.security {
            ConnectionSecurity::Plain => {
                let (client, connection) = tokio_postgres::connect(&self.connection_string, NoTls)
                    .await
                    .map_err(unavailable)?;
                tokio::spawn(async move {
                    if let Err(error) = connection.await {
                        tracing::error!(%error, "postgres connection terminated");
                    }
                });
                configure_session(&client, self.lock_timeout_ms, self.statement_timeout_ms).await?;
                Ok(client)
            }
            #[cfg(feature = "tls")]
            ConnectionSecurity::Tls(connector) => {
                let connector = postgres_openssl::MakeTlsConnector::new(connector.clone());
                let (client, connection) =
                    tokio_postgres::connect(&self.connection_string, connector)
                        .await
                        .map_err(unavailable)?;
                tokio::spawn(async move {
                    if let Err(error) = connection.await {
                        tracing::error!(%error, "postgres TLS connection terminated");
                    }
                });
                configure_session(&client, self.lock_timeout_ms, self.statement_timeout_ms).await?;
                Ok(client)
            }
        }
    }

    async fn check_schema(&self, client: &Client) -> Result<(), LedgerStoreError> {
        let row = client
            .query_opt(
                "SELECT schema_version FROM sc_schema_meta WHERE schema_name='statechronicle'",
                &[],
            )
            .await
            .map_err(unavailable)?
            .ok_or_else(|| unavailable("statechronicle schema metadata is missing"))?;
        let version: i32 = row.get(0);
        if version != SUPPORTED_SCHEMA_VERSION {
            return Err(LedgerStoreError::Unavailable(format!(
                "unsupported PostgreSQL schema version {version}; adapter supports {SUPPORTED_SCHEMA_VERSION}"
            )));
        }
        let has_outbox_last_error: bool = client
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM information_schema.columns
                     WHERE table_schema=current_schema()
                       AND table_name='sc_outbox' AND column_name='last_error'
                 )",
                &[],
            )
            .await
            .map_err(unavailable)?
            .get(0);
        if !has_outbox_last_error {
            return Err(unavailable(
                "PostgreSQL schema is missing sc_outbox.last_error migration",
            ));
        }
        for table in ["sc_heads", "sc_idempotency", "sc_events", "sc_outbox"] {
            let has_foreign_key: bool = client
                .query_one(
                    "SELECT EXISTS (
                         SELECT 1 FROM pg_constraint c
                         JOIN pg_class r ON r.oid=c.conrelid
                         WHERE r.relname=$1 AND c.contype='f'
                     )",
                    &[&table],
                )
                .await
                .map_err(unavailable)?
                .get(0);
            if !has_foreign_key {
                return Err(unavailable(format!(
                    "PostgreSQL schema is missing required foreign-key constraints on {table}"
                )));
            }
        }
        for (table, trigger) in [
            ("sc_commits", "sc_commits_immutable"),
            ("sc_events", "sc_events_immutable"),
            ("sc_projections", "sc_projections_monotonic"),
            ("sc_idempotency", "sc_idempotency_state_guard"),
        ] {
            let present: bool = client
                .query_one(
                    "SELECT EXISTS (
                         SELECT 1 FROM pg_trigger t
                         JOIN pg_class r ON r.oid=t.tgrelid
                         WHERE r.relname=$1 AND t.tgname=$2 AND NOT t.tgisinternal
                     )",
                    &[&table, &trigger],
                )
                .await
                .map_err(unavailable)?
                .get(0);
            if !present {
                return Err(unavailable(format!(
                    "PostgreSQL schema is missing required trigger {trigger} on {table}"
                )));
            }
        }
        Ok(())
    }

    async fn read_client(&self) -> Result<Client, CommitStoreError> {
        let client = self
            .connect()
            .await
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        self.check_schema(&client)
            .await
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        Ok(client)
    }

    async fn outbox_client(&self) -> Result<Client, OutboxError> {
        let client = self
            .connect()
            .await
            .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        self.check_schema(&client)
            .await
            .map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        Ok(client)
    }

    async fn outbox_update(
        &self,
        delivery_key: &str,
        worker_id: Option<&str>,
        operation: &str,
    ) -> Result<(), OutboxError> {
        self.outbox_update_with_error(delivery_key, worker_id, operation, "")
            .await
    }

    async fn outbox_update_with_error(
        &self,
        delivery_key: &str,
        worker_id: Option<&str>,
        operation: &str,
        error: &str,
    ) -> Result<(), OutboxError> {
        let client = self.outbox_client().await?;
        let bounded = error.chars().take(1024).collect::<String>();
        let changed = match (operation, worker_id) {
            ("delivered", Some(worker)) => client.execute(
                "UPDATE sc_outbox SET delivered_at=now(),lease_owner=NULL,lease_until=NULL
                 WHERE delivery_key=$1 AND lease_owner=$2 AND delivered_at IS NULL",
                &[&delivery_key, &worker],
            ).await.map_err(outbox_db)?,
            ("delivered", None) => client.execute(
                "UPDATE sc_outbox SET delivered_at=now(),lease_owner=NULL,lease_until=NULL
                 WHERE delivery_key=$1 AND delivered_at IS NULL",
                &[&delivery_key],
            ).await.map_err(outbox_db)?,
            ("quarantine", Some(worker)) => client.execute(
                "UPDATE sc_outbox SET quarantined_at=now(),quarantine_error=$3,lease_owner=NULL,lease_until=NULL
                 WHERE delivery_key=$1 AND lease_owner=$2 AND delivered_at IS NULL",
                &[&delivery_key, &worker, &bounded],
            ).await.map_err(outbox_db)?,
            ("quarantine", None) => client.execute(
                "UPDATE sc_outbox SET quarantined_at=now(),quarantine_error=$2,lease_owner=NULL,lease_until=NULL
                 WHERE delivery_key=$1 AND delivered_at IS NULL",
                &[&delivery_key, &bounded],
            ).await.map_err(outbox_db)?,
            ("release", Some(worker)) => client.execute(
                "UPDATE sc_outbox SET lease_owner=NULL,lease_until=NULL,last_error=$3
                 WHERE delivery_key=$1 AND lease_owner=$2 AND delivered_at IS NULL",
                &[&delivery_key, &worker, &bounded],
            ).await.map_err(outbox_db)?,
            ("release", None) => client.execute(
                "UPDATE sc_outbox SET lease_owner=NULL,lease_until=NULL,last_error=$2
                 WHERE delivery_key=$1 AND delivered_at IS NULL",
                &[&delivery_key, &bounded],
            ).await.map_err(outbox_db)?,
            _ => return Err(OutboxError::Unavailable(String::from("unknown outbox operation"))),
        };
        if changed == 0 {
            if operation == "delivered" {
                let already_delivered = client
                    .query_opt(
                        "SELECT 1 FROM sc_outbox WHERE delivery_key=$1 AND delivered_at IS NOT NULL",
                        &[&delivery_key],
                    )
                    .await
                    .map_err(outbox_db)?
                    .is_some();
                if already_delivered {
                    return Ok(());
                }
            }
            return Err(OutboxError::Unavailable(String::from(
                "outbox delivery key is missing or owned by another worker",
            )));
        }
        Ok(())
    }
}

const STARTUP_VERIFY_ATTEMPTS: usize = 10;
const STARTUP_VERIFY_BACKOFF_MS: u64 = 500;

async fn verify_startup_with_retries(store: &PostgresLedgerStore) -> Result<(), LedgerStoreError> {
    let mut attempt: usize = 0;
    loop {
        attempt = attempt.saturating_add(1);
        match store.verify_all_integrity().await {
            Ok(reports) => {
                drop(reports);
                return Ok(());
            }
            Err(error) if error.is_retryable() && attempt < STARTUP_VERIFY_ATTEMPTS => {
                tokio::time::sleep(std::time::Duration::from_millis(STARTUP_VERIFY_BACKOFF_MS))
                    .await;
            }
            Err(error) => return Err(error),
        }
    }
}

#[async_trait]
impl LedgerStore for PostgresLedgerStore {
    async fn begin(
        &self,
        tenant: &TenantId,
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        validate_tenant(tenant)?;
        let client = self.connect().await?;
        self.check_schema(&client).await?;
        client
            .batch_execute("BEGIN ISOLATION LEVEL SERIALIZABLE")
            .await
            .map_err(unavailable)?;
        configure_transaction(&client, self.lock_timeout_ms, self.statement_timeout_ms).await?;
        Ok(Box::new(PostgresLedgerTransaction {
            client,
            tenants: vec![tenant.clone()],
            events: Vec::new(),
            closed: false,
            commit_inserted: false,
            committed: Vec::new(),
            claimed_intents: Vec::new(),
            finalized_intents: Vec::new(),
        }))
    }

    async fn begin_multi(
        &self,
        tenants: &[TenantId],
    ) -> Result<Box<dyn LedgerTransaction>, LedgerStoreError> {
        if tenants.len() < 2 || tenants.iter().any(|tenant| tenant.0.is_empty()) {
            return Err(LedgerStoreError::Invariant(String::from(
                "PostgreSQL multi-tenant transaction requires at least two non-empty tenants",
            )));
        }
        let mut sorted = tenants.to_vec();
        sorted.sort_by(|left, right| left.0.cmp(&right.0));
        if sorted.windows(2).any(|pair| {
            pair.first()
                .zip(pair.get(1))
                .is_some_and(|(left, right)| left == right)
        }) {
            return Err(LedgerStoreError::Invariant(String::from(
                "multi-tenant transaction contains duplicate tenants",
            )));
        }
        let client = self.connect().await?;
        self.check_schema(&client).await?;
        client
            .batch_execute("BEGIN ISOLATION LEVEL SERIALIZABLE")
            .await
            .map_err(unavailable)?;
        configure_transaction(&client, self.lock_timeout_ms, self.statement_timeout_ms).await?;
        // Acquire every canonical-head lock in deterministic tenant order so
        // concurrent multi-tenant settlements cannot deadlock on lock order.
        for tenant in &sorted {
            client
                .query_opt(
                    "SELECT commit_id FROM sc_heads WHERE tenant_id=$1 FOR UPDATE",
                    &[&tenant.0],
                )
                .await
                .map_err(map_db)?;
        }
        Ok(Box::new(PostgresLedgerTransaction {
            client,
            tenants: sorted,
            events: Vec::new(),
            closed: false,
            commit_inserted: false,
            committed: Vec::new(),
            claimed_intents: Vec::new(),
            finalized_intents: Vec::new(),
        }))
    }
}

#[async_trait]
impl CommitStore for PostgresLedgerStore {
    async fn put_commit(
        &self,
        tenant: &TenantId,
        commit: &Signed<Commit>,
    ) -> Result<(), CommitStoreError> {
        validate_tenant(tenant)
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        let client = self
            .connect()
            .await
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        self.check_schema(&client)
            .await
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        let Some(scope_tenant) = commit.body.scope.tenant_id.as_ref() else {
            return Err(CommitStoreError::Unavailable(String::from(
                "global checkpoint cannot be stored in a tenant commit store",
            )));
        };
        if commit.body.scope.kind != ScopeKind::Tenant || scope_tenant != tenant {
            return Err(CommitStoreError::Unavailable(String::from(
                "commit tenant does not match store scope",
            )));
        }
        client
            .batch_execute("BEGIN ISOLATION LEVEL SERIALIZABLE")
            .await
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        let result = async {
            let head = client
                .query_opt(
                    "SELECT commit_id,sequence,state_root FROM sc_heads WHERE tenant_id=$1 FOR UPDATE",
                    &[&tenant.0],
                )
                .await
                .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
            if let Some(row) = head {
                let parent: String = row.get(0);
                let sequence: i64 = row.get(1);
                let root: Vec<u8> = row.get(2);
                if commit.body.parent_commit_id.as_ref().map(|id| id.0.as_str()) != Some(parent.as_str())
                    || i64::try_from(commit.body.sequence).ok() != Some(sequence.saturating_add(1))
                    || commit.body.previous_state_root.as_bytes() != root.as_slice()
                {
                    return Err(CommitStoreError::Unavailable(String::from(
                        "commit does not extend canonical head",
                    )));
                }
            } else if commit.body.parent_commit_id.is_some() || commit.body.sequence != 1 {
                return Err(CommitStoreError::Unavailable(String::from(
                    "genesis commit does not match empty head",
                )));
            }
            let payload = bcs::to_bytes(commit)
                .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
            let sequence = i64::try_from(commit.body.sequence)
                .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
            client
                .execute(
                    "INSERT INTO sc_commits (tenant_id,commit_id,sequence,payload) VALUES ($1,$2,$3,$4)",
                    &[&tenant.0, &commit.body.commit_id.0, &sequence, &payload],
                )
                .await
                .map_err(|error| {
                    if error.as_db_error().is_some_and(|db| db.code().code() == "23505") {
                        CommitStoreError::Duplicate
                    } else {
                        CommitStoreError::Unavailable(error.to_string())
                    }
                })?;
            client
                .execute(
                    "INSERT INTO sc_heads (tenant_id,commit_id,sequence,state_root) VALUES ($1,$2,$3,$4)
                     ON CONFLICT (tenant_id) DO UPDATE SET commit_id=EXCLUDED.commit_id,sequence=EXCLUDED.sequence,state_root=EXCLUDED.state_root",
                    &[&tenant.0, &commit.body.commit_id.0, &sequence, &commit.body.next_state_root.as_bytes().as_slice()],
                )
                .await
                .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => client
                .batch_execute("COMMIT")
                .await
                .map_err(|error| CommitStoreError::Unavailable(error.to_string())),
            Err(error) => {
                if let Err(rollback_error) = client.batch_execute("ROLLBACK").await {
                    tracing::warn!(%rollback_error, "postgres commit-store rollback failed");
                }
                Err(error)
            }
        }
    }

    async fn commit_by_id(
        &self,
        tenant: &TenantId,
        commit_id: &CommitId,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        let client = self.read_client().await?;
        let row = client
            .query_opt(
                "SELECT payload FROM sc_commits WHERE tenant_id=$1 AND commit_id=$2",
                &[&tenant.0, &commit_id.0],
            )
            .await
            .map_err(|error| pg_commit_error(&error))?;
        row.map(|row| {
            bcs::from_bytes::<Signed<Commit>>(&row.get::<_, Vec<u8>>(0))
                .map_err(|error| CommitStoreError::Unavailable(error.to_string()))
        })
        .transpose()
    }

    async fn commit_by_sequence(
        &self,
        tenant: &TenantId,
        sequence: u64,
    ) -> Result<Option<Signed<Commit>>, CommitStoreError> {
        let client = self.read_client().await?;
        let sequence = i64::try_from(sequence)
            .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
        let row = client
            .query_opt(
                "SELECT payload FROM sc_commits WHERE tenant_id=$1 AND sequence=$2",
                &[&tenant.0, &sequence],
            )
            .await
            .map_err(|error| pg_commit_error(&error))?;
        row.map(|row| {
            bcs::from_bytes::<Signed<Commit>>(&row.get::<_, Vec<u8>>(0))
                .map_err(|error| CommitStoreError::Unavailable(error.to_string()))
        })
        .transpose()
    }

    async fn canonical_head(
        &self,
        tenant: &TenantId,
    ) -> Result<Option<CanonicalHead>, CommitStoreError> {
        let client = self.read_client().await?;
        let row = client
            .query_opt(
                "SELECT commit_id,sequence,state_root FROM sc_heads WHERE tenant_id=$1",
                &[&tenant.0],
            )
            .await
            .map_err(|error| pg_commit_error(&error))?;
        row.map(|row| {
            let commit_id = CommitId::new(row.get(0))
                .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
            let sequence: i64 = row.get(1);
            let sequence = u64::try_from(sequence)
                .map_err(|error| CommitStoreError::Unavailable(error.to_string()))?;
            let root: Vec<u8> = row.get(2);
            let root: [u8; 32] = root.try_into().map_err(|error| {
                CommitStoreError::Unavailable(format!(
                    "canonical head root must be 32 bytes: {error:?}"
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
}

#[async_trait]
impl OutboxStore for PostgresLedgerStore {
    async fn claim(
        &self,
        worker_id: &str,
        limit: usize,
        lease_until_unix: i64,
    ) -> Result<Vec<(String, OutboxPayload)>, OutboxError> {
        if worker_id.is_empty() {
            return Err(OutboxError::Unavailable(String::from(
                "outbox worker id must not be empty",
            )));
        }
        if limit == 0 {
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
        let client = self.outbox_client().await?;
        client
            .batch_execute("BEGIN ISOLATION LEVEL READ COMMITTED")
            .await
            .map_err(outbox_db)?;
        configure_outbox_transaction(&client, self.lock_timeout_ms, self.statement_timeout_ms)
            .await?;
        let limit =
            i64::try_from(limit).map_err(|error| OutboxError::Unavailable(error.to_string()))?;
        let lease = lease_until_unix as f64;
        let rows = client
            .query(
                "SELECT delivery_key,tenant_id,commit_id,payload_digest,payload
                 FROM sc_outbox
                 WHERE delivered_at IS NULL AND quarantined_at IS NULL
                   AND (lease_until IS NULL OR lease_until <= now())
                 ORDER BY delivery_key FOR UPDATE SKIP LOCKED LIMIT $1",
                &[&limit],
            )
            .await
            .map_err(outbox_db)?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let key: String = row.get(0);
            let tenant = TenantId(row.get(1));
            let raw_commit_id: String = row.get(2);
            let (commit_id, malformed_commit_id) = CommitId::new(raw_commit_id).map_or_else(
                |_| (CommitId(String::from("cmt_poison")), true),
                |commit_id| (commit_id, false),
            );
            let digest: Vec<u8> = row.get(3);
            let payload: Vec<u8> = row.get(4);
            // Preserve malformed stored digests as a leased payload. The
            // dispatcher performs the integrity check and can apply its
            // configured retry-versus-quarantine policy instead of losing the
            // poison row through an early claim error.
            let digest: [u8; 32] = if malformed_commit_id {
                *hash_bytes(b"statechronicle.invalid-outbox-commit-id").as_bytes()
            } else {
                digest.try_into().unwrap_or_else(|_| {
                    *hash_bytes(b"statechronicle.invalid-outbox-digest").as_bytes()
                })
            };
            client
                .execute(
                    "UPDATE sc_outbox SET lease_owner=$1,lease_until=to_timestamp($2)
                     WHERE delivery_key=$3",
                    &[&worker_id, &lease, &key],
                )
                .await
                .map_err(outbox_db)?;
            claimed.push((
                key,
                OutboxPayload::Opaque {
                    tenant,
                    commit_id,
                    payload_digest: ContentDigest::new(digest),
                    payload,
                },
            ));
        }
        client.batch_execute("COMMIT").await.map_err(outbox_db)?;
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
        self.outbox_update(delivery_key, Some(worker_id), "delivered")
            .await
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
        self.outbox_update_with_error(delivery_key, Some(worker_id), "release", error)
            .await
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
        self.outbox_update_with_error(delivery_key, Some(worker_id), "quarantine", error)
            .await
    }

    async fn pending_count(&self, tenant: Option<&TenantId>) -> Result<u64, OutboxError> {
        let client = self.outbox_client().await?;
        let count: i64 = match tenant {
            Some(tenant) => client.query_one(
                "SELECT COUNT(*) FROM sc_outbox WHERE tenant_id=$1 AND delivered_at IS NULL AND quarantined_at IS NULL",
                &[&tenant.0],
            ).await.map_err(outbox_db)?.get(0),
            None => client.query_one(
                "SELECT COUNT(*) FROM sc_outbox WHERE delivered_at IS NULL AND quarantined_at IS NULL",
                &[],
            ).await.map_err(outbox_db)?.get(0),
        };
        u64::try_from(count).map_err(|error| OutboxError::Unavailable(error.to_string()))
    }
}

#[async_trait]
impl ConsumerDedupStore for PostgresLedgerStore {
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
        let client = self.outbox_client().await?;
        client
            .batch_execute("BEGIN ISOLATION LEVEL READ COMMITTED")
            .await
            .map_err(outbox_db)?;
        let attempt = uuid::Uuid::new_v4().to_string();
        let lease = lease_until_unix as f64;
        let inserted = client
            .execute(
                "INSERT INTO sc_consumer_deliveries (delivery_key,status,attempt_id,lease_until)
             VALUES ($1,'in_progress',$2,to_timestamp($3)) ON CONFLICT DO NOTHING",
                &[&delivery_key, &attempt, &lease],
            )
            .await
            .map_err(outbox_db)?;
        if inserted == 1 {
            client.batch_execute("COMMIT").await.map_err(outbox_db)?;
            return Ok(ConsumerDeliveryClaim::New {
                attempt_id: attempt,
            });
        }
        let row = client.query_one(
            "SELECT status,attempt_id,EXTRACT(EPOCH FROM lease_until)::bigint FROM sc_consumer_deliveries WHERE delivery_key=$1 FOR UPDATE",
            &[&delivery_key],
        ).await.map_err(outbox_db)?;
        let status: &str = row.get(0);
        let result = if status == "applied" {
            ConsumerDeliveryClaim::AlreadyApplied
        } else {
            ConsumerDeliveryClaim::InProgress {
                attempt_id: row.get(1),
                lease_expires_at_unix: row.get(2),
            }
        };
        client.batch_execute("COMMIT").await.map_err(outbox_db)?;
        Ok(result)
    }

    async fn mark_delivery_applied(
        &self,
        delivery_key: &str,
        attempt_id: &str,
    ) -> Result<(), OutboxError> {
        let client = self.outbox_client().await?;
        let changed = client.execute(
            "UPDATE sc_consumer_deliveries SET status='applied',applied_at=now(),lease_until=now()
             WHERE delivery_key=$1 AND attempt_id=$2 AND status='in_progress'",
            &[&delivery_key, &attempt_id],
        ).await.map_err(outbox_db)?;
        if changed == 0 {
            return Err(OutboxError::Unavailable(String::from(
                "consumer delivery ownership mismatch",
            )));
        }
        Ok(())
    }

    async fn release_delivery(
        &self,
        delivery_key: &str,
        attempt_id: &str,
        error: &str,
    ) -> Result<(), OutboxError> {
        let client = self.outbox_client().await?;
        let changed = client
            .execute(
                "UPDATE sc_consumer_deliveries SET lease_until=to_timestamp(0),last_error=$3
             WHERE delivery_key=$1 AND attempt_id=$2 AND status='in_progress'",
                &[
                    &delivery_key,
                    &attempt_id,
                    &error.chars().take(1024).collect::<String>(),
                ],
            )
            .await
            .map_err(outbox_db)?;
        if changed == 0 {
            return Err(OutboxError::Unavailable(String::from(
                "consumer delivery ownership mismatch",
            )));
        }
        Ok(())
    }
}

struct PostgresLedgerTransaction {
    client: Client,
    tenants: Vec<TenantId>,
    events: Vec<Event>,
    closed: bool,
    commit_inserted: bool,
    committed: Vec<(TenantId, CommitId)>,
    claimed_intents: Vec<(TenantId, IntentId, SubjectId, Operation)>,
    finalized_intents: Vec<(TenantId, IntentId)>,
}

impl PostgresLedgerTransaction {
    fn allows(&self, tenant: &TenantId) -> bool {
        self.tenants.iter().any(|candidate| candidate == tenant)
    }

    const fn ensure_open(&self) -> Result<(), LedgerStoreError> {
        if self.closed {
            Err(LedgerStoreError::Closed)
        } else {
            Ok(())
        }
    }

    async fn abort(mut self) -> Result<(), LedgerStoreError> {
        if !self.closed {
            self.client
                .batch_execute("ROLLBACK")
                .await
                .map_err(unavailable)?;
            self.closed = true;
        }
        Ok(())
    }
}

#[async_trait]
impl LedgerTransaction for PostgresLedgerTransaction {
    async fn claim_idempotency(
        &mut self,
        tenant: &TenantId,
        intent: &Intent,
        payload_digest: &ContentDigest,
    ) -> Result<IdempotencyClaim, LedgerStoreError> {
        self.ensure_open()?;
        if !self.allows(tenant) || intent.tenant_id != *tenant {
            return Err(LedgerStoreError::Invariant(String::from(
                "idempotency tenant mismatch",
            )));
        }
        let attempt = uuid::Uuid::new_v4().to_string();
        let inserted = self.client.execute(
            "INSERT INTO sc_idempotency (tenant_id,intent_id,payload_digest,status,attempt_id,lease_expires_at,intent_payload)
             VALUES ($1,$2,$3,'in_progress',$4,now()+interval '60 seconds',$5)
             ON CONFLICT (tenant_id,intent_id) DO NOTHING",
            &[&tenant.0, &intent.intent_id.0, &payload_digest.as_bytes().as_slice(), &attempt,
              &bcs::to_bytes(intent).map_err(|error| invalid(error.to_string()))?],
        )
        .await
        .map_err(|error| {
            if is_contention_error(&error) {
                return LedgerStoreError::Conflict(String::from(
                    "idempotency claim is contended; retry after the current owner resolves",
                ));
            }
            map_db(error)
        });
        let inserted = match inserted {
            Ok(inserted) => inserted,
            Err(LedgerStoreError::Conflict(message))
                if message.contains("idempotency claim is contended") =>
            {
                return Ok(IdempotencyClaim::InProgress {
                    attempt_id: String::from("contended"),
                    lease_expires_at_unix: chrono::Utc::now().timestamp().saturating_add(60),
                });
            }
            Err(error) => return Err(error),
        };
        if inserted == 1 {
            self.claimed_intents.push((
                tenant.clone(),
                intent.intent_id.clone(),
                intent.actor.clone(),
                intent.operation.clone(),
            ));
            return Ok(IdempotencyClaim::NewReservation {
                attempt_id: attempt,
            });
        }
        let row = self.client.query_one(
            "SELECT payload_digest,status,attempt_id,EXTRACT(EPOCH FROM lease_expires_at)::bigint,commit_id
             FROM sc_idempotency WHERE tenant_id=$1 AND intent_id=$2 FOR UPDATE",
            &[&tenant.0, &intent.intent_id.0],
        )
        .await
        .map_err(|error| {
            if is_contention_error(&error) {
                return LedgerStoreError::Conflict(String::from(
                    "idempotency claim is contended; retry after the current owner resolves",
                ));
            }
            map_db(error)
        });
        let row = match row {
            Ok(row) => row,
            Err(LedgerStoreError::Conflict(message))
                if message.contains("idempotency claim is contended") =>
            {
                return Ok(IdempotencyClaim::InProgress {
                    attempt_id: String::from("contended"),
                    lease_expires_at_unix: chrono::Utc::now().timestamp().saturating_add(60),
                });
            }
            Err(error) => return Err(error),
        };
        let existing: Vec<u8> = row.get(0);
        if existing.as_slice() != payload_digest.as_bytes() {
            return Ok(IdempotencyClaim::ConflictDifferentPayload);
        }
        let status: &str = row.get(1);
        if status == "committed" {
            let commit_id: String = row.get(4);
            return Ok(IdempotencyClaim::Committed {
                commit_id: CommitId::new(commit_id).map_err(domain)?,
            });
        }
        let owner: String = row.get(2);
        let lease: i64 = row.get(3);
        let now = chrono::Utc::now().timestamp();
        if status == "in_progress" && lease <= now {
            let replacement = uuid::Uuid::new_v4().to_string();
            let changed = self
                .client
                .execute(
                    "UPDATE sc_idempotency
                     SET attempt_id=$1,lease_expires_at=now()+interval '60 seconds'
                     WHERE tenant_id=$2 AND intent_id=$3 AND status='in_progress'
                       AND attempt_id=$4 AND lease_expires_at <= now()",
                    &[&replacement, &tenant.0, &intent.intent_id.0, &owner],
                )
                .await
                .map_err(map_db)?;
            if changed == 1 {
                self.claimed_intents.push((
                    tenant.clone(),
                    intent.intent_id.clone(),
                    intent.actor.clone(),
                    intent.operation.clone(),
                ));
                return Ok(IdempotencyClaim::NewReservation {
                    attempt_id: replacement,
                });
            }
        }
        Ok(IdempotencyClaim::InProgress {
            attempt_id: owner,
            lease_expires_at_unix: lease,
        })
    }

    async fn append_events(&mut self, events: &[Event]) -> Result<(), LedgerStoreError> {
        self.ensure_open()?;
        if self.commit_inserted {
            return Err(LedgerStoreError::Invariant(String::from(
                "events cannot be appended after the commit",
            )));
        }
        for event in events {
            if !self.allows(&event.tenant_id)
                || event.event_id.0.is_empty()
                || event.intent_id.0.is_empty()
                || event.actor.0.is_empty()
                || !self
                    .claimed_intents
                    .iter()
                    .any(|(scope, intent, actor, operation)| {
                        scope == &event.tenant_id
                            && intent.0 == event.intent_id.0
                            && actor == &event.actor
                            && operation == &event.operation
                    })
            {
                return Err(LedgerStoreError::Invariant(String::from(
                    "event tenant, identifier, or claimed intent is invalid",
                )));
            }
        }
        let total_count = self.events.len().saturating_add(events.len());
        if total_count > MAX_EVENTS_PER_COMMIT {
            return Err(LedgerStoreError::Invariant(format!(
                "event count {total_count} exceeds durable limit {MAX_EVENTS_PER_COMMIT}"
            )));
        }
        let total_bytes = self
            .events
            .iter()
            .chain(events.iter())
            .map(|event| {
                let payload = bcs::to_bytes(event).map_err(|error| invalid(error.to_string()))?;
                check_size("event", MAX_EVENT_BYTES, payload.len())
                    .map_err(|error| invalid(error.to_string()))?;
                Ok::<usize, LedgerStoreError>(payload.len())
            })
            .try_fold(0usize, |total, size| {
                size.map(|size| total.saturating_add(size))
            })?;
        check_size("event_batch", MAX_EVENT_BATCH_BYTES, total_bytes)
            .map_err(|error| invalid(error.to_string()))?;
        self.events.extend_from_slice(events);
        Ok(())
    }

    async fn append_commit(&mut self, commit: &Signed<Commit>) -> Result<(), LedgerStoreError> {
        self.ensure_open()?;
        if self.commit_inserted && self.tenants.len() == 1 {
            return Err(LedgerStoreError::Invariant(String::from(
                "only one commit may be appended per ledger transaction",
            )));
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
        let Some(tenant) = commit.body.scope.tenant_id.as_ref() else {
            return Err(LedgerStoreError::Invariant(String::from(
                "global checkpoint is not a ledger commit",
            )));
        };
        if commit.body.scope.kind != ScopeKind::Tenant || !self.allows(tenant) {
            return Err(LedgerStoreError::Invariant(String::from(
                "commit tenant mismatch",
            )));
        }
        let head = self
            .client
            .query_opt(
                "SELECT commit_id,sequence,state_root FROM sc_heads WHERE tenant_id=$1 FOR UPDATE",
                &[&tenant.0],
            )
            .await
            .map_err(map_db)?;
        if let Some(row) = head {
            let parent: String = row.get(0);
            let sequence: i64 = row.get(1);
            let root: Vec<u8> = row.get(2);
            if commit
                .body
                .parent_commit_id
                .as_ref()
                .map(|id| id.0.as_str())
                != Some(parent.as_str())
                || i64::try_from(commit.body.sequence).ok() != Some(sequence.saturating_add(1))
                || commit.body.previous_state_root.as_bytes() != root.as_slice()
            {
                return Err(LedgerStoreError::Conflict(String::from(
                    "commit does not extend canonical head",
                )));
            }
        } else if commit.body.parent_commit_id.is_some() || commit.body.sequence != 1 {
            return Err(LedgerStoreError::Conflict(String::from(
                "genesis commit does not match empty head",
            )));
        }
        // Validate against a non-destructive view of the staged events. A
        // rejected count/root must leave the transaction retryable; draining
        // before validation would silently discard the caller's events.
        let commit_events: Vec<Event> = self
            .events
            .iter()
            .filter(|event| event.tenant_id == *tenant)
            .cloned()
            .collect();
        if commit_events.is_empty() {
            return Err(LedgerStoreError::Invariant(String::from(
                "commit must include at least one event in its tenant scope",
            )));
        }
        let event_count = u64::try_from(commit_events.len())
            .map_err(|error| invalid(format!("event count overflow: {error}")))?;
        let computed_root =
            event_root(&commit_events).map_err(|error| invalid(error.to_string()))?;
        if commit.body.event_count != event_count || commit.body.event_merkle_root != computed_root
        {
            return Err(LedgerStoreError::Invariant(String::from(
                "commit event count or Merkle root does not match staged events",
            )));
        }
        self.events.retain(|event| event.tenant_id != *tenant);
        let payload = bcs::to_bytes(commit).map_err(|error| invalid(error.to_string()))?;
        check_size("commit", MAX_COMMIT_BYTES, payload.len())
            .map_err(|error| invalid(error.to_string()))?;
        self.client.execute(
            "INSERT INTO sc_commits (tenant_id,commit_id,sequence,payload) VALUES ($1,$2,$3,$4)",
            &[&tenant.0, &commit.body.commit_id.0, &i64::try_from(commit.body.sequence).map_err(|error| invalid(format!("sequence overflow: {error}")))?, &payload],
        ).await.map_err(map_db)?;
        for (index, event) in commit_events.iter().enumerate() {
            let event_payload = bcs::to_bytes(event).map_err(|error| invalid(error.to_string()))?;
            self.client.execute(
                "INSERT INTO sc_events (tenant_id,event_id,commit_id,event_index,payload) VALUES ($1,$2,$3,$4,$5)",
                &[&tenant.0, &event.event_id.0, &commit.body.commit_id.0, &i32::try_from(index).map_err(|error| invalid(format!("event index overflow: {error}")))?, &event_payload],
            ).await.map_err(map_db)?;
        }
        self.client.execute(
            "INSERT INTO sc_heads (tenant_id,commit_id,sequence,state_root) VALUES ($1,$2,$3,$4)
             ON CONFLICT (tenant_id) DO UPDATE SET commit_id=EXCLUDED.commit_id,sequence=EXCLUDED.sequence,state_root=EXCLUDED.state_root",
            &[&tenant.0, &commit.body.commit_id.0, &i64::try_from(commit.body.sequence).map_err(|error| invalid(format!("sequence overflow: {error}")))?, &commit.body.next_state_root.as_bytes().as_slice()],
        ).await.map_err(map_db)?;
        self.commit_inserted = true;
        self.committed
            .push((tenant.clone(), commit.body.commit_id.clone()));
        Ok(())
    }

    async fn upsert_projection(
        &mut self,
        projection: &StateProjection,
    ) -> Result<(), LedgerStoreError> {
        self.ensure_open()?;
        if !self.allows(&projection.tenant_id)
            || projection.resource_id.0.is_empty()
            || projection.last_event_id.0.is_empty()
            || projection.last_commit_id.0.is_empty()
        {
            return Err(LedgerStoreError::Invariant(String::from(
                "projection scope or identifiers are invalid",
            )));
        }
        let payload = bcs::to_bytes(projection).map_err(|error| invalid(error.to_string()))?;
        let version = i64::try_from(projection.version)
            .map_err(|error| invalid(format!("projection version overflow: {error}")))?;
        let row = self.client.query_opt("SELECT version,payload FROM sc_projections WHERE tenant_id=$1 AND resource_id=$2 FOR UPDATE", &[&projection.tenant_id.0, &projection.resource_id.0]).await.map_err(map_db)?;
        if let Some(row) = row {
            let current: i64 = row.get(0);
            if current > version {
                return Err(LedgerStoreError::Conflict(String::from(
                    "projection version regressed",
                )));
            }
            if current == version && row.get::<_, Vec<u8>>(1) != payload {
                return Err(LedgerStoreError::Invariant(String::from(
                    "equal-version projection conflict",
                )));
            }
            if current == version {
                return Ok(());
            }
        }
        self.client.execute(
            "INSERT INTO sc_projections (tenant_id,resource_id,version,payload) VALUES ($1,$2,$3,$4)
             ON CONFLICT (tenant_id,resource_id) DO UPDATE SET version=EXCLUDED.version,payload=EXCLUDED.payload
             WHERE sc_projections.version < EXCLUDED.version",
            &[&projection.tenant_id.0, &projection.resource_id.0, &version, &payload],
        ).await.map_err(map_db)?;
        Ok(())
    }

    async fn enqueue_outbox(&mut self, record: &OutboxRecord) -> Result<(), LedgerStoreError> {
        self.ensure_open()?;
        if !self.allows(&record.tenant)
            || record.delivery_key.is_empty()
            || record.commit_id.0.is_empty()
            || hash_bytes(&record.payload) != record.payload_digest
        {
            return Err(LedgerStoreError::Invariant(String::from(
                "outbox scope, identifiers, or digest are invalid",
            )));
        }
        check_size(
            "outbox_payload",
            MAX_OUTBOX_PAYLOAD_BYTES,
            record.payload.len(),
        )
        .map_err(|error| LedgerStoreError::Invariant(error.to_string()))?;
        self.client.execute(
            "INSERT INTO sc_outbox (delivery_key,tenant_id,commit_id,payload_digest,payload) VALUES ($1,$2,$3,$4,$5)",
            &[&record.delivery_key, &record.tenant.0, &record.commit_id.0, &record.payload_digest.as_bytes().as_slice(), &record.payload],
        ).await.map_err(map_db)?;
        Ok(())
    }

    async fn finalize_idempotency(
        &mut self,
        tenant: &TenantId,
        intent_id: &IntentId,
        attempt_id: &str,
        commit_id: &CommitId,
    ) -> Result<(), LedgerStoreError> {
        self.ensure_open()?;
        if !self.allows(tenant)
            || !self
                .committed
                .iter()
                .any(|(scope, id)| scope == tenant && id == commit_id)
        {
            return Err(LedgerStoreError::Invariant(String::from(
                "idempotency finalization must target a commit appended for this tenant",
            )));
        }
        if commit_id.0.is_empty() {
            return Err(LedgerStoreError::Idempotency(String::from(
                "commit identifier must not be empty",
            )));
        }
        let changed = self
            .client
            .execute(
                "UPDATE sc_idempotency SET status='committed',commit_id=$4,lease_expires_at=now()
             WHERE tenant_id=$1 AND intent_id=$2 AND attempt_id=$3 AND status='in_progress'",
                &[&tenant.0, &intent_id.0, &attempt_id, &commit_id.0],
            )
            .await
            .map_err(map_db)?;
        if changed != 1 {
            return Err(LedgerStoreError::Idempotency(String::from(
                "idempotency reservation ownership mismatch",
            )));
        }
        self.finalized_intents
            .push((tenant.clone(), intent_id.clone()));
        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> Result<(), LedgerStoreError> {
        self.ensure_open()?;
        let all_reservations_finalized =
            self.claimed_intents.iter().all(|(tenant, intent, _, _)| {
                self.finalized_intents
                    .iter()
                    .any(|(final_tenant, final_intent)| {
                        final_tenant == tenant && final_intent == intent
                    })
            });
        if !self.commit_inserted || !self.events.is_empty() || !all_reservations_finalized {
            if let Err(error) = self.abort().await {
                tracing::warn!(%error, "postgres ledger rollback failed");
            }
            return Err(LedgerStoreError::Invariant(String::from(
                "ledger transaction is missing a committed commit or has unbound events",
            )));
        }
        self.client
            .batch_execute("COMMIT")
            .await
            .map_err(unavailable)?;
        self.closed = true;
        Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<(), LedgerStoreError> {
        self.abort().await
    }
}

fn validate_tenant(tenant: &TenantId) -> Result<(), LedgerStoreError> {
    if tenant.0.is_empty() {
        Err(LedgerStoreError::Invariant(String::from(
            "tenant identifier must not be empty",
        )))
    } else {
        Ok(())
    }
}

fn unavailable(error: impl std::fmt::Display) -> LedgerStoreError {
    LedgerStoreError::Unavailable(error.to_string())
}

async fn configure_transaction(
    client: &Client,
    lock_timeout_ms: u64,
    statement_timeout_ms: u64,
) -> Result<(), LedgerStoreError> {
    let lock = format!("{lock_timeout_ms}ms");
    let statement = format!("{statement_timeout_ms}ms");
    client
        .execute(
            "SELECT set_config('lock_timeout',$1,true),
                    set_config('statement_timeout',$2,true)",
            &[&lock, &statement],
        )
        .await
        .map_err(unavailable)?;
    Ok(())
}

async fn configure_session(
    client: &Client,
    lock_timeout_ms: u64,
    statement_timeout_ms: u64,
) -> Result<(), LedgerStoreError> {
    let lock = format!("{lock_timeout_ms}ms");
    let statement = format!("{statement_timeout_ms}ms");
    client
        .execute(
            "SELECT set_config('lock_timeout',$1,false),
                    set_config('statement_timeout',$2,false)",
            &[&lock, &statement],
        )
        .await
        .map_err(unavailable)?;
    Ok(())
}

async fn configure_outbox_transaction(
    client: &Client,
    lock_timeout_ms: u64,
    statement_timeout_ms: u64,
) -> Result<(), OutboxError> {
    let lock = format!("{lock_timeout_ms}ms");
    let statement = format!("{statement_timeout_ms}ms");
    client
        .execute(
            "SELECT set_config('lock_timeout',$1,true),
                    set_config('statement_timeout',$2,true)",
            &[&lock, &statement],
        )
        .await
        .map_err(outbox_db)?;
    Ok(())
}
const fn invalid(message: String) -> LedgerStoreError {
    LedgerStoreError::Invariant(message)
}
fn domain(error: impl std::fmt::Display) -> LedgerStoreError {
    LedgerStoreError::Invariant(error.to_string())
}

fn pg_commit_error(error: &tokio_postgres::Error) -> CommitStoreError {
    CommitStoreError::Unavailable(error.to_string())
}

#[allow(clippy::needless_pass_by_value)]
fn outbox_db(error: tokio_postgres::Error) -> OutboxError {
    OutboxError::Unavailable(error.to_string())
}

fn map_db(error: tokio_postgres::Error) -> LedgerStoreError {
    if let Some(db) = error.as_db_error() {
        match db.code().code() {
            "40001" | "40P01" | "55P03" | "57014" => {
                return LedgerStoreError::Conflict(db.message().to_owned());
            }
            "23505" => return LedgerStoreError::Conflict(db.message().to_owned()),
            _ => {}
        }
    }
    unavailable(error)
}

fn is_contention_error(error: &tokio_postgres::Error) -> bool {
    error
        .as_db_error()
        .map(|db| matches!(db.code().code(), "55P03" | "57014"))
        .unwrap_or(false)
}

#[allow(dead_code)]
fn _row_bytes(row: &Row, index: usize) -> Vec<u8> {
    row.get(index)
}

#[cfg(all(test, feature = "tls"))]
mod tls_tests {
    use super::PostgresLedgerStore;

    #[test]
    fn tls_constructor_requires_a_valid_ca_file() {
        let result = PostgresLedgerStore::new_with_tls(
            "host=localhost user=game dbname=ledger sslmode=require",
            "/definitely/missing/statechronicle-ca.pem",
        );
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod config_tests {
    use super::PostgresLedgerStore;

    #[test]
    fn timeout_configuration_rejects_zero_values() {
        assert!(
            PostgresLedgerStore::new("host=localhost")
                .with_timeouts(0, 1)
                .is_err()
        );
        assert!(
            PostgresLedgerStore::new("host=localhost")
                .with_timeouts(1, 0)
                .is_err()
        );
        assert!(
            PostgresLedgerStore::new("host=localhost")
                .with_timeouts(1, 1)
                .is_ok()
        );
    }
}
