//! Integration tests for the execution pipeline (protocol §18.1–§18.3).
//!
//! Exercises the executor end-to-end over real domain types and the in-memory
//! fakes in [`common`]. Covers the full mint → transfer → lock lifecycle
//! (correct versions, canonical state hashes, authority binding), idempotent
//! replay, atomic batches, and every §18.2 fail-closed gate (wrong expected
//! version, unknown tenant, denied/stale authority, expired intent, duplicate
//! intent id, locked resource, insufficient quantity, missing state type).

#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::unwrap_in_result,
    clippy::panic_in_result_fn,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::type_complexity
)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;

use statechronicle_core::amount::Amount;
use statechronicle_core::canonicalize::canonicalize;
use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::ContentDigest;
use statechronicle_core::signature::{sign, verify};

use statechronicle_domain::authority::{
    AggregationPolicy, AuthorityProof, EvaluationResult, TrustGrantOutcome,
    aggregate_evaluation_digest,
};
use statechronicle_domain::intent::{KeyId, Operation, SignatureAlg, SignatureBlock};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;

use statechronicle_accumulator::key::StateKey;
use statechronicle_executor::atomicity;
use statechronicle_executor::error::ExecutorError;
use statechronicle_executor::pipeline::TrustGrantPort;
use statechronicle_executor::transition;
use statechronicle_ports::state_index::StateIndex;
use statechronicle_ports::trustgrant_evaluator::TrustGrantError;
use statechronicle_profiles::error::ProfileError;
use statechronicle_profiles::registry::{ProfileRegistry, ProfileRules};
use statechronicle_profiles::unique_asset::UniqueAssetRules;

use common::{
    FakeTrustGrant, Harness, balance_create, balance_transfer, executor_subject, intent, lock,
    mint, sample_authority, stack_create, stack_transfer, tenant, transfer,
    transfer_with_authority,
};

#[tokio::test]
async fn mint_transfer_lock_lifecycle_produces_correct_events() {
    let harness = Harness::new(FakeTrustGrant::allow());

    // Mint: no prior state, version 0 -> 1.
    let minted = harness
        .executor
        .execute(&mint("mint_001", "asset:sword_001", "alice"))
        .await
        .unwrap();
    assert_eq!(minted.len(), 1);
    let mint_event = &minted[0];
    assert_eq!(mint_event.before.version, 0);
    assert_eq!(mint_event.before.state, serde_json::json!({}));
    assert_eq!(mint_event.after.version, 1);
    assert_eq!(
        mint_event.after.state,
        serde_json::json!({ "owner": "alice", "status": "active" })
    );
    assert_eq!(
        mint_event.after.state_hash,
        canonicalize_and_digest(&mint_event.after.state).unwrap()
    );
    assert_eq!(mint_event.actor, SubjectId(String::from("alice")));
    assert_eq!(mint_event.executor, executor_subject());
    assert_eq!(mint_event.tenant_id, tenant());
    assert_eq!(mint_event.intent_id.as_str(), "int_mint_001");
    assert_eq!(mint_event.operation.as_str(), "asset.mint");
    assert!(mint_event.authority.is_none());

    harness.index.apply(mint_event, StateType::UniqueAsset);

    // Transfer: 1 -> 2, owner becomes bob. `asset.transfer` is authority-
    // required, so the transfer binds an authority proof.
    let transferred = harness
        .executor
        .execute(&transfer_with_authority(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            1,
        ))
        .await
        .unwrap();
    assert_eq!(transferred.len(), 1);
    let transfer_event = &transferred[0];
    assert_eq!(transfer_event.before.version, 1);
    assert_eq!(transfer_event.after.version, 2);
    assert_eq!(
        transfer_event.before.state,
        serde_json::json!({ "owner": "alice", "status": "active" })
    );
    assert_eq!(
        transfer_event.after.state,
        serde_json::json!({ "owner": "bob", "status": "active" })
    );
    assert_eq!(
        transfer_event.after.state_hash,
        canonicalize_and_digest(&transfer_event.after.state).unwrap()
    );
    assert!(
        transfer_event.authority.is_some(),
        "transfer binds authority"
    );

    harness.index.apply(transfer_event, StateType::UniqueAsset);

    // Lock: 2 -> 3, owner preserved, status locked.
    let locked = harness
        .executor
        .execute(&lock("lock_001", "asset:sword_001", "bob", 2))
        .await
        .unwrap();
    assert_eq!(locked.len(), 1);
    let lock_event = &locked[0];
    assert_eq!(lock_event.before.version, 2);
    assert_eq!(lock_event.after.version, 3);
    assert_eq!(
        lock_event.after.state,
        serde_json::json!({ "owner": "bob", "status": "locked" })
    );
}

