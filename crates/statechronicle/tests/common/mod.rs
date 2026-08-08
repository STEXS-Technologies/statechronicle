//! Shared in-memory fakes and harness for the umbrella e2e integration tests.
//!
//! These are minimal, HashMap/Mutex-backed implementations of the executor's
//! port traits, used to wire the FULL cross-crate pipeline (`submit → parse+validate
//! → execute → commit → state root → proof → verify`) through the real crates.
//! Each test binary that includes this module only exercises a subset of the
//! helpers, so dead code is allowed here.
//!
//! The fakes deliberately mirror the executor crate's own `tests/common/mod.rs`
//! pattern; they are re-declared here (not imported) because that module is
//! `cfg(test)`-only to the `statechronicle-executor` crate and not a public API.

#![allow(
    dead_code,
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::type_complexity
)]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use statechronicle::core::canonicalize::canonicalize;
use statechronicle::core::digest::hash_bytes;
use statechronicle::core::signature::{sign, verify};

use statechronicle::domain::authority::{
    AuthorityProof, EvaluationResult, TRUSTGRANT_EVALUATION_KIND, TrustGrantOutcome,
};
use statechronicle::domain::event::Event;
use statechronicle::domain::ids::{CommitId, EventId, IntentId};
use statechronicle::domain::intent::{Intent, KeyId, SignatureAlg, SignatureBlock};
use statechronicle::domain::resource::ResourceId;
use statechronicle::domain::state::StateProjection;
use statechronicle::domain::state_type::StateType;
use statechronicle::domain::subject::SubjectId;
use statechronicle::domain::tenant::TenantId;

use statechronicle::executor::error::ExecutorError;
use statechronicle::executor::pipeline::{Executor, Ports, TrustGrantPort};
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::ports::intent_store::{IntentStore, IntentStoreError};
use statechronicle::ports::state_index::{StateIndex, StateIndexError};
use statechronicle::ports::tenant_store::{TenantStore, TenantStoreError};
use statechronicle::ports::transaction_manager::{
    TransactionHandle, TransactionManager, TransactionManagerError,
};
use statechronicle::ports::trustgrant_evaluator::TrustGrantError;
use statechronicle::profiles::registry::ProfileRegistry;

/// The Ed25519 key that signs every intent and commit in this e2e lane.
const FIXED_SEED: [u8; 32] = [42u8; 32];

/// The fixed tenant all intents target.
pub fn tenant() -> TenantId {
    TenantId(String::from("acme.game.alpha"))
}

/// The executor identity recorded on every emitted event.
pub fn executor_subject() -> SubjectId {
    SubjectId(String::from("service:statechronicle.example.net"))
}

/// The fixed wall clock shared by every executor in this lane.
pub fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// A fixed commit-formation timestamp (slightly later than the executor clock).
pub fn fixed_timestamp_placeholder() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:02Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// The fixed commit-signing key.
pub fn fixed_key() -> SigningKey {
    SigningKey::from_bytes(&FIXED_SEED)
}

/// The fixed key id bound to [`fixed_key`] in signature blocks.
pub fn key_id() -> KeyId {
    KeyId::new(String::from("did:key:z6Mk...#statechronicle-e2e")).unwrap()
}

/// A pre-baked `allow` authority proof bound to an `asset.transfer` intent.
///
/// `asset.transfer` is authority-required for the unique-asset profile
/// (protocol §11.2, ADR-006 §36 Q5), so the transfer intent must bind one.
pub fn sample_authority() -> AuthorityProof {
    AuthorityProof {
        kind: String::from(TRUSTGRANT_EVALUATION_KIND),
        evaluation_digest: hash_bytes(b"evaluation"),
        result: EvaluationResult::Allow,
        evaluated_at: fixed_now(),
    }
}

/// Parses + validates a raw JSON intent payload and signs its canonical body
/// for real with [`fixed_key`], binding an optional authority proof.
///
/// This exercises the real `statechronicle-intent` parse/validate lane and the
/// real `statechronicle-core` canonicalize/sign lane. The resulting
/// `ValidatedIntent` carries a genuine detached Ed25519 signature that the
/// executor's injected verifier must accept.
pub fn validated_intent(
    payload: &serde_json::Value,
    authority: Option<AuthorityProof>,
) -> ValidatedIntent {
    let raw =
        statechronicle::intent::parse::parse_intent(&serde_json::to_vec(payload).unwrap()).unwrap();
    let mut validated = statechronicle::intent::validate::validate(&raw).unwrap();
    validated.intent.authority = authority;
    let body_bytes = canonicalize(&validated.intent).unwrap();
    let sig = sign(&body_bytes, &fixed_key());
    validated.signature = Some(SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id: key_id(),
        sig,
    });
    validated
}

