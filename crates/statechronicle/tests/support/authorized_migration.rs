//! Protected migration authorization from independently pinned original history.
use super::*;
use async_trait::async_trait;
use serde::Deserialize;
use statechronicle::commit::sign::CommitSigner;
use statechronicle::core::signature::{Signature, sign};
use statechronicle::domain::intent::{Intent, KeyId};
use statechronicle::index::rebuild::verified::{
    HistoryPin, HistoryWitness, VerifiedHistory, verify_history,
};
use statechronicle::index::rebuild::{CheckpointStore, ProjectionSink};
use statechronicle::mutation::{DurableError, DurableOutcome, RecoveryOutcome, SignedMutation};
use statechronicle::ports::authorization::{AuthorizationContext, AuthorizationError, Authorizer};
use statechronicle::ports::key_registry::{KeyRecord, KeyStatus, MemoryKeyRegistry};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

const OLD_TIP: &str = "sha256:19882ff97faa18ac310e67ece704dcf2b76eafed11e373e6e81f91ed22d26593";
const OLD_ROOT: &str = "sha256:9bfb4e38dbe48a7d74b11572f9af16a001519cbf6562bafc435b8821a1e5f607";
const MIGRATOR: &str = "service:migrator";
#[derive(Clone, Deserialize)]
struct Original {
    producer_commit: String,
    events: Vec<Event>,
    commits: Vec<Signed<Commit>>,
    intents: Vec<Intent>,
    events_bcs: Vec<Vec<u8>>,
    commits_bcs: Vec<Vec<u8>>,
    intents_bcs: Vec<Vec<u8>>,
}
fn original() -> Original {
    let bytes = include_bytes!("../fixtures/original_configuration_history.json");
    assert_eq!(
        hash_bytes(bytes).as_str(),
        "sha256:72941224f3d024f087905e4ab3cfa84e3b9d8027d8f7a287e04bc81db2af8e9f"
    );
    let source: Original = serde_json::from_slice(bytes).unwrap();
    assert_eq!(
        source.producer_commit,
        "c418e98b91720188158c531cd5c2c4b8ab71cbcf"
    );
    assert_eq!(source.events.len(), 4);
    assert_eq!(source.commits.len(), 4);
    assert_eq!(source.intents.len(), 4);
    for index in 0..4 {
        assert_eq!(
            canonicalize(&source.events[index]).unwrap(),
            source.events_bcs[index]
        );
        assert_eq!(
            canonicalize(&source.commits[index]).unwrap(),
            source.commits_bcs[index]
        );
        assert_eq!(
            canonicalize(&source.intents[index]).unwrap(),
            source.intents_bcs[index]
        );
    }
    assert_eq!(
        source.commits.last().unwrap().body.next_state_root.as_str(),
        OLD_ROOT
    );
    source
}
fn verified(source: &Original) -> VerifiedHistory {
    let witnesses: Vec<_> = source
        .commits
        .iter()
        .zip(&source.events)
        .map(|(commit, event)| HistoryWitness {
            commit,
            events: from_ref(event),
        })
        .collect();
    verify_history(
        &witnesses,
        &HistoryPin {
            tenant: common::tenant(),
            tip_digest: OLD_TIP.parse().unwrap(),
        },
        &Verify,
    )
    .unwrap()
}
struct RestorePolicy(Vec<ContentDigest>);
#[async_trait]
impl Authorizer for RestorePolicy {
    async fn authorize(&self, context: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
        if context.principal.credential_id == "verified-history-restore"
            && self
                .0
                .contains(&canonicalize_and_digest(context.intent).unwrap())
        {
            Ok(())
        } else {
            Err(AuthorizationError::Denied)
        }
    }
}
#[derive(Clone)]
struct MigrationPolicy {
    allowed: bool,
    expected: BTreeMap<String, Value>,
}
#[async_trait]
impl Authorizer for MigrationPolicy {
    async fn authorize(&self, context: AuthorizationContext<'_>) -> Result<(), AuthorizationError> {
        if self.allowed
            && context.principal.subject.0 == MIGRATOR
            && context.principal.credential_id == "migration-session"
            && context.tenant == &common::tenant()
            && context.resource.0 == "config:version2"
            && context.intent.operation.as_str() == "configuration.migrate"
            && context.intent.expected_version == 0
            && context.intent.inputs == self.expected
        {
            Ok(())
        } else {
            Err(AuthorizationError::Denied)
        }
    }
}
struct Signer;
impl CommitSigner for Signer {
    fn sign_bytes(&self, _: &KeyId, bytes: &[u8]) -> Result<Signature, CommitError> {
        Ok(sign(bytes, &fixed_key()))
    }
}
async fn restore(h: &Harness, ledger: &MemoryLedger, source: &Original) {
    let _checked = verified(source);
    let policy = RestorePolicy(
        source
            .intents
            .iter()
            .map(|i| canonicalize_and_digest(i).unwrap())
            .collect(),
    );
    for ((intent, event), commit) in source
        .intents
        .iter()
        .zip(&source.events)
        .zip(&source.commits)
    {
        let principal = AuthenticatedPrincipal {
            tenant: h.tenant(),
            subject: intent.actor.clone(),
            credential_id: "verified-history-restore".into(),
        };
        let entries = [CommittedEvent {
            event,
            state_type: StateType::Custom,
        }];
        let digest = canonicalize_and_digest(intent).unwrap();
        let projections = projections_for(&commit.body.commit_id, &entries);
        persist_durable_verified(
            ledger,
            &policy,
            &Verify,
            DurableCommitRequest {
                principal: &principal,
                intent,
                payload_digest: &digest,
                entries: &entries,
                commit,
                projections: &projections,
                outbox: &[],
            },
        )
        .await
        .unwrap();
        h.index.apply(event, StateType::Custom);
    }
}
fn migration_raw(source: &Original, history: &VerifiedHistory) -> Value {
    let old = CONFIGURATION
        .decode_state(&source.events.last().unwrap().after.state)
        .unwrap();
    assert_eq!(old.revision, 4);
    assert!(!old.enabled);
    let mut raw = payload(
        "int_authorized_migration",
        "config:version2",
        0,
        &ConfigurationV2,
        &VersionedCommand::Migrate {
            enabled: old.enabled,
            source_revision: old.revision,
        },
    );
    raw["actor"] = json!(MIGRATOR);
    raw["inputs"]["source_resource"] = json!("config:version1");
    raw["inputs"]["source_history"] = json!(history.source_digest().as_str());
    raw["inputs"]["source_state"] = json!(source.events.last().unwrap().after.state_hash.as_str());
    raw
}
fn migration_principal(h: &Harness, actor: &str) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        tenant: h.tenant(),
        subject: SubjectId(actor.into()),
        credential_id: "migration-session".into(),
    }
}
fn key(actor: &str) -> KeyId {
    KeyId::new(format!("fixture:{actor}")).unwrap()
}
fn migration_keys(h: &Harness) -> MemoryKeyRegistry {
    let keys = MemoryKeyRegistry::new();
    for actor in [MIGRATOR, "operator:alice"] {
        keys.register(KeyRecord {
            key_id: key(actor),
            tenant: h.tenant(),
            subject: Some(SubjectId(actor.into())),
            operations: vec![Operation::from_static("configuration.migrate")],
            valid_from: h.now(),
            valid_until: None,
            status: KeyStatus::Active,
        })
        .unwrap();
    }
    keys
}
fn accept(h: &Harness, raw: &Value) -> ValidatedIntent {
    let mut accepted = h.accept(raw, Some(h.authority()));
    accepted.signature.as_mut().unwrap().key_id = key(&accepted.intent.actor.0);
    accepted
}
fn unchanged(ledger: &MemoryLedger, before: &super::super::durable::Stored) {
    let after = ledger.data.lock().unwrap();
    assert_eq!(after.events, before.events);
    assert_eq!(after.commits, before.commits);
    assert_eq!(after.projections, before.projections);
    assert_eq!(after.identities, before.identities);
    assert_eq!(after.mutations, before.mutations);
    assert_eq!(after.outbox, before.outbox);
    assert_eq!(after.batches, before.batches);
}
async fn recover_plan(
    plan: &SignedMutation,
    ledger: &MemoryLedger,
    policy: &MigrationPolicy,
) -> Result<RecoveryOutcome, DurableError> {
    statechronicle::mutation::recover(
        ledger,
        policy,
        &Verify,
        plan.prepared().principal(),
        &plan.prepared().validated().intent,
    )
    .await
}
fn migration_preparer(h: &Harness) -> statechronicle::mutation::Preparer {
    use statechronicle::mutation::{PreparationConfig, PreparationPorts, Preparer};
    let counter = AtomicUsize::new(1);
    Preparer::new(
        PreparationPorts {
            state_index: Box::new(h.index.clone()),
            tenant_store: Box::new(h.tenant_store.clone()),
            trustgrant: vec![Box::new(common::FakeTrustGrant::allow())],
        },
        PreparationConfig {
            profiles: ProfileRegistry::with_custom(&*PROFILES).unwrap(),
            executor: common::executor_subject(),
            now: Box::new(common::fixed_now),
            // Independent namespace: restoring historical projections must not
            // restart the producer's event counter and reuse historical identities.
            event_id_fn: Box::new(move || {
                statechronicle::EventId::new(format!(
                    "evt_migration_{}",
                    counter.fetch_add(1, Ordering::SeqCst)
                ))
                .unwrap()
            }),
            intent_verifier: Arc::new(|signature, body| {
                statechronicle::core::signature::verify(
                    body,
                    &fixed_key().verifying_key(),
                    &signature.sig,
                )
                .map_err(|error| ExecutorError::ActorAuthenticationFailed(error.to_string()))
            }),
        },
    )
}
async fn protected_migration() -> (Original, SignedMutation, MemoryLedger) {
    let source = original();
    let history = verified(&source);
    let h = Harness::with_registry(ProfileRegistry::with_custom(&*PROFILES).unwrap());
    let ledger = MemoryLedger::default();
    restore(&h, &ledger, &source).await;
    let raw = migration_raw(&source, &history);
    let request = accept(&h, &raw);
    // Independently fix the grant from the verified source, before considering
    // any client mutation of the proposal below. Never copy client inputs into
    // an authorization expectation in a deployment.
    let policy = MigrationPolicy {
        allowed: true,
        expected: serde_json::from_value(migration_raw(&source, &history)["inputs"].clone())
            .unwrap(),
    };
    let preparer = migration_preparer(&h);
    let principal = migration_principal(&h, MIGRATOR);
    let keys = migration_keys(&h);
    let before = ledger.data.lock().unwrap().clone();
    let symbolic = h.transactions.log();
    for case in [
        "ordinary_actor",
        "missing_source",
        "wrong_history",
        "wrong_source_state",
        "wrong_command",
        "wrong_target",
        "revoked",
        "bad_signature",
    ] {
        let mut bad = raw.clone();
        let mut refusal_policy = policy.clone();
        match case {
            "ordinary_actor" => bad["actor"] = json!("operator:alice"),
            "missing_source" => {
                bad["inputs"]
                    .as_object_mut()
                    .unwrap()
                    .remove("source_history");
            }
            "wrong_history" => {
                bad["inputs"]["source_history"] = json!(hash_bytes(b"untrusted history").as_str())
            }
            "wrong_source_state" => {
                bad["inputs"]["source_state"] = json!(hash_bytes(b"untrusted state").as_str())
            }
            "wrong_command" => {
                bad["inputs"]["command"] = json!(
                    canonicalize(&VersionedCommand::Migrate {
                        enabled: true,
                        source_revision: 99
                    })
                    .unwrap()
                )
            }
            "wrong_target" => bad["resource_id"] = json!("config:other"),
            "revoked" => refusal_policy.allowed = false,
            "bad_signature" => {}
            _ => panic!("unknown migration refusal case"),
        }
        let mut bad = accept(&h, &bad);
        if case == "bad_signature" {
            bad.signature.as_mut().unwrap().sig = Signature::from_bytes([0; 64]);
        }
        let actor = migration_principal(&h, &bad.intent.actor.0);
        assert!(
            preparer
                .prepare_player(&bad, &actor, &refusal_policy, &keys)
                .await
                .is_err(),
            "{case} escaped migration policy"
        );
        unchanged(&ledger, &before);
        assert_eq!(h.transactions.log(), symbolic);
    }
    let prepared = preparer
        .prepare_player(&request, &principal, &policy, &keys)
        .await
        .unwrap();
    let (mut commit, _) = h.commit_events(prepared.events());
    let mut tree = StateAccumulator::empty();
    for event in source.events.iter().chain(prepared.events()) {
        tree.insert_batch(&state_root_updates(from_ref(event)).unwrap())
            .unwrap();
    }
    commit.body.commit_id = CommitId::new("cmt_authorized_migration".into()).unwrap();
    commit.body.sequence = 5;
    commit.body.parent_commit_id = Some(source.commits.last().unwrap().body.commit_id.clone());
    commit.body.previous_state_root = OLD_ROOT.parse().unwrap();
    commit.body.next_state_root = ContentDigest::new(*tree.root().as_bytes());
    let plan = SignedMutation::sign(prepared, &commit.body, h.key_id(), &Signer, vec![]).unwrap();
    let denied = MigrationPolicy {
        allowed: false,
        ..policy.clone()
    };
    ledger.calls.lock().unwrap().clear();
    assert!(matches!(
        plan.commit(&ledger, &denied, &Verify).await.unwrap_err(),
        DurableError::Authorization(_)
    ));
    assert!(ledger.calls.lock().unwrap().is_empty());
    unchanged(&ledger, &before);
    let lossy = MemoryLedger {
        lose_commit_response: true,
        ..ledger.clone()
    };
    assert!(matches!(
        plan.commit(&lossy, &policy, &Verify).await.unwrap_err(),
        DurableError::Unknown { .. }
    ));
    let committed = ledger.data.lock().unwrap().clone();
    ledger.calls.lock().unwrap().clear();
    assert!(matches!(
        recover_plan(&plan, &ledger, &denied).await.unwrap_err(),
        DurableError::Authorization(_)
    ));
    assert!(ledger.calls.lock().unwrap().is_empty());
    unchanged(&ledger, &committed);
    let reopened = MemoryLedger {
        data: ledger.data.clone(),
        reservations: ledger.reservations.clone(),
        ..Default::default()
    };
    match recover_plan(&plan, &reopened, &policy).await.unwrap() {
        RecoveryOutcome::Committed(receipt) => {
            assert_eq!(receipt.record().commit, *plan.signed_commit())
        }
        RecoveryOutcome::NotFound(_) | RecoveryOutcome::InProgress { .. } => {
            panic!("lost canonical migration")
        }
    }
    assert!(matches!(
        plan.commit(&reopened, &policy, &Verify).await.unwrap(),
        DurableOutcome::Replay(_)
    ));
    unchanged(&ledger, &committed);
    assert_eq!(committed.commits.len(), 5);
    assert_eq!(committed.projections.len(), 2);
    (source, plan, ledger)
}