#[tokio::test]
async fn idempotent_replay_returns_empty() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let request = mint("mint_001", "asset:sword_001", "alice");

    let first = harness.executor.execute(&request).await.unwrap();
    assert_eq!(first.len(), 1);
    let replay = harness.executor.execute(&request).await.unwrap();
    assert!(replay.is_empty());
}

#[tokio::test]
async fn duplicate_intent_id_with_different_payload_fails_closed() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let first = mint("dup_001", "asset:sword_001", "alice");
    let mut conflict = mint("dup_001", "asset:sword_001", "alice");
    conflict.intent.inputs = [("to_owner", serde_json::json!("mallory"))]
        .iter()
        .map(|(k, v)| (String::from(*k), v.clone()))
        .collect();

    assert_eq!(harness.executor.execute(&first).await.unwrap().len(), 1);
    let error = harness.executor.execute(&conflict).await.unwrap_err();
    assert!(matches!(
        error,
        ExecutorError::DuplicateIntent { intent_id } if intent_id == "int_dup_001"
    ));
}

#[tokio::test]
async fn wrong_expected_version_fails_closed() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let minted = harness
        .executor
        .execute(&mint("mint_001", "asset:sword_001", "alice"))
        .await
        .unwrap();
    harness.index.apply(&minted[0], StateType::UniqueAsset);

    let error = harness
        .executor
        .execute(&transfer_with_authority(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            99,
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ExecutorError::ExpectedVersionMismatch { resource, expected, actual }
        if resource == "asset:sword_001" && expected == 99 && actual == 1
    ));
}

#[tokio::test]
async fn unknown_tenant_fails_closed() {
    let harness = Harness::new(FakeTrustGrant::allow());
    // Unregister the default tenant so the scope does not resolve.
    harness.tenant_store.clear();

    let error = harness
        .executor
        .execute(&mint("mint_001", "asset:sword_001", "alice"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ExecutorError::TenantNotFound { tenant }
        if tenant == "acme.game.alpha"
    ));
}

#[tokio::test]
async fn authority_deny_fails_closed() {
    let harness = Harness::new(FakeTrustGrant::deny());
    let request = intent(
        "mint_001",
        "asset.mint",
        Some(StateType::UniqueAsset),
        0,
        "asset:sword_001",
        "alice",
        &[("to_owner", serde_json::json!("alice"))],
        Some(sample_authority()),
        None,
    );
    let error = harness.executor.execute(&request).await.unwrap_err();
    assert!(matches!(error, ExecutorError::AuthorityDenied));
}

#[tokio::test]
async fn authority_stale_fails_closed() {
    let harness = Harness::new(FakeTrustGrant::stale());
    let request = intent(
        "mint_001",
        "asset.mint",
        Some(StateType::UniqueAsset),
        0,
        "asset:sword_001",
        "alice",
        &[("to_owner", serde_json::json!("alice"))],
        Some(sample_authority()),
        None,
    );
    let error = harness.executor.execute(&request).await.unwrap_err();
    assert!(matches!(error, ExecutorError::AuthorityStale));
}