/// The intent verifier wired into the executor: resolves the fixed key and
/// verifies the detached Ed25519 signature over the BCS canonical intent body.
pub fn intent_verifier()
-> Arc<dyn Fn(&SignatureBlock, &[u8]) -> Result<(), ExecutorError> + Send + Sync> {
    Arc::new(|block, body_bytes| {
        verify(body_bytes, &fixed_key().verifying_key(), &block.sig).map_err(|_source| {
            ExecutorError::ActorAuthenticationFailed(String::from("intent signature invalid"))
        })
    })
}

// ---------------------------------------------------------------------------
// In-memory fakes.
// ---------------------------------------------------------------------------

/// Key for stored intents: (tenant, intent id).
type IntentKey = (TenantId, IntentId);

#[derive(Clone, Default)]
pub struct FakeIntentStore {
    inner: Arc<Mutex<HashMap<IntentKey, Intent>>>,
}

#[async_trait]
impl IntentStore for FakeIntentStore {
    async fn put_intent(&self, tenant: &TenantId, stored: &Intent) -> Result<(), IntentStoreError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|err| IntentStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        let key = (tenant.clone(), stored.intent_id.clone());
        if let Some(existing) = inner.get(&key) {
            if existing != stored {
                return Err(IntentStoreError::Duplicate);
            }
            return Ok(());
        }
        inner.insert(key, stored.clone());
        Ok(())
    }

    async fn get_intent(
        &self,
        tenant: &TenantId,
        intent_id: &IntentId,
    ) -> Result<Option<Intent>, IntentStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| IntentStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner.get(&(tenant.clone(), intent_id.clone())).cloned())
    }
}

/// Key for state projections: (tenant, resource, optional subject).
type ProjectionKey = (TenantId, ResourceId, Option<SubjectId>);

#[derive(Clone, Default)]
pub struct FakeStateIndex {
    inner: Arc<Mutex<HashMap<ProjectionKey, StateProjection>>>,
}

/// Returns whether a state type is keyed by its holder.
const fn subject_held(state_type: StateType) -> bool {
    matches!(
        state_type,
        StateType::ConsumableStack
            | StateType::FungibleBalance
            | StateType::Entitlement
            | StateType::MeteredResource
    )
}

impl FakeStateIndex {
    /// Composition-root index adapter: applies a derived projection, keyed by
    /// the holder for subject-held types and by resource for owner-based types.
    pub fn apply(&self, event: &Event, state_type: StateType) {
        let mut inner = self
            .inner
            .lock()
            .map_err(|err| StateIndexError::Unavailable(format!("lock poisoned: {err}")))
            .unwrap();
        let subject = if subject_held(state_type) {
            event
                .after
                .state
                .get("subject")
                .and_then(serde_json::Value::as_str)
                .map(|subject| SubjectId(String::from(subject)))
        } else {
            None
        };
        inner.insert(
            (event.tenant_id.clone(), event.resource_id.clone(), subject),
            StateProjection {
                tenant_id: event.tenant_id.clone(),
                resource_id: event.resource_id.clone(),
                state_type,
                version: event.after.version,
                last_event_id: event.event_id.clone(),
                last_commit_id: CommitId::new(String::from("cmt_00000000000000000001")).unwrap(),
                state_hash: event.after.state_hash.clone(),
                state: event.after.state.clone(),
            },
        );
    }
}

#[async_trait]
impl StateIndex for FakeStateIndex {
    async fn get_state(
        &self,
        tenant: &TenantId,
        resource_id: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| StateIndexError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner
            .get(&(tenant.clone(), resource_id.clone(), None))
            .cloned())
    }

    async fn get_subject_state(
        &self,
        tenant: &TenantId,
        subject: &SubjectId,
        resource_id: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| StateIndexError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner
            .get(&(tenant.clone(), resource_id.clone(), Some(subject.clone())))
            .cloned())
    }
}

#[derive(Clone, Default)]
pub struct FakeTenantStore {
    inner: Arc<Mutex<HashSet<TenantId>>>,
}

impl FakeTenantStore {
    pub fn register(&self, id: TenantId) {
        self.inner.lock().unwrap().insert(id);
    }
}

#[async_trait]
impl TenantStore for FakeTenantStore {
    async fn tenant_exists(&self, tenant: &TenantId) -> Result<bool, TenantStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|err| TenantStoreError::Unavailable(format!("lock poisoned: {err}")))?;
        Ok(inner.contains(tenant))
    }

    async fn get_tenant_root(
        &self,
        _tenant: &TenantId,
    ) -> Result<Option<statechronicle::core::digest::ContentDigest>, TenantStoreError> {
        Ok(None)
    }
}

