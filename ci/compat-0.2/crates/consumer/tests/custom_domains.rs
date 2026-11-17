//! Release gates for consumer-defined domains through the shared pipeline.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod common;
#[path = "../examples/common/domains.rs"]
mod domains;
#[path = "support/durable.rs"]
mod durable;
#[path = "support/versioned_recovery.rs"]
mod versioned_recovery;
use common::{Harness, fixed_key};
use domains::{
    CONFIGURATION, Configuration, ConfigurationCommand, EQUIPMENT, Equipment, EquipmentCommand,
    EquipmentMode, registry,
};
use durable::{Allow, MemoryLedger, Step, Verify};
use proptest::prelude::*;
use serde::Serialize;
use serde_json::{Value, json};
use statechronicle::CustomState;
use statechronicle::accumulator::key::StateKey;
use statechronicle::accumulator::sparse_merkle::{StateAccumulator, StateUpdate};
use statechronicle::commit::error::CommitError;
use statechronicle::commit::persist::{
    CommittedEvent, DurableCommitRequest, DurablePersistResult, persist_durable_verified,
    projections_for,
};
use statechronicle::commit::roots::state_root_updates;
use statechronicle::commit::sign::sign_commit;
use statechronicle::core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle::core::digest::hash_bytes;
use statechronicle::domain::intent::INTENT_SCHEMA;
use statechronicle::domain::resource_state::{ResourceState, UniqueAssetState};
use statechronicle::domain::status::Status;
use statechronicle::domain::subject::SubjectId;
use statechronicle::executor::error::ExecutorError;
use statechronicle::intent::validated::ValidatedIntent;
use statechronicle::portable::transition::{
    EncodedState, TransitionError, TransitionRequest, execute as execute_portable,
};
use statechronicle::ports::authorization::{AuthenticatedPrincipal, DenyAllAuthorizer};
use statechronicle::ports::ledger_store::{IdempotencyClaim, LedgerStore, OutboxRecord};
use statechronicle::ports::state_index::StateIndex;
use statechronicle::profiles::registry::{ProfileRegistry, ProfileRules};
use statechronicle::proof::bundle::{build_non_membership_proof, build_state_proof};
use statechronicle::proof::custom::verify_custom_bundle;
use statechronicle::proof::verify::{verify_bundle, verify_non_membership_bundle};
use statechronicle::{Commit, CommitId, ContentDigest, Event, Signed};
use statechronicle::{Operation, ProfileId, ResourceId, StateProfile, StateProjection, StateType};
use std::collections::BTreeMap;
use std::slice::from_ref;

fn inputs<P: StateProfile>(command: &P::Command) -> BTreeMap<String, Value>
where
    P::Command: Serialize,
{
    BTreeMap::from([
        (String::from("profile"), json!(P::DOMAIN_ID)),
        (String::from("schema_version"), json!(P::SCHEMA_VERSION)),
        (
            String::from("command"),
            json!(canonicalize(command).unwrap()),
        ),
    ])
}
fn payload<P: StateProfile>(
    id: &str,
    resource: &str,
    version: u64,
    profile: &P,
    command: &P::Command,
) -> Value
where
    P::Command: Serialize,
{
    json!({ "schema": INTENT_SCHEMA, "tenant_id": "acme.game.alpha", "intent_id": id,
        "operation": profile.operation(command), "actor": "operator:alice", "resource_id": resource,
        "state_type": "custom", "expected_version": version, "inputs": inputs::<P>(command),
        "created_at": "2026-07-14T00:00:00Z", "nonce": "b64u:AA" })
}

struct PreparedMutation {
    accepted: ValidatedIntent,
    event: Event,
    commit: Signed<Commit>,
}
async fn deliver_prepared(
    ledger: &MemoryLedger,
    principal: &AuthenticatedPrincipal,
    prepared: &PreparedMutation,
) -> Result<DurablePersistResult, CommitError> {
    let entries = [CommittedEvent {
        event: &prepared.event,
        state_type: StateType::Custom,
    }];
    let projections = projections_for(&prepared.commit.body.commit_id, &entries);
    let digest = canonicalize_and_digest(&prepared.accepted.intent).unwrap();
    persist_durable_verified(
        ledger,
        &Allow,
        &Verify,
        DurableCommitRequest {
            principal,
            intent: &prepared.accepted.intent,
            payload_digest: &digest,
            entries: &entries,
            commit: &prepared.commit,
            projections: &projections,
            outbox: &[],
        },
    )
    .await
}

