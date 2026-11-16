//! Development principal, actor key, independent commit authority and composition.
use crate::{domains, ledger::FileLedger};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use serde::{Serialize, de::DeserializeOwned};
use statechronicle::accumulator::sparse_merkle::StateAccumulator;
use statechronicle::commit::{
    batch::CommitBatch,
    builder::CommitBuilder,
    error::CommitError,
    persist::{SignedCommitVerifier, validate_tenant_commit},
    sign::{CommitSigner, verify_commit},
};
use statechronicle::core::{
    canonicalize::canonicalize,
    signature::{Signature, sign, verify},
};
use statechronicle::domain::{
    commit::CommitScope,
    intent::{Intent, KeyId, Nonce, SignatureAlg, SignatureBlock},
};
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::mutation::{PreparationConfig, PreparationPorts, Preparer, SignedMutation};
use statechronicle::ports::{
    authorization::{AuthenticatedPrincipal, AuthorizationContext, AuthorizationError, Authorizer},
    key_registry::{KeyRecord, KeyRegistry, KeyRegistryError, KeyStatus, MemoryKeyRegistry},
    ledger_store::OutboxRecord,
};
use statechronicle::{
    Commit, CommitId, ContentDigest, EventId, IntentId, Operation, ProfileId, ResourceId, Signed,
    StateProfile, StateType, SubjectId, TenantId, TypedProfile,
};
use std::{
    error::Error,
    fmt::Display,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub fn tenant() -> TenantId {
    TenantId(String::from("example.durable"))
}
fn actor() -> SubjectId {
    SubjectId(String::from("account:operator"))
}
fn executor() -> SubjectId {
    SubjectId(String::from("service:durable-tutorial"))
}
fn actor_key_id() -> Result<KeyId, Box<dyn Error>> {
    Ok(KeyId::new(String::from("development:operator"))?)
}
fn commit_key_id() -> Result<KeyId, CommitError> {
    KeyId::new(String::from("development:commit-authority")).map_err(CommitError::from)
}
fn now() -> Result<DateTime<Utc>, Box<dyn Error>> {
    Ok(DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")?.with_timezone(&Utc))
}
fn actor_key() -> SigningKey {
    SigningKey::from_bytes(&[93; 32])
}
fn commit_key() -> SigningKey {
    SigningKey::from_bytes(&[17; 32])
}
pub fn trusted_commit_key() -> ed25519_dalek::VerifyingKey {
    commit_key().verifying_key()
}

pub struct OperatorPolicy {
    keys: MemoryKeyRegistry,
    at: DateTime<Utc>,
}
#[async_trait]
impl Authorizer for OperatorPolicy {
    async fn authorize(&self, context: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
        let allowed = context.principal.subject == actor()
            && context.principal.tenant == tenant()
            && context.principal.credential_id == "development-session"
            && context.tenant == &tenant()
            && match context.intent.operation.as_str() {
                "configuration.create" | "configuration.set_enabled" => {
                    context.resource.0 == "configuration:main"
                }
                "equipment.install" | "equipment.start" => context.resource.0 == "equipment:main",
                _ => false,
            };
        if !allowed {
            return Err(AuthorizationError::Denied);
        }
        let key =
            actor_key_id().map_err(|error| AuthorizationError::Unavailable(error.to_string()))?;
        self.keys
            .resolve_intent_key(
                context.tenant,
                &context.principal.subject,
                &key,
                &context.intent.operation,
                self.at,
            )
            .await
            .map_err(|error| match error {
                KeyRegistryError::Unavailable(message) => AuthorizationError::Unavailable(message),
                _ => AuthorizationError::Denied,
            })?;
        Ok(())
    }
}
/// Fixed fixture authority. Real deployments independently resolve tenant/time/revocation policy.
pub struct CommitAuthority;
impl CommitSigner for CommitAuthority {
    fn sign_bytes(&self, key_id: &KeyId, canonical: &[u8]) -> Result<Signature, CommitError> {
        if key_id != &commit_key_id()? {
            return Err(CommitError::InvalidEvent(String::from(
                "unknown commit signer",
            )));
        }
        Ok(sign(canonical, &commit_key()))
    }
}
impl SignedCommitVerifier for CommitAuthority {
    fn verify(&self, signed: &Signed<Commit>) -> Result<(), String> {
        let scope = validate_tenant_commit(&signed.body).map_err(|error| error.to_string())?;
        if scope != &tenant()
            || signed.signature.key_id != commit_key_id().map_err(|error| error.to_string())?
            || signed.body.executor != executor()
            || signed.body.profile
                != ProfileId::new(String::from("example.durable"))
                    .map_err(|error| error.to_string())?
        {
            return Err(String::from("untrusted tenant/key/executor/profile"));
        }
        verify_commit(signed, &trusted_commit_key()).map_err(|error| error.to_string())
    }
}

pub struct Application {
    pub ledger: FileLedger,
    pub preparer: Preparer,
    pub principal: AuthenticatedPrincipal,
    pub keys: MemoryKeyRegistry,
    pub policy: OperatorPolicy,
    clock: DateTime<Utc>,
}
impl Application {
    pub fn new(ledger: FileLedger) -> Result<Self, Box<dyn Error>> {
        let clock = now()?;
        let principal = AuthenticatedPrincipal {
            subject: actor(),
            tenant: tenant(),
            credential_id: String::from("development-session"),
        };
        let keys = MemoryKeyRegistry::new();
        keys.register(KeyRecord {
            key_id: actor_key_id()?,
            tenant: tenant(),
            subject: Some(actor()),
            operations: vec![
                Operation::from_static("configuration.create"),
                Operation::from_static("configuration.set_enabled"),
                Operation::from_static("equipment.install"),
                Operation::from_static("equipment.start"),
            ],
            valid_from: clock,
            valid_until: None,
            status: KeyStatus::Active,
        })?;
        let count = u64::try_from(ledger.records()?.len())?;
        let event_counter = AtomicU64::new(count);
        let preparer = Preparer::new(
            PreparationPorts {
                state_index: Box::new(ledger.clone()),
                tenant_store: Box::new(ledger.clone()),
                trustgrant: Vec::new(),
            },
            PreparationConfig {
                profiles: domains::registry()?,
                executor: executor(),
                now: Box::new(move || clock),
                event_id_fn: Box::new(move || {
                    EventId::new(format!(
                        "evt_durable_{}",
                        event_counter.fetch_add(1, Ordering::Relaxed)
                    ))
                    .expect("valid bounded generated identifier")
                }),
                intent_verifier: Arc::new(|signature, bytes| {
                    let expected =
                        KeyId::new(String::from("development:operator")).map_err(|error| {
                            ExecutorError::ActorAuthenticationFailed(error.to_string())
                        })?;
                    if signature.key_id != expected {
                        return Err(ExecutorError::ActorAuthenticationFailed(String::from(
                            "unknown actor key",
                        )));
                    }
                    verify(bytes, &actor_key().verifying_key(), &signature.sig).map_err(|error| {
                        ExecutorError::ActorAuthenticationFailed(error.to_string())
                    })
                }),
            },
        );
        let policy = OperatorPolicy {
            keys: keys.clone(),
            at: clock,
        };
        Ok(Self {
            ledger,
            preparer,
            principal,
            keys,
            policy,
            clock,
        })
    }
    pub fn request<P>(
        &self,
        original_id: &str,
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
        let resource = ResourceId(format!("{resource}:main"));
        let expected_version = self
            .ledger
            .projection(&resource)?
            .map_or(0, |row| row.version);
        let intent = Intent::builder()
            .tenant(tenant())
            .intent_id(IntentId::new(String::from(original_id))?)
            .actor(actor())
            .resource(resource)
            .state_type(StateType::Custom)
            .operation(operation)
            .inputs(inputs)
            .expected_version(expected_version)
            .created_at(self.clock)
            .nonce(Nonce::from_bytes(vec![1])?)
            .build()?;
        let signature = SignatureBlock {
            alg: SignatureAlg::Ed25519,
            key_id: actor_key_id()?,
            sig: sign(&canonicalize(&intent)?, &actor_key()),
        };
        Ok(ValidatedIntent::try_from_intent(intent, Some(signature))?)
    }
    pub async fn plan(&self, request: &ValidatedIntent) -> Result<SignedMutation, Box<dyn Error>> {
        let prepared = self
            .preparer
            .prepare_player(request, &self.principal, &self.policy, &self.keys)
            .await?;
        let scope = CommitScope::tenant(tenant());
        let mut batch = CommitBatch::new(scope.clone());
        for event in prepared.events() {
            batch.add_event(event.clone())?;
        }
        let (head, updates) = self.ledger.head_and_updates()?;
        let sequence = head.as_ref().map_or(Ok(1), |row| {
            row.body.sequence.checked_add(1).ok_or("sequence overflow")
        })?;
        let root = head.as_ref().map_or_else(
            || ContentDigest::new(*StateAccumulator::empty().root().as_bytes()),
            |row| row.body.next_state_root.clone(),
        );
        let body = CommitBuilder::builder()
            .scope(scope)
            .sequence(sequence)
            .executor(executor())
            .profile(ProfileId::new(String::from("example.durable"))?)
            .created_at(self.clock)
            .parent(head.map(|row| row.body.commit_id))
            .build(&batch, root, &updates, || {
                CommitId::new(format!("cmt_durable_{sequence}")).map_err(CommitError::from)
            })?;
        let payload = format!("changed:{}", request.intent.resource_id.0).into_bytes();
        let outbox = vec![OutboxRecord {
            delivery_key: format!("notice:{}", body.commit_id),
            tenant: tenant(),
            commit_id: body.commit_id.clone(),
            payload_digest: statechronicle::core::digest::hash_bytes(&payload),
            payload,
        }];
        Ok(SignedMutation::sign(
            prepared,
            &body,
            commit_key_id()?,
            &CommitAuthority,
            outbox,
        )?)
    }
}
