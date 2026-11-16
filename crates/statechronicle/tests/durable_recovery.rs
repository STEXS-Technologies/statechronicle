//! Durable outcomes are verified against the atomic ledger, never intent markers.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod common;
#[path = "support/durable.rs"]
mod ledger;
use common::Harness;
use ledger::{Allow, MemoryLedger, Verify};
use statechronicle::commit::durable::{
    DurableError, DurableOutcome, RecoveryOutcome, commit_verified, recover,
};
use statechronicle::commit::persist::{CommittedEvent, DurableCommitRequest, projections_for};
use statechronicle::domain::intent::{Intent, Nonce};
use statechronicle::ports::authorization::AuthenticatedPrincipal;
use statechronicle::ports::key_registry::{KeyRecord, KeyStatus, MemoryKeyRegistry};
use statechronicle::ports::ledger_store::OutboxRecord;
use statechronicle::{
    Commit, IntentId, Operation, PreparedMutation, ResourceId, Signed, StateProjection, StateType,
    SubjectId,
};

struct Fixture {
    prepared: PreparedMutation,
    commit: Signed<Commit>,
    projections: Vec<StateProjection>,
    outbox: Vec<OutboxRecord>,
}
impl Fixture {
    async fn new() -> Self {
        Self::new_at("int_durable_recovery", "asset:durable").await
    }
    async fn new_at(id: &str, resource: &str) -> Self {
        let harness = Harness::new();
        let actor = SubjectId(String::from("account:alice"));
        let intent = Intent::builder()
            .tenant(harness.tenant())
            .intent_id(IntentId::new(String::from(id)).unwrap())
            .operation(Operation::from_static("asset.mint"))
            .actor(actor.clone())
            .resource(ResourceId(String::from(resource)))
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
            credential_id: String::from("session"),
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
        let prepared = harness
            .executor
            .prepare_player(&request, &principal, &Allow, &registry)
            .await
            .unwrap();
        let (commit, _) = harness.commit_events(prepared.events());
        let entries: Vec<_> = prepared
            .events()
            .iter()
            .map(|event| CommittedEvent {
                event,
                state_type: event.after.state.state_type(),
            })
            .collect();
        let projections = projections_for(&commit.body.commit_id, &entries);
        let payload = vec![1, 2, 3];
        let outbox = vec![OutboxRecord {
            delivery_key: format!("notification-{id}"),
            tenant: harness.tenant(),
            commit_id: commit.body.commit_id.clone(),
            payload_digest: statechronicle::core::digest::hash_bytes(&payload),
            payload,
        }];
        Self {
            prepared,
            commit,
            projections,
            outbox,
        }
    }
    async fn persist<S: statechronicle::ports::ledger_store::LedgerRecovery + ?Sized>(
        &self,
        ledger: &S,
    ) -> Result<DurableOutcome, DurableError> {
        self.persist_with(ledger, &Allow, &Verify).await
    }
    async fn persist_with<S: statechronicle::ports::ledger_store::LedgerRecovery + ?Sized>(
        &self,
        ledger: &S,
        authorizer: &dyn statechronicle::ports::authorization::Authorizer,
        verifier: &dyn statechronicle::commit::persist::SignedCommitVerifier,
    ) -> Result<DurableOutcome, DurableError> {
        let entries: Vec<_> = self
            .prepared
            .events()
            .iter()
            .map(|event| CommittedEvent {
                event,
                state_type: event.after.state.state_type(),
            })
            .collect();
        commit_verified(
            ledger,
            authorizer,
            verifier,
            DurableCommitRequest {
                principal: self.prepared.principal(),
                intent: &self.prepared.validated().intent,
                payload_digest: self.prepared.payload_digest(),
                entries: &entries,
                commit: &self.commit,
                projections: &self.projections,
                outbox: &self.outbox,
            },
        )
        .await
    }
}

