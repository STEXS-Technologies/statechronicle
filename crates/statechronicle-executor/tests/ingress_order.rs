//! Multi-fault ordering fixtures supported by the unmodified original release.
//! Signatures are placeholders: every case must refuse before crypto/execution.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod common;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::{FakeTrustGrant, Harness, mint, tenant};
use statechronicle_core::signature::Signature;
use statechronicle_domain::{
    event::Event,
    intent::{KeyId, Operation, SignatureAlg, SignatureBlock},
    subject::SubjectId,
    tenant::TenantId,
};
use statechronicle_executor::{
    error::ExecutorError,
    pipeline::{DurableBatchSink, DurableMutationSink, PlayerBatchItem},
};
use statechronicle_intent::validated::ValidatedIntent;
use statechronicle_ports::{
    authorization::{AuthenticatedPrincipal, AuthorizationContext, AuthorizationError, Authorizer},
    key_registry::{KeyRecord, KeyRegistry, KeyRegistryError},
};
use std::sync::{Arc, Mutex};

type Trace = Arc<Mutex<Vec<String>>>;
struct DenyRegistry(Trace);
#[async_trait]
impl KeyRegistry for DenyRegistry {
    async fn resolve_intent_key(
        &self,
        tenant: &TenantId,
        subject: &SubjectId,
        _key: &KeyId,
        operation: &Operation,
        _at: DateTime<Utc>,
    ) -> Result<KeyRecord, KeyRegistryError> {
        self.0.lock().unwrap().push(format!(
            "key:{}:{}:{}",
            tenant.0,
            subject.0,
            operation.as_str()
        ));
        Err(KeyRegistryError::Unavailable(String::from(
            "registry-sentinel",
        )))
    }
    async fn resolve_commit_key(
        &self,
        _tenant: &TenantId,
        _key: &KeyId,
        _at: DateTime<Utc>,
    ) -> Result<KeyRecord, KeyRegistryError> {
        Err(KeyRegistryError::NotFound)
    }
}
struct DenyPolicy(Trace);
#[async_trait]
impl Authorizer for DenyPolicy {
    async fn authorize(
        &self,
        _context: AuthorizationContext<'_>,
    ) -> Result<(), AuthorizationError> {
        self.0.lock().unwrap().push(String::from("authorize"));
        Err(AuthorizationError::Denied)
    }
}
struct RefuseSink;
#[async_trait]
impl DurableMutationSink for RefuseSink {
    async fn persist_player_mutation(
        &self,
        _request: &ValidatedIntent,
        _principal: &AuthenticatedPrincipal,
        _events: &[Event],
    ) -> Result<(), String> {
        Err(String::from("unexpected sink"))
    }
}
#[async_trait]
impl DurableBatchSink for RefuseSink {
    async fn persist_batch(
        &self,
        _intents: &[ValidatedIntent],
        _events: &[Event],
    ) -> Result<(), String> {
        Err(String::from("unexpected sink"))
    }
}
fn request(signed: bool) -> ValidatedIntent {
    let mut request = mint("gate_order", "asset:order", "alice");
    if signed {
        request.signature = Some(SignatureBlock {
            alg: SignatureAlg::Ed25519,
            key_id: KeyId::new(String::from("did:key:order")).unwrap(),
            sig: Signature::from_bytes([0; 64]),
        });
    }
    request
}
fn principal(subject: &str) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        tenant: tenant(),
        subject: SubjectId(String::from(subject)),
        credential_id: String::from("gate-session"),
    }
}
fn failure(result: Result<Vec<Event>, ExecutorError>) -> String {
    match result {
        Err(ExecutorError::ActorAuthenticationFailed(reason)) => reason,
        other => panic!("unexpected outcome: {other:?}"),
    }
}

#[tokio::test]
async fn single_and_durable_registry_failure_precedes_principal_and_policy_failure() {
    let h = Harness::new(FakeTrustGrant::allow());
    let trace = Trace::default();
    let registry = DenyRegistry(trace.clone());
    let policy = DenyPolicy(trace.clone());
    let request = request(true);
    let caller = principal("mallory");
    let reason = failure(
        h.executor
            .execute_player_with_key_registry(&request, &caller, &policy, &registry)
            .await,
    );
    assert_eq!(reason, "key registry unavailable: registry-sentinel");
    assert_eq!(
        *trace.lock().unwrap(),
        vec!["key:acme.game.alpha:mallory:asset.mint"]
    );
    trace.lock().unwrap().clear();
    let reason = failure(
        h.executor
            .execute_player_durable_with_key_registry(
                &request,
                &caller,
                &policy,
                &registry,
                &RefuseSink,
            )
            .await,
    );
    assert_eq!(reason, "key registry unavailable: registry-sentinel");
    assert_eq!(
        *trace.lock().unwrap(),
        vec!["key:acme.game.alpha:mallory:asset.mint"]
    );
}

#[tokio::test]
async fn batch_binding_and_policy_fail_before_registry_in_their_original_order() {
    let h = Harness::new(FakeTrustGrant::allow());
    let trace = Trace::default();
    let registry = DenyRegistry(trace.clone());
    let policy = DenyPolicy(trace.clone());
    let request = request(true);
    let mismatch = principal("mallory");
    let reason = failure(
        h.executor
            .execute_player_batch_durable_with_key_registry(
                &[PlayerBatchItem {
                    validated: &request,
                    principal: &mismatch,
                }],
                &policy,
                &registry,
                &RefuseSink,
            )
            .await,
    );
    assert_eq!(
        reason,
        "authenticated principal is not bound to intent actor and tenant"
    );
    assert!(trace.lock().unwrap().is_empty());
    let matching = principal("alice");
    let reason = failure(
        h.executor
            .execute_player_batch_durable_with_key_registry(
                &[PlayerBatchItem {
                    validated: &request,
                    principal: &matching,
                }],
                &policy,
                &registry,
                &RefuseSink,
            )
            .await,
    );
    assert_eq!(reason, AuthorizationError::Denied.to_string());
    assert_eq!(*trace.lock().unwrap(), vec!["authorize"]);
}

#[tokio::test]
async fn signature_presence_precedes_registry_binding_and_policy_in_all_player_lanes() {
    let h = Harness::new(FakeTrustGrant::allow());
    let trace = Trace::default();
    let registry = DenyRegistry(trace.clone());
    let policy = DenyPolicy(trace.clone());
    let request = request(false);
    let caller = principal("mallory");
    let results = [
        h.executor.execute_player(&request, &caller, &policy).await,
        h.executor
            .execute_player_durable(&request, &caller, &policy, &RefuseSink)
            .await,
        h.executor
            .execute_player_with_key_registry(&request, &caller, &policy, &registry)
            .await,
        h.executor
            .execute_player_durable_with_key_registry(
                &request,
                &caller,
                &policy,
                &registry,
                &RefuseSink,
            )
            .await,
        h.executor
            .execute_player_batch_durable_with_key_registry(
                &[PlayerBatchItem {
                    validated: &request,
                    principal: &caller,
                }],
                &policy,
                &registry,
                &RefuseSink,
            )
            .await,
    ];
    for result in results {
        assert_eq!(
            failure(result),
            "player mutation requires a detached intent signature"
        );
    }
    assert!(trace.lock().unwrap().is_empty());
}
