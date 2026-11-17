//! Canonical economic contention against one atomic in-memory backend.
#![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
use super::*;
#[allow(dead_code)]
#[path = "durable.rs"]
mod ledger;
use ledger::{Allow, MemoryLedger, Step, Verify};
use statechronicle::commit::{error::CommitError, sign::CommitSigner};
use statechronicle::core::signature::{Signature, sign};
use statechronicle::domain::intent::KeyId;
use statechronicle::mutation::{
    BatchRecoveryOutcome, DurableBatchOutcome, DurableError, SignedBatch,
};
use statechronicle::ports::ledger_store::OutboxRecord;
use std::sync::Arc;

struct Signer;
impl CommitSigner for Signer {
    fn sign_bytes(&self, _: &KeyId, bytes: &[u8]) -> Result<Signature, CommitError> {
        Ok(sign(bytes, &common::fixed_key()))
    }
}
fn registry(h: &Harness, buyers: &[String]) -> MemoryKeyRegistry {
    let registry = MemoryKeyRegistry::new();
    for actor in std::iter::once(SELLER).chain(buyers.iter().map(String::as_str)) {
        registry
            .register(KeyRecord {
                key_id: actor_key(actor),
                tenant: h.tenant(),
                subject: Some(SubjectId(actor.into())),
                operations: [
                    "asset.mint",
                    "trade.lock",
                    "trade.settle",
                    "balance.create",
                    "balance.transfer",
                ]
                .map(Operation::from_static)
                .into(),
                valid_from: h.now(),
                valid_until: None,
                status: KeyStatus::Active,
            })
            .unwrap();
    }
    registry
}
async fn plan(
    h: &Harness,
    ledger: &MemoryLedger,
    requests: &[ValidatedIntent],
    registry: &MemoryKeyRegistry,
    coordinated: bool,
    label: &str,
) -> SignedBatch {
    let principals: Vec<_> = requests
        .iter()
        .map(|r| principal(h, &r.intent.actor.0))
        .collect();
    let items: Vec<_> = requests
        .iter()
        .zip(&principals)
        .map(|(validated, principal)| PlayerBatchItem {
            validated,
            principal,
        })
        .collect();
    let prepared = if coordinated {
        h.executor
            .prepare_player_settlement(&items, &Allow, registry)
            .await
    } else {
        h.executor
            .prepare_player_batch(&items, &Allow, registry)
            .await
    }
    .unwrap();
    let events: Vec<_> = prepared.events().cloned().collect();
    let (mut commit, _) = h.commit_events(&events);
    // Independently enumerate the latest leaf for each known baseline address.
    // Do not use the adapter's projection-key helper to derive the signed root.
    let data = ledger.data.lock().unwrap();
    let mut leaves = std::collections::BTreeMap::new();
    for event in data.events.iter().chain(&events) {
        let key = match event.after.state.state_type() {
            StateType::UniqueAsset => statechronicle::accumulator::key::StateKey::for_resource(
                &event.tenant_id.0,
                &event.resource_id.0,
            ),
            StateType::FungibleBalance => {
                statechronicle::accumulator::key::StateKey::for_subject_held(
                    &event.tenant_id.0,
                    &event.resource_id.0,
                    &event.after.state.subject().unwrap().0,
                )
            }
            kind @ (StateType::ConsumableStack
            | StateType::Entitlement
            | StateType::MeteredResource
            | StateType::Listing
            | StateType::Escrow
            | StateType::Custom) => panic!("unexpected fixture state {kind:?}"),
        };
        leaves.insert(key, *event.after.state_hash.as_bytes());
    }
    let updates: Vec<_> = leaves
        .into_iter()
        .map(|(key, hash)| statechronicle::accumulator::sparse_merkle::StateUpdate::new(key, hash))
        .collect();
    let root = statechronicle::commit::roots::compute_state_root(&updates).unwrap();
    commit.body.next_state_root = statechronicle::ContentDigest::new(*root.as_bytes());
    commit.body.commit_id = statechronicle::CommitId::new(format!("cmt_{label}")).unwrap();
    if let Some(head) = data.commits.last() {
        commit.body.sequence = head.body.sequence + 1;
        commit.body.parent_commit_id = Some(head.body.commit_id.clone());
        commit.body.previous_state_root = head.body.next_state_root.clone();
    }
    drop(data);
    let payload = label.as_bytes().to_vec();
    let outbox = vec![OutboxRecord {
        delivery_key: format!("notice:{label}"),
        tenant: h.tenant(),
        commit_id: commit.body.commit_id.clone(),
        payload_digest: statechronicle::core::digest::hash_bytes(&payload),
        payload,
    }];
    SignedBatch::sign(prepared, &commit.body, h.key_id(), &Signer, outbox).unwrap()
}
async fn seed_canonical(
    h: &Harness,
    ledger: &MemoryLedger,
    buyers: &[String],
    registry: &MemoryKeyRegistry,
) {
    let mut requests = vec![request(
        h,
        "canonical_mint",
        "asset.mint",
        SELLER,
        ASSET,
        0,
        &[("to_owner", json!(SELLER))],
    )];
    for (index, buyer) in buyers.iter().enumerate() {
        requests.push(request(
            h,
            &format!("fund_{index}"),
            "balance.create",
            buyer,
            WALLET,
            0,
            &[
                ("subject", json!(buyer)),
                ("balance", json!("1000")),
                ("unit", json!("gold")),
            ],
        ));
    }
    let genesis = plan(h, ledger, &requests, registry, false, "canonical_genesis").await;
    assert!(matches!(
        genesis.commit(ledger, &Allow, &Verify).await.unwrap(),
        DurableBatchOutcome::Committed(_)
    ));
    for event in genesis.prepared().events() {
        h.index.apply(event, event.after.state.state_type());
    }
    let lock = request(
        h,
        "canonical_lock",
        "trade.lock",
        SELLER,
        ASSET,
        1,
        &[
            ("from_owner", json!(SELLER)),
            ("trade_id", json!("trade_contested")),
        ],
    );
    let locked = plan(h, ledger, &[lock], registry, false, "canonical_lock").await;
    assert!(matches!(
        locked.commit(ledger, &Allow, &Verify).await.unwrap(),
        DurableBatchOutcome::Committed(_)
    ));
    for event in locked.prepared().events() {
        h.index.apply(event, event.after.state.state_type());
    }
}
async fn purchase(
    h: &Harness,
    ledger: &MemoryLedger,
    buyer: &str,
    index: usize,
    registry: &MemoryKeyRegistry,
) -> SignedBatch {
    let sale = settle(h, &format!("purchase_{index}"), buyer, Some("100"), None);
    let payment = request(
        h,
        &format!("pay_{index}"),
        "balance.transfer",
        buyer,
        WALLET,
        1,
        &[("to_subject", json!(SELLER)), ("amount", json!("100"))],
    );
    plan(
        h,
        ledger,
        &[sale, payment],
        registry,
        true,
        &format!("purchase_{index}"),
    )
    .await
}
fn assert_economy(ledger: &MemoryLedger, buyers: &[String], winner: Option<usize>) {
    let data = ledger.data.lock().unwrap();
    assert_snapshot_economy(&data, buyers, winner);
}
fn assert_snapshot_economy(data: &ledger::Stored, buyers: &[String], winner: Option<usize>) {
    let asset = data
        .projections
        .iter()
        .find(|row| row.resource_id.0 == ASSET)
        .unwrap();
    assert_eq!(
        asset.state.owner().unwrap().0,
        winner.map_or(SELLER, |i| buyers[i].as_str())
    );
    for (index, buyer) in buyers.iter().enumerate() {
        let balance = data
            .projections
            .iter()
            .find(|row| row.resource_id.0 == WALLET && row.state.subject().unwrap().0 == *buyer)
            .unwrap();
        assert_eq!(
            balance.state.get("balance").unwrap(),
            json!(if winner == Some(index) { "900" } else { "1000" })
        );
    }
    let seller = data
        .projections
        .iter()
        .find(|row| row.resource_id.0 == WALLET && row.state.subject().unwrap().0 == SELLER);
    if winner.is_some() {
        assert_eq!(seller.unwrap().state.get("balance").unwrap(), json!("100"));
    } else {
        assert!(seller.is_none());
    }
    assert_eq!(data.commits.len(), if winner.is_some() { 3 } else { 2 });
    assert_eq!(
        data.events.len(),
        buyers.len() + 2 + if winner.is_some() { 3 } else { 0 }
    );
    assert_eq!(
        data.identities.len(),
        buyers.len() + 2 + if winner.is_some() { 2 } else { 0 }
    );
    assert_eq!(data.batches.len(), data.identities.len());
    assert_eq!(
        data.projections.len(),
        buyers.len() + 1 + usize::from(winner.is_some())
    );
    assert_eq!(data.outbox.len(), if winner.is_some() { 3 } else { 2 });
    for row in &data.projections {
        assert_eq!(
            row.state_hash,
            statechronicle::core::canonicalize::canonicalize_and_digest(&row.state).unwrap()
        );
        let event = data
            .events
            .iter()
            .find(|event| event.event_id == row.last_event_id)
            .unwrap();
        assert_eq!(event.after.state, row.state);
        assert_eq!(event.after.state_hash, row.state_hash);
        assert_eq!(event.after.version, row.version);
        assert!(
            data.commits
                .iter()
                .any(|commit| commit.body.commit_id == row.last_commit_id)
        );
    }
    for commit in &data.commits {
        statechronicle::commit::persist::SignedCommitVerifier::verify(&Verify, commit).unwrap();
        let batch = data
            .batches
            .values()
            .find(|batch| batch.commit == *commit)
            .unwrap();
        assert_eq!(
            statechronicle::commit::roots::event_root(&batch.events).unwrap(),
            commit.body.event_merkle_root
        );
    }
    let updates: Vec<_> = data
        .projections
        .iter()
        .map(|row| {
            let key = if row.resource_id.0 == WALLET {
                statechronicle::accumulator::key::StateKey::for_subject_held(
                    &row.tenant_id.0,
                    WALLET,
                    &row.state.subject().unwrap().0,
                )
            } else {
                assert_eq!(row.resource_id.0, ASSET);
                statechronicle::accumulator::key::StateKey::for_resource(&row.tenant_id.0, ASSET)
            };
            statechronicle::accumulator::sparse_merkle::StateUpdate::new(
                key,
                *row.state_hash.as_bytes(),
            )
        })
        .collect();
    let root = statechronicle::commit::roots::compute_state_root(&updates).unwrap();
    assert_eq!(
        data.commits.last().unwrap().body.next_state_root.as_bytes(),
        root.as_bytes()
    );
}