#[tokio::test]
async fn lost_ack_is_unknown_until_authoritative_lookup_and_exact_retry() {
    let fixture = Fixture::new().await;
    let ledger = MemoryLedger {
        lose_commit_response: true,
        ..MemoryLedger::default()
    };
    let unknown = fixture.persist(&ledger).await.unwrap_err();
    assert!(matches!(unknown, DurableError::Unknown { .. }));
    let reopened = MemoryLedger {
        lose_commit_response: false,
        ..ledger.clone()
    };
    let RecoveryOutcome::Committed(receipt) = recover(
        &reopened,
        &Allow,
        &Verify,
        fixture.prepared.principal(),
        &fixture.prepared.validated().intent,
    )
    .await
    .unwrap() else {
        panic!("authoritative record must resolve the lost acknowledgement");
    };
    assert_eq!(receipt.commit(), &fixture.commit);
    assert_eq!(
        receipt.identity().payload_digest(),
        fixture.prepared.payload_digest()
    );
    assert!(matches!(
        fixture.persist(&reopened).await.unwrap(),
        DurableOutcome::Replay(_)
    ));
    let data = reopened.data.lock().unwrap();
    assert_eq!(data.events.len(), 1);
    assert_eq!(data.commits.len(), 1);
    assert_eq!(data.outbox.len(), 1);
}

#[tokio::test]
async fn confirmed_commit_returns_original_receipt_and_conflicting_payload_fails() {
    let fixture = Fixture::new().await;
    let ledger = MemoryLedger::default();
    let DurableOutcome::Committed(receipt) = fixture.persist(&ledger).await.unwrap() else {
        panic!("new commit expected");
    };
    assert_eq!(receipt.record().events, fixture.prepared.events());
    assert_eq!(receipt.record().projections, fixture.projections);
    assert_eq!(receipt.record().outbox, fixture.outbox);
    let mut other = fixture.prepared.validated().intent.clone();
    other.nonce = Nonce::from_bytes(vec![9]).unwrap();
    assert!(matches!(
        recover(
            &ledger,
            &Allow,
            &Verify,
            fixture.prepared.principal(),
            &other
        )
        .await,
        Err(DurableError::PayloadConflict)
    ));
    assert!(matches!(
        fixture.persist(&ledger).await.unwrap(),
        DurableOutcome::Replay(_)
    ));
    assert_eq!(ledger.data.lock().unwrap().commits.len(), 1);
}

#[tokio::test]
async fn precommit_failures_rollback_and_commit_or_rollback_uncertainty_never_succeeds() {
    use ledger::Step;
    for step in [
        Step::Begin,
        Step::Claim,
        Step::Events,
        Step::CommitRow,
        Step::Projection,
        Step::Outbox,
        Step::Finalize,
        Step::Commit,
    ] {
        for rollback_fails in [false, true] {
            let fixture = Fixture::new().await;
            let mut faults = vec![step];
            if rollback_fails {
                faults.push(Step::Rollback);
            }
            let ledger = MemoryLedger {
                faults,
                ..MemoryLedger::default()
            };
            let result = fixture.persist(&ledger).await;
            let uncertain = step == Step::Commit || (step != Step::Begin && rollback_fails);
            if uncertain {
                assert!(
                    matches!(result, Err(DurableError::Unknown { .. })),
                    "{step:?}"
                );
            } else {
                assert!(
                    matches!(result, Err(DurableError::Unavailable(_))),
                    "{step:?}"
                );
            }
            {
                let data = ledger.data.lock().unwrap();
                assert!(data.events.is_empty());
                assert!(data.commits.is_empty());
                assert!(data.projections.is_empty());
                assert!(data.outbox.is_empty());
                assert!(data.identities.is_empty());
                assert!(data.mutations.is_empty());
            }
            let reopened = MemoryLedger {
                faults: vec![],
                ..ledger
            };
            assert!(matches!(
                recover(
                    &reopened,
                    &Allow,
                    &Verify,
                    fixture.prepared.principal(),
                    &fixture.prepared.validated().intent
                )
                .await
                .unwrap(),
                RecoveryOutcome::NotFound(_)
            ));
            assert!(matches!(
                fixture.persist(&reopened).await.unwrap(),
                DurableOutcome::Committed(_)
            ));
        }
    }
}

