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