#[tokio::test]
async fn distinct_purchases_have_one_atomic_winner_and_recover_original_losers() {
    for count in [2, 8, 32] {
        let h = Harness::new();
        let ledger = MemoryLedger {
            yield_each_step: true,
            ..Default::default()
        };
        let buyers: Vec<_> = (0..count).map(|i| format!("account:buyer{i}")).collect();
        let registry = registry(&h, &buyers);
        seed_canonical(&h, &ledger, &buyers, &registry).await;
        assert_economy(&ledger, &buyers, None);
        let mut plans = Vec::new();
        for (index, buyer) in buyers.iter().enumerate() {
            plans.push(Arc::new(
                purchase(&h, &ledger, buyer, index, &registry).await,
            ));
        }
        let barrier = Arc::new(std::sync::Barrier::new(count));
        let mut tasks = Vec::new();
        for plan in &plans {
            let ledger = ledger.clone();
            let plan = plan.clone();
            let barrier = barrier.clone();
            tasks.push(std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap();
                barrier.wait();
                runtime.block_on(plan.commit(&ledger, &Allow, &Verify))
            }));
        }

        let mut observations = 0;
        while tasks.iter().any(|task| !task.is_finished()) {
            {
                let data = ledger.data.lock().unwrap();
                let asset = data
                    .projections
                    .iter()
                    .find(|row| row.resource_id.0 == ASSET)
                    .unwrap();
                let observed = buyers
                    .iter()
                    .position(|buyer| buyer == &asset.state.owner().unwrap().0);
                assert_snapshot_economy(&data, &buyers, observed);
                observations += 1;
            }
            tokio::task::yield_now().await;
        }
        assert!(observations > 0);
        println!("contenders={count} atomic_snapshots={observations}");
        let mut winner = None;
        for (index, task) in tasks.into_iter().enumerate() {
            match task.join().unwrap() {
                Ok(DurableBatchOutcome::Committed(_)) => {
                    assert!(winner.replace(index).is_none(), "multiple economic winners");
                }
                Ok(DurableBatchOutcome::Replay(_)) => {
                    panic!("distinct identity replayed another buyer")
                }
                Err(error) => assert!(
                    matches!(error.cause(), DurableError::Unknown {
                        source: statechronicle::ports::ledger_store::LedgerStoreError::Conflict(reason),
                        rollback_error: None, ..
                    } if reason == "canonical head changed"),
                    "{error:?}"
                ),
            }
        }
        let winner = winner.unwrap();
        assert_economy(&ledger, &buyers, Some(winner));
        let reopened = MemoryLedger {
            data: ledger.data.clone(),
            reservations: ledger.reservations.clone(),
            ..Default::default()
        };
        for (index, plan) in plans.iter().enumerate() {
            match plan.recover(&reopened, &Allow, &Verify).await.unwrap() {
                BatchRecoveryOutcome::Committed(receipt) => {
                    assert_eq!(index, winner);
                    assert_eq!(
                        receipt.record().events,
                        plan.prepared().events().cloned().collect::<Vec<_>>()
                    );
                    assert_eq!(receipt.record().intents.len(), 2);
                }
                BatchRecoveryOutcome::NotFound(_) => assert_ne!(index, winner),
                BatchRecoveryOutcome::InProgress { .. } => {
                    panic!("reservation survived terminal transaction")
                }
            }
            if index == winner {
                assert!(matches!(
                    plan.commit(&reopened, &Allow, &Verify).await.unwrap(),
                    DurableBatchOutcome::Replay(_)
                ));
            } else {
                assert!(matches!(
                    plan.commit(&reopened, &Allow, &Verify)
                        .await
                        .unwrap_err()
                        .cause(),
                    DurableError::Unknown { .. }
                ));
            }
            assert_economy(&reopened, &buyers, Some(winner));
        }
        assert!(ledger.reservations.lock().unwrap().is_empty());
    }
}