#[tokio::test]
async fn successful_write_with_failed_readback_remains_unknown_and_recovers_without_new_write() {
    let fixture = Fixture::new().await;
    let ledger = MemoryLedger {
        faults: vec![ledger::Step::Lookup],
        ..MemoryLedger::default()
    };
    assert!(matches!(
        fixture.persist(&ledger).await,
        Err(DurableError::Unknown {
            source: statechronicle::ports::ledger_store::LedgerStoreError::Unavailable(_),
            ..
        })
    ));
    assert_eq!(ledger.data.lock().unwrap().commits.len(), 1);
    let reopened = MemoryLedger {
        faults: vec![],
        ..ledger
    };
    assert!(matches!(
        recover(
            &reopened,
            &Allow,
            &Verify,
            fixture.prepared.principal(),
            &fixture.prepared.validated().intent
        )
        .await
        .unwrap(),
        RecoveryOutcome::Committed(_)
    ));
    assert_eq!(reopened.data.lock().unwrap().commits.len(), 1);
}

#[tokio::test]
async fn active_reservations_and_absent_markers_never_become_receipts() {
    use statechronicle::ports::ledger_store::LedgerStore;
    let fixture = Fixture::new().await;
    let ledger = MemoryLedger::default();
    assert!(matches!(
        recover(
            &ledger,
            &Allow,
            &Verify,
            fixture.prepared.principal(),
            &fixture.prepared.validated().intent
        )
        .await
        .unwrap(),
        RecoveryOutcome::NotFound(_)
    ));
    let mut transaction = ledger
        .begin(&fixture.prepared.validated().intent.tenant_id)
        .await
        .unwrap();
    transaction
        .claim_idempotency(
            &fixture.prepared.validated().intent.tenant_id,
            &fixture.prepared.validated().intent,
            fixture.prepared.payload_digest(),
        )
        .await
        .unwrap();
    assert!(matches!(
        recover(
            &ledger,
            &Allow,
            &Verify,
            fixture.prepared.principal(),
            &fixture.prepared.validated().intent
        )
        .await
        .unwrap(),
        RecoveryOutcome::InProgress { .. }
    ));
    assert!(matches!(
        fixture.persist(&ledger).await,
        Err(DurableError::InProgress { .. })
    ));
    let mut conflict = fixture.prepared.validated().intent.clone();
    conflict.nonce = Nonce::from_bytes(vec![8]).unwrap();
    assert!(matches!(
        recover(
            &ledger,
            &Allow,
            &Verify,
            fixture.prepared.principal(),
            &conflict
        )
        .await,
        Err(DurableError::PayloadConflict)
    ));
    transaction.rollback().await.unwrap();
    assert!(matches!(
        fixture.persist(&ledger).await.unwrap(),
        DurableOutcome::Committed(_)
    ));
}

#[tokio::test]
async fn corrupted_or_partial_ledger_records_and_bare_replay_claims_fail_closed() {
    let fixture = Fixture::new().await;
    for corruption in 0..8 {
        let ledger = MemoryLedger::default();
        fixture.persist(&ledger).await.unwrap();
        {
            let mut data = ledger.data.lock().unwrap();
            let row = data.mutations.values_mut().next().unwrap();
            match corruption {
                0 => row.intent.tenant_id.0 = String::from("other"),
                1 => row.intent.intent_id = IntentId::new(String::from("int_other")).unwrap(),
                2 => row.payload_digest = statechronicle::ContentDigest::new([0; 32]),
                3 => row.events.clear(),
                4 => row.projections.clear(),
                5 => row.commit.body.sequence = 99,
                6 => row.outbox[0].payload.push(9),
                7 => row.events[0].operation = Operation::from_static("asset.transfer"),
                _ => panic!("unexpected corruption index"),
            }
        }
        assert!(
            matches!(
                recover(
                    &ledger,
                    &Allow,
                    &Verify,
                    fixture.prepared.principal(),
                    &fixture.prepared.validated().intent
                )
                .await,
                Err(DurableError::Corrupt(_))
            ),
            "corruption {corruption}"
        );
        assert!(
            matches!(
                fixture.persist(&ledger).await,
                Err(DurableError::Unknown { .. })
            ),
            "corruption {corruption}"
        );
    }
    let ledger = MemoryLedger::default();
    let identity = (
        &fixture.prepared.validated().intent.tenant_id.0,
        fixture.prepared.validated().intent.intent_id.as_str(),
    );
    ledger.data.lock().unwrap().identities.insert(
        (identity.0.clone(), identity.1.to_owned()),
        (
            fixture.prepared.payload_digest().clone(),
            fixture.commit.body.commit_id.clone(),
        ),
    );
    assert!(matches!(
        fixture.persist(&ledger).await,
        Err(DurableError::Unknown { .. })
    ));
    assert!(ledger.data.lock().unwrap().commits.is_empty());
}

