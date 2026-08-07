//! Integration tests for cross-tenant atomicity (protocol §8.2, §18.3).
//!
//! Exercises `execute_cross_tenant` end-to-end over real domain types and the
//! in-memory fakes in [`common`]: a committed cross-tenant debit + credit
//! (one tenant-scoped group per affected tenant, sharing one intent id), a
//! mid-batch failure that rolls the whole transaction back with no partial
//! groups escaping, and a tenant-scoped authority deny that aborts the whole
//! transaction as an `AtomicityViolation`.

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

use async_trait::async_trait;

use statechronicle_core::digest::hash_bytes;
use statechronicle_domain::authority::{AuthorityProof, EvaluationResult, TrustGrantOutcome};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_executor::error::ExecutorError;
use statechronicle_executor::pipeline::TrustGrantPort;
use statechronicle_ports::trustgrant_evaluator::TrustGrantError;
use statechronicle_profiles::registry::ProfileRegistry;

use common::{
    FakeTrustGrant, Harness, cross_asset_transfer, cross_balance_create, cross_balance_transfer,
    cross_mint,
};

fn alpha() -> TenantId {
    TenantId(String::from("stexs.game.alpha"))
}

fn beta() -> TenantId {
    TenantId(String::from("stexs.game.beta"))
}

#[tokio::test]
async fn cross_tenant_debit_credit_commits_per_tenant() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let alpha = alpha();
    let beta = beta();
    harness.tenant_store.register(beta.clone());

    // Seed each tenant's source balance.
    let a = harness
        .executor
        .execute(&cross_balance_create(
            alpha.clone(),
            "create_a",
            "currency:gold",
            "alice",
            "100",
        ))
        .await
        .unwrap();
    harness.index.apply(&a[0], StateType::FungibleBalance);
    let b = harness
        .executor
        .execute(&cross_balance_create(
            beta.clone(),
            "create_b",
            "currency:gold",
            "carol",
            "100",
        ))
        .await
        .unwrap();
    harness.index.apply(&b[0], StateType::FungibleBalance);

    // Both legs share one intent id, the cross-tenant linkage.
    let intents = vec![
        cross_balance_transfer(
            alpha.clone(),
            "xt_001",
            "currency:gold",
            "alice",
            "bob",
            1,
            "40",
        ),
        cross_balance_transfer(
            beta.clone(),
            "xt_001",
            "currency:gold",
            "carol",
            "dave",
            1,
            "40",
        ),
    ];
    let groups = harness
        .executor
        .execute_cross_tenant(&intents)
        .await
        .unwrap();

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].tenant, alpha);
    assert_eq!(groups[0].events.len(), 2);
    assert_eq!(groups[1].tenant, beta);
    assert_eq!(groups[1].events.len(), 2);

    assert_eq!(
        harness.transactions.log(),
        vec!["begin_multi:stexs.game.alpha,stexs.game.beta", "commit"]
    );
}

#[tokio::test]
async fn cross_tenant_mid_batch_failure_rolls_back() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let alpha = alpha();
    let beta = beta();
    harness.tenant_store.register(beta.clone());

    // Seed both sources at version 1.
    let a = harness
        .executor
        .execute(&cross_balance_create(
            alpha.clone(),
            "create_a",
            "currency:gold",
            "alice",
            "100",
        ))
        .await
        .unwrap();
    harness.index.apply(&a[0], StateType::FungibleBalance);
    let b = harness
        .executor
        .execute(&cross_balance_create(
            beta.clone(),
            "create_b",
            "currency:gold",
            "carol",
            "100",
        ))
        .await
        .unwrap();
    harness.index.apply(&b[0], StateType::FungibleBalance);

    // Alpha's leg is valid; beta's leg declares a stale expected version.
    let intents = vec![
        cross_balance_transfer(
            alpha.clone(),
            "xt_002",
            "currency:gold",
            "alice",
            "bob",
            1,
            "40",
        ),
        cross_balance_transfer(
            beta.clone(),
            "xt_002",
            "currency:gold",
            "carol",
            "dave",
            99,
            "40",
        ),
    ];
    let error = harness
        .executor
        .execute_cross_tenant(&intents)
        .await
        .unwrap_err();
    assert!(matches!(error, ExecutorError::AtomicityViolation(_)));

    assert_eq!(
        harness.transactions.log(),
        vec!["begin_multi:stexs.game.alpha,stexs.game.beta", "rollback"]
    );
}

/// A TrustGrant fake that allows every tenant except a single denied one,
/// proving authority is evaluated per tenant scope.
struct TenantScopedDeny {
    denied: TenantId,
}