#[tokio::test]
async fn seeded_reordered_concurrent_domain_history_recovers_without_forks() {
    let producer = Harness::with_registry(registry().unwrap());
    let mut prepared = Vec::new();
    let mut tree = StateAccumulator::empty();
    let mut parent = None;
    let mut previous_root = ContentDigest::new(*tree.root().as_bytes());
    for index in 0_u64..24 {
        let raw = if index % 2 == 0 {
            let command = if index == 0 {
                ConfigurationCommand::Create { enabled: true }
            } else {
                ConfigurationCommand::SetEnabled {
                    enabled: index % 4 == 0,
                }
            };
            payload(
                &format!("int_chaos_{index}"),
                "config:chaos",
                index / 2,
                &Configuration,
                &command,
            )
        } else {
            let command = match (index / 2) % 3 {
                0 if index == 1 => EquipmentCommand::Install,
                0 => EquipmentCommand::Recover,
                1 => EquipmentCommand::Start,
                _ => EquipmentCommand::Fail,
            };
            payload(
                &format!("int_chaos_{index}"),
                "equipment:chaos",
                index / 2,
                &Equipment,
                &command,
            )
        };
        let accepted = producer.accept(&raw, Some(producer.authority()));
        let event = producer.run(&accepted, StateType::Custom).await;
        let (mut signed, _) = producer.commit_events(from_ref(&event));
        tree.insert_batch(&state_root_updates(from_ref(&event)).unwrap())
            .unwrap();
        signed.body.commit_id = CommitId::new(format!("cmt_chaos_{index}")).unwrap();
        signed.body.sequence = index + 1;
        signed.body.parent_commit_id = parent;
        signed.body.previous_state_root = previous_root;
        signed.body.next_state_root = ContentDigest::new(*tree.root().as_bytes());
        parent = Some(signed.body.commit_id.clone());
        previous_root = signed.body.next_state_root.clone();
        signed = sign_commit(&signed.body, &fixed_key(), producer.key_id()).unwrap();
        prepared.push(PreparedMutation {
            accepted,
            event,
            commit: signed,
        });
    }
    let principal = AuthenticatedPrincipal {
        tenant: producer.tenant(),
        subject: prepared[0].accepted.intent.actor.clone(),
        credential_id: String::from("chaos-session"),
    };
    for seed in 1_u64..=8 {
        let mut random = seed;
        let mut ledger = MemoryLedger {
            yield_each_step: true,
            ..MemoryLedger::default()
        };
        for _ in 0..64 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let index = random as usize % prepared.len();
            ledger.faults = if random % 5 == 0 {
                vec![Step::Events]
            } else {
                vec![]
            };
            ledger.lose_commit_response = random % 7 == 0;
            let next_index = (index + 1) % prepared.len();
            let (_first, _second) = tokio::join!(
                deliver_prepared(&ledger, &principal, &prepared[index]),
                deliver_prepared(&ledger, &principal, &prepared[next_index])
            );
            let stored = ledger.data.lock().unwrap();
            assert_eq!(stored.events.len(), stored.commits.len());
            assert_eq!(stored.events.len(), stored.identities.len());
            for (actual, expected) in stored.events.iter().zip(&prepared) {
                assert_eq!(actual, &expected.event);
            }
            for (actual, expected) in stored.commits.iter().zip(&prepared) {
                assert_eq!(actual, &expected.commit);
            }
            assert!(stored.projections.len() <= 2);
            assert!(ledger.reservations.lock().unwrap().is_empty());
        }
        // Restart with the same persisted state and healthy facilities. Retry
        // original identities in order; already committed requests replay.
        ledger.faults.clear();
        ledger.lose_commit_response = false;
        let restarted = MemoryLedger { ..ledger };
        for mutation in &prepared {
            let (one, two) = tokio::join!(
                deliver_prepared(&restarted, &principal, mutation),
                deliver_prepared(&restarted, &principal, mutation)
            );
            assert!(one.is_ok() || two.is_ok());
            assert!(matches!(
                deliver_prepared(&restarted, &principal, mutation)
                    .await
                    .unwrap(),
                DurablePersistResult::Replay { .. }
            ));
        }
        let stored = restarted.data.lock().unwrap();
        assert_eq!(stored.events.len(), prepared.len());
        assert_eq!(stored.commits.len(), prepared.len());
        assert_eq!(stored.projections.len(), 2);
        assert_eq!(
            stored
                .commits
                .last()
                .unwrap()
                .body
                .next_state_root
                .as_bytes(),
            tree.root().as_bytes()
        );
        let mut replayed = StateAccumulator::empty();
        for event in &stored.events {
            replayed
                .insert_batch(&state_root_updates(from_ref(event)).unwrap())
                .unwrap();
        }
        assert_eq!(replayed.root(), tree.root());
    }
}
#[tokio::test]
async fn independent_domains_execute_replay_commit_and_prove() {
    let harness = Harness::with_registry(registry().unwrap());
    let create = harness.accept(
        &payload(
            "int_config_create",
            "config:main",
            0,
            &Configuration,
            &ConfigurationCommand::Create { enabled: false },
        ),
        Some(harness.authority()),
    );
    let created = harness.run(&create, StateType::Custom).await;
    CONFIGURATION
        .verify_transition(None, &create.intent, &created)
        .unwrap();
    let prior = harness
        .index
        .get_state(&harness.tenant(), &ResourceId(String::from("config:main")))
        .await
        .unwrap()
        .unwrap();
    let update = harness.accept(
        &payload(
            "int_config_update",
            "config:main",
            1,
            &Configuration,
            &ConfigurationCommand::SetEnabled { enabled: true },
        ),
        Some(harness.authority()),
    );
    let updated = harness.run(&update, StateType::Custom).await;
    CONFIGURATION
        .verify_transition(Some(&prior), &update.intent, &updated)
        .unwrap();
    let mut unsupported_event = updated.clone();
    unsupported_event.schema = String::from("statechronicle.event.future");
    assert!(
        CONFIGURATION
            .verify_transition(Some(&prior), &update.intent, &unsupported_event)
            .is_err()
    );
    let mut zero_version = prior.clone();
    zero_version.version = 0;
    let mut zero_intent = update.intent.clone();
    zero_intent.expected_version = 0;
    let mut zero_event = updated.clone();
    zero_event.before.version = 0;
    zero_event.after.version = 1;
    assert!(
        CONFIGURATION
            .verify_transition(Some(&zero_version), &zero_intent, &zero_event)
            .is_err()
    );
    let install = harness.accept(
        &payload(
            "int_equipment_install",
            "machine:pump",
            0,
            &Equipment,
            &EquipmentCommand::Install,
        ),
        Some(harness.authority()),
    );
    let installed = harness.run(&install, StateType::Custom).await;
    let start = harness.accept(
        &payload(
            "int_equipment_start",
            "machine:pump",
            1,
            &Equipment,
            &EquipmentCommand::Start,
        ),
        Some(harness.authority()),
    );
    let started = harness.run(&start, StateType::Custom).await;
    assert!(harness.executor.execute(&update).await.unwrap().is_empty());
    let events = vec![created, updated.clone(), installed, started.clone()];
    let (signed, accumulator) = harness.commit_events(&events);
    let mut replayed = StateAccumulator::empty();
    for event in &events {
        replayed
            .insert_batch(&state_root_updates(from_ref(event)).unwrap())
            .unwrap();
    }
    assert_eq!(replayed.root(), accumulator.root());
    for event in [updated.clone(), started] {
        let projection = StateProjection {
            tenant_id: event.tenant_id.clone(),
            resource_id: event.resource_id.clone(),
            state_type: StateType::Custom,
            version: event.after.version,
            last_event_id: event.event_id.clone(),
            last_commit_id: signed.body.commit_id.clone(),
            state_hash: event.after.state_hash.clone(),
            state: event.after.state.clone(),
        };
        let key = StateKey::for_resource(&projection.tenant_id.0, &projection.resource_id.0);
        let proof = build_state_proof(
            &projection,
            &signed,
            &accumulator.prove_inclusion(&key).unwrap(),
            &event.operation,
            None,
            key,
        )
        .unwrap();
        verify_bundle(&proof, &signed, &fixed_key().verifying_key(), &key).unwrap();
        if event.resource_id == ResourceId(String::from("config:main")) {
            let state = verify_custom_bundle(
                &proof,
                &signed,
                &fixed_key().verifying_key(),
                &key,
                &CONFIGURATION,
            )
            .unwrap();
            assert_eq!(state.revision, 2);
            assert!(state.enabled);
            assert!(
                verify_custom_bundle(
                    &proof,
                    &signed,
                    &fixed_key().verifying_key(),
                    &key,
                    &EQUIPMENT
                )
                .is_err()
            );
            let mut tampered = proof.clone();
            if let ResourceState::Custom(custom) = &mut tampered.claimed_state {
                custom.schema_version = 2;
            }
            assert!(verify_bundle(&tampered, &signed, &fixed_key().verifying_key(), &key).is_err());
            let mut retargeted = proof.clone();
            retargeted.resource_id = ResourceId(String::from("config:other"));
            assert!(
                verify_custom_bundle(
                    &retargeted,
                    &signed,
                    &fixed_key().verifying_key(),
                    &key,
                    &CONFIGURATION
                )
                .is_err()
            );
        } else {
            assert_eq!(
                verify_custom_bundle(
                    &proof,
                    &signed,
                    &fixed_key().verifying_key(),
                    &key,
                    &EQUIPMENT
                )
                .unwrap()
                .mode,
                EquipmentMode::Running
            );
        }
    }
    let absent = ResourceId(String::from("config:absent"));
    let key = StateKey::for_resource(&harness.tenant().0, &absent.0);
    let proof = build_non_membership_proof(
        &harness.tenant(),
        &absent,
        &key,
        &signed,
        &accumulator.prove_non_membership(&key).unwrap(),
    )
    .unwrap();
    verify_non_membership_bundle(&proof, &signed, &fixed_key().verifying_key(), &key).unwrap();
    let mut tampered = updated;
    tampered.after.version = 7;
    assert!(
        CONFIGURATION
            .verify_transition(Some(&prior), &update.intent, &tampered)
            .is_err()
    );
}