fn assert_unchanged(ledger: &MemoryLedger, before: &ledger::Stored) {
    let after = ledger.data.lock().unwrap();
    assert_eq!(after.events, before.events);
    assert_eq!(after.commits, before.commits);
    assert_eq!(after.projections, before.projections);
    assert_eq!(after.outbox, before.outbox);
    assert_eq!(after.identities, before.identities);
    assert_eq!(after.batches, before.batches);
    assert_eq!(after.mutations, before.mutations);
    assert_eq!(after.finalized, before.finalized);
}

#[tokio::test]
async fn economic_group_failure_preserves_every_seeded_effect_and_original_identity() {
    for (step, occurrence) in [
        (Step::Begin, 1),
        (Step::Claim, 1),
        (Step::Claim, 2),
        (Step::Events, 1),
        (Step::CommitRow, 1),
        (Step::Projection, 1),
        (Step::Projection, 2),
        (Step::Projection, 3),
        (Step::Outbox, 1),
        (Step::Finalize, 1),
        (Step::Finalize, 2),
        (Step::RetainBatch, 1),
        (Step::Commit, 1),
    ] {
        let h = Harness::new();
        let ledger = MemoryLedger::default();
        let buyers = vec![String::from("account:buyer0")];
        let registry = registry(&h, &buyers);
        seed_canonical(&h, &ledger, &buyers, &registry).await;
        let plan = purchase(&h, &ledger, &buyers[0], 0, &registry).await;
        let before = ledger.data.lock().unwrap().clone();
        ledger.calls.lock().unwrap().clear();
        let faulty = MemoryLedger {
            fail_at: Some((step, occurrence)),
            ..ledger.clone()
        };
        let error = plan.commit(&faulty, &Allow, &Verify).await.unwrap_err();
        assert_eq!(error.identity().unwrap().members().len(), 2);
        assert!(
            matches!(error.cause(), DurableError::Unknown { .. }) == (step == Step::Commit),
            "{step:?}: {error:?}"
        );
        assert_unchanged(&ledger, &before);
        assert!(ledger.reservations.lock().unwrap().is_empty());
        assert!(matches!(
            plan.recover(&ledger, &Allow, &Verify).await.unwrap(),
            BatchRecoveryOutcome::NotFound(_)
        ));
        assert!(matches!(
            plan.commit(&ledger, &Allow, &Verify).await.unwrap(),
            DurableBatchOutcome::Committed(_)
        ));
        assert_economy(&ledger, &buyers, Some(0));
    }
}