#[async_trait]
impl TrustGrantPort for TenantScopedDeny {
    async fn evaluate(
        &self,
        scope: &TenantId,
        _actor: &SubjectId,
        _operation: &str,
        _resource: &ResourceId,
    ) -> Result<TrustGrantOutcome, TrustGrantError> {
        if *scope == self.denied {
            Err(TrustGrantError::Denied)
        } else {
            Ok(TrustGrantOutcome {
                evaluation_digest: hash_bytes(b"allowed"),
                result: EvaluationResult::Allow,
                evaluated_at: common::fixed_now(),
            })
        }
    }

    async fn check_revocation_freshness(
        &self,
        _proof: &AuthorityProof,
    ) -> Result<(), TrustGrantError> {
        Ok(())
    }
}

#[tokio::test]
async fn tenant_scoped_authority_deny_aborts_whole_transaction() {
    let scoped = TenantScopedDeny { denied: beta() };
    let harness = Harness::with_profiles_and_ports(
        ProfileRegistry::baseline(),
        vec![Box::new(scoped) as Box<dyn TrustGrantPort + Send + Sync>],
    );
    let alpha = alpha();
    let beta = beta();
    harness.tenant_store.register(beta.clone());

    // Mint an asset in each tenant.
    let a = harness
        .executor
        .execute(&cross_mint(
            alpha.clone(),
            "mint_a",
            "asset:sword_001",
            "alice",
        ))
        .await
        .unwrap();
    harness.index.apply(&a[0], StateType::UniqueAsset);
    let b = harness
        .executor
        .execute(&cross_mint(
            beta.clone(),
            "mint_b",
            "asset:shield_002",
            "carol",
        ))
        .await
        .unwrap();
    harness.index.apply(&b[0], StateType::UniqueAsset);

    // `asset.transfer` is authority-required; the scoped grant denies beta only,
    // so the whole cross-tenant transaction aborts atomically.
    let intents = vec![
        cross_asset_transfer(
            alpha.clone(),
            "xt_003",
            "asset:sword_001",
            "alice",
            "bob",
            1,
        ),
        cross_asset_transfer(
            beta.clone(),
            "xt_003",
            "asset:shield_002",
            "carol",
            "dave",
            1,
        ),
    ];
    let error = harness
        .executor
        .execute_cross_tenant(&intents)
        .await
        .unwrap_err();
    assert!(matches!(error, ExecutorError::AtomicityViolation(_)));

    assert_eq!(
        harness.transactions.log(),
        vec!["begin_multi:stexs.game.alpha,stexs.game.beta", "rollback"]
    );
}

#[tokio::test]
async fn fully_replayed_cross_tenant_batch_aborts_and_rolls_back() {
    let harness = Harness::new(FakeTrustGrant::allow());
    let alpha = alpha();
    let beta = beta();
    harness.tenant_store.register(beta.clone());

    // Seed each tenant's source balance.
    let a = harness
        .executor
        .execute(&cross_balance_create(
            alpha.clone(),
            "create_a",
            "currency:gold",
            "alice",
            "100",
        ))
        .await
        .unwrap();
    harness.index.apply(&a[0], StateType::FungibleBalance);
    let b = harness
        .executor
        .execute(&cross_balance_create(
            beta.clone(),
            "create_b",
            "currency:gold",
            "carol",
            "100",
        ))
        .await
        .unwrap();
    harness.index.apply(&b[0], StateType::FungibleBalance);

    // Both legs share one intent id, the cross-tenant linkage.
    let intents = vec![
        cross_balance_transfer(
            alpha.clone(),
            "xt_004",
            "currency:gold",
            "alice",
            "bob",
            1,
            "40",
        ),
        cross_balance_transfer(
            beta.clone(),
            "xt_004",
            "currency:gold",
            "carol",
            "dave",
            1,
            "40",
        ),
    ];

    // First submission commits the batch; the intent store now holds every
    // intent with an identical payload.
    let first = harness
        .executor
        .execute_cross_tenant(&intents)
        .await
        .unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(
        harness.transactions.log(),
        vec!["begin_multi:stexs.game.alpha,stexs.game.beta", "commit"]
    );

    // Re-submitting the same batch fully replays: every leg emits no events,
    // so the empty groups carry no cross-tenant intent linkage and the whole
    // transaction aborts as an AtomicityViolation with a rollback.
    let error = harness
        .executor
        .execute_cross_tenant(&intents)
        .await
        .unwrap_err();
    assert!(matches!(error, ExecutorError::AtomicityViolation(_)));

    assert_eq!(
        harness.transactions.log(),
        vec![
            "begin_multi:stexs.game.alpha,stexs.game.beta",
            "commit",
            "begin_multi:stexs.game.alpha,stexs.game.beta",
            "rollback",
        ]
    );
}
