//! Candidate-only external consumer of authenticated, uncommitted preparation.
mod common;

use async_trait::async_trait;
use statechronicle::core::canonicalize::canonicalize_and_digest;
use statechronicle::domain::intent::{Intent, Nonce};
use statechronicle::ports::authorization::{
    AuthenticatedPrincipal, AuthorizationContext, AuthorizationError, Authorizer,
};
use statechronicle::ports::key_registry::{KeyRecord, KeyStatus, MemoryKeyRegistry};
use statechronicle::{IntentId, Operation, PreparedMutation, ResourceId, StateType, SubjectId};

struct Allow;

#[async_trait]
impl Authorizer for Allow {
    async fn authorize(&self, _: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
        Ok(())
    }
}

#[tokio::test]
async fn public_preparation_never_treats_a_planning_marker_as_a_receipt() {
    let harness = common::Harness::new();
    let actor = SubjectId(String::from("account:alice"));
    let intent = Intent::builder()
        .tenant(harness.tenant())
        .intent_id(IntentId::new(String::from("int_canonical_prepare")).unwrap())
        .operation(Operation::from_static("asset.mint"))
        .actor(actor.clone())
        .resource(ResourceId(String::from("asset:prepared")))
        .state_type(StateType::UniqueAsset)
        .expected_version(0)
        .inputs([(String::from("to_owner"), serde_json::json!("account:alice"))].into())
        .created_at(harness.now())
        .nonce(Nonce::from_bytes(vec![1]).unwrap())
        .build()
        .unwrap();
    let request = harness.sign(intent, None);
    let principal = AuthenticatedPrincipal {
        subject: actor,
        tenant: harness.tenant(),
        credential_id: String::from("independently-authenticated-session"),
    };
    let registry = MemoryKeyRegistry::new();
    registry
        .register(KeyRecord {
            key_id: harness.key_id(),
            tenant: harness.tenant(),
            subject: Some(principal.subject.clone()),
            operations: vec![request.intent.operation.clone()],
            valid_from: harness.now(),
            valid_until: None,
            status: KeyStatus::Active,
        })
        .unwrap();
    // Legacy execution records a marker, without a ledger transaction.
    assert_eq!(harness.executor.execute(&request).await.unwrap().len(), 1);
    assert!(harness.executor.execute(&request).await.unwrap().is_empty());
    let prepared: PreparedMutation = harness
        .executor
        .prepare_player(&request, &principal, &Allow, &registry)
        .await
        .unwrap();
    assert_eq!(prepared.events().len(), 1);
    assert_eq!(prepared.validated(), &request);
    assert_eq!(prepared.principal(), &principal);
    assert_eq!(
        prepared.payload_digest(),
        &canonicalize_and_digest(&request.intent).unwrap()
    );
}
