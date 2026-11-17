//! Protected batch preparation: immutable ownership, trust ordering and no writes.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod common;
use common::{FakeTrustGrant, Harness, mint};
use statechronicle_core::canonicalize::canonicalize_and_digest;

#[tokio::test]
async fn original_independent_batch_event_contract() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let requests = [
        mint("batch_prepare_a", "asset:a", "alice"),
        mint("batch_prepare_b", "asset:b", "alice"),
    ];
    let events = harness.executor.execute_batch(&requests).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        canonicalize_and_digest(&events).unwrap().to_string(),
        "sha256:51cfb77e4dbd0af5220576928ff7e0d11e0f68b5eaf8981eaced3855f721f017"
    );
}

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use statechronicle_core::{
    canonicalize::canonicalize,
    signature::{Signature, sign, verify},
};
use statechronicle_domain::{
    ids::EventId,
    intent::{KeyId, SignatureAlg, SignatureBlock},
    subject::SubjectId,
};
use statechronicle_executor::{
    error::ExecutorError,
    pipeline::{Executor, PlayerBatchItem, Ports},
    preparation::PreparedBatch,
};
use statechronicle_intent::validated::ValidatedIntent;
use statechronicle_ports::{
    authorization::{AuthenticatedPrincipal, AuthorizationContext, AuthorizationError, Authorizer},
    intent_store::{IntentStore, IntentStoreError},
    key_registry::{KeyRecord, KeyStatus, MemoryKeyRegistry},
    state_index::StateIndex,
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

struct Allow;
#[async_trait]
impl Authorizer for Allow {
    async fn authorize(&self, _: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
        Ok(())
    }
}
struct DenySecond;
#[async_trait]
impl Authorizer for DenySecond {
    async fn authorize(&self, context: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
        if context.intent.intent_id.as_str() == "int_batch_prepare_b" {
            Err(AuthorizationError::Denied)
        } else {
            Ok(())
        }
    }
}
fn signed(mut request: ValidatedIntent) -> ValidatedIntent {
    let key = SigningKey::from_bytes(&[42; 32]);
    request.signature = Some(SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id: KeyId::new(format!(
            "test:{}:{}",
            request.intent.tenant_id.0,
            request.intent.intent_id.as_str()
        ))
        .unwrap(),
        sig: sign(&canonicalize(&request.intent).unwrap(), &key),
    });
    request
}
fn requests() -> Vec<ValidatedIntent> {
    vec![
        signed(mint("batch_prepare_a", "asset:a", "alice")),
        signed(mint("batch_prepare_b", "asset:b", "alice")),
    ]
}
fn context(requests: &[ValidatedIntent]) -> (Vec<AuthenticatedPrincipal>, MemoryKeyRegistry) {
    let registry = MemoryKeyRegistry::new();
    let mut keys = BTreeSet::new();
    let principals = requests
        .iter()
        .map(|request| {
            let principal = AuthenticatedPrincipal {
                subject: request.intent.actor.clone(),
                tenant: request.intent.tenant_id.clone(),
                credential_id: "test-session".into(),
            };
            let key_id = request.signature.as_ref().unwrap().key_id.clone();
            if keys.insert(key_id.clone()) {
                registry
                    .register(KeyRecord {
                        key_id,
                        tenant: principal.tenant.clone(),
                        subject: Some(principal.subject.clone()),
                        operations: vec![request.intent.operation.clone()],
                        valid_from: request.intent.created_at,
                        valid_until: None,
                        status: KeyStatus::Active,
                    })
                    .unwrap();
            }
            principal
        })
        .collect();
    (principals, registry)
}
fn items<'request>(
    requests: &'request [ValidatedIntent],
    principals: &'request [AuthenticatedPrincipal],
) -> Vec<PlayerBatchItem<'request>> {
    requests
        .iter()
        .zip(principals)
        .map(|(validated, principal)| PlayerBatchItem {
            validated,
            principal,
        })
        .collect()
}
fn harness() -> (Harness, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let key = SigningKey::from_bytes(&[42; 32]).verifying_key();
    (
        Harness::with_verifier(
            FakeTrustGrant::allow(),
            Arc::new(move |signature, bytes| {
                observed.fetch_add(1, Ordering::SeqCst);
                verify(bytes, &key, &signature.sig)
                    .map_err(|error| ExecutorError::ActorAuthenticationFailed(error.to_string()))
            }),
        ),
        calls,
    )
}
async fn unchanged(h: &Harness, requests: &[ValidatedIntent]) {
    for request in requests {
        assert!(
            h.intent_store
                .get_intent(&request.intent.tenant_id, &request.intent.intent_id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            h.index
                .get_state(&request.intent.tenant_id, &request.intent.resource_id)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(h.transactions.log().is_empty());
}
#[tokio::test]
async fn protected_batch_matches_original_events_and_owns_every_input() {
    let (h, calls) = harness();
    let mut requests = requests();
    let (mut principals, registry) = context(&requests);
    requests[0].idempotency_key.actor = SubjectId("untrusted-cache".into());
    let batch: PreparedBatch = h
        .executor
        .prepare_player_batch(&items(&requests, &principals), &Allow, &registry)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let events: Vec<_> = batch.events().cloned().collect();
    assert_eq!(
        canonicalize_and_digest(&events).unwrap().to_string(),
        "sha256:51cfb77e4dbd0af5220576928ff7e0d11e0f68b5eaf8981eaced3855f721f017"
    );
    assert_eq!(batch.mutations().len(), 2);
    assert_eq!(
        batch.mutations()[0].validated().idempotency_key.actor.0,
        "alice"
    );
    for (mutation, request) in batch.mutations().iter().zip(&requests) {
        assert_eq!(
            mutation.payload_digest(),
            &canonicalize_and_digest(&request.intent).unwrap()
        );
    }
    unchanged(&h, &requests).await;
    requests[0].intent.actor = SubjectId("mallory".into());
    principals[1].subject = SubjectId("mallory".into());
    assert_eq!(batch.mutations()[0].validated().intent.actor.0, "alice");
    assert_eq!(batch.mutations()[1].principal().subject.0, "alice");
}
#[tokio::test]
async fn every_policy_gate_runs_before_any_leg_executes() {
    let (h, calls) = harness();
    let requests = requests();
    let (mut principals, registry) = context(&requests);
    assert!(matches!(
        h.executor
            .prepare_player_batch(&items(&requests, &principals), &DenySecond, &registry)
            .await,
        Err(ExecutorError::ActorAuthenticationFailed(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    principals[1].subject = SubjectId("mallory".into());
    assert!(matches!(
        h.executor
            .prepare_player_batch(&items(&requests, &principals), &Allow, &registry)
            .await,
        Err(ExecutorError::ActorAuthenticationFailed(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    unchanged(&h, &requests).await;
}
#[tokio::test]
async fn a_later_bad_signature_or_transition_returns_no_successful_subset() {
    for bad_signature in [true, false] {
        let (h, calls) = harness();
        let mut requests = requests();
        if bad_signature {
            requests[1].signature.as_mut().unwrap().sig = Signature::from_bytes([0; 64]);
        } else {
            requests[1].intent.expected_version = 1;
            requests[1] = signed(requests[1].clone());
        }
        let (principals, registry) = context(&requests);
        let result = h
            .executor
            .prepare_player_batch(&items(&requests, &principals), &Allow, &registry)
            .await;
        if bad_signature {
            assert!(matches!(
                result,
                Err(ExecutorError::ActorAuthenticationFailed(_))
            ));
        } else {
            assert!(matches!(
                result,
                Err(ExecutorError::ResourceNotFound { .. })
            ));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        unchanged(&h, &requests).await;
    }
}
#[tokio::test]
async fn duplicate_or_mixed_tenant_identity_refuses_before_execution() {
    for mixed in [false, true] {
        let (h, calls) = harness();
        let mut requests = requests();
        if mixed {
            requests[1].intent.tenant_id.0 = "acme.game.beta".into();
            requests[1] = signed(requests[1].clone());
        } else {
            requests[1] = requests[0].clone();
        }
        let (principals, registry) = context(&requests);
        assert!(matches!(
            h.executor
                .prepare_player_batch(&items(&requests, &principals), &Allow, &registry)
                .await,
            Err(ExecutorError::AtomicityViolation(_))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        unchanged(&h, &requests).await;
    }
}
#[tokio::test]
async fn public_structural_fields_are_rechecked_for_every_leg() {
    let (h, calls) = harness();
    let mut requests = requests();
    requests[1].intent.schema = "unknown.intent.schema".into();
    requests[1] = signed(requests[1].clone());
    let (principals, registry) = context(&requests);
    assert!(matches!(
        h.executor
            .prepare_player_batch(&items(&requests, &principals), &Allow, &registry)
            .await,
        Err(ExecutorError::Intent(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    unchanged(&h, &requests).await;
}

struct ForbiddenStore;
#[async_trait]
impl IntentStore for ForbiddenStore {
    async fn put_intent(
        &self,
        _: &statechronicle_domain::tenant::TenantId,
        _: &statechronicle_domain::intent::Intent,
    ) -> Result<(), IntentStoreError> {
        panic!("batch preparation wrote an independent marker")
    }
    async fn get_intent(
        &self,
        _: &statechronicle_domain::tenant::TenantId,
        _: &statechronicle_domain::ids::IntentId,
    ) -> Result<Option<statechronicle_domain::intent::Intent>, IntentStoreError> {
        panic!("batch preparation read an independent marker")
    }
}
fn forbidden_harness(duplicate_ids: bool) -> Harness {
    let (mut h, _) = harness();
    let ports = Ports::builder()
        .intent_store(Box::new(ForbiddenStore))
        .state_index(Box::new(h.index.clone()))
        .tenant_store(Box::new(h.tenant_store.clone()))
        .trustgrant(vec![Box::new(FakeTrustGrant::allow())])
        .transaction_manager(Box::new(h.transactions.clone()))
        .build()
        .unwrap();
    let counter = AtomicU64::new(1);
    let key = SigningKey::from_bytes(&[42; 32]).verifying_key();
    h.executor = Executor::builder()
        .ports(ports)
        .executor(common::executor_subject())
        .clock(common::fixed_now)
        .event_id_gen(move || {
            let next = counter.fetch_add(1, Ordering::SeqCst);
            EventId::new(format!("evt_{:020}", if duplicate_ids { 1 } else { next })).unwrap()
        })
        .intent_verifier(Arc::new(move |signature, bytes| {
            verify(bytes, &key, &signature.sig)
                .map_err(|error| ExecutorError::ActorAuthenticationFailed(error.to_string()))
        }))
        .build()
        .unwrap();
    h
}
#[tokio::test]
async fn no_independent_store_or_symbolic_transaction_is_called_and_batch_validation_stays_active()
{
    for duplicate_ids in [false, true] {
        let h = forbidden_harness(duplicate_ids);
        let requests = requests();
        let (principals, registry) = context(&requests);
        let result = h
            .executor
            .prepare_player_batch(&items(&requests, &principals), &Allow, &registry)
            .await;
        if duplicate_ids {
            assert!(
                matches!(result, Err(ExecutorError::AtomicityViolation(message)) if message == "duplicate event id in batch")
            );
        } else {
            assert_eq!(result.unwrap().events().count(), 2);
        }
        assert!(h.transactions.log().is_empty());
    }
}
#[tokio::test]
async fn empty_and_oversized_inputs_refuse_before_reading_any_marker() {
    let h = forbidden_harness(false);
    let requests = requests();
    let (principals, registry) = context(&requests);
    assert!(matches!(
        h.executor
            .prepare_player_batch(&[], &Allow, &registry)
            .await,
        Err(ExecutorError::AtomicityViolation(_))
    ));
    let too_many: Vec<_> = (0..257)
        .map(|_| PlayerBatchItem {
            validated: &requests[0],
            principal: &principals[0],
        })
        .collect();
    assert!(
        matches!(h.executor.prepare_player_batch(&too_many, &Allow, &registry).await, Err(ExecutorError::SizeLimitExceeded { name, actual:257, limit:256 }) if name == "batch_intents")
    );
    assert!(h.transactions.log().is_empty());
}

#[tokio::test]
async fn a_later_missing_signature_refuses_before_any_crypto_or_store_call() {
    let (h, calls) = harness();
    let mut requests = requests();
    let (principals, registry) = context(&requests);
    requests[1].signature = None;
    assert!(matches!(
        h.executor
            .prepare_player_batch(&items(&requests, &principals), &Allow, &registry)
            .await,
        Err(ExecutorError::ActorAuthenticationFailed(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    unchanged(&h, &requests).await;
}