#[tokio::test]
async fn economic_group_lost_ack_recovers_original_with_current_access_checks() {
    use async_trait::async_trait;
    use statechronicle::ports::authorization::{
        AuthorizationContext, AuthorizationError, Authorizer,
    };
    struct Deny;
    #[async_trait]
    impl Authorizer for Deny {
        async fn authorize(&self, _: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
            Err(AuthorizationError::Denied)
        }
    }
    let h = Harness::new();
    let ledger = MemoryLedger::default();
    let buyers = vec![String::from("account:buyer0")];
    let registry = registry(&h, &buyers);
    seed_canonical(&h, &ledger, &buyers, &registry).await;
    let plan = purchase(&h, &ledger, &buyers[0], 0, &registry).await;
    let lossy = MemoryLedger {
        lose_commit_response: true,
        ..ledger.clone()
    };
    assert!(matches!(
        plan.commit(&lossy, &Allow, &Verify)
            .await
            .unwrap_err()
            .cause(),
        DurableError::Unknown { .. }
    ));
    assert_economy(&ledger, &buyers, Some(0));
    let canonical = ledger.data.lock().unwrap().clone();
    ledger.calls.lock().unwrap().clear();
    for error in [
        plan.commit(&ledger, &Deny, &Verify).await.unwrap_err(),
        plan.recover(&ledger, &Deny, &Verify).await.unwrap_err(),
    ] {
        assert!(matches!(
            error.cause(),
            DurableError::Authorization(AuthorizationError::Denied)
        ));
    }
    assert!(ledger.calls.lock().unwrap().is_empty());
    assert_unchanged(&ledger, &canonical);
    let receipt = match plan.recover(&ledger, &Allow, &Verify).await.unwrap() {
        BatchRecoveryOutcome::Committed(receipt) => receipt,
        other @ (BatchRecoveryOutcome::NotFound(_) | BatchRecoveryOutcome::InProgress { .. }) => {
            panic!("missing original after lost ack: {other:?}")
        }
    };
    assert_eq!(receipt.record().commit, *plan.signed_commit());
    assert_eq!(receipt.record().events.len(), 3);
    assert!(matches!(
        plan.commit(&ledger, &Allow, &Verify).await.unwrap(),
        DurableBatchOutcome::Replay(_)
    ));
    assert_unchanged(&ledger, &canonical);
}