#[tokio::test]
async fn migration_requires_separate_grant_and_pinned_original_source() {
    let (source, plan, _) = protected_migration().await;
    let mut current = None;
    for (intent, event, commit) in source
        .intents
        .iter()
        .zip(&source.events)
        .zip(&source.commits)
        .map(|((i, e), c)| (i, e, c))
    {
        CONFIGURATION
            .verify_transition(current.as_ref(), intent, event)
            .unwrap();
        current = Some(
            projections_for(
                &commit.body.commit_id,
                &[CommittedEvent {
                    event,
                    state_type: StateType::Custom,
                }],
            )
            .remove(0),
        );
    }
    let event = &plan.prepared().events()[0];
    V2.verify_transition(None, &plan.prepared().validated().intent, event)
        .unwrap();
    let migrated = V2.decode_state(&event.after.state).unwrap();
    assert_eq!(
        migrated,
        VersionedConfiguration {
            enabled: false,
            source_revision: 4,
            updates: 0
        }
    );
    assert!(
        V2.decode_state(&source.events.last().unwrap().after.state)
            .is_err()
    );
    assert!(CONFIGURATION.decode_state(&event.after.state).is_err());
    let mut relabeled = source.events.last().unwrap().after.state.clone();
    let ResourceState::Custom(ref mut state) = relabeled else {
        panic!("expected custom state");
    };
    state.schema_version = 2;
    assert!(
        V2.decode_state(&relabeled).is_err(),
        "old bytes reinterpreted as new schema"
    );
}