#[tokio::test]
async fn authority_binds_into_event_when_evaluated() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let request = intent(
        "mint_001",
        "asset.mint",
        Some(StateType::UniqueAsset),
        0,
        "asset:sword_001",
        "alice",
        &[("to_owner", serde_json::json!("alice"))],
        Some(sample_authority()),
        None,
    );
    let events = harness.executor.execute(&request).await.unwrap();
    let authority = events[0].authority.as_ref().expect("authority bound");
    assert_eq!(authority.kind, "trustgrant.evaluation");
    assert_eq!(authority.result, EvaluationResult::Allow);
}

#[tokio::test]
async fn expired_intent_fails_closed() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let expired_at = chrono::DateTime::parse_from_rfc3339("2026-07-13T23:59:59Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let request = intent(
        "mint_001",
        "asset.mint",
        Some(StateType::UniqueAsset),
        0,
        "asset:sword_001",
        "alice",
        &[("to_owner", serde_json::json!("alice"))],
        None,
        Some(expired_at),
    );
    let error = harness.executor.execute(&request).await.unwrap_err();
    assert!(matches!(
        error,
        ExecutorError::Expired { intent_id } if intent_id == "int_mint_001"
    ));
}

#[tokio::test]
async fn resource_locked_fails_closed() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let minted = harness
        .executor
        .execute(&mint("mint_001", "asset:sword_001", "alice"))
        .await
        .unwrap();
    harness.index.apply(&minted[0], StateType::UniqueAsset);

    let locked = harness
        .executor
        .execute(&lock("lock_001", "asset:sword_001", "alice", 1))
        .await
        .unwrap();
    harness.index.apply(&locked[0], StateType::UniqueAsset);

    // Transfer from a locked resource is blocked by the §18.2 availability gate.
    let error = harness
        .executor
        .execute(&transfer_with_authority(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            2,
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ExecutorError::ResourceLocked { resource } if resource == "asset:sword_001"
    ));
}

#[tokio::test]
async fn insufficient_quantity_fails_closed_via_profiles() {
    let harness = Harness::new(FakeTrustGrant::allow());

    let created = intent(
        "create_001",
        "balance.create",
        Some(StateType::FungibleBalance),
        0,
        "currency:gold",
        "alice",
        &[
            ("subject", serde_json::json!("alice")),
            ("unit", serde_json::json!("gold_minor")),
            ("balance", serde_json::json!("10")),
        ],
        None,
        None,
    );
    let events = harness.executor.execute(&created).await.unwrap();
    harness.index.apply(&events[0], StateType::FungibleBalance);

    let debit = intent(
        "debit_001",
        "balance.debit",
        Some(StateType::FungibleBalance),
        1,
        "currency:gold",
        "alice",
        &[("amount", serde_json::json!("11"))],
        None,
        None,
    );
    let error = harness.executor.execute(&debit).await.unwrap_err();
    assert!(matches!(
        error,
        ExecutorError::Profile(ProfileError::InsufficientQuantity { available, requested })
        if available == Amount::from_u64(10) && requested == Amount::from_u64(11)
    ));
}

#[tokio::test]
async fn missing_state_type_fails_closed() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let request = intent(
        "mint_001",
        "asset.mint",
        None,
        0,
        "asset:sword_001",
        "alice",
        &[("to_owner", serde_json::json!("alice"))],
        None,
        None,
    );
    let error = harness.executor.execute(&request).await.unwrap_err();
    assert!(matches!(error, ExecutorError::StateTypeRequired));
}

#[tokio::test]
async fn execute_batch_commits_all_atomically() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let intents = vec![
        mint("mint_001", "asset:sword_001", "alice"),
        mint("mint_002", "asset:shield_002", "bob"),
    ];

    let events = harness.executor.execute_batch(&intents).await.unwrap();
    assert_eq!(events.len(), 2);
    assert!(atomicity::validate_batch_consistency(&events).is_ok());

    assert_eq!(
        harness.transactions.log(),
        vec!["begin:acme.game.alpha", "commit"]
    );
}