#[tokio::test]
async fn unsupported_cross_tenant_settlement_refuses_before_preparation_effects() {
    let h = Harness::new();
    let ledger = MemoryLedger::default();
    let buyers = vec![String::from("account:buyer0")];
    let registry = registry(&h, &buyers);
    seed_canonical(&h, &ledger, &buyers, &registry).await;
    let sale = settle(&h, "cross_tenant_sale", &buyers[0], Some("100"), None);
    let mut intent = request(
        &h,
        "cross_tenant_payment",
        "balance.transfer",
        &buyers[0],
        WALLET,
        1,
        &[("to_subject", json!(SELLER)), ("amount", json!("100"))],
    )
    .intent;
    intent.tenant_id = common::beta();
    let mut payment = h.sign(intent, Some(h.authority()));
    let beta_key = KeyId::new(String::from("fixture:buyer_beta")).unwrap();
    payment.signature.as_mut().unwrap().key_id = beta_key.clone();
    registry
        .register(KeyRecord {
            key_id: beta_key,
            tenant: common::beta(),
            subject: Some(SubjectId(buyers[0].clone())),
            operations: vec![Operation::from_static("balance.transfer")],
            valid_from: h.now(),
            valid_until: None,
            status: KeyStatus::Active,
        })
        .unwrap();
    let seller = principal(&h, SELLER);
    let mut buyer = principal(&h, &buyers[0]);
    buyer.tenant = common::beta();
    let before = ledger.data.lock().unwrap().clone();
    let symbolic = h.transactions.log();
    let error = h
        .executor
        .prepare_player_settlement(
            &[
                PlayerBatchItem {
                    validated: &sale,
                    principal: &seller,
                },
                PlayerBatchItem {
                    validated: &payment,
                    principal: &buyer,
                },
            ],
            &Allow,
            &registry,
        )
        .await
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("mixed tenant scopes"),
        "{error:?}"
    );
    assert_eq!(h.transactions.log(), symbolic);
    assert_unchanged(&ledger, &before);
    // The optional storage capability itself also refuses unsupported scope.
    use statechronicle::ports::ledger_store::LedgerStore;
    assert!(
        ledger
            .begin_multi(&[h.tenant(), common::beta()])
            .await
            .is_err()
    );
    assert_unchanged(&ledger, &before);
}