#[tokio::test]
async fn recovery_checks_principal_and_policy_before_reading_adapter_data() {
    use statechronicle::ports::authorization::DenyAllAuthorizer;
    let fixture = Fixture::new().await;
    let ledger = MemoryLedger::default();
    assert!(matches!(
        recover(
            &ledger,
            &DenyAllAuthorizer,
            &Verify,
            fixture.prepared.principal(),
            &fixture.prepared.validated().intent
        )
        .await,
        Err(DurableError::Authorization(
            statechronicle::ports::authorization::AuthorizationError::Denied
        ))
    ));
    let mut stranger = fixture.prepared.principal().clone();
    stranger.subject = SubjectId(String::from("account:mallory"));
    assert!(matches!(
        recover(
            &ledger,
            &Allow,
            &Verify,
            &stranger,
            &fixture.prepared.validated().intent
        )
        .await,
        Err(DurableError::Authorization(
            statechronicle::ports::authorization::AuthorizationError::ContextMismatch(_)
        ))
    ));
    assert!(ledger.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn original_recovery_and_replay_remain_valid_after_a_later_canonical_commit() {
    use statechronicle::accumulator::sparse_merkle::StateAccumulator;
    use statechronicle::commit::roots::state_root_updates;
    use statechronicle::commit::sign::sign_commit;
    let first = Fixture::new().await;
    let ledger = MemoryLedger::default();
    first.persist(&ledger).await.unwrap();
    let mut second = Fixture::new_at("int_second_commit", "asset:second").await;
    let all_events: Vec<_> = first
        .prepared
        .events()
        .iter()
        .chain(second.prepared.events())
        .cloned()
        .collect();
    let mut tree = StateAccumulator::empty();
    tree.insert_batch(&state_root_updates(&all_events).unwrap())
        .unwrap();
    let mut body = second.commit.body.clone();
    body.commit_id =
        statechronicle::CommitId::new(String::from("cmt_00000000000000000002")).unwrap();
    body.sequence = 2;
    body.parent_commit_id = Some(first.commit.body.commit_id.clone());
    body.previous_state_root = first.commit.body.next_state_root.clone();
    body.next_state_root = statechronicle::ContentDigest::new(*tree.root().as_bytes());
    second.commit = sign_commit(&body, &common::fixed_key(), common::key_id()).unwrap();
    let entries: Vec<_> = second
        .prepared
        .events()
        .iter()
        .map(|event| CommittedEvent {
            event,
            state_type: event.after.state.state_type(),
        })
        .collect();
    second.projections = projections_for(&second.commit.body.commit_id, &entries);
    second.outbox[0].commit_id = second.commit.body.commit_id.clone();
    assert!(matches!(
        second.persist(&ledger).await.unwrap(),
        DurableOutcome::Committed(_)
    ));
    assert!(matches!(
        first.persist(&ledger).await.unwrap(),
        DurableOutcome::Replay(_)
    ));
    let RecoveryOutcome::Committed(original) = recover(
        &ledger,
        &Allow,
        &Verify,
        first.prepared.principal(),
        &first.prepared.validated().intent,
    )
    .await
    .unwrap() else {
        panic!("historical receipt expected");
    };
    assert_eq!(original.commit(), &first.commit);
    assert_eq!(ledger.data.lock().unwrap().commits.len(), 2);
}

struct OmittedOutbox(MemoryLedger);
#[async_trait::async_trait]
impl statechronicle::ports::ledger_store::LedgerStore for OmittedOutbox {
    async fn begin(
        &self,
        tenant: &statechronicle::TenantId,
    ) -> Result<
        Box<dyn statechronicle::ports::ledger_store::LedgerTransaction>,
        statechronicle::ports::ledger_store::LedgerStoreError,
    > {
        self.0.begin(tenant).await
    }
    async fn begin_multi(
        &self,
        tenants: &[statechronicle::TenantId],
    ) -> Result<
        Box<dyn statechronicle::ports::ledger_store::LedgerTransaction>,
        statechronicle::ports::ledger_store::LedgerStoreError,
    > {
        self.0.begin_multi(tenants).await
    }
}
#[async_trait::async_trait]
impl statechronicle::ports::ledger_store::LedgerRecovery for OmittedOutbox {
    async fn lookup_mutation(
        &self,
        tenant: &statechronicle::TenantId,
        id: &IntentId,
    ) -> Result<
        statechronicle::ports::ledger_store::LedgerLookup,
        statechronicle::ports::ledger_store::LedgerStoreError,
    > {
        let mut result = self.0.lookup_mutation(tenant, id).await?;
        if let statechronicle::ports::ledger_store::LedgerLookup::Committed(record) = &mut result {
            record.outbox.clear();
        }
        Ok(result)
    }
}

#[tokio::test]
async fn acknowledged_write_requires_exact_outbox_readback_and_replay_requires_claimed_commit() {
    let fixture = Fixture::new().await;
    let broken = OmittedOutbox(MemoryLedger::default());
    assert!(matches!(
        fixture.persist(&broken).await,
        Err(DurableError::Unknown { .. })
    ));
    assert_eq!(broken.0.data.lock().unwrap().outbox.len(), 1);
    let ledger = MemoryLedger::default();
    fixture.persist(&ledger).await.unwrap();
    ledger
        .data
        .lock()
        .unwrap()
        .identities
        .values_mut()
        .next()
        .unwrap()
        .1 = statechronicle::CommitId::new(String::from("cmt_wrong_replay_claim")).unwrap();
    assert!(matches!(
        fixture.persist(&ledger).await,
        Err(DurableError::Unknown { .. })
    ));
}

struct RefuseSigner;
impl statechronicle::commit::persist::SignedCommitVerifier for RefuseSigner {
    fn verify(&self, _: &Signed<Commit>) -> Result<(), String> {
        Err(String::from("untrusted commit signer"))
    }
}

#[tokio::test]
async fn signer_and_authorization_refusal_happen_before_any_ledger_operation() {
    let fixture = Fixture::new().await;
    let ledger = MemoryLedger::default();
    assert!(matches!(
        fixture.persist_with(&ledger, &Allow, &RefuseSigner).await,
        Err(DurableError::Rejected(_))
    ));
    assert!(matches!(
        fixture
            .persist_with(
                &ledger,
                &statechronicle::ports::authorization::DenyAllAuthorizer,
                &Verify
            )
            .await,
        Err(DurableError::Authorization(
            statechronicle::ports::authorization::AuthorizationError::Denied
        ))
    ));
    assert!(ledger.calls.lock().unwrap().is_empty());
    fixture.persist(&ledger).await.unwrap();
    assert!(matches!(
        recover(
            &ledger,
            &Allow,
            &RefuseSigner,
            fixture.prepared.principal(),
            &fixture.prepared.validated().intent
        )
        .await,
        Err(DurableError::Corrupt(_))
    ));
}

struct PolicyUnavailable;
#[async_trait::async_trait]
impl statechronicle::ports::authorization::Authorizer for PolicyUnavailable {
    async fn authorize(
        &self,
        _: statechronicle::ports::authorization::AuthorizationContext<'_>,
    ) -> Result<(), statechronicle::ports::authorization::AuthorizationError> {
        Err(
            statechronicle::ports::authorization::AuthorizationError::Unavailable(String::from(
                "policy outage",
            )),
        )
    }
}

#[tokio::test]
async fn policy_unavailability_retains_its_typed_cause_without_touching_the_ledger() {
    use statechronicle::ports::authorization::AuthorizationError;
    let fixture = Fixture::new().await;
    let ledger = MemoryLedger::default();
    assert!(matches!(
        fixture
            .persist_with(&ledger, &PolicyUnavailable, &Verify)
            .await,
        Err(DurableError::Authorization(
            AuthorizationError::Unavailable(_)
        ))
    ));
    assert!(matches!(
        recover(
            &ledger,
            &PolicyUnavailable,
            &Verify,
            fixture.prepared.principal(),
            &fixture.prepared.validated().intent
        )
        .await,
        Err(DurableError::Authorization(
            AuthorizationError::Unavailable(_)
        ))
    ));
    assert!(ledger.calls.lock().unwrap().is_empty());
}
