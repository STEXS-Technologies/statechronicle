//! Consumer composition with only read adapters; no intent or transaction adapter.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn
)]
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use statechronicle::core::canonicalize::canonicalize;
use statechronicle::core::digest::ContentDigest;
use statechronicle::core::signature::{sign, verify};
use statechronicle::domain::intent::{Intent, KeyId, Nonce, SignatureAlg, SignatureBlock};
use statechronicle::domain::state::StateProjection;
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::mutation::{PreparationConfig, PreparationPorts, Preparer};
use statechronicle::ports::authorization::{
    AuthenticatedPrincipal, AuthorizationContext, AuthorizationError, Authorizer,
};
use statechronicle::ports::key_registry::{KeyRecord, KeyRegistry, KeyRegistryError, KeyStatus};
use statechronicle::ports::state_index::{StateIndex, StateIndexError};
use statechronicle::ports::tenant_store::{TenantStore, TenantStoreError};
use statechronicle::profiles::registry::ProfileRegistry;
use statechronicle::{EventId, IntentId, Operation, ResourceId, StateType, SubjectId, TenantId};

pub(crate) type Trace = Arc<Mutex<Vec<&'static str>>>;

pub(crate) fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}
pub(crate) fn tenant() -> TenantId {
    TenantId(String::from("acme.game.alpha"))
}
pub(crate) fn key_id() -> KeyId {
    KeyId::new(String::from("test:readonly")).unwrap()
}
pub(crate) fn principal() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: SubjectId(String::from("account:alice")),
        tenant: tenant(),
        credential_id: String::from("host-authenticated-session"),
    }
}

pub(crate) struct Reads(pub Trace);
#[async_trait]
impl StateIndex for Reads {
    async fn get_state(
        &self,
        _: &TenantId,
        _: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        self.0.lock().unwrap().push("state");
        Ok(None)
    }
    async fn get_subject_state(
        &self,
        _: &TenantId,
        _: &SubjectId,
        _: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        panic!("unique-asset mint must use its resource address")
    }
}
#[async_trait]
impl TenantStore for Reads {
    async fn tenant_exists(&self, expected: &TenantId) -> Result<bool, TenantStoreError> {
        self.0.lock().unwrap().push("tenant");
        Ok(expected == &tenant())
    }
    async fn get_tenant_root(
        &self,
        _: &TenantId,
    ) -> Result<Option<ContentDigest>, TenantStoreError> {
        panic!("preparation must not load a history checkpoint")
    }
}

pub(crate) struct Policy {
    pub trace: Trace,
    pub deny: bool,
}
#[async_trait]
impl Authorizer for Policy {
    async fn authorize(&self, _: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
        self.trace.lock().unwrap().push("authorize");
        if self.deny {
            Err(AuthorizationError::Denied)
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl KeyRegistry for Policy {
    async fn resolve_intent_key(
        &self,
        _: &TenantId,
        _: &SubjectId,
        _: &KeyId,
        _: &Operation,
        _: DateTime<Utc>,
    ) -> Result<KeyRecord, KeyRegistryError> {
        self.trace.lock().unwrap().push("key");
        if self.deny {
            return Err(KeyRegistryError::NotTrusted);
        }
        Ok(KeyRecord {
            key_id: key_id(),
            tenant: tenant(),
            subject: Some(principal().subject),
            operations: vec![Operation::from_static("asset.mint")],
            valid_from: now(),
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
        panic!("preparation must not resolve commit signer trust")
    }
}

pub(crate) fn config(trace: &Trace) -> PreparationConfig {
    let clock_trace = trace.clone();
    let event_trace = trace.clone();
    let signature_trace = trace.clone();
    PreparationConfig {
        profiles: ProfileRegistry::baseline(),
        executor: SubjectId(String::from("service:readonly")),
        now: Box::new(move || {
            clock_trace.lock().unwrap().push("clock");
            now()
        }),
        event_id_fn: Box::new(move || {
            let mut recorded = event_trace.lock().unwrap();
            let count = recorded.iter().filter(|entry| **entry == "event").count();
            recorded.push("event");
            EventId::new(format!("evt_readonly_{count}")).unwrap()
        }),
        intent_verifier: Arc::new(move |signature: &SignatureBlock, bytes: &[u8]| {
            signature_trace.lock().unwrap().push("signature");
            if signature.key_id != key_id() {
                return Err(ExecutorError::ActorAuthenticationFailed(String::from(
                    "wrong key",
                )));
            }
            verify(
                bytes,
                &SigningKey::from_bytes(&[71; 32]).verifying_key(),
                &signature.sig,
            )
            .map_err(|err| ExecutorError::ActorAuthenticationFailed(err.to_string()))
        }),
    }
}
pub(crate) fn preparer(trace: &Trace) -> Preparer {
    Preparer::new(
        PreparationPorts {
            state_index: Box::new(Reads(trace.clone())),
            tenant_store: Box::new(Reads(trace.clone())),
            trustgrant: Vec::new(),
        },
        config(trace),
    )
}
pub(crate) fn request(name: &str) -> ValidatedIntent {
    let intent = Intent::builder()
        .tenant(tenant())
        .intent_id(IntentId::new(format!("int_readonly_{name}")).unwrap())
        .operation(Operation::from_static("asset.mint"))
        .actor(principal().subject)
        .resource(ResourceId(format!("asset:{name}")))
        .state_type(StateType::UniqueAsset)
        .expected_version(0)
        .inputs([(String::from("to_owner"), serde_json::json!("account:alice"))].into())
        .created_at(now())
        .nonce(Nonce::from_bytes(vec![1]).unwrap())
        .build()
        .unwrap();
    let signature = sign(
        &canonicalize(&intent).unwrap(),
        &SigningKey::from_bytes(&[71; 32]),
    );
    ValidatedIntent::try_from_intent(
        intent,
        Some(SignatureBlock {
            alg: SignatureAlg::Ed25519,
            key_id: key_id(),
            sig: signature,
        }),
    )
    .unwrap()
}
pub(crate) fn policies(trace: &Trace) -> (Policy, Policy) {
    (
        Policy {
            trace: trace.clone(),
            deny: false,
        },
        Policy {
            trace: trace.clone(),
            deny: false,
        },
    )
}