#[tokio::test]
async fn custom_failures_reject_before_emitting_events() {
    let harness = Harness::with_registry(registry().unwrap());
    let raw = payload(
        "int_missing_authority",
        "config:main",
        0,
        &Configuration,
        &ConfigurationCommand::Create { enabled: false },
    );
    let missing = harness.accept(&raw, None);
    assert!(matches!(
        harness.executor.execute(&missing).await,
        Err(ExecutorError::AuthorityMissing { .. })
    ));
    let create = harness.accept(
        &payload(
            "int_create",
            "config:main",
            0,
            &Configuration,
            &ConfigurationCommand::Create { enabled: false },
        ),
        Some(harness.authority()),
    );
    harness.run(&create, StateType::Custom).await;
    let stale = harness.accept(
        &payload(
            "int_stale",
            "config:main",
            0,
            &Configuration,
            &ConfigurationCommand::SetEnabled { enabled: true },
        ),
        Some(harness.authority()),
    );
    assert!(matches!(
        harness.executor.execute(&stale).await,
        Err(ExecutorError::ExpectedVersionMismatch { .. })
    ));
    let wrong_domain = harness.accept(
        &payload(
            "int_wrong_domain",
            "config:main",
            1,
            &Equipment,
            &EquipmentCommand::Start,
        ),
        Some(harness.authority()),
    );
    assert!(harness.executor.execute(&wrong_domain).await.is_err());
    let mut bad_schema = payload(
        "int_schema",
        "config:main",
        1,
        &Configuration,
        &ConfigurationCommand::SetEnabled { enabled: true },
    );
    bad_schema["inputs"]["schema_version"] = json!(2);
    assert!(
        harness
            .executor
            .execute(&harness.accept(&bad_schema, Some(harness.authority())))
            .await
            .is_err()
    );
    let illegal = harness.accept(
        &payload(
            "int_illegal",
            "machine:pump",
            0,
            &Equipment,
            &EquipmentCommand::Start,
        ),
        Some(harness.authority()),
    );
    assert!(harness.executor.execute(&illegal).await.is_err());
    let mut swapped = payload(
        "int_swapped",
        "config:other",
        0,
        &Configuration,
        &ConfigurationCommand::Create { enabled: true },
    );
    swapped["operation"] = json!("configuration.set_enabled");
    assert!(
        harness
            .executor
            .execute(&harness.accept(&swapped, Some(harness.authority())))
            .await
            .is_err()
    );
}

