//! Shared in-memory fakes and harness for executor integration tests.
//!
//! HashMap/Mutex-backed implementations of every port plus an [`Executor`]
//! harness with a fixed wall clock and a counter-based event-id generator.
//! Each test binary that includes this module only exercises a subset of the
//! helpers, so dead code is allowed here.

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
use serde_json::Value;

use statechronicle_core::digest::hash_bytes;

use statechronicle_domain::authority::{
    AuthorityProof, EvaluationResult, TRUSTGRANT_EVALUATION_KIND, TrustGrantOutcome,
};
use statechronicle_domain::ids::{CommitId, EventId, IntentId};
use statechronicle_domain::intent::{Intent, Nonce, Operation, SignatureBlock};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;

use statechronicle_executor::error::ExecutorError;
use statechronicle_executor::pipeline::{Executor, Ports, TrustGrantPort};
use statechronicle_intent::validated::{IdempotencyKey, ValidatedIntent};
use statechronicle_ports::intent_store::{IntentStore, IntentStoreError};
use statechronicle_ports::state_index::{StateIndex, StateIndexError};
use statechronicle_ports::tenant_store::{TenantStore, TenantStoreError};
use statechronicle_ports::transaction_manager::{
    TransactionHandle, TransactionManager, TransactionManagerError,
};
use statechronicle_ports::trustgrant_evaluator::TrustGrantError;
use statechronicle_profiles::registry::ProfileRegistry;

pub fn tenant() -> TenantId {
    TenantId(String::from("stexs.game.alpha"))
}

pub fn executor_subject() -> SubjectId {
    SubjectId(String::from("service:statechronicle.stexs.net"))
}

pub fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

pub fn sample_authority() -> AuthorityProof {
    AuthorityProof {
        kind: String::from(TRUSTGRANT_EVALUATION_KIND),
        evaluation_digest: hash_bytes(b"evaluation"),
        result: EvaluationResult::Allow,
        evaluated_at: fixed_now(),
    }
}

/// Builds a validated intent with fixed ids and a deterministic nonce.
#[allow(clippy::too_many_arguments)]
pub fn intent(
    intent_id: &str,
    operation: &str,
    state_type: Option<StateType>,
    expected_version: u64,
    resource: &str,
    acting_actor: &str,
    inputs: &[(&str, serde_json::Value)],
    authority: Option<AuthorityProof>,
    expires_at: Option<DateTime<Utc>>,
) -> ValidatedIntent {
    let body = Intent::new(
        tenant(),
        IntentId::new(format!("int_{intent_id}")).unwrap(),
        Operation::new(String::from(operation)).unwrap(),
        SubjectId(String::from(acting_actor)),
        ResourceId(String::from(resource)),
        state_type,
        expected_version,
        inputs
            .iter()
            .map(|(key, value)| (String::from(*key), value.clone()))
            .collect(),
        authority,
        fixed_now(),
        expires_at,
        Nonce::from_bytes(vec![0]).unwrap(),
    );
    let idempotency_key = IdempotencyKey::new(
        body.tenant_id.clone(),
        body.intent_id.clone(),
        body.actor.clone(),
        body.resource_id.clone(),
        body.operation.clone(),
    );
    ValidatedIntent {
        intent: body,
        idempotency_key,
        signature: None,
    }
}