#[tokio::test]
async fn execute_batch_rolls_back_on_failure() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let minted = harness
        .executor
        .execute(&mint("mint_001", "asset:sword_001", "alice"))
        .await
        .unwrap();
    harness.index.apply(&minted[0], StateType::UniqueAsset);

    // Second intent conflicts on expected version.
    let intents = vec![
        transfer_with_authority("transfer_001", "asset:sword_001", "alice", "bob", 1),
        transfer_with_authority("transfer_002", "asset:sword_001", "alice", "eve", 99),
    ];

    let error = harness.executor.execute_batch(&intents).await.unwrap_err();
    assert!(matches!(error, ExecutorError::AtomicityViolation(_)));

    assert_eq!(
        harness.transactions.log(),
        vec!["begin:acme.game.alpha", "rollback"]
    );
}

// ---------------------------------------------------------------------------
// Gap 1: atomic transfer debit + credit pairs (§20.5, §18.3).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn balance_transfer_emits_atomic_debit_credit_pair() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let created = harness
        .executor
        .execute(&balance_create(
            "create_001",
            "currency:gold",
            "alice",
            "100",
        ))
        .await
        .unwrap();
    harness.index.apply(&created[0], StateType::FungibleBalance);

    let events = harness
        .executor
        .execute(&balance_transfer(
            "transfer_001",
            "currency:gold",
            "alice",
            "bob",
            1,
            "40",
        ))
        .await
        .unwrap();

    assert_eq!(events.len(), 2, "transfer emits exactly two events");
    assert_eq!(events[0].intent_id, events[1].intent_id);
    assert_ne!(events[0].event_id, events[1].event_id);
    assert_eq!(events[0].operation.as_str(), "balance.transfer");
    assert_eq!(events[1].operation.as_str(), "balance.transfer");
    assert_eq!(events[0].resource_id, events[1].resource_id);

    // Source debit: 100 -> 60.
    assert_eq!(events[0].after.state["balance"], serde_json::json!("60"));
    assert_eq!(events[0].after.state["subject"], serde_json::json!("alice"));

    // Destination credit: create-on-credit at version 0 -> balance 40.
    assert_eq!(events[1].before.version, 0);
    assert_eq!(events[1].before.state, serde_json::json!({}));
    assert_eq!(events[1].after.version, 1);
    assert_eq!(events[1].after.state["balance"], serde_json::json!("40"));
    assert_eq!(events[1].after.state["subject"], serde_json::json!("bob"));

    // The pair is the atomic net-zero unit.
    assert!(atomicity::validate_batch_consistency(&events).is_ok());

    // Both events persist-able: applying them makes bob's balance readable.
    harness.index.apply(&events[0], StateType::FungibleBalance);
    harness.index.apply(&events[1], StateType::FungibleBalance);
    let bob = harness
        .index
        .get_subject_state(
            &tenant(),
            &SubjectId(String::from("bob")),
            &ResourceId(String::from("currency:gold")),
        )
        .await
        .unwrap()
        .expect("bob's balance persisted");
    assert_eq!(bob.state["balance"], serde_json::json!("40"));
}

#[tokio::test]
async fn stack_transfer_emits_atomic_debit_credit_pair() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let created = harness
        .executor
        .execute(&stack_create("create_001", "stack:arrows", "alice", "10"))
        .await
        .unwrap();
    harness.index.apply(&created[0], StateType::ConsumableStack);

    let events = harness
        .executor
        .execute(&stack_transfer(
            "transfer_001",
            "stack:arrows",
            "alice",
            "bob",
            1,
            "4",
        ))
        .await
        .unwrap();

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].intent_id, events[1].intent_id);
    assert_eq!(events[0].after.state["quantity"], serde_json::json!("6"));
    assert_eq!(events[0].after.state["subject"], serde_json::json!("alice"));
    assert_eq!(events[1].before.version, 0);
    assert_eq!(events[1].after.state["quantity"], serde_json::json!("4"));
    assert_eq!(events[1].after.state["subject"], serde_json::json!("bob"));
    assert!(atomicity::validate_batch_consistency(&events).is_ok());
}