#[test]
fn registry_and_encoding_boundaries_fail_closed() {
    static DUPLICATES: std::sync::LazyLock<[&'static dyn ProfileRules; 2]> =
        std::sync::LazyLock::new(|| [&*CONFIGURATION, &*CONFIGURATION]);
    assert!(ProfileRegistry::with_custom(&*DUPLICATES).is_err());
    let mut input = inputs::<Configuration>(&ConfigurationCommand::Create { enabled: true });
    input.insert(String::from("command"), json!(vec![0u8; 65]));
    assert!(
        CONFIGURATION
            .apply(
                None,
                &Operation::from_static("configuration.create"),
                &input
            )
            .is_err()
    );
    input.insert(String::from("command"), json!([0, 2])); // invalid BCS bool
    assert!(
        CONFIGURATION
            .apply(
                None,
                &Operation::from_static("configuration.create"),
                &input
            )
            .is_err()
    );
    let state = CONFIGURATION
        .apply(
            None,
            &Operation::from_static("configuration.create"),
            &inputs::<Configuration>(&ConfigurationCommand::Create { enabled: true }),
        )
        .unwrap();
    let mut other = state.clone();
    if let ResourceState::Custom(custom) = &mut other {
        custom.domain = ProfileId::new(String::from(Equipment::DOMAIN_ID)).unwrap();
    }
    assert_ne!(
        canonicalize_and_digest(&state).unwrap(),
        canonicalize_and_digest(&other).unwrap()
    );
    assert!(CONFIGURATION.decode_state(&other).is_err());
}

#[test]
fn baseline_bcs_discriminants_are_unchanged() {
    let state = ResourceState::UniqueAsset(UniqueAssetState {
        owner: SubjectId(String::from("a")),
        status: Status::from_static("active"),
        trade_id: None,
    });
    assert_eq!(
        canonicalize(&state).unwrap(),
        vec![0, 1, b'a', 6, b'a', b'c', b't', b'i', b'v', b'e', 0]
    );
    assert_eq!(canonicalize(&StateType::Escrow).unwrap(), vec![6]);
    assert_eq!(canonicalize(&StateType::Custom).unwrap(), vec![7]);
}

#[test]
fn portable_transition_rejects_versions_identity_encoding_and_capacity() {
    let live = EncodedState {
        domain: Configuration::DOMAIN_ID,
        schema: 1,
        version: 1,
        payload: &[1, 0, 0, 0, 1],
    };
    let request = TransitionRequest {
        domain: Configuration::DOMAIN_ID,
        schema: 1,
        expected_version: 1,
        operation: "configuration.set_enabled",
        command: &[1, 0],
        current: Some(live),
    };
    let mut output = [0; 64];
    let mut scratch = [0; 64];
    assert_eq!(
        execute_portable(
            &Configuration,
            &TransitionRequest {
                expected_version: 0,
                ..request
            },
            &mut output,
            &mut scratch
        ),
        Err(TransitionError::Version)
    );
    assert_eq!(
        execute_portable(
            &Configuration,
            &TransitionRequest {
                expected_version: u64::MAX,
                current: Some(EncodedState {
                    version: u64::MAX,
                    ..live
                }),
                ..request
            },
            &mut output,
            &mut scratch
        ),
        Err(TransitionError::Version)
    );
    assert_eq!(
        execute_portable(
            &Configuration,
            &TransitionRequest {
                schema: 2,
                ..request
            },
            &mut output,
            &mut scratch
        ),
        Err(TransitionError::Identity)
    );
    assert_eq!(
        execute_portable(
            &Configuration,
            &TransitionRequest {
                current: Some(EncodedState {
                    domain: Equipment::DOMAIN_ID,
                    ..live
                }),
                ..request
            },
            &mut output,
            &mut scratch
        ),
        Err(TransitionError::Identity)
    );
    assert_eq!(
        execute_portable(
            &Configuration,
            &TransitionRequest {
                operation: "configuration.create",
                ..request
            },
            &mut output,
            &mut scratch
        ),
        Err(TransitionError::Operation)
    );
    assert_eq!(
        execute_portable(
            &Configuration,
            &TransitionRequest {
                command: &[1, 2],
                ..request
            },
            &mut output,
            &mut scratch
        ),
        Err(TransitionError::Codec(
            statechronicle::portable::codec::CodecError::NonCanonical
        ))
    );
    assert_eq!(
        execute_portable(
            &Configuration,
            &TransitionRequest {
                current: Some(EncodedState {
                    payload: &[0, 0, 0, 0, 1],
                    ..live
                }),
                ..request
            },
            &mut output,
            &mut scratch
        ),
        Err(TransitionError::Domain("revision must be positive"))
    );
    assert_eq!(
        execute_portable(&Configuration, &request, &mut output[..4], &mut scratch),
        Err(TransitionError::Codec(
            statechronicle::portable::codec::CodecError::Capacity
        ))
    );
    assert_eq!(
        execute_portable(&Configuration, &request, &mut output, &mut scratch[..1]),
        Err(TransitionError::Codec(
            statechronicle::portable::codec::CodecError::Capacity
        ))
    );
}
proptest! {
    #[test]
    fn configuration_roundtrip_and_reduction_are_deterministic(enabled in any::<bool>()) {
        let input = inputs::<Configuration>(&ConfigurationCommand::Create { enabled });
        let operation = Operation::from_static("configuration.create");
        let first = CONFIGURATION.apply(None, &operation, &input).unwrap();
        prop_assert_eq!(&first, &CONFIGURATION.apply(None, &operation, &input).unwrap());
        prop_assert_eq!(CONFIGURATION.decode_state(&first).unwrap().enabled, enabled);
        let encoded = canonicalize(&first).unwrap();
        prop_assert_eq!(first, bcs::from_bytes::<ResourceState>(&encoded).unwrap());
    }
    #[test]
    fn portable_configuration_matches_standard_transitions(
        actions in proptest::collection::vec(any::<bool>(), 1..64),
    ) {
        let mut current = None;
        let mut version = 0_u64;
        for enabled in actions {
            let command = if current.is_none() { ConfigurationCommand::Create { enabled } }
                else { ConfigurationCommand::SetEnabled { enabled } };
            let operation = Configuration.operation(&command);
            let canonical_command = canonicalize(&command).unwrap();
            let native_current = current.as_ref().map(|state| CONFIGURATION.decode_state(state).unwrap());
            let native_bytes = native_current.as_ref().map(|native| canonicalize(native).unwrap());
            let request = TransitionRequest {
                domain: Configuration::DOMAIN_ID, schema: 1, expected_version: version,
                operation, command: &canonical_command,
                current: native_bytes.as_ref().map(|bytes| EncodedState {
                    domain: Configuration::DOMAIN_ID, schema: 1, version, payload: bytes,
                }),
            };
            let mut output = [0_u8; 64];
            let mut scratch = [0_u8; 64];
            let reduced = execute_portable(&Configuration, &request, &mut output, &mut scratch).unwrap();
            let native_next = Configuration.apply(native_current.as_ref(), &command).unwrap();
            let next = ResourceState::Custom(CustomState {
                domain: ProfileId::new(String::from(Configuration::DOMAIN_ID)).unwrap(),
                schema_version: 1, payload: canonicalize(&native_next).unwrap(),
            });
            prop_assert_eq!(reduced.state_hash, *canonicalize_and_digest(&next).unwrap().as_bytes());
            prop_assert_eq!(&output[..reduced.payload_len], canonicalize(&CONFIGURATION.decode_state(&next).unwrap()).unwrap());
            let substituted = TransitionRequest { domain: Equipment::DOMAIN_ID, ..request };
            prop_assert_eq!(execute_portable(&Configuration, &substituted, &mut output, &mut scratch), Err(TransitionError::Identity));
            version = reduced.version;
            current = Some(next);
        }
    }
}

#[tokio::test]
async fn custom_domain_uses_atomic_ledger_port_with_rollback_and_replay() {
    let harness = Harness::with_registry(registry().unwrap());
    let create = harness.accept(
        &payload(
            "int_durable",
            "config:main",
            0,
            &Configuration,
            &ConfigurationCommand::Create { enabled: true },
        ),
        Some(harness.authority()),
    );
    let event = harness.run(&create, StateType::Custom).await;
    let (signed, _) = harness.commit_events(from_ref(&event));
    let principal = AuthenticatedPrincipal {
        tenant: create.intent.tenant_id.clone(),
        subject: create.intent.actor.clone(),
        credential_id: String::from("operator-session"),
    };
    let entries = [CommittedEvent {
        event: &event,
        state_type: StateType::Custom,
    }];
    let projections = projections_for(&signed.body.commit_id, &entries);
    let digest = canonicalize_and_digest(&create.intent).unwrap();
    let request = || DurableCommitRequest {
        principal: &principal,
        intent: &create.intent,
        payload_digest: &digest,
        entries: &entries,
        commit: &signed,
        projections: &projections,
        outbox: &[],
    };
    let ledger = MemoryLedger {
        fail_projection: true,
        ..MemoryLedger::default()
    };
    assert!(
        persist_durable_verified(&ledger, &Allow, &Verify, request())
            .await
            .is_err()
    );
    assert!(!ledger.data.lock().unwrap().finalized);
    assert!(ledger.data.lock().unwrap().events.is_empty());
    let ledger = MemoryLedger {
        fail_projection: false,
        ..ledger
    };
    assert!(matches!(
        persist_durable_verified(&ledger, &Allow, &Verify, request())
            .await
            .unwrap(),
        DurablePersistResult::Committed { .. }
    ));
    assert_eq!(ledger.data.lock().unwrap().projections, projections);
    assert!(matches!(
        persist_durable_verified(&ledger, &Allow, &Verify, request())
            .await
            .unwrap(),
        DurablePersistResult::Replay { .. }
    ));
    assert_eq!(ledger.data.lock().unwrap().events.len(), 1);
    let mut wrong = create.intent.clone();
    wrong
        .inputs
        .insert(String::from("profile"), json!(Equipment::DOMAIN_ID));
    let wrong_digest = canonicalize_and_digest(&wrong).unwrap();
    let wrong_request = DurableCommitRequest {
        intent: &wrong,
        payload_digest: &wrong_digest,
        ..request()
    };
    assert!(
        persist_durable_verified(&MemoryLedger::default(), &Allow, &Verify, wrong_request)
            .await
            .is_err()
    );
}

#[test]
fn custom_domain_conformance_vectors() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../docs/CONFORMANCE/custom_domains.json"
    ))
    .unwrap();
    let natives = [
        canonicalize(&domains::ConfigurationState {
            revision: 1,
            enabled: true,
        })
        .unwrap(),
        canonicalize(&domains::EquipmentState {
            mode: EquipmentMode::Idle,
            cycles: 0,
        })
        .unwrap(),
    ];
    for (vector, native) in fixture["vectors"].as_array().unwrap().iter().zip(natives) {
        let state = ResourceState::Custom(CustomState {
            domain: ProfileId::new(vector["domain"].as_str().unwrap().to_owned()).unwrap(),
            schema_version: 1,
            payload: native,
        });
        let bytes = canonicalize(&state).unwrap();
        let hex = |value: &[u8]| {
            value
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        assert_eq!(hex(&bytes), vector["state_bcs_hex"]);
        let digest = canonicalize_and_digest(&state).unwrap();
        assert_eq!(digest.as_str(), vector["state_digest"].as_str().unwrap());
        let key = StateKey::for_resource(
            vector["tenant"].as_str().unwrap(),
            vector["resource"].as_str().unwrap(),
        );
        assert_eq!(hex(key.as_bytes()), vector["state_key_hex"]);
        let mut tree = StateAccumulator::empty();
        tree.insert_batch(&[StateUpdate::new(key, *digest.as_bytes())])
            .unwrap();
        assert_eq!(hex(tree.root().as_bytes()), vector["single_leaf_root_hex"]);
    }
}

