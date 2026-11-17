//! Public legacy executor characterization: errors, canonical events and effects.
//! Faults are sentinels supplied by a test host, never production policies.
#[allow(dead_code)]
mod domains;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use statechronicle::core::{
    canonicalize::{canonicalize, canonicalize_and_digest},
    digest::hash_bytes,
};
use statechronicle::domain::{
    authority::{AuthorityProof, EvaluationResult, TRUSTGRANT_EVALUATION_KIND, TrustGrantOutcome},
    intent::{KeyId, Nonce, SignatureAlg, SignatureBlock},
    resource_state::ResourceState,
};
use statechronicle::executor::{
    error::ExecutorError,
    pipeline::{DurableBatchSink, DurableMutationSink, PlayerBatchItem, TrustGrantPort},
};
use statechronicle::ports::{
    authorization::{AuthenticatedPrincipal, AuthorizationContext, AuthorizationError, Authorizer},
    intent_store::{IntentStore, IntentStoreError},
    key_registry::{KeyRecord, KeyRegistry, KeyRegistryError, KeyStatus},
    state_index::{StateIndex, StateIndexError},
    tenant_store::{TenantStore, TenantStoreError},
    transaction_manager::{TransactionHandle, TransactionManager, TransactionManagerError},
    trustgrant_evaluator::TrustGrantError,
};
use statechronicle::{
    CommitId, ContentDigest, Event, EventId, Executor, Intent, IntentId, Operation, Ports,
    ResourceId, StateProfile, StateProjection, StateType, SubjectId, TenantId, ValidatedIntent,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

const FAULTS: usize = 29;
const TEMPLATES: usize = 6;
const MODES: usize = 9;
fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}
fn tenant() -> TenantId {
    TenantId("acme.game.alpha".into())
}
fn authority() -> AuthorityProof {
    AuthorityProof {
        kind: TRUSTGRANT_EVALUATION_KIND.into(),
        evaluation_digest: hash_bytes(b"client"),
        result: EvaluationResult::Allow,
        evaluated_at: now(),
    }
}
fn key_id(target: bool) -> KeyId {
    KeyId::new(if target { "test:target" } else { "test:warmup" }.into()).unwrap()
}
fn principal(request: &ValidatedIntent, mismatch: bool) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        tenant: request.intent.tenant_id.clone(),
        subject: if mismatch {
            SubjectId("account:other".into())
        } else {
            request.intent.actor.clone()
        },
        credential_id: "characterization-session".into(),
    }
}
fn signed(intent: Intent, target: bool, absent: bool) -> ValidatedIntent {
    let key = ed25519_dalek::SigningKey::from_bytes(&[42; 32]);
    let sig = SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id: key_id(target),
        sig: statechronicle::core::signature::sign(&canonicalize(&intent).unwrap(), &key),
    };
    ValidatedIntent::try_from_intent(intent, (!absent).then_some(sig)).unwrap()
}
#[expect(
    clippy::too_many_arguments,
    reason = "Fixture fields remain explicit and independent"
)]
fn body(
    id: &str,
    resource: &str,
    operation: &str,
    state_type: StateType,
    version: u64,
    inputs: Value,
    delegated: bool,
) -> Intent {
    let mut intent = Intent::builder()
        .tenant(tenant())
        .intent_id(IntentId::new(id.into()).unwrap())
        .actor(SubjectId("account:alice".into()))
        .resource(ResourceId(resource.into()))
        .operation(Operation::new(operation.into()).unwrap())
        .state_type(state_type)
        .expected_version(version)
        .created_at(now())
        .nonce(Nonce::from_bytes(vec![1]).unwrap())
        .build()
        .unwrap();
    intent.inputs = serde_json::from_value(inputs).unwrap();
    intent.authority = delegated.then(authority);
    intent
}
fn projection(intent: &Intent, state: ResourceState) -> StateProjection {
    StateProjection {
        tenant_id: intent.tenant_id.clone(),
        resource_id: intent.resource_id.clone(),
        state_type: state.state_type(),
        version: 1,
        state_hash: canonicalize_and_digest(&state).unwrap(),
        state,
        last_event_id: EventId::new("evt_existing".into()).unwrap(),
        last_commit_id: CommitId::new("cmt_existing".into()).unwrap(),
    }
}
fn template(index: usize) -> (Intent, Option<StateProjection>, Option<StateProjection>) {
    let (operation, kind, version, inputs) = match index {
        0 => (
            "asset.mint",
            StateType::UniqueAsset,
            0,
            json!({"to_owner":"account:alice"}),
        ),
        1 => (
            "asset.transfer",
            StateType::UniqueAsset,
            1,
            json!({"from_owner":"account:alice","to_owner":"account:bob"}),
        ),
        2 => (
            "balance.transfer",
            StateType::FungibleBalance,
            1,
            json!({"to_subject":"account:bob","amount":"3"}),
        ),
        3 => (
            "stack.transfer",
            StateType::ConsumableStack,
            1,
            json!({"to_subject":"account:bob","quantity":"3"}),
        ),
        4 => (
            "configuration.create",
            StateType::Custom,
            0,
            json!({"profile":domains::Configuration::DOMAIN_ID,"schema_version":1,"command":canonicalize(&domains::ConfigurationCommand::Create {enabled:true}).unwrap()}),
        ),
        _ => (
            "equipment.install",
            StateType::Custom,
            0,
            json!({"profile":domains::Equipment::DOMAIN_ID,"schema_version":1,"command":canonicalize(&domains::EquipmentCommand::Install).unwrap()}),
        ),
    };
    let intent = body(
        "int_target",
        "resource:target",
        operation,
        kind,
        version,
        inputs,
        true,
    );
    let current = match index {
        1 => Some(
            ResourceState::from_legacy_json(
                kind,
                json!({"owner":"account:alice","status":"active"}),
            )
            .unwrap(),
        ),
        2 => Some(
            ResourceState::from_legacy_json(
                kind,
                json!({"subject":"account:alice","balance":"10","unit":"COIN"}),
            )
            .unwrap(),
        ),
        3 => Some(
            ResourceState::from_legacy_json(
                kind,
                json!({"subject":"account:alice","quantity":"10","unit":"ITEM"}),
            )
            .unwrap(),
        ),
        _ => None,
    };
    let companion = match index {
        2 => Some(
            ResourceState::from_legacy_json(
                kind,
                json!({"subject":"account:bob","balance":"2","unit":"COIN"}),
            )
            .unwrap(),
        ),
        3 => Some(
            ResourceState::from_legacy_json(
                kind,
                json!({"subject":"account:bob","quantity":"2","unit":"ITEM"}),
            )
            .unwrap(),
        ),
        _ => None,
    };
    let current = current.map(|s| projection(&intent, s));
    let companion = companion.map(|s| projection(&intent, s));
    (intent, current, companion)
}
#[derive(Clone)]
struct Host {
    mask: u64,
    target: Intent,
    primary: Option<StateProjection>,
    companion: Option<StateProjection>,
    trace: Arc<Mutex<Vec<String>>>,
    claims: Arc<Mutex<Vec<Intent>>>,
    ids: Arc<AtomicU64>,
}
impl Host {
    fn has(&self, bit: usize) -> bool {
        self.mask & (1 << bit) != 0
    }
    fn record(&self, message: impl Into<String>) {
        self.trace.lock().unwrap().push(message.into());
    }
    fn target(&self, intent: &Intent) -> bool {
        intent.intent_id == self.target.intent_id
    }
    fn state(
        &self,
        resource: &ResourceId,
        subject: Option<&SubjectId>,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        self.record(format!(
            "state:{}:{}",
            resource.0,
            subject.map_or("resource", |s| s.0.as_str())
        ));
        if resource != &self.target.resource_id {
            return Ok(None);
        }
        let destination = subject.is_some_and(|s| s.0 == "account:bob");
        if self.has(14) || (destination && self.has(21)) {
            return Err(StateIndexError::Unavailable("index-sentinel".into()));
        }
        Ok(if destination {
            self.companion.clone()
        } else {
            self.primary.clone()
        })
    }
}
#[async_trait]
impl IntentStore for Host {
    async fn put_intent(&self, tenant: &TenantId, intent: &Intent) -> Result<(), IntentStoreError> {
        self.record(format!("claim:{}:{}", tenant.0, intent.intent_id.0));
        if self.target(intent) && self.has(22) {
            return Err(IntentStoreError::Unavailable("claim-sentinel".into()));
        }
        self.claims.lock().unwrap().push(intent.clone());
        Ok(())
    }
    async fn get_intent(
        &self,
        tenant: &TenantId,
        id: &IntentId,
    ) -> Result<Option<Intent>, IntentStoreError> {
        self.record(format!("marker:{}:{}", tenant.0, id.0));
        if id != &self.target.intent_id {
            return Ok(None);
        }
        if self.has(5) {
            return Err(IntentStoreError::Unavailable("marker-sentinel".into()));
        }
        if self.has(6) || self.has(7) {
            let mut prior = self.target.clone();
            if self.has(7) {
                prior.nonce = Nonce::from_bytes(vec![2]).unwrap();
            }
            return Ok(Some(prior));
        }
        Ok(None)
    }
}
#[async_trait]
impl StateIndex for Host {
    async fn get_state(
        &self,
        _: &TenantId,
        resource: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        self.state(resource, None)
    }
    async fn get_subject_state(
        &self,
        _: &TenantId,
        subject: &SubjectId,
        resource: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        self.state(resource, Some(subject))
    }
}
#[async_trait]
impl TenantStore for Host {
    async fn tenant_exists(&self, tenant: &TenantId) -> Result<bool, TenantStoreError> {
        self.record(format!("tenant:{}", tenant.0));
        if self.has(12) {
            return Err(TenantStoreError::Unavailable("tenant-sentinel".into()));
        }
        Ok(!self.has(11))
    }
    async fn get_tenant_root(
        &self,
        _: &TenantId,
    ) -> Result<Option<ContentDigest>, TenantStoreError> {
        panic!("unexpected tenant root lookup")
    }
}
#[async_trait]
impl Authorizer for Host {
    async fn authorize(&self, context: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
        self.record(format!("authorize:{}", context.intent.intent_id.0));
        if self.target(context.intent) && self.has(2) {
            Err(AuthorizationError::Denied)
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl KeyRegistry for Host {
    async fn resolve_intent_key(
        &self,
        tenant: &TenantId,
        subject: &SubjectId,
        key: &KeyId,
        op: &Operation,
        at: DateTime<Utc>,
    ) -> Result<KeyRecord, KeyRegistryError> {
        self.record(format!(
            "key:{}:{}:{}:{}",
            tenant.0,
            subject.0,
            key.as_str(),
            op.as_str()
        ));
        if key == &key_id(true) && self.has(3) {
            return Err(KeyRegistryError::Unavailable("key-sentinel".into()));
        }
        Ok(KeyRecord {
            key_id: key.clone(),
            tenant: tenant.clone(),
            subject: Some(subject.clone()),
            operations: vec![op.clone()],
            valid_from: at - Duration::seconds(1),
            valid_until: None,
            status: KeyStatus::Active,
        })
    }
    async fn resolve_commit_key(
        &self,
        _: &TenantId,
        _: &KeyId,
        _: DateTime<Utc>,
    ) -> Result<KeyRecord, KeyRegistryError> {
        panic!("unexpected commit-key lookup")
    }
}
struct Authority(Host, usize);
#[async_trait]
impl TrustGrantPort for Authority {
    async fn evaluate(
        &self,
        _: &TenantId,
        _: &SubjectId,
        operation: &str,
        resource: &ResourceId,
    ) -> Result<TrustGrantOutcome, TrustGrantError> {
        self.0
            .record(format!("authority:{}:{}:{}", self.1, operation, resource.0));
        if resource == &self.0.target.resource_id {
            if self.1 == 0 && self.0.has(18) {
                return Err(TrustGrantError::Denied);
            }
            if self.1 == 1 && self.0.has(28) {
                return Err(TrustGrantError::Stale);
            }
            if self.1 == 2 && self.0.has(19) {
                return Err(TrustGrantError::Unavailable("authority-sentinel".into()));
            }
        }
        Ok(TrustGrantOutcome {
            evaluation_digest: hash_bytes(format!("evaluation-{}", self.1).as_bytes()),
            result: EvaluationResult::Allow,
            evaluated_at: now(),
        })
    }
    async fn check_revocation_freshness(
        &self,
        proof: &AuthorityProof,
    ) -> Result<(), TrustGrantError> {
        let client = proof.evaluation_digest == hash_bytes(b"client");
        self.0.record(format!(
            "fresh:{}:{}",
            self.1,
            if client { "client" } else { "aggregate" }
        ));
        if (client && self.0.has(9)) || (!client && self.0.has(27)) {
            Err(TrustGrantError::Stale)
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl TransactionManager for Host {
    async fn begin(
        &self,
        tenant: &TenantId,
    ) -> Result<Box<dyn TransactionHandle + Send>, TransactionManagerError> {
        self.record(format!("begin:{}", tenant.0));
        Ok(Box::new(self.clone()))
    }
    async fn begin_multi(
        &self,
        _: &[TenantId],
    ) -> Result<Box<dyn TransactionHandle + Send>, TransactionManagerError> {
        panic!("unexpected cross-tenant transaction")
    }
}
#[async_trait]
impl TransactionHandle for Host {
    async fn commit(self: Box<Self>) -> Result<(), TransactionManagerError> {
        self.record("transaction_commit");
        if self.has(26) {
            Err(TransactionManagerError::CommitFailed(
                "commit-sentinel".into(),
            ))
        } else {
            Ok(())
        }
    }
    async fn rollback(self: Box<Self>) -> Result<(), TransactionManagerError> {
        self.record("transaction_rollback");
        if self.has(25) {
            Err(TransactionManagerError::RollbackFailed(
                "rollback-sentinel".into(),
            ))
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl DurableMutationSink for Host {
    async fn persist_player_mutation(
        &self,
        request: &ValidatedIntent,
        _: &AuthenticatedPrincipal,
        events: &[Event],
    ) -> Result<(), String> {
        self.record(format!(
            "single_sink:{}:{}",
            request.intent.intent_id.0,
            canonicalize_and_digest(&events).unwrap()
        ));
        if self.has(23) {
            Err("sink-sentinel".into())
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl DurableBatchSink for Host {
    async fn persist_batch(
        &self,
        requests: &[ValidatedIntent],
        events: &[Event],
    ) -> Result<(), String> {
        self.record(format!(
            "batch_sink:{}:{}",
            requests.len(),
            canonicalize_and_digest(&events).unwrap()
        ));
        if self.has(23) {
            Err("sink-sentinel".into())
        } else {
            Ok(())
        }
    }
}
fn execute(
    runtime: &tokio::runtime::Runtime,
    template_index: usize,
    mode: usize,
    mask: u64,
) -> Value {
    let (mut intent, mut current, companion) = template(template_index);
    if mask & (1 << 4) != 0 {
        intent.expires_at = Some(now() - Duration::seconds(1));
    }
    if mask & (1 << 10) != 0 {
        intent.tenant_id = TenantId("".into());
    }
    if mask & (1 << 13) != 0 {
        intent.state_type = None;
    }
    if mask & (1 << 15) != 0
        && let Some(row) = &mut current
    {
        row.state_type = if row.state_type == StateType::UniqueAsset {
            StateType::FungibleBalance
        } else {
            StateType::UniqueAsset
        };
    }
    if mask & (1 << 16) != 0 {
        intent.expected_version += 1;
    }
    if mask & (1 << 17) != 0 {
        intent.authority = None;
    }
    if mask & (1 << 20) != 0 {
        intent.inputs.clear();
    }
    if mask & (1 << 24) != 0
        && let Some(row) = &mut current
    {
        row.version = u64::MAX;
        intent.expected_version = u64::MAX;
    }
    // Compatibility fields remain public; preserve malformed structural cases
    // after constructing the validated base so the execution-core gate is tested.
    let base = body(
        "int_target",
        "resource:target",
        "asset.mint",
        StateType::UniqueAsset,
        0,
        json!({"to_owner":"account:alice"}),
        false,
    );
    let mut request = signed(base, true, false);
    request.intent = intent.clone();
    request.idempotency_key = statechronicle::IdempotencyKey::new(
        intent.tenant_id.clone(),
        intent.intent_id.clone(),
        intent.actor.clone(),
        intent.resource_id.clone(),
        intent.operation.clone(),
    );
    if mask & 1 != 0 {
        request.signature = None;
    } else {
        request.signature = Some(SignatureBlock {
            alg: SignatureAlg::Ed25519,
            key_id: key_id(true),
            sig: statechronicle::core::signature::sign(
                &canonicalize(&intent).unwrap(),
                &ed25519_dalek::SigningKey::from_bytes(&[42; 32]),
            ),
        });
    }
    let host = Host {
        mask,
        target: intent,
        primary: current,
        companion,
        trace: Arc::default(),
        claims: Arc::default(),
        ids: Arc::new(AtomicU64::new(1)),
    };
    let ports = Ports::builder()
        .intent_store(Box::new(host.clone()))
        .state_index(Box::new(host.clone()))
        .tenant_store(Box::new(host.clone()))
        .transaction_manager(Box::new(host.clone()))
        .trustgrant(
            (0..3)
                .map(|i| {
                    Box::new(Authority(host.clone(), i)) as Box<dyn TrustGrantPort + Send + Sync>
                })
                .collect(),
        )
        .build()
        .unwrap();
    let clock = host.clone();
    let ids = host.clone();
    let verifier = host.clone();
    let executor = Executor::builder()
        .ports(ports)
        .profiles(domains::registry().unwrap())
        .executor(SubjectId("service:characterization".into()))
        .clock(move || {
            clock.record("clock");
            now()
        })
        .event_id_gen(move || {
            let n = ids.ids.fetch_add(1, Ordering::SeqCst);
            ids.record(format!("id:{n}"));
            EventId::new(format!("evt_characterization_{n}")).unwrap()
        })
        .intent_verifier(Arc::new(move |block, bytes| {
            verifier.record(format!("verify:{}", block.key_id.as_str()));
            if block.key_id == key_id(true) && verifier.has(8) {
                return Err(ExecutorError::ActorAuthenticationFailed(
                    "crypto-sentinel".into(),
                ));
            }
            statechronicle::core::signature::verify(
                bytes,
                &ed25519_dalek::SigningKey::from_bytes(&[42; 32]).verifying_key(),
                &block.sig,
            )
            .map_err(|e| ExecutorError::ActorAuthenticationFailed(e.to_string()))
        }))
        .build()
        .unwrap();
    let caller = principal(&request, host.has(1));
    let warmup = signed(
        body(
            "int_warmup",
            "asset:warmup",
            "asset.mint",
            StateType::UniqueAsset,
            0,
            json!({"to_owner":"account:alice"}),
            false,
        ),
        false,
        false,
    );
    let warmup_caller = principal(&warmup, false);
    let requests = [warmup.clone(), request.clone()];
    let items = [
        PlayerBatchItem {
            validated: &warmup,
            principal: &warmup_caller,
        },
        PlayerBatchItem {
            validated: &request,
            principal: &caller,
        },
    ];
    let outcome = runtime.block_on(async {
        match mode {
            0 => executor.execute(&request).await,
            1 => {
                executor
                    .execute_authenticated(&request, &caller, &host)
                    .await
            }
            2 => executor.execute_player(&request, &caller, &host).await,
            3 => {
                executor
                    .execute_player_with_key_registry(&request, &caller, &host, &host)
                    .await
            }
            4 => {
                executor
                    .execute_player_durable(&request, &caller, &host, &host)
                    .await
            }
            5 => {
                executor
                    .execute_player_durable_with_key_registry(
                        &request, &caller, &host, &host, &host,
                    )
                    .await
            }
            6 => executor.execute_batch(&requests).await,
            7 => executor.execute_batch_durable(&requests, &host).await,
            _ => {
                executor
                    .execute_player_batch_durable_with_key_registry(&items, &host, &host, &host)
                    .await
            }
        }
    });
    if mask == 0 {
        assert!(
            outcome.as_ref().is_ok_and(|events| !events.is_empty()),
            "positive control failed: template {template_index}, mode {mode}: {outcome:?}"
        );
    }
    let outcome = match outcome {
        Ok(events) => {
            json!({"count":events.len(),"event_bytes_sha256":canonicalize_and_digest(&events).unwrap()})
        }
        Err(error) => json!({"debug":format!("{error:?}"),"display":error.to_string()}),
    };
    let claims = host.claims.lock().unwrap();
    let claim_count = claims.len();
    let claim_digest = canonicalize_and_digest(&*claims).unwrap();
    drop(claims);
    let value = json!({"case":[template_index,mode,mask],"outcome":outcome,"calls":*host.trace.lock().unwrap(),"claims":claim_digest,"claim_count":claim_count,"consumed_ids":host.ids.load(Ordering::SeqCst)-1});
    value
}
fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut masks = vec![0];
    for first in 0..FAULTS {
        masks.push(1 << first);
        for second in first + 1..FAULTS {
            masks.push((1 << first) | (1 << second));
        }
    }
    assert_eq!(masks.len(), 436);
    let mut trace = Vec::new();
    for index in 0..TEMPLATES {
        for mode in 0..MODES {
            for mask in &masks {
                trace.push(execute(&runtime, index, mode, *mask));
            }
        }
    }
    assert_eq!(trace.len(), 23544);
    println!("{}", serde_json::to_string(&trace).unwrap());
}
