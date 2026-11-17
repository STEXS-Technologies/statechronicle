//! Development composition. Fixed identities/keys stand in for host login and KMS.
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use serde::{Serialize, de::DeserializeOwned};
use statechronicle::core::{
    canonicalize::canonicalize,
    digest::ContentDigest,
    signature::{sign, verify},
};
use statechronicle::domain::intent::{Intent, KeyId, Nonce, SignatureAlg, SignatureBlock};
use statechronicle::domain::state::StateProjection;
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::mutation::{PreparationConfig, PreparationPorts, Preparer};
use statechronicle::ports::authorization::{
    AuthenticatedPrincipal, AuthorizationContext, AuthorizationError, Authorizer,
};
use statechronicle::ports::key_registry::{KeyRecord, KeyStatus, MemoryKeyRegistry};
use statechronicle::ports::state_index::{StateIndex, StateIndexError};
use statechronicle::ports::tenant_store::{TenantStore, TenantStoreError};
use statechronicle::{
    EventId, IntentId, Operation, ResourceId, StateProfile, StateType, SubjectId, TenantId,
    TypedProfile,
};
use std::{
    error::Error,
    fmt::Display,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::domains;

fn tenant() -> TenantId {
    TenantId(String::from("example.native"))
}
fn actor() -> SubjectId {
    SubjectId(String::from("account:operator"))
}
fn key_id() -> Result<KeyId, Box<dyn Error>> {
    Ok(KeyId::new(String::from("development:operator"))?)
}
fn now() -> Result<DateTime<Utc>, Box<dyn Error>> {
    Ok(DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")?.with_timezone(&Utc))
}
fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[93; 32])
}

/// Default-deny policy: only this independent operator session can create either resource.
pub struct OperatorPolicy;
#[async_trait]
impl Authorizer for OperatorPolicy {
    async fn authorize(&self, context: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
        let allowed = context.principal.subject == actor()
            && context.principal.tenant == tenant()
            && context.principal.credential_id == "development-session"
            && context.tenant == &tenant()
            && match context.intent.operation.as_str() {
                "configuration.create" => context.resource.0 == "configuration:main",
                "equipment.install" => context.resource.0 == "equipment:main",
                _ => false,
            };
        if allowed {
            Ok(())
        } else {
            Err(AuthorizationError::Denied)
        }
    }
}

/// Empty development projections. Read methods never claim or store an intent.
struct EmptyReads;
#[async_trait]
impl StateIndex for EmptyReads {
    async fn get_state(
        &self,
        _: &TenantId,
        _: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        Ok(None)
    }
    async fn get_subject_state(
        &self,
        _: &TenantId,
        _: &SubjectId,
        _: &ResourceId,
    ) -> Result<Option<StateProjection>, StateIndexError> {
        Ok(None)
    }
}
#[async_trait]
impl TenantStore for EmptyReads {
    async fn tenant_exists(&self, expected: &TenantId) -> Result<bool, TenantStoreError> {
        Ok(expected == &tenant())
    }
    async fn get_tenant_root(
        &self,
        _: &TenantId,
    ) -> Result<Option<ContentDigest>, TenantStoreError> {
        Ok(None)
    }
}

pub struct Application {
    pub preparer: Preparer,
    pub principal: AuthenticatedPrincipal,
    pub keys: MemoryKeyRegistry,
    pub policy: OperatorPolicy,
    clock: DateTime<Utc>,
}
impl Application {
    /// Fixed development credentials are intentionally visible; replace at the host boundary.
    pub fn development() -> Result<Self, Box<dyn Error>> {
        let clock = now()?;
        let principal = AuthenticatedPrincipal {
            subject: actor(),
            tenant: tenant(),
            credential_id: String::from("development-session"),
        };
        let keys = MemoryKeyRegistry::new();
        keys.register(KeyRecord {
            key_id: key_id()?,
            tenant: tenant(),
            subject: Some(actor()),
            operations: vec![
                Operation::from_static("configuration.create"),
                Operation::from_static("equipment.install"),
            ],
            valid_from: clock,
            valid_until: None,
            status: KeyStatus::Active,
        })?;
        let event_counter = AtomicU64::new(0);
        let preparer = Preparer::new(
            PreparationPorts {
                state_index: Box::new(EmptyReads),
                tenant_store: Box::new(EmptyReads),
                trustgrant: Vec::new(),
            },
            PreparationConfig {
                profiles: domains::registry()?,
                executor: SubjectId(String::from("service:native-tutorial")),
                now: Box::new(move || clock),
                event_id_fn: Box::new(move || {
                    // Prefix plus decimal u64 always satisfies EventId's bounded identifier grammar.
                    EventId::new(format!(
                        "evt_native_{}",
                        event_counter.fetch_add(1, Ordering::Relaxed)
                    ))
                    .expect("valid generated event identifier")
                }),
                intent_verifier: Arc::new(|signature, body| {
                    let expected = KeyId::new(String::from("development:operator"))
                        .map_err(|err| ExecutorError::ActorAuthenticationFailed(err.to_string()))?;
                    if signature.key_id != expected || signature.alg != SignatureAlg::Ed25519 {
                        return Err(ExecutorError::ActorAuthenticationFailed(String::from(
                            "untrusted intent key",
                        )));
                    }
                    verify(body, &signing_key().verifying_key(), &signature.sig)
                        .map_err(|err| ExecutorError::ActorAuthenticationFailed(err.to_string()))
                }),
            },
        );
        Ok(Self {
            preparer,
            principal,
            keys,
            policy: OperatorPolicy,
            clock,
        })
    }

    /// Encode a native command once, bind its returned operation and sign canonical bytes.
    pub fn request<P>(
        &self,
        resource: &str,
        profile: &TypedProfile<P>,
        command: &P::Command,
    ) -> Result<ValidatedIntent, Box<dyn Error>>
    where
        P: StateProfile,
        P::State: Serialize + DeserializeOwned,
        P::Command: Serialize + DeserializeOwned,
        P::Error: Display,
    {
        let (operation, inputs) = profile.encode_command(command)?;
        let intent = Intent::builder()
            .tenant(tenant())
            .intent_id(IntentId::new(format!("int_native_{resource}"))?)
            .actor(actor())
            .resource(ResourceId(format!("{resource}:main")))
            .state_type(StateType::Custom)
            .operation(operation)
            .inputs(inputs)
            .expected_version(0)
            .created_at(self.clock)
            .nonce(Nonce::from_bytes(vec![1])?)
            .build()?;
        let sig = sign(&canonicalize(&intent)?, &signing_key());
        Ok(ValidatedIntent::try_from_intent(
            intent,
            Some(SignatureBlock {
                alg: SignatureAlg::Ed25519,
                key_id: key_id()?,
                sig,
            }),
        )?)
    }
}