/// The TrustGrant adapter's evaluation mode.
#[derive(Clone, Copy)]
pub enum TrustMode {
    /// Everything is allowed and fresh.
    Allow,
}

#[derive(Clone, Copy)]
pub struct FakeTrustGrant {
    mode: TrustMode,
}

impl FakeTrustGrant {
    pub const fn allow() -> Self {
        Self {
            mode: TrustMode::Allow,
        }
    }
}

#[async_trait]
impl TrustGrantPort for FakeTrustGrant {
    async fn evaluate(
        &self,
        _scope: &TenantId,
        _actor: &SubjectId,
        _operation: &str,
        _resource: &ResourceId,
    ) -> Result<TrustGrantOutcome, TrustGrantError> {
        match self.mode {
            TrustMode::Allow => Ok(TrustGrantOutcome {
                evaluation_digest: hash_bytes(b"allowed"),
                result: EvaluationResult::Allow,
                evaluated_at: fixed_now(),
            }),
        }
    }

    async fn check_revocation_freshness(
        &self,
        _proof: &AuthorityProof,
    ) -> Result<(), TrustGrantError> {
        match self.mode {
            TrustMode::Allow => Ok(()),
        }
    }
}

/// Records transaction lifecycle events for atomicity assertions.
#[derive(Clone, Default)]
pub struct FakeTransactionManager {
    log: Arc<Mutex<Vec<String>>>,
}

impl FakeTransactionManager {
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

struct FakeTransactionHandle {
    log: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl TransactionManager for FakeTransactionManager {
    async fn begin(
        &self,
        tenant: &TenantId,
    ) -> Result<Box<dyn TransactionHandle + Send>, TransactionManagerError> {
        self.log
            .lock()
            .map_err(|err| TransactionManagerError::Unavailable(format!("lock poisoned: {err}")))?
            .push(format!("begin:{}", tenant.0));
        Ok(Box::new(FakeTransactionHandle {
            log: self.log.clone(),
        }))
    }

    async fn begin_multi(
        &self,
        tenants: &[TenantId],
    ) -> Result<Box<dyn TransactionHandle + Send>, TransactionManagerError> {
        let joined = tenants
            .iter()
            .map(|tenant| tenant.0.as_str())
            .collect::<Vec<_>>()
            .join(",");
        self.log
            .lock()
            .map_err(|err| TransactionManagerError::Unavailable(format!("lock poisoned: {err}")))?
            .push(format!("begin_multi:{joined}"));
        Ok(Box::new(FakeTransactionHandle {
            log: self.log.clone(),
        }))
    }
}

#[async_trait]
impl TransactionHandle for FakeTransactionHandle {
    async fn commit(self: Box<Self>) -> Result<(), TransactionManagerError> {
        self.log
            .lock()
            .map_err(|err| TransactionManagerError::CommitFailed(format!("lock poisoned: {err}")))?
            .push(String::from("commit"));
        Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<(), TransactionManagerError> {
        self.log
            .lock()
            .map_err(|err| {
                TransactionManagerError::RollbackFailed(format!("lock poisoned: {err}"))
            })?
            .push(String::from("rollback"));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Harness.
// ---------------------------------------------------------------------------

/// An executor wired to fresh fakes with the default tenant registered and the
/// real (key-resolving) intent verifier installed.
pub struct Harness {
    pub executor: Executor,
    pub index: FakeStateIndex,
    pub tenant_store: FakeTenantStore,
    pub transactions: FakeTransactionManager,
}

impl Harness {
    /// Wires the executor with the default tenant and the injected verifier.
    pub fn with_verifier(
        intent_verifier: Arc<
            dyn Fn(&SignatureBlock, &[u8]) -> Result<(), ExecutorError> + Send + Sync,
        >,
    ) -> Self {
        let index = FakeStateIndex::default();
        let tenant_store = FakeTenantStore::default();
        tenant_store.register(tenant());
        let transactions = FakeTransactionManager::default();

        let ports = Ports::new(
            Box::new(FakeIntentStore::default()),
            Box::new(index.clone()),
            Box::new(tenant_store.clone()),
            vec![Box::new(FakeTrustGrant::allow())],
            Box::new(transactions.clone()),
        );
        let counter = Arc::new(AtomicU64::new(1));
        let executor = Executor::new(
            ports,
            ProfileRegistry::baseline(),
            executor_subject(),
            Box::new(fixed_now),
            Box::new(move || {
                let next = counter.fetch_add(1, Ordering::SeqCst);
                EventId::new(format!("evt_{next:020}")).unwrap()
            }),
            intent_verifier,
        );
        Self {
            executor,
            index,
            tenant_store,
            transactions,
        }
    }
}