#[tokio::test]
async fn durable_fault_matrix_cancellation_and_response_loss() {
    let harness = Harness::with_registry(registry().unwrap());
    let accepted = harness.accept(
        &payload(
            "int_fault_matrix",
            "config:fault",
            0,
            &Configuration,
            &ConfigurationCommand::Create { enabled: true },
        ),
        Some(harness.authority()),
    );
    let event = harness.run(&accepted, StateType::Custom).await;
    let (signed, _) = harness.commit_events(from_ref(&event));
    let principal = AuthenticatedPrincipal {
        tenant: accepted.intent.tenant_id.clone(),
        subject: accepted.intent.actor.clone(),
        credential_id: String::from("fault-test"),
    };
    let entries = [CommittedEvent {
        event: &event,
        state_type: StateType::Custom,
    }];
    let projections = projections_for(&signed.body.commit_id, &entries);
    let digest = canonicalize_and_digest(&accepted.intent).unwrap();
    let notification = canonicalize(&event).unwrap();
    let outbox = [OutboxRecord {
        delivery_key: String::from("fault-notification"),
        tenant: principal.tenant.clone(),
        commit_id: signed.body.commit_id.clone(),
        payload_digest: hash_bytes(&notification),
        payload: notification,
    }];
    let request = || DurableCommitRequest {
        principal: &principal,
        intent: &accepted.intent,
        payload_digest: &digest,
        entries: &entries,
        commit: &signed,
        projections: &projections,
        outbox: &outbox,
    };
    let steps = [
        Step::Begin,
        Step::Claim,
        Step::Events,
        Step::CommitRow,
        Step::Projection,
        Step::Outbox,
        Step::Finalize,
        Step::Commit,
    ];
    let preflight = MemoryLedger::default();
    assert!(
        persist_durable_verified(&preflight, &DenyAllAuthorizer, &Verify, request())
            .await
            .is_err()
    );
    assert!(preflight.calls.lock().unwrap().is_empty());
    let mut corrupted = signed.clone();
    corrupted.body.event_count += 1;
    assert!(
        persist_durable_verified(
            &preflight,
            &Allow,
            &Verify,
            DurableCommitRequest {
                commit: &corrupted,
                ..request()
            }
        )
        .await
        .is_err()
    );
    assert!(preflight.calls.lock().unwrap().is_empty());
    for step in steps {
        for rollback_fails in [false, true] {
            let ledger = MemoryLedger {
                faults: if rollback_fails {
                    vec![step, Step::Rollback]
                } else {
                    vec![step]
                },
                ..MemoryLedger::default()
            };
            let error = persist_durable_verified(&ledger, &Allow, &Verify, request())
                .await
                .unwrap_err();
            assert!(error.to_string().contains(&format!("{step:?}")));
            assert!(ledger.calls.lock().unwrap().contains(&step));
            if step != Step::Begin && step != Step::Commit {
                assert!(ledger.calls.lock().unwrap().contains(&Step::Rollback));
            }
            assert!(ledger.data.lock().unwrap().events.is_empty());
            assert!(ledger.data.lock().unwrap().commits.is_empty());
            assert!(ledger.data.lock().unwrap().projections.is_empty());
            assert!(ledger.data.lock().unwrap().outbox.is_empty());
            assert!(ledger.data.lock().unwrap().identities.is_empty());
            assert!(ledger.reservations.lock().unwrap().is_empty());
            let recovered = MemoryLedger {
                faults: vec![],
                ..ledger
            };
            assert!(matches!(
                persist_durable_verified(&recovered, &Allow, &Verify, request())
                    .await
                    .unwrap(),
                DurablePersistResult::Committed { .. }
            ));
            assert!(matches!(
                persist_durable_verified(&recovered, &Allow, &Verify, request())
                    .await
                    .unwrap(),
                DurablePersistResult::Replay { .. }
            ));
            assert_eq!(recovered.data.lock().unwrap().events.len(), 1);
            assert_eq!(recovered.data.lock().unwrap().outbox, outbox);
        }
    }
    // Cancel at each await boundary. A consumer adapter owns cleanup when its
    // transaction is dropped; there is no runtime-specific cancellation hook.
    for step in steps {
        let ledger = MemoryLedger {
            pause: Some(step),
            ..MemoryLedger::default()
        };
        let mut future = Box::pin(persist_durable_verified(
            &ledger,
            &Allow,
            &Verify,
            request(),
        ));
        assert!(matches!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(future.as_mut().poll(cx))).await,
            std::task::Poll::Pending
        ));
        drop(future);
        assert!(ledger.data.lock().unwrap().events.is_empty());
        assert!(ledger.reservations.lock().unwrap().is_empty());
        let recovered = MemoryLedger {
            pause: None,
            ..ledger
        };
        assert!(
            persist_durable_verified(&recovered, &Allow, &Verify, request())
                .await
                .is_ok()
        );
    }
    let ledger = MemoryLedger {
        lose_commit_response: true,
        ..MemoryLedger::default()
    };
    assert!(
        persist_durable_verified(&ledger, &Allow, &Verify, request())
            .await
            .is_err()
    );
    assert_eq!(ledger.data.lock().unwrap().events.len(), 1);
    let restarted = MemoryLedger {
        lose_commit_response: false,
        ..ledger
    };
    assert!(matches!(
        persist_durable_verified(&restarted, &Allow, &Verify, request())
            .await
            .unwrap(),
        DurablePersistResult::Replay { .. }
    ));
    assert_eq!(restarted.data.lock().unwrap().events.len(), 1);

    // Competing owners cannot reserve one identity twice; altered canonical
    // payloads cannot replay a previously committed identity.
    let fresh = MemoryLedger::default();
    let mut owner = fresh.begin(&principal.tenant).await.unwrap();
    assert!(matches!(
        owner
            .claim_idempotency(&principal.tenant, &accepted.intent, &digest)
            .await
            .unwrap(),
        IdempotencyClaim::NewReservation { .. }
    ));
    let mut competitor = fresh.begin(&principal.tenant).await.unwrap();
    assert!(matches!(
        competitor
            .claim_idempotency(&principal.tenant, &accepted.intent, &digest)
            .await
            .unwrap(),
        IdempotencyClaim::InProgress { .. }
    ));
    competitor.rollback().await.unwrap();
    drop(owner);
    assert!(fresh.reservations.lock().unwrap().is_empty());
    let mut transaction = restarted.begin(&principal.tenant).await.unwrap();
    assert_eq!(
        transaction
            .claim_idempotency(&principal.tenant, &accepted.intent, &hash_bytes(b"changed"))
            .await
            .unwrap(),
        IdempotencyClaim::ConflictDifferentPayload
    );
    transaction.rollback().await.unwrap();

    // Reproducible schedules mix outages, acknowledgement loss and repeated
    // delivery. Observe durable invariants after every action, not just at the end.
    for seed in 1_u64..=16 {
        let mut random = seed;
        let mut chaos_ledger = MemoryLedger::default();
        let mut committed = false;
        for _ in 0..32 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            chaos_ledger.faults = match random % 5 {
                0 => vec![steps[(random as usize >> 8) % steps.len()]],
                1 => vec![Step::Rollback],
                _ => vec![],
            };
            chaos_ledger.lose_commit_response = random % 7 == 0;
            let result = persist_durable_verified(&chaos_ledger, &Allow, &Verify, request()).await;
            let data = chaos_ledger.data.lock().unwrap();
            committed |= !data.events.is_empty();
            assert_eq!(data.events.len(), usize::from(committed));
            assert_eq!(data.commits.len(), data.events.len());
            assert_eq!(data.projections.len(), data.events.len());
            assert_eq!(data.outbox.len(), data.events.len());
            assert_eq!(data.identities.len(), data.events.len());
            if result.is_ok() {
                assert!(committed);
            }
            assert!(chaos_ledger.reservations.lock().unwrap().is_empty());
        }
        chaos_ledger.faults.clear();
        chaos_ledger.lose_commit_response = false;
        assert!(
            persist_durable_verified(&chaos_ledger, &Allow, &Verify, request())
                .await
                .is_ok()
        );
        assert_eq!(chaos_ledger.data.lock().unwrap().events.len(), 1);
    }
}