#[tokio::test]
async fn transfer_to_nonexistent_destination_creates_it_at_version_zero() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let created = harness
        .executor
        .execute(&balance_create(
            "create_001",
            "currency:gold",
            "alice",
            "100",
        ))
        .await
        .unwrap();
    harness.index.apply(&created[0], StateType::FungibleBalance);

    let events = harness
        .executor
        .execute(&balance_transfer(
            "transfer_001",
            "currency:gold",
            "alice",
            "carol",
            1,
            "25",
        ))
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    // Destination did not exist: created at version 0, credited 25.
    assert_eq!(events[1].before.version, 0);
    assert_eq!(events[1].after.version, 1);
    assert_eq!(events[1].after.state["balance"], serde_json::json!("25"));
    assert_eq!(events[1].after.state["subject"], serde_json::json!("carol"));
}

// ---------------------------------------------------------------------------
// Gap 2: explicit actor authentication via the injected intent verifier.
// ---------------------------------------------------------------------------

const FIXED_SEED: [u8; 32] = [42u8; 32];

fn fixed_key() -> SigningKey {
    SigningKey::from_bytes(&FIXED_SEED)
}

/// Attaches a detached Ed25519 signature over the intent's canonical body.
fn with_signature(
    mut validated: statechronicle_intent::validated::ValidatedIntent,
    key: &SigningKey,
) -> statechronicle_intent::validated::ValidatedIntent {
    let bytes = canonicalize(&validated.intent).unwrap();
    let sig = sign(&bytes, key);
    validated.signature = Some(SignatureBlock {
        alg: SignatureAlg::Ed25519,
        key_id: KeyId::new(String::from("did:key:z6Mk...#key-1")).unwrap(),
        sig,
    });
    validated
}

#[tokio::test]
async fn valid_signature_passes_actor_authentication() {
    let key = fixed_key();
    let verifying = key.verifying_key();
    let verifier: Arc<dyn Fn(&SignatureBlock, &[u8]) -> Result<(), ExecutorError> + Send + Sync> =
        Arc::new(move |block, bytes| {
            verify(bytes, &verifying, &block.sig)
                .map_err(|err| ExecutorError::ActorAuthenticationFailed(err.to_string()))
        });
    let harness = Harness::with_verifier(FakeTrustGrant::allow(), verifier);

    let request = with_signature(mint("mint_001", "asset:sword_001", "alice"), &key);
    let events = harness.executor.execute(&request).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].intent_id.as_str(), "int_mint_001");
}

#[tokio::test]
async fn tampered_signature_fails_actor_authentication() {
    let key = fixed_key();
    let verifying = key.verifying_key();
    let verifier: Arc<dyn Fn(&SignatureBlock, &[u8]) -> Result<(), ExecutorError> + Send + Sync> =
        Arc::new(move |block, bytes| {
            verify(bytes, &verifying, &block.sig)
                .map_err(|err| ExecutorError::ActorAuthenticationFailed(err.to_string()))
        });
    let harness = Harness::with_verifier(FakeTrustGrant::allow(), verifier);

    // Sign with a different key than the verifier trusts.
    let wrong_key = SigningKey::from_bytes(&[7u8; 32]);
    let request = with_signature(mint("mint_001", "asset:sword_001", "alice"), &wrong_key);
    let error = harness.executor.execute(&request).await.unwrap_err();
    assert!(matches!(error, ExecutorError::ActorAuthenticationFailed(_)));
}

#[tokio::test]
async fn absent_signature_on_authority_requiring_op_is_denied_per_profile() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let created = harness
        .executor
        .execute(&balance_create(
            "create_001",
            "currency:gold",
            "alice",
            "100",
        ))
        .await
        .unwrap();
    harness.index.apply(&created[0], StateType::FungibleBalance);

    // balance.mint requires `authorized_by` (authority-required profile path).
    // With no signature and no authorization input, the profile denies it.
    let request = intent(
        "mint_001",
        "balance.mint",
        Some(StateType::FungibleBalance),
        1,
        "currency:gold",
        "alice",
        &[("amount", serde_json::json!("10"))],
        None,
        None,
    );
    let error = harness.executor.execute(&request).await.unwrap_err();
    assert!(matches!(
        error,
        ExecutorError::Profile(ProfileError::InvalidInput(_))
    ));
}