#[derive(Default)]
struct RebuildData {
    rows: BTreeMap<ResourceId, StateProjection>,
    offsets: BTreeMap<String, usize>,
}
#[derive(Default)]
struct Restart {
    data: Arc<Mutex<RebuildData>>,
    saves: AtomicUsize,
    fail_save: Option<(usize, bool)>,
}
#[async_trait]
impl ProjectionSink for Restart {
    async fn upsert_projection(&self, row: &StateProjection) -> Result<(), String> {
        let mut data = self.data.lock().unwrap();
        if let Some(prior) = data.rows.get(&row.resource_id) {
            if prior.version > row.version {
                return Ok(());
            }
            if prior.version == row.version && prior != row {
                return Err("conflicting equal version".into());
            }
        }
        data.rows.insert(row.resource_id.clone(), row.clone());
        Ok(())
    }
}
#[async_trait]
impl CheckpointStore for Restart {
    async fn load(&self, key: &str) -> Result<Option<usize>, String> {
        Ok(self.data.lock().unwrap().offsets.get(key).copied())
    }
    async fn save(&self, key: &str, offset: usize) -> Result<(), String> {
        let count = self.saves.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_save == Some((count, false)) {
            return Err("checkpoint interrupted before save".into());
        }
        self.data.lock().unwrap().offsets.insert(key.into(), offset);
        if self.fail_save == Some((count, true)) {
            return Err("checkpoint acknowledgement lost".into());
        }
        Ok(())
    }
}
fn rebuilt_root(store: &Restart) -> ContentDigest {
    let data = store.data.lock().unwrap();
    let updates: Vec<_> = data
        .rows
        .values()
        .map(|row| {
            assert_eq!(row.state_hash, canonicalize_and_digest(&row.state).unwrap());
            StateUpdate::new(
                StateKey::for_resource(&row.tenant_id.0, &row.resource_id.0),
                *row.state_hash.as_bytes(),
            )
        })
        .collect();
    let root = statechronicle::commit::roots::compute_state_root(&updates).unwrap();
    ContentDigest::new(*root.as_bytes())
}
async fn interruptions(history: &VerifiedHistory, count: usize, root: &ContentDigest) {
    let mut cases = 0;
    for width in 1..=count {
        for failure in 1..=count.div_ceil(width) {
            for after in [false, true] {
                let store = Restart {
                    fail_save: Some((failure, after)),
                    ..Default::default()
                };
                let key = history.checkpoint_key("migration-rebuild").unwrap();
                assert!(
                    history
                        .rebuild_resumable("migration-rebuild", width, &store, &store)
                        .await
                        .is_err()
                );
                let offset = store
                    .data
                    .lock()
                    .unwrap()
                    .offsets
                    .get(&key)
                    .copied()
                    .unwrap_or(0);
                let expected = if after {
                    (failure * width).min(count)
                } else {
                    (failure - 1) * width
                };
                assert_eq!(offset, expected);
                let reopened = Restart {
                    data: store.data.clone(),
                    ..Default::default()
                };
                history
                    .rebuild_resumable("migration-rebuild", width, &reopened, &reopened)
                    .await
                    .unwrap();
                assert_eq!(
                    reopened.data.lock().unwrap().offsets.get(&key),
                    Some(&count)
                );
                assert_eq!(&rebuilt_root(&reopened), root);
                let rows = reopened.data.lock().unwrap().rows.clone();
                history
                    .rebuild_resumable("migration-rebuild", width, &reopened, &reopened)
                    .await
                    .unwrap();
                assert_eq!(rows, reopened.data.lock().unwrap().rows);
                cases += 1;
            }
        }
    }
    println!("source_events={count} interrupted_checkpoint_cases={cases}");
}
#[tokio::test]
async fn original_and_migrated_histories_resume_at_every_bounded_checkpoint() {
    let (source, plan, _) = protected_migration().await;
    let old = verified(&source);
    interruptions(&old, 4, &OLD_ROOT.parse().unwrap()).await;
    let mut witnesses: Vec<_> = source
        .commits
        .iter()
        .zip(&source.events)
        .map(|(commit, event)| HistoryWitness {
            commit,
            events: from_ref(event),
        })
        .collect();
    witnesses.push(HistoryWitness {
        commit: plan.signed_commit(),
        events: plan.prepared().events(),
    });
    let pin = HistoryPin {
        tenant: common::tenant(),
        tip_digest: canonicalize_and_digest(&plan.signed_commit().body).unwrap(),
    };
    let full = verify_history(&witnesses, &pin, &Verify).unwrap();
    interruptions(&full, 5, &plan.signed_commit().body.next_state_root).await;
    assert_ne!(
        old.checkpoint_key("same-job").unwrap(),
        full.checkpoint_key("same-job").unwrap()
    );
    let store = Restart::default();
    old.rebuild_resumable("same-job", 1, &store, &store)
        .await
        .unwrap();
    assert_eq!(rebuilt_root(&store).as_str(), OLD_ROOT);
    full.rebuild_resumable("same-job", 2, &store, &store)
        .await
        .unwrap();
    assert_eq!(
        rebuilt_root(&store),
        plan.signed_commit().body.next_state_root
    );
    let rows = &store.data.lock().unwrap().rows;
    let old_state = CONFIGURATION
        .decode_state(
            &rows
                .get(&ResourceId("config:version1".into()))
                .unwrap()
                .state,
        )
        .unwrap();
    assert_eq!(old_state.revision, 4);
    assert!(!old_state.enabled);
    let new_state = V2
        .decode_state(
            &rows
                .get(&ResourceId("config:version2".into()))
                .unwrap()
                .state,
        )
        .unwrap();
    assert_eq!(new_state.source_revision, 4);
    assert!(!new_state.enabled);
}

