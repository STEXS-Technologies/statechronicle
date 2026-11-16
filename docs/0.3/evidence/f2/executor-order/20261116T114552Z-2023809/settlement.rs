//! Original-release coordinator errors, ordering, markers and rollback warnings.
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
    atomicity::{SettleLeg, TradeManifest, ValueLeg},
    error::ExecutorError,
    pipeline::{DurableBatchSink, TrustGrantPort},
};
use statechronicle::ports::{
    intent_store::{IntentStore, IntentStoreError},
    state_index::{StateIndex, StateIndexError},
    tenant_store::{TenantStore, TenantStoreError},
    transaction_manager::{TransactionHandle, TransactionManager, TransactionManagerError},
    trustgrant_evaluator::TrustGrantError,
};
use statechronicle::{
    CommitId, ContentDigest, Event, EventId, Executor, Intent, IntentId, Operation, Ports,
    ResourceId, StateProjection, StateType, SubjectId, TenantId, ValidatedIntent,
};
use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

const FAULTS: usize = 20;
const STRUCTURES: usize = 6;
const ROUTES: usize = 8;
fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}
fn proof() -> AuthorityProof {
    AuthorityProof {
        kind: TRUSTGRANT_EVALUATION_KIND.into(),
        evaluation_digest: hash_bytes(b"client"),
        result: EvaluationResult::Allow,
        evaluated_at: now(),
    }
}
fn signed(mut intent: Intent) -> ValidatedIntent {
    intent.authority = Some(proof());
    let sig = SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id: KeyId::new("test:coordinator".into()).unwrap(),
        sig: statechronicle::core::signature::sign(
            &canonicalize(&intent).unwrap(),
            &ed25519_dalek::SigningKey::from_bytes(&[42; 32]),
        ),
    };
    ValidatedIntent::try_from_intent(intent, Some(sig)).unwrap()
}
fn body(
    tenant: &str,
    id: &str,
    resource: &str,
    operation: &str,
    version: u64,
    inputs: Value,
) -> Intent {
    let actor = if operation == "balance.transfer" && id != "int_link" {
        "account:bob"
    } else {
        "account:alice"
    };
    let kind = if operation == "balance.transfer" {
        StateType::FungibleBalance
    } else {
        StateType::UniqueAsset
    };
    let mut intent = Intent::builder()
        .tenant(TenantId(tenant.into()))
        .intent_id(IntentId::new(id.into()).unwrap())
        .actor(SubjectId(actor.into()))
        .resource(ResourceId(resource.into()))
        .operation(Operation::new(operation.into()).unwrap())
        .state_type(kind)
        .expected_version(version)
        .created_at(now())
        .nonce(Nonce::from_bytes(vec![1]).unwrap())
        .build()
        .unwrap();
    intent.inputs = serde_json::from_value(inputs).unwrap();
    intent
}
type Address = (String, String, String);
fn address(tenant: &TenantId, resource: &ResourceId, subject: Option<&SubjectId>) -> Address {
    (
        tenant.0.clone(),
        resource.0.clone(),
        subject.map_or("resource", |s| s.0.as_str()).into(),
    )
}
fn row(intent: &Intent, subject: Option<&str>, payload: Value) -> (Address, StateProjection) {
    let state = ResourceState::from_legacy_json(intent.state_type.unwrap(), payload).unwrap();
    let subject = subject.map(|s| SubjectId(s.into()));
    (
        address(&intent.tenant_id, &intent.resource_id, subject.as_ref()),
        StateProjection {
            tenant_id: intent.tenant_id.clone(),
            resource_id: intent.resource_id.clone(),
            state_type: state.state_type(),
            version: intent.expected_version,
            state_hash: canonicalize_and_digest(&state).unwrap(),
            state,
            last_event_id: EventId::new("evt_old".into()).unwrap(),
            last_commit_id: CommitId::new("cmt_old".into()).unwrap(),
        },
    )
}
fn requests(
    kind: usize,
) -> (
    Vec<Intent>,
    BTreeMap<Address, StateProjection>,
    TradeManifest,
) {
    let alpha = "acme.game.alpha";
    let beta = "acme.game.beta";
    let mint = |id, resource| {
        body(
            alpha,
            id,
            resource,
            "asset.mint",
            0,
            json!({"to_owner":"account:alice"}),
        )
    };
    let settle = body(
        alpha,
        "int_settle",
        "asset:trade",
        "trade.settle",
        2,
        json!({"from_owner":"account:alice","to_owner":"account:bob","trade_id":"trade:one","value_resource":"wallet:coin","value_amount":"3","value_to_subject":"account:alice"}),
    );
    let balance = |tenant, id, recipient| {
        body(
            tenant,
            id,
            "wallet:coin",
            "balance.transfer",
            1,
            json!({"to_subject":recipient,"amount":"3"}),
        )
    };
    let values = match kind {
        0 => vec![
            mint("int_first", "asset:first"),
            mint("int_last", "asset:last"),
        ],
        1 => vec![settle, balance(alpha, "int_value", "account:alice")],
        2 => vec![
            balance(beta, "int_link", "account:bob"),
            balance(alpha, "int_link", "account:bob"),
        ],
        _ => vec![balance(beta, "int_value", "account:alice"), settle],
    };
    let mut states = BTreeMap::new();
    for intent in &values {
        if intent.operation.as_str() == "trade.settle" {
            let (key, value) = row(
                intent,
                None,
                json!({"owner":"account:alice","status":"trade_held","trade_id":"trade:one"}),
            );
            states.insert(key, value);
        }
        if intent.operation.as_str() == "balance.transfer" {
            let source = intent.actor.0.as_str();
            let recipient = intent.inputs["to_subject"].as_str().unwrap();
            for (subject, amount) in [(source, "10"), (recipient, "2")] {
                let (key, value) = row(
                    intent,
                    Some(subject),
                    json!({"subject":subject,"balance":amount,"unit":"COIN"}),
                );
                states.insert(key, value);
            }
        }
    }
    (
        values,
        states,
        TradeManifest {
            trade_id: "trade:one".into(),
            settle_legs: vec![SettleLeg {
                asset: ResourceId("asset:trade".into()),
                settle_intent_id: IntentId::new("int_settle".into()).unwrap(),
            }],
            value_legs: vec![ValueLeg {
                resource: ResourceId("wallet:coin".into()),
                amount: "3".into(),
                to_subject: SubjectId("account:alice".into()),
            }],
        },
    )
}
#[derive(Clone)]
struct Host {
    mask: u64,
    first: (TenantId, IntentId),
    last: (TenantId, IntentId),
    bodies: Vec<Intent>,
    states: Arc<BTreeMap<Address, StateProjection>>,
    calls: Arc<Mutex<Vec<String>>>,
    claims: Arc<Mutex<Vec<Intent>>>,
    ids: Arc<AtomicU64>,
}
impl Host {
    fn has(&self, bit: usize) -> bool {
        self.mask & (1 << bit) != 0
    }
    fn record(&self, value: impl Into<String>) {
        self.calls.lock().unwrap().push(value.into());
    }
    fn is(&self, tenant: &TenantId, id: &IntentId, last: bool) -> bool {
        let target = if last { &self.last } else { &self.first };
        &target.0 == tenant && &target.1 == id
    }
    fn read(
        &self,
        tenant: &TenantId,
        resource: &ResourceId,
        subject: Option<&SubjectId>,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        let key = address(tenant, resource, subject);
        self.record(format!("state:{}:{}:{}", key.0, key.1, key.2));
        if self.has(1)
            && self.bodies.iter().any(|i| {
                self.is(&i.tenant_id, &i.intent_id, true)
                    && &i.tenant_id == tenant
                    && &i.resource_id == resource
            })
        {
            return Err(StateIndexError::Unavailable("index-sentinel".into()));
        }
        Ok(self.states.get(&key).cloned())
    }
}
#[async_trait]
impl IntentStore for Host {
    async fn put_intent(&self, tenant: &TenantId, intent: &Intent) -> Result<(), IntentStoreError> {
        self.record(format!("claim:{}:{}", tenant.0, intent.intent_id.0));
        if (self.has(8) && self.is(tenant, &intent.intent_id, false))
            || (self.has(9) && self.is(tenant, &intent.intent_id, true))
        {
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
        if self.has(18) && self.is(tenant, id, false) {
            return Err(IntentStoreError::Unavailable("marker-sentinel".into()));
        }
        let first = self.is(tenant, id, false);
        let last = self.is(tenant, id, true);
        if (first && (self.has(4) || self.has(6))) || (last && (self.has(5) || self.has(7))) {
            let mut prior = self
                .bodies
                .iter()
                .find(|i| &i.tenant_id == tenant && &i.intent_id == id)
                .unwrap()
                .clone();
            if (first && self.has(6)) || (last && self.has(7)) {
                prior.nonce = Nonce::from_bytes(vec![2]).unwrap();
            }
            return Ok(Some(prior));
        }
        Ok(self
            .claims
            .lock()
            .unwrap()
            .iter()
            .find(|i| &i.tenant_id == tenant && &i.intent_id == id)
            .cloned())
    }
}
#[async_trait]
impl StateIndex for Host {
    async fn get_state(
        &self,
        t: &TenantId,
        r: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        self.read(t, r, None)
    }
    async fn get_subject_state(
        &self,
        t: &TenantId,
        s: &SubjectId,
        r: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        self.read(t, r, Some(s))
    }
}
#[async_trait]
impl TenantStore for Host {
    async fn tenant_exists(&self, t: &TenantId) -> Result<bool, TenantStoreError> {
        self.record(format!("tenant:{}", t.0));
        Ok(true)
    }
    async fn get_tenant_root(
        &self,
        _: &TenantId,
    ) -> Result<Option<ContentDigest>, TenantStoreError> {
        panic!("unexpected tenant-root read")
    }
}
#[async_trait]
impl TrustGrantPort for Host {
    async fn evaluate(
        &self,
        t: &TenantId,
        _: &SubjectId,
        op: &str,
        r: &ResourceId,
    ) -> Result<TrustGrantOutcome, TrustGrantError> {
        self.record(format!("authority:{}:{}:{}", t.0, op, r.0));
        Ok(TrustGrantOutcome {
            evaluation_digest: hash_bytes(b"evaluation"),
            result: EvaluationResult::Allow,
            evaluated_at: now(),
        })
    }
    async fn check_revocation_freshness(&self, _: &AuthorityProof) -> Result<(), TrustGrantError> {
        self.record("fresh");
        Ok(())
    }
}
#[async_trait]
impl TransactionManager for Host {
    async fn begin(
        &self,
        t: &TenantId,
    ) -> Result<Box<dyn TransactionHandle + Send>, TransactionManagerError> {
        self.record(format!("begin:{}", t.0));
        self.handle()
    }
    async fn begin_multi(
        &self,
        t: &[TenantId],
    ) -> Result<Box<dyn TransactionHandle + Send>, TransactionManagerError> {
        self.record(format!(
            "begin_multi:{}",
            t.iter().map(|t| t.0.as_str()).collect::<Vec<_>>().join(",")
        ));
        self.handle()
    }
}
impl Host {
    fn handle(&self) -> Result<Box<dyn TransactionHandle + Send>, TransactionManagerError> {
        if self.has(0) {
            Err(TransactionManagerError::Unavailable(
                "begin-sentinel".into(),
            ))
        } else if self.has(19) {
            Err(TransactionManagerError::Aborted)
        } else {
            Ok(Box::new(self.clone()))
        }
    }
}
#[async_trait]
impl TransactionHandle for Host {
    async fn commit(self: Box<Self>) -> Result<(), TransactionManagerError> {
        self.record("commit");
        if self.has(11) {
            Err(TransactionManagerError::CommitFailed(
                "commit-sentinel".into(),
            ))
        } else {
            Ok(())
        }
    }
    async fn rollback(self: Box<Self>) -> Result<(), TransactionManagerError> {
        self.record("rollback");
        if self.has(12) {
            Err(TransactionManagerError::RollbackFailed(
                "rollback-sentinel".into(),
            ))
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl DurableBatchSink for Host {
    async fn persist_batch(&self, i: &[ValidatedIntent], e: &[Event]) -> Result<(), String> {
        self.record(format!(
            "sink:{}:{}:{}",
            i.len(),
            e.len(),
            canonicalize_and_digest(&e).unwrap()
        ));
        if self.has(10) {
            Err("sink-sentinel".into())
        } else {
            Ok(())
        }
    }
}
struct Warnings(Host);
struct Fields(Vec<String>);
impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }
}
impl tracing::Subscriber for Warnings {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::WARN
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = Fields(vec![]);
        event.record(&mut fields);
        fields.0.sort();
        self.0.record(format!("warn:{}", fields.0.join("|")));
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}
fn execute(runtime: &tokio::runtime::Runtime, route: usize, structure: usize, mask: u64) -> Value {
    let kind = route / 2;
    let durable = route % 2 == 1;
    let (mut bodies, mut states, mut manifest) = requests(kind);
    if structure == 1 {
        bodies.clear();
    }
    if structure == 2 {
        bodies.truncate(1);
    }
    if structure == 3 {
        bodies.reverse();
    }
    if structure == 4 {
        bodies.push(bodies[0].clone());
    }
    if structure == 5 {
        for i in &mut bodies {
            i.tenant_id = TenantId("acme.game.alpha".into());
        }
    }
    if let Some(last) = bodies.last_mut() {
        if mask & (1 << 13) != 0 {
            last.expected_version += 1;
        }
        if mask & (1 << 15) != 0 {
            for state in states
                .values_mut()
                .filter(|s| s.tenant_id == last.tenant_id && s.resource_id == last.resource_id)
            {
                state.version = u64::MAX;
            }
            last.expected_version = u64::MAX;
        }
        if mask & (1 << 17) != 0 {
            last.created_at = now() - Duration::seconds(3);
            last.expires_at = Some(now() - Duration::seconds(1));
        }
    }
    if mask & (1 << 16) != 0 {
        if kind == 0
            && let Some(last) = bodies.last_mut()
        {
            last.tenant_id = TenantId("acme.game.beta".into());
        }
        if kind == 1
            && let Some(settle) = bodies
                .iter_mut()
                .find(|i| i.operation.as_str() == "trade.settle")
        {
            settle.inputs.insert("value_amount".into(), json!("4"));
        }
        if kind == 2
            && let Some(last) = bodies.last_mut()
        {
            last.intent_id = IntentId::new("int_unlinked".into()).unwrap();
        }
        if kind == 3 {
            manifest.value_legs[0].amount = "4".into();
        }
    }
    for intent in &mut bodies {
        intent.authority = Some(proof());
    }
    let identity = |i: Option<&Intent>| {
        i.map(|i| (i.tenant_id.clone(), i.intent_id.clone()))
            .unwrap_or((
                TenantId("none".into()),
                IntentId::new("int_none".into()).unwrap(),
            ))
    };
    let host = Host {
        mask,
        first: identity(bodies.first()),
        last: identity(bodies.last()),
        bodies: bodies.clone(),
        states: Arc::new(states),
        calls: Arc::default(),
        claims: Arc::default(),
        ids: Arc::new(AtomicU64::new(1)),
    };
    let requests: Vec<_> = bodies.into_iter().map(signed).collect();
    let clock = host.clone();
    let ids = host.clone();
    let verifier = host.clone();
    let executor = Executor::builder()
        .ports(
            Ports::builder()
                .intent_store(Box::new(host.clone()))
                .state_index(Box::new(host.clone()))
                .tenant_store(Box::new(host.clone()))
                .trustgrant(vec![Box::new(host.clone())])
                .transaction_manager(Box::new(host.clone()))
                .build()
                .unwrap(),
        )
        .executor(SubjectId("service:coordinator".into()))
        .clock(move || {
            clock.record("clock");
            now()
        })
        .event_id_gen(move || {
            let id = ids.ids.fetch_add(1, Ordering::SeqCst);
            ids.record(format!("id:{id}"));
            EventId::new(format!("evt_{}", if ids.has(14) { 1 } else { id })).unwrap()
        })
        .intent_verifier(Arc::new(move |block, bytes| {
            let intent = verifier
                .bodies
                .iter()
                .find(|intent| canonicalize(*intent).unwrap() == bytes)
                .unwrap();
            verifier.record(format!(
                "verify:{}:{}",
                intent.tenant_id.0, intent.intent_id.0
            ));
            if (verifier.has(2) && verifier.is(&intent.tenant_id, &intent.intent_id, false))
                || (verifier.has(3) && verifier.is(&intent.tenant_id, &intent.intent_id, true))
            {
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
    let _warnings = tracing::subscriber::set_default(Warnings(host.clone()));
    let result: Result<(Vec<usize>, ContentDigest), ExecutorError> = runtime.block_on(async {
        match kind {
            0 => {
                let events = if durable {
                    executor.execute_batch_durable(&requests, &host).await
                } else {
                    executor.execute_batch(&requests).await
                }?;
                Ok((
                    vec![events.len()],
                    canonicalize_and_digest(&events).unwrap(),
                ))
            }
            1 => {
                let events = if durable {
                    executor.execute_settle_durable(&requests, &host).await
                } else {
                    executor.execute_settle(&requests).await
                }?;
                Ok((
                    vec![events.len()],
                    canonicalize_and_digest(&events).unwrap(),
                ))
            }
            _ => {
                let groups = if kind == 2 {
                    if durable {
                        executor
                            .execute_cross_tenant_durable(&requests, &host)
                            .await
                    } else {
                        executor.execute_cross_tenant(&requests).await
                    }
                } else if durable {
                    executor
                        .execute_cross_tenant_trade_durable(&requests, &manifest, &host)
                        .await
                } else {
                    executor
                        .execute_cross_tenant_trade(&requests, &manifest)
                        .await
                }?;
                let sizes = groups.iter().map(|g| g.events.len()).collect();
                let groups: Vec<_> = groups.into_iter().map(|g| (g.tenant, g.events)).collect();
                Ok((sizes, canonicalize_and_digest(&groups).unwrap()))
            }
        }
    });
    if structure == 0 && mask == 0 {
        let expected = match kind {
            0 => vec![2],
            1 => vec![3],
            2 => vec![2, 2],
            _ => vec![1, 2],
        };
        assert!(
            matches!(&result,Ok((sizes,_))if sizes==&expected),
            "positive control route {route}: {result:?}"
        );
    }
    let outcome = match result {
        Ok((sizes, digest)) => json!({"sizes":sizes,"canonical_sha256":digest}),
        Err(error) => json!({"debug":format!("{error:?}"),"display":error.to_string()}),
    };
    let claims = host.claims.lock().unwrap();
    let claim_digest = canonicalize_and_digest(&*claims).unwrap();
    let claim_count = claims.len();
    drop(claims);
    json!({"case":[route,structure,mask],"outcome":outcome,"calls":*host.calls.lock().unwrap(),"claim_count":claim_count,"claims":claim_digest,"consumed_ids":host.ids.load(Ordering::SeqCst)-1})
}
fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut masks = vec![0];
    for a in 0..FAULTS {
        masks.push(1 << a);
        for b in a + 1..FAULTS {
            masks.push((1 << a) | (1 << b));
        }
    }
    assert_eq!(masks.len(), 211);
    let mut trace = vec![];
    for route in 0..ROUTES {
        for structure in 0..STRUCTURES {
            for mask in &masks {
                trace.push(execute(&runtime, route, structure, *mask));
            }
        }
    }
    assert_eq!(trace.len(), 10128);
    println!("{}", serde_json::to_string(&trace).unwrap());
}