// ---------------------------------------------------------------------------
// Gap 3: subject-held resources key by the state's holder, not the actor.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transfer_destination_keys_by_holder_via_state_key() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let created = harness
        .executor
        .execute(&balance_create(
            "create_001",
            "currency:gold",
            "alice",
            "100",
        ))
        .await
        .unwrap();
    harness.index.apply(&created[0], StateType::FungibleBalance);

    let events = harness
        .executor
        .execute(&balance_transfer(
            "transfer_001",
            "currency:gold",
            "alice",
            "bob",
            1,
            "40",
        ))
        .await
        .unwrap();
    harness.index.apply(&events[1], StateType::FungibleBalance);

    // The destination's accumulator key derives from the holder `to_subject`,
    // not the acting actor.
    let holder_key = transition::state_key_for(
        StateType::FungibleBalance,
        &tenant(),
        Some(&SubjectId(String::from("bob"))),
        &ResourceId(String::from("currency:gold")),
    )
    .unwrap();
    assert_eq!(
        holder_key,
        StateKey::for_subject_held("acme.game.alpha", "currency:gold", "bob")
    );
    // And it differs from the source holder's key.
    let source_key = transition::state_key_for(
        StateType::FungibleBalance,
        &tenant(),
        Some(&SubjectId(String::from("alice"))),
        &ResourceId(String::from("currency:gold")),
    )
    .unwrap();
    assert_ne!(holder_key, source_key);
}

// ---------------------------------------------------------------------------
// Phase 2: multi-authority semantics + event-level authority mandatory-ness.
// ---------------------------------------------------------------------------

/// A statically-configurable TrustGrant adapter for tests that need distinct
/// sub-evaluation digests (multi-authority aggregation, identity, ordering).
#[derive(Clone)]
struct StaticGrant {
    result: EvaluationResult,
    digest: ContentDigest,
    evaluated_at: DateTime<Utc>,
}

#[async_trait]
impl TrustGrantPort for StaticGrant {
    async fn evaluate(
        &self,
        _scope: &statechronicle_domain::tenant::TenantId,
        _actor: &SubjectId,
        _operation: &str,
        _resource: &ResourceId,
    ) -> Result<TrustGrantOutcome, TrustGrantError> {
        match self.result {
            EvaluationResult::Allow => Ok(TrustGrantOutcome {
                evaluation_digest: self.digest.clone(),
                result: EvaluationResult::Allow,
                evaluated_at: self.evaluated_at,
            }),
            EvaluationResult::Deny => Err(TrustGrantError::Denied),
        }
    }

    async fn check_revocation_freshness(
        &self,
        _proof: &AuthorityProof,
    ) -> Result<(), TrustGrantError> {
        Ok(())
    }
}

fn allow_grant(fill: u8, evaluated_at: DateTime<Utc>) -> StaticGrant {
    StaticGrant {
        result: EvaluationResult::Allow,
        digest: ContentDigest::new([fill; 32]),
        evaluated_at,
    }
}

fn fixed_ts() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// A unique-asset profile that aggregates the authority set with any-of.
///
/// Test-support: the baseline unique asset profile uses the default
/// require-all policy; this wrapper declares any-of so a single allowing
/// authority suffices even when a member denies.
#[derive(Clone, Copy)]
struct AnyOfUniqueAsset(UniqueAssetRules);

impl ProfileRules for AnyOfUniqueAsset {
    fn state_type(&self) -> StateType {
        self.0.state_type()
    }

    fn profile_id(&self) -> &'static str {
        "any_of_unique_asset_test"
    }

    fn allowed_operations(&self) -> &'static [&'static str] {
        self.0.allowed_operations()
    }

    fn check(
        &self,
        operation: &Operation,
        current: Option<&StateProjection>,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> Result<(), ProfileError> {
        self.0.check(operation, current, inputs)
    }

    fn requires_authority(&self, operation: &Operation) -> bool {
        self.0.requires_authority(operation)
    }

    fn authority_policy(&self, _operation: &Operation) -> AggregationPolicy {
        AggregationPolicy::AnyOf
    }
}