#[test]
fn untrusted_original_history_cannot_supply_migration_authority() {
    let source = original();
    for case in [
        "altered_event",
        "missing_genesis",
        "wrong_signature",
        "resigned_fork",
    ] {
        let mut bad = source.clone();
        match case {
            "altered_event" => {
                bad.events[0].resource_id = ResourceId("config:provider-substitution".into())
            }
            "missing_genesis" => {
                bad.events.remove(0);
                bad.commits.remove(0);
            }
            "wrong_signature" => {
                bad.commits.last_mut().unwrap().signature.sig = Signature::from_bytes([0; 64])
            }
            "resigned_fork" => {
                let last = bad.commits.last_mut().unwrap();
                last.body.commit_id = CommitId::new("cmt_provider_fork".into()).unwrap();
                *last = sign_commit(&last.body, &fixed_key(), common::key_id()).unwrap();
            }
            _ => panic!("unknown history substitution"),
        }
        let witnesses: Vec<_> = bad
            .commits
            .iter()
            .zip(&bad.events)
            .map(|(commit, event)| HistoryWitness {
                commit,
                events: from_ref(event),
            })
            .collect();
        let pin = HistoryPin {
            tenant: common::tenant(),
            tip_digest: OLD_TIP.parse().unwrap(),
        };
        assert!(
            verify_history(&witnesses, &pin, &Verify).is_err(),
            "{case} supplied unchecked history authority"
        );
    }
}