pub fn mint(intent_id: &str, resource: &str, to_owner: &str) -> ValidatedIntent {
    intent(
        intent_id,
        "asset.mint",
        Some(StateType::UniqueAsset),
        0,
        resource,
        to_owner,
        &[("to_owner", serde_json::json!(to_owner))],
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn transfer(
    intent_id: &str,
    resource: &str,
    from_owner: &str,
    to_owner: &str,
    version: u64,
) -> ValidatedIntent {
    intent(
        intent_id,
        "asset.transfer",
        Some(StateType::UniqueAsset),
        version,
        resource,
        from_owner,
        &[
            ("from_owner", serde_json::json!(from_owner)),
            ("to_owner", serde_json::json!(to_owner)),
        ],
        None,
        None,
    )
}

/// Builds an `asset.transfer` validated intent that binds an authority proof
/// (`asset.transfer` is authority-required for the unique asset profile).
#[allow(clippy::too_many_arguments)]
pub fn transfer_with_authority(
    intent_id: &str,
    resource: &str,
    from_owner: &str,
    to_owner: &str,
    version: u64,
) -> ValidatedIntent {
    intent(
        intent_id,
        "asset.transfer",
        Some(StateType::UniqueAsset),
        version,
        resource,
        from_owner,
        &[
            ("from_owner", serde_json::json!(from_owner)),
            ("to_owner", serde_json::json!(to_owner)),
        ],
        Some(sample_authority()),
        None,
    )
}

pub fn lock(intent_id: &str, resource: &str, owner: &str, version: u64) -> ValidatedIntent {
    intent(
        intent_id,
        "asset.lock",
        Some(StateType::UniqueAsset),
        version,
        resource,
        owner,
        &[],
        None,
        None,
    )
}

/// Builds a `balance.transfer` validated intent debiting `amount` from
/// `from_holder`'s balance of `resource` to `to_subject` at `version`.
pub fn balance_transfer(
    intent_id: &str,
    resource: &str,
    from_holder: &str,
    to_subject: &str,
    version: u64,
    amount: &str,
) -> ValidatedIntent {
    intent(
        intent_id,
        "balance.transfer",
        Some(StateType::FungibleBalance),
        version,
        resource,
        from_holder,
        &[
            ("to_subject", serde_json::json!(to_subject)),
            ("amount", serde_json::json!(amount)),
        ],
        None,
        None,
    )
}

/// Builds a `stack.transfer` validated intent moving `quantity` from
/// `from_holder`'s stack of `resource` to `to_subject` at `version`.
pub fn stack_transfer(
    intent_id: &str,
    resource: &str,
    from_holder: &str,
    to_subject: &str,
    version: u64,
    quantity: &str,
) -> ValidatedIntent {
    intent(
        intent_id,
        "stack.transfer",
        Some(StateType::ConsumableStack),
        version,
        resource,
        from_holder,
        &[
            ("to_subject", serde_json::json!(to_subject)),
            ("quantity", serde_json::json!(quantity)),
        ],
        None,
        None,
    )
}

/// Builds a `balance.create` validated intent.
pub fn balance_create(
    intent_id: &str,
    resource: &str,
    holder: &str,
    balance: &str,
) -> ValidatedIntent {
    intent(
        intent_id,
        "balance.create",
        Some(StateType::FungibleBalance),
        0,
        resource,
        holder,
        &[
            ("subject", serde_json::json!(holder)),
            ("unit", serde_json::json!("gold_minor")),
            ("balance", serde_json::json!(balance)),
        ],
        None,
        None,
    )
}

/// Builds a `stack.create` validated intent.
pub fn stack_create(
    intent_id: &str,
    resource: &str,
    holder: &str,
    quantity: &str,
) -> ValidatedIntent {
    intent(
        intent_id,
        "stack.create",
        Some(StateType::ConsumableStack),
        0,
        resource,
        holder,
        &[
            ("subject", serde_json::json!(holder)),
            ("unit", serde_json::json!("arrows")),
            ("quantity", serde_json::json!(quantity)),
        ],
        None,
        None,
    )
}

/// Builds a validated intent for an arbitrary tenant with fixed ids and a
/// deterministic nonce (cross-tenant harness). The idempotency tuple uses the
/// intent's own tenant, so two tenants may carry the same intent id without
/// colliding in the intent store (which keys by `(tenant, intent_id)`).
#[allow(clippy::too_many_arguments)]
pub fn intent_for_tenant(
    tenant: TenantId,
    intent_id: &str,
    operation: &str,
    state_type: Option<StateType>,
    expected_version: u64,
    resource: &str,
    acting_actor: &str,
    inputs: &[(&str, serde_json::Value)],
    authority: Option<AuthorityProof>,
) -> ValidatedIntent {
    let body = Intent::new(
        tenant,
        IntentId::new(format!("int_{intent_id}")).unwrap(),
        Operation::new(String::from(operation)).unwrap(),
        SubjectId(String::from(acting_actor)),
        ResourceId(String::from(resource)),
        state_type,
        expected_version,
        inputs
            .iter()
            .map(|(key, value)| (String::from(*key), value.clone()))
            .collect(),
        authority,
        fixed_now(),
        None,
        Nonce::from_bytes(vec![0]).unwrap(),
    );
    let idempotency_key = IdempotencyKey::new(
        body.tenant_id.clone(),
        body.intent_id.clone(),
        body.actor.clone(),
        body.resource_id.clone(),
        body.operation.clone(),
    );
    ValidatedIntent {
        intent: body,
        idempotency_key,
        signature: None,
    }
}

/// Builds a `balance.create` intent for an arbitrary tenant.
#[allow(clippy::too_many_arguments)]
pub fn cross_balance_create(
    tenant: TenantId,
    intent_id: &str,
    resource: &str,
    holder: &str,
    balance: &str,
) -> ValidatedIntent {
    intent_for_tenant(
        tenant,
        intent_id,
        "balance.create",
        Some(StateType::FungibleBalance),
        0,
        resource,
        holder,
        &[
            ("subject", serde_json::json!(holder)),
            ("unit", serde_json::json!("gold_minor")),
            ("balance", serde_json::json!(balance)),
        ],
        None,
    )
}

/// Builds a `balance.transfer` intent for an arbitrary tenant.
#[allow(clippy::too_many_arguments)]
pub fn cross_balance_transfer(
    tenant: TenantId,
    intent_id: &str,
    resource: &str,
    from_holder: &str,
    to_subject: &str,
    version: u64,
    amount: &str,
) -> ValidatedIntent {
    intent_for_tenant(
        tenant,
        intent_id,
        "balance.transfer",
        Some(StateType::FungibleBalance),
        version,
        resource,
        from_holder,
        &[
            ("to_subject", serde_json::json!(to_subject)),
            ("amount", serde_json::json!(amount)),
        ],
        None,
    )
}

/// Builds an `asset.mint` intent for an arbitrary tenant.
#[allow(clippy::too_many_arguments)]
pub fn cross_mint(
    tenant: TenantId,
    intent_id: &str,
    resource: &str,
    to_owner: &str,
) -> ValidatedIntent {
    intent_for_tenant(
        tenant,
        intent_id,
        "asset.mint",
        Some(StateType::UniqueAsset),
        0,
        resource,
        to_owner,
        &[("to_owner", serde_json::json!(to_owner))],
        None,
    )
}

/// Builds an `asset.transfer` intent for an arbitrary tenant, bound with an
/// authority proof (`asset.transfer` is authority-required for the unique
/// asset profile).
#[allow(clippy::too_many_arguments)]
pub fn cross_asset_transfer(
    tenant: TenantId,
    intent_id: &str,
    resource: &str,
    from_owner: &str,
    to_owner: &str,
    version: u64,
) -> ValidatedIntent {
    intent_for_tenant(
        tenant,
        intent_id,
        "asset.transfer",
        Some(StateType::UniqueAsset),
        version,
        resource,
        from_owner,
        &[
            ("from_owner", serde_json::json!(from_owner)),
            ("to_owner", serde_json::json!(to_owner)),
        ],
        Some(sample_authority()),
    )
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

/// Key for state projections: (tenant, resource, optional subject). Subject-held
/// types are keyed by their holder (the projection's `subject` field); owner-based
/// types carry `None`.
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
    pub fn apply(&self, event: &statechronicle_domain::event::Event, state_type: StateType) {
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
                .and_then(Value::as_str)
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

    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
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
    ) -> Result<Option<statechronicle_core::digest::ContentDigest>, TenantStoreError> {
        Ok(None)
    }
}

/// The TrustGrant adapter's evaluation mode.
#[derive(Clone, Copy)]
pub enum TrustMode {
    /// Everything is allowed and fresh.
    Allow,
    /// Evaluations are denied.
    Deny,
    /// Proofs are stale.
    Stale,
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

    pub const fn deny() -> Self {
        Self {
            mode: TrustMode::Deny,
        }
    }

    pub const fn stale() -> Self {
        Self {
            mode: TrustMode::Stale,
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
            TrustMode::Deny => Err(TrustGrantError::Denied),
            TrustMode::Stale => Err(TrustGrantError::Stale),
        }
    }

    async fn check_revocation_freshness(
        &self,
        _proof: &AuthorityProof,
    ) -> Result<(), TrustGrantError> {
        match self.mode {
            TrustMode::Allow => Ok(()),
            TrustMode::Deny => Err(TrustGrantError::Denied),
            TrustMode::Stale => Err(TrustGrantError::Stale),
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

/// An executor wired to fresh fakes with the default tenant registered.
pub struct Harness {
    pub executor: Executor,
    pub index: FakeStateIndex,
    pub tenant_store: FakeTenantStore,
    pub transactions: FakeTransactionManager,
}

impl Harness {
    pub fn new(trustgrant: FakeTrustGrant) -> Self {
        // Default verifier: permissive. Signature-verification tests opt into a
        // real verifier via `with_verifier`.
        let permissive: Arc<
            dyn Fn(&SignatureBlock, &[u8]) -> Result<(), ExecutorError> + Send + Sync,
        > = Arc::new(|_block, _bytes| Ok(()));
        Self::with_verifier(trustgrant, permissive)
    }

    pub fn with_verifier(
        trustgrant: FakeTrustGrant,
        intent_verifier: Arc<
            dyn Fn(&SignatureBlock, &[u8]) -> Result<(), ExecutorError> + Send + Sync,
        >,
    ) -> Self {
        Self::from_authority_set(&[trustgrant], ProfileRegistry::baseline(), intent_verifier)
    }

    /// Wires the harness with a deployment authority set of TrustGrant
    /// adapters (protocol §18.1 step 8). An empty slice configures no
    /// authority.
    pub fn with_authority_set(trustgrants: &[FakeTrustGrant]) -> Self {
        let permissive: Arc<
            dyn Fn(&SignatureBlock, &[u8]) -> Result<(), ExecutorError> + Send + Sync,
        > = Arc::new(|_block, _bytes| Ok(()));
        Self::from_authority_set(trustgrants, ProfileRegistry::baseline(), permissive)
    }

    /// Wires the harness with an explicit profile registry and an arbitrary
    /// deployment authority set (protocol §18.1 step 8, ADR-006 §36 Q5).
    ///
    /// Lets integration tests exercise a profile that overrides the authority
    /// aggregation policy (e.g. any-of) alongside custom TrustGrant adapters.
    pub fn with_profiles_and_ports(
        profiles: ProfileRegistry,
        authority_ports: Vec<Box<dyn TrustGrantPort + Send + Sync>>,
    ) -> Self {
        let permissive: Arc<
            dyn Fn(&SignatureBlock, &[u8]) -> Result<(), ExecutorError> + Send + Sync,
        > = Arc::new(|_block, _bytes| Ok(()));
        Self::from_parts(profiles, authority_ports, permissive)
    }

    fn from_authority_set(
        trustgrants: &[FakeTrustGrant],
        profiles: ProfileRegistry,
        intent_verifier: Arc<
            dyn Fn(&SignatureBlock, &[u8]) -> Result<(), ExecutorError> + Send + Sync,
        >,
    ) -> Self {
        let authority_ports: Vec<Box<dyn TrustGrantPort + Send + Sync>> = trustgrants
            .iter()
            .map(|trustgrant| Box::new(*trustgrant) as _)
            .collect();
        Self::from_parts(profiles, authority_ports, intent_verifier)
    }

    fn from_parts(
        profiles: ProfileRegistry,
        authority_ports: Vec<Box<dyn TrustGrantPort + Send + Sync>>,
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
            authority_ports,
            Box::new(transactions.clone()),
        );
        let counter = Arc::new(AtomicU64::new(1));
        let executor = Executor::new(
            ports,
            profiles,
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