static ANY_OF: AnyOfUniqueAsset = AnyOfUniqueAsset(UniqueAssetRules);

async fn minted(harness: &Harness, resource: &str) {
    let events = harness
        .executor
        .execute(&mint("mint_001", resource, "alice"))
        .await
        .unwrap();
    harness.index.apply(&events[0], StateType::UniqueAsset);
}

#[tokio::test]
async fn authority_required_op_without_binding_fails_closed() {
    let harness = Harness::new(FakeTrustGrant::allow());
    minted(&harness, "asset:sword_001").await;

    // `asset.transfer` is authority-required; a bare transfer (no authority)
    // must be rejected with AuthorityMissing.
    let error = harness
        .executor
        .execute(&transfer(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            1,
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ExecutorError::AuthorityMissing { operation } if operation == "asset.transfer"
    ));
}

#[tokio::test]
async fn authority_required_op_with_binding_succeeds() {
    let harness = Harness::new(FakeTrustGrant::allow());
    minted(&harness, "asset:sword_001").await;

    let events = harness
        .executor
        .execute(&transfer_with_authority(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            1,
        ))
        .await
        .unwrap();
    let authority = events[0].authority.as_ref().expect("authority bound");
    assert_eq!(authority.kind, "trustgrant.evaluation");
    assert_eq!(authority.result, EvaluationResult::Allow);
}

#[tokio::test]
async fn missing_authority_surfaces_after_conflict_gates() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let minted = harness
        .executor
        .execute(&mint("mint_001", "asset:sword_001", "alice"))
        .await
        .unwrap();
    harness.index.apply(&minted[0], StateType::UniqueAsset);
    let locked = harness
        .executor
        .execute(&lock("lock_001", "asset:sword_001", "alice", 1))
        .await
        .unwrap();
    harness.index.apply(&locked[0], StateType::UniqueAsset);

    // Even with NO authority binding, the §18.2 availability conflict gate
    // (step 7) fires before the authority gate (step 8): locks still win.
    let error = harness
        .executor
        .execute(&transfer(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            2,
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ExecutorError::ResourceLocked { resource } if resource == "asset:sword_001"
    ));
}

#[tokio::test]
async fn multi_authority_all_allow_binds_aggregate() {
    let g1 = allow_grant(1, fixed_ts());
    let g2 = allow_grant(2, fixed_ts());
    let harness = Harness::with_profiles_and_ports(
        ProfileRegistry::baseline(),
        vec![Box::new(g1) as _, Box::new(g2) as _],
    );
    minted(&harness, "asset:sword_001").await;

    let events = harness
        .executor
        .execute(&transfer_with_authority(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            1,
        ))
        .await
        .unwrap();
    let authority = events[0].authority.as_ref().expect("authority bound");
    let expected = aggregate_evaluation_digest(
        AggregationPolicy::RequireAll,
        &[ContentDigest::new([1; 32]), ContentDigest::new([2; 32])],
    );
    assert_eq!(
        authority.evaluation_digest, expected,
        "aggregate digest bound"
    );
    assert_eq!(authority.result, EvaluationResult::Allow);
}

