//! Frozen event construction and refusal ordering against the original release.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod common;
use common::{FakeTrustGrant, Harness, balance_create, balance_transfer, mint, tenant};
use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_domain::state_type::StateType;
use statechronicle_executor::error::ExecutorError;
use statechronicle_ports::intent_store::IntentStore;

#[tokio::test]
async fn event_bytes_cover_primary_and_companion_commitments() {
    let h = Harness::new(FakeTrustGrant::allow());
    let created = h
        .executor
        .execute(&balance_create(
            "emission_create",
            "currency:gold",
            "alice",
            "100",
        ))
        .await
        .unwrap();
    h.index.apply(&created[0], StateType::FungibleBalance);
    let transferred = h
        .executor
        .execute(&balance_transfer(
            "emission_transfer",
            "currency:gold",
            "alice",
            "bob",
            1,
            "40",
        ))
        .await
        .unwrap();
    assert_eq!(transferred.len(), 2);
    assert_eq!(
        canonicalize_and_digest(&created).unwrap().to_string(),
        "sha256:5391abe919bbd4d6215f6c32f81456cce88f42062cfbade20fa17f9f8ff6b684"
    );
    assert_eq!(
        canonicalize_and_digest(&transferred).unwrap().to_string(),
        "sha256:2333b332a1739961667e3b056fde475081168a5285d297b2314da18c73c899f2"
    );
}

#[tokio::test]
async fn overflow_does_not_claim_and_preserves_event_id_consumption_order() {
    for companion in [false, true] {
        let h = Harness::new(FakeTrustGrant::allow());
        let mut source = h
            .executor
            .execute(&balance_create("source", "currency:gold", "alice", "100"))
            .await
            .unwrap();
        if !companion {
            source[0].after.version = u64::MAX;
        }
        h.index.apply(&source[0], StateType::FungibleBalance);
        if companion {
            let mut destination = h
                .executor
                .execute(&balance_create("destination", "currency:gold", "bob", "10"))
                .await
                .unwrap();
            destination[0].after.version = u64::MAX;
            h.index.apply(&destination[0], StateType::FungibleBalance);
        }
        let request = balance_transfer(
            "overflow",
            "currency:gold",
            "alice",
            "bob",
            if companion { 1 } else { u64::MAX },
            "40",
        );
        assert!(
            matches!(h.executor.execute(&request).await, Err(ExecutorError::TransitionInvalid(reason)) if reason == "version overflow")
        );
        assert!(
            h.intent_store
                .get_intent(&tenant(), &request.intent.intent_id)
                .await
                .unwrap()
                .is_none()
        );
        let next = h
            .executor
            .execute(&mint("next", "asset:next", "alice"))
            .await
            .unwrap();
        assert_eq!(
            next[0].event_id.as_str(),
            if companion {
                "evt_00000000000000000004"
            } else {
                "evt_00000000000000000002"
            }
        );
    }
}