#[tokio::test]
async fn aggregate_binds_oldest_evaluated_at() {
    // Two distinct evaluated_at values, both Allow: the bound proof's
    // evaluated_at is the OLDER sub-evaluation timestamp (§18.1 step 8).
    let newer = fixed_ts();
    let older = DateTime::parse_from_rfc3339("2026-07-13T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let g1 = allow_grant(1, older);
    let g2 = allow_grant(2, newer);
    let harness = Harness::with_profiles_and_ports(
        ProfileRegistry::baseline(),
        vec![Box::new(g1) as _, Box::new(g2) as _],
    );
    minted(&harness, "asset:sword_001").await;

    let events = harness
        .executor
        .execute(&transfer_with_authority(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            1,
        ))
        .await
        .unwrap();
    let authority = events[0].authority.as_ref().expect("authority bound");
    assert_eq!(authority.result, EvaluationResult::Allow);
    assert_eq!(
        authority.evaluated_at, older,
        "aggregate evaluated_at is the oldest sub-evaluation"
    );
}

#[tokio::test]
async fn multi_authority_one_deny_fails_closed() {
    // RequireAll (default unique asset policy): Allow + Deny must fail closed.
    let harness = Harness::with_authority_set(&[FakeTrustGrant::allow(), FakeTrustGrant::deny()]);
    minted(&harness, "asset:sword_001").await;

    let error = harness
        .executor
        .execute(&transfer_with_authority(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            1,
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, ExecutorError::AuthorityDenied));
}

#[tokio::test]
async fn multi_authority_any_of_passes() {
    // AnyOf profile: Deny + Allow passes because at least one authority allows.
    // (The allow member is placed first so step 4's primary freshness gate,
    // the v0 early check on the client proof, passes; step 8 still sees the
    // deny member and AnyOf tolerates it.)
    let profiles = ProfileRegistry::with_unique_asset(&ANY_OF);
    let harness = Harness::with_profiles_and_ports(
        profiles,
        vec![
            Box::new(FakeTrustGrant::allow()) as _,
            Box::new(FakeTrustGrant::deny()) as _,
        ],
    );
    minted(&harness, "asset:sword_001").await;

    let events = harness
        .executor
        .execute(&transfer_with_authority(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            1,
        ))
        .await
        .unwrap();
    let authority = events[0].authority.as_ref().expect("authority bound");
    assert_eq!(authority.result, EvaluationResult::Allow);
}

#[tokio::test]
async fn stale_sub_evaluation_fails_closed() {
    // RequireAll: an Allow + Stale member fails closed with AuthorityStale.
    let harness = Harness::with_authority_set(&[FakeTrustGrant::allow(), FakeTrustGrant::stale()]);
    minted(&harness, "asset:sword_001").await;

    let error = harness
        .executor
        .execute(&transfer_with_authority(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            1,
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, ExecutorError::AuthorityStale));
}

#[tokio::test]
async fn single_evaluator_binds_sub_digest_identity() {
    let g = allow_grant(9, fixed_ts());
    let harness =
        Harness::with_profiles_and_ports(ProfileRegistry::baseline(), vec![Box::new(g) as _]);
    minted(&harness, "asset:sword_001").await;

    let events = harness
        .executor
        .execute(&transfer_with_authority(
            "transfer_001",
            "asset:sword_001",
            "alice",
            "bob",
            1,
        ))
        .await
        .unwrap();
    let authority = events[0].authority.as_ref().expect("authority bound");
    // Identity rule: a single-member aggregate preserves the sub-digest bytes.
    assert_eq!(authority.evaluation_digest, ContentDigest::new([9; 32]));
}

#[tokio::test]
async fn aggregate_digest_independent_of_evaluator_order() {
    let evaluate = |order: Vec<StaticGrant>| async move {
        let harness = Harness::with_profiles_and_ports(
            ProfileRegistry::baseline(),
            order
                .into_iter()
                .map(|g| Box::new(g) as Box<dyn TrustGrantPort + Send + Sync>)
                .collect(),
        );
        let minted = harness
            .executor
            .execute(&mint("mint_001", "asset:sword_001", "alice"))
            .await
            .unwrap();
        harness.index.apply(&minted[0], StateType::UniqueAsset);
        let events = harness
            .executor
            .execute(&transfer_with_authority(
                "transfer_001",
                "asset:sword_001",
                "alice",
                "bob",
                1,
            ))
            .await
            .unwrap();
        events[0]
            .authority
            .as_ref()
            .unwrap()
            .evaluation_digest
            .clone()
    };
    let g1 = allow_grant(1, fixed_ts());
    let g2 = allow_grant(2, fixed_ts());
    let forward = evaluate(vec![g1.clone(), g2.clone()]).await;
    let reverse = evaluate(vec![g2, g1]).await;
    assert_eq!(forward, reverse, "aggregate digest is order-independent");
}
