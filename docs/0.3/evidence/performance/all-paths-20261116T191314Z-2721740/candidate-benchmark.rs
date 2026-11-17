//! Repeated, fixed-workload observations; no universal performance threshold.
#![deny(unsafe_code)]
#[path = "common/batch_bench.rs"]
mod batch_bench;
#[path = "../tests/common/mod.rs"]
mod common;
#[path = "common/domains.rs"]
mod domains;
#[path = "../tests/support/durable.rs"]
mod durable;
#[path = "common/proof_bench.rs"]
mod proof_bench;
#[path = "common/rebuild_bench.rs"]
mod rebuild_bench;
#[path = "common/trade_bench.rs"]
mod trade_bench;

use common::{Harness, fixed_key};
use domains::{CONFIGURATION, Configuration, ConfigurationCommand, ConfigurationState, registry};
use durable::{Allow, MemoryLedger, Verify};
use serde::Deserialize;
use serde_json::{Value, json};
use statechronicle::commit::persist::{
    CommittedEvent, DurableCommitRequest, DurablePersistResult, persist_durable_verified,
    projections_for,
};
use statechronicle::commit::roots::state_root_updates;
use statechronicle::commit::sign::sign_commit;
use statechronicle::core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle::domain::proof::{NonMembershipProofBundle, ResourceStateProof};
use statechronicle::portable::transition::{EncodedState, TransitionRequest, execute};
use statechronicle::ports::authorization::AuthenticatedPrincipal;
use statechronicle::profiles::registry::ProfileRules;
use statechronicle::proof::verify::{verify_bundle, verify_non_membership_bundle};
use statechronicle::{
    Commit, Event, Operation, Signed, StateAccumulator, StateKey, StateProfile, StateProjection,
    StateType,
};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

#[derive(Deserialize)]
struct Corpus {
    events: Vec<Event>,
    signed_commit: Signed<Commit>,
    state_proof: ResourceStateProof,
    absence_proof: NonMembershipProofBundle,
}

fn measure(name: &str, iterations: u32, only: &Option<String>, mut action: impl FnMut()) -> Option<Value> {
    if only.as_ref().is_some_and(|requested| requested != name) { return None; }
    let multiplier: u32 = std::env::var("STATECHRONICLE_BENCH_MULTIPLIER").unwrap().parse().unwrap();
    assert!((1..=20).contains(&multiplier));
    let iterations = iterations.checked_mul(multiplier).unwrap();
    if std::env::var("STATECHRONICLE_ALLOCATION_PROBE").ok().as_deref()==Some("1") {
        for _ in 0..1000 { action(); }
        std::process::exit(0);
    }
    for _ in 0..10 { action(); }
    let mut samples = Vec::with_capacity(31 * iterations as usize);
    for _ in 0..31 {
        for _ in 0..iterations {
            let start = Instant::now();
            action();
            samples.push(start.elapsed().as_nanos() as f64);
        }
    }
    samples.sort_by(f64::total_cmp);
    let n = samples.len();
    Some(json!({"workload": name, "timing_kind": "individual action", "sample_count": n,
        "iterations_per_sample": 1, "median_ns": samples[n/2],
        "p95_ns": samples[(n * 95).div_ceil(100)-1], "p99_ns": samples[(n * 99).div_ceil(100)-1],
        "samples_ns_per_operation": samples}))
}

fn payload(id: &str, resource: &str) -> Value {
    json!({"schema":"statechronicle.intent.v0", "tenant_id":"acme.game.alpha",
        "intent_id":id, "operation":"configuration.create", "actor":"operator:alice",
        "resource_id":resource, "state_type":"custom", "expected_version":0,
        "created_at":"2026-07-14T00:00:00Z", "nonce":"b64u:AA",
        "inputs":{"profile": Configuration::DOMAIN_ID, "schema_version":1,
                  "command":canonicalize(&ConfigurationCommand::Create{enabled:true}).unwrap()}})
}

fn main() {
    let only = std::env::args().nth(1).map(|mut s| { let n = s.trim_end_matches('_').len(); s.truncate(n); s });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let corpus: Corpus = serde_json::from_str(include_str!("../../../wire/history.json")).unwrap();
    let trusted = fixed_key().verifying_key();
    let key = StateKey::for_resource("acme.game.alpha", "asset:wire");
    let absent = StateKey::for_resource("acme.game.alpha", "asset:absent");
    let current_proofs = proof_bench::service(
        corpus.state_proof.tenant_id.clone(),
        &corpus.signed_commit,
        false,
    );
    let stale_proofs = proof_bench::service(
        corpus.state_proof.tenant_id.clone(),
        &corpus.signed_commit,
        true,
    );
    let native = ConfigurationState {
        revision: 1,
        enabled: false,
    };
    let command = ConfigurationCommand::SetEnabled { enabled: true };
    let command_bytes = canonicalize(&command).unwrap();
    let native_bytes = canonicalize(&native).unwrap();
    let portable = TransitionRequest {
        domain: Configuration::DOMAIN_ID,
        schema: 1,
        expected_version: 1,
        operation: "configuration.set_enabled",
        command: &command_bytes,
        current: Some(EncodedState {
            domain: Configuration::DOMAIN_ID,
            schema: 1,
            version: 1,
            payload: &native_bytes,
        }),
    };
    let producer = Harness::with_registry(registry().unwrap());
    let accepted = producer.accept(
        &payload("int_bench", "config:bench"),
        Some(producer.authority()),
    );
    let event = runtime.block_on(producer.run(&accepted, StateType::Custom));
    let (signed, _) = producer.commit_events(std::slice::from_ref(&event));
    let entries = [CommittedEvent {
        event: &event,
        state_type: StateType::Custom,
    }];
    let projections = projections_for(&signed.body.commit_id, &entries);
    let principal = AuthenticatedPrincipal {
        tenant: accepted.intent.tenant_id.clone(),
        subject: accepted.intent.actor.clone(),
        credential_id: String::from("benchmark"),
    };
    let digest = canonicalize_and_digest(&accepted.intent).unwrap();
    let request = || DurableCommitRequest {
        principal: &principal,
        intent: &accepted.intent,
        payload_digest: &digest,
        entries: &entries,
        commit: &signed,
        projections: &projections,
        outbox: &[],
    };
    let current: StateProjection = projections[0].clone();
    let inputs = BTreeMap::from([
        (String::from("profile"), json!(Configuration::DOMAIN_ID)),
        (String::from("schema_version"), json!(1)),
        (String::from("command"), json!(command_bytes)),
    ]);
    let operation = Operation::from_static("configuration.set_enabled");
    // Fixed-width command arguments and identical setup/output make control
    // subtraction useful under an external allocation profiler, without unsafe
    // allocator instrumentation or a new runtime dependency.
    if let Some(label) = only.as_deref().filter(|label| label.starts_with("alloc-")) {
        let label = label.trim_end_matches('_');
        for _ in 0..1000 {
            match label {
                "alloc-control" => {
                    black_box(());
                }
                "alloc-native" => {
                    Configuration.validate_state(black_box(&native)).unwrap();
                    Configuration
                        .validate(Some(&native), black_box(&command))
                        .unwrap();
                    let next = Configuration.apply(Some(&native), &command).unwrap();
                    Configuration.validate_state(black_box(&next)).unwrap();
                }
                "alloc-portable" => {
                    black_box(
                        execute(
                            &Configuration,
                            black_box(&portable),
                            &mut [0; 64],
                            &mut [0; 64],
                        )
                        .unwrap(),
                    );
                }
                "alloc-hosted" => {
                    CONFIGURATION
                        .check(&operation, Some(black_box(&current)), &inputs)
                        .unwrap();
                    black_box(
                        CONFIGURATION
                            .apply(Some(&current), &operation, &inputs)
                            .unwrap(),
                    );
                }
                "alloc-proof" => {
                    verify_bundle(
                        black_box(&corpus.state_proof),
                        &corpus.signed_commit,
                        &trusted,
                        &key,
                    )
                    .unwrap();
                }
                _ => panic!("unknown allocation workload"),
            }
        }
        println!("allocation probe complete");
        return;
    }
    let mut rows = Vec::new();
    macro_rules! bench {
        ($name:literal, $count:expr, $body:expr) => {
            if let Some(row) = measure($name, $count, &only, $body) {
                rows.push(row);
            }
        };
    }
    let (trade_proof, trade_commits) = runtime.block_on(trade_bench::proof_fixture());
    bench!("cross_tenant_trade_plan_including_seed_setup", 2, || {
        runtime.block_on(trade_bench::plan_including_setup());
    });
    bench!("trade_state_proof_verification", 10, || {
        statechronicle::proof::trade::verify_trade_proof(black_box(&trade_proof), &trade_commits)
            .unwrap();
    });
    let history_harness = Harness::with_registry(registry().unwrap());
    let mut history = Vec::new();
    for index in 0..256 {
        let intent = history_harness.accept(
            &payload(
                &format!("int_rebuild_{index}"),
                &format!("config:rebuild:{index}"),
            ),
            Some(history_harness.authority()),
        );
        history.push(runtime.block_on(history_harness.run(&intent, StateType::Custom)));
    }
    let (history_commit, _) = history_harness.commit_events(&history);
    statechronicle::commit::sign::verify_commit(&history_commit, &trusted).unwrap();
    let history: Vec<_> = history
        .into_iter()
        .map(|event| (event, history_commit.body.commit_id.clone()))
        .collect();
    bench!("checkpoint_resume_256_events_chunk_32_and_root", 2, || {
        runtime.block_on(rebuild_bench::resume_and_check(
            black_box(&history),
            history_commit.body.next_state_root.as_bytes(),
        ));
    });
    bench!("control", 100, || {
        black_box(());
    });
    bench!("native_validate_reduce", 1000, || {
        let state = black_box(&native);
        let cmd = black_box(&command);
        Configuration.validate_state(state).unwrap();
        Configuration.validate(Some(state), cmd).unwrap();
        let next = Configuration.apply(Some(state), cmd).unwrap();
        Configuration.validate_state(black_box(&next)).unwrap();
    });
    bench!("hosted_typed_check_apply", 100, || {
        CONFIGURATION
            .check(
                black_box(&operation),
                Some(black_box(&current)),
                black_box(&inputs),
            )
            .unwrap();
        black_box(
            CONFIGURATION
                .apply(Some(&current), &operation, &inputs)
                .unwrap(),
        );
    });
    bench!("portable_validate_reduce_hash", 100, || {
        black_box(
            execute(
                &Configuration,
                black_box(&portable),
                &mut [0; 64],
                &mut [0; 64],
            )
            .unwrap(),
        );
    });
    bench!("canonical_commit_bytes", 100, || {
        black_box(canonicalize(black_box(&corpus.signed_commit.body)).unwrap());
    });
    bench!("sign_commit", 10, || {
        black_box(
            sign_commit(
                black_box(&corpus.signed_commit.body),
                &fixed_key(),
                common::key_id(),
            )
            .unwrap(),
        );
    });
    bench!("historical_state_proof", 10, || {
        verify_bundle(
            black_box(&corpus.state_proof),
            &corpus.signed_commit,
            &trusted,
            &key,
        )
        .unwrap();
    });
    bench!("historical_absence_proof", 10, || {
        verify_non_membership_bundle(
            black_box(&corpus.absence_proof),
            &corpus.signed_commit,
            &trusted,
            &absent,
        )
        .unwrap();
    });
    bench!("current_state_proof_reference_read_adapter", 10, || {
        runtime
            .block_on(current_proofs.verify_with_key_canonical(
                black_box(&corpus.state_proof),
                &trusted,
                &key,
            ))
            .unwrap();
    });
    bench!("historical_state_proof_reference_read_adapter", 10, || {
        runtime
            .block_on(current_proofs.verify_with_key(
                black_box(&corpus.state_proof),
                &trusted,
                &key,
            ))
            .unwrap();
    });
    bench!("stale_head_refusal_reference_read_adapter", 10, || {
        assert!(matches!(
            runtime.block_on(stale_proofs.verify_with_key_canonical(
                black_box(&corpus.state_proof),
                &trusted,
                &key,
            )),
            Err(statechronicle::proof::error::ProofError::NonCanonicalCommit { .. })
        ));
    });
    bench!("wrong_key_refusal", 10, || {
        assert!(
            verify_non_membership_bundle(
                black_box(&corpus.absence_proof),
                &corpus.signed_commit,
                &trusted,
                &key
            )
            .is_err()
        );
    });
    bench!("replay_two_event_state_root", 10, || {
        let mut tree = StateAccumulator::empty();
        for event in black_box(&corpus.events) {
            tree.insert_batch(&state_root_updates(std::slice::from_ref(event)).unwrap())
                .unwrap();
        }
        assert_eq!(
            tree.root().as_bytes(),
            corpus.signed_commit.body.next_state_root.as_bytes()
        );
        black_box(tree);
    });
    bench!("authenticated_prepare_including_adapter_setup", 5, || {
        let harness = Harness::with_registry(registry().unwrap());
        let events = runtime
            .block_on(
                harness
                    .executor
                    .execute_player(&accepted, &principal, &Allow),
            )
            .unwrap();
        assert_eq!(events.len(), 1);
        black_box(events);
    });
    let second = producer.accept(
        &payload("int_bench_second", "config:second"),
        Some(producer.authority()),
    );
    let keys = statechronicle::ports::key_registry::MemoryKeyRegistry::new();
    keys.register(statechronicle::ports::key_registry::KeyRecord {
        key_id: common::key_id(),
        tenant: principal.tenant.clone(),
        subject: Some(principal.subject.clone()),
        operations: vec![accepted.intent.operation.clone()],
        valid_from: accepted.intent.created_at,
        valid_until: None,
        status: statechronicle::ports::key_registry::KeyStatus::Active,
    })
    .unwrap();
    bench!(
        "authenticated_two_intent_batch_to_refusing_sink_including_setup",
        5,
        || {
            use statechronicle::executor::pipeline::PlayerBatchItem;
            let harness = Harness::with_registry(registry().unwrap());
            let sink = batch_bench::RefuseSink::default();
            let items = [
                PlayerBatchItem {
                    validated: &accepted,
                    principal: &principal,
                },
                PlayerBatchItem {
                    validated: &second,
                    principal: &principal,
                },
            ];
            let result = runtime.block_on(
                harness
                    .executor
                    .execute_player_batch_durable_with_key_registry(&items, &Allow, &keys, &sink),
            );
            assert!(
                matches!(result, Err(statechronicle::executor::error::ExecutorError::Store(message))
            if message == "benchmark refuses persistence")
            );
            assert_eq!(sink.0.load(std::sync::atomic::Ordering::Relaxed), 1);
        }
    );
    bench!("two_intent_planning_including_adapter_setup", 5, || {
        let harness = Harness::with_registry(registry().unwrap());
        let events = runtime
            .block_on(
                harness
                    .executor
                    .execute_batch(&[accepted.clone(), second.clone()]),
            )
            .unwrap();
        assert_eq!(events.len(), 2);
        black_box(events);
    });
    bench!("verified_durable_write_reference_memory_adapter", 5, || {
        let ledger = MemoryLedger::default();
        let receipt = runtime
            .block_on(persist_durable_verified(
                &ledger,
                &Allow,
                &Verify,
                request(),
            ))
            .unwrap();
        assert!(matches!(receipt, DurablePersistResult::Committed { .. }));
        black_box(ledger);
    });
    let replay_ledger = MemoryLedger::default();
    runtime
        .block_on(persist_durable_verified(
            &replay_ledger,
            &Allow,
            &Verify,
            request(),
        ))
        .unwrap();
    bench!("verified_exact_retry_reference_memory_adapter", 5, || {
        let receipt = runtime
            .block_on(persist_durable_verified(
                &replay_ledger,
                &Allow,
                &Verify,
                request(),
            ))
            .unwrap();
        assert!(matches!(receipt, DurablePersistResult::Replay { .. }));
    });
    bench!("ambiguous_ack_reference_memory_adapter", 5, || {
        let ledger = MemoryLedger {
            lose_commit_response: true,
            ..MemoryLedger::default()
        };
        assert!(
            runtime
                .block_on(persist_durable_verified(
                    &ledger,
                    &Allow,
                    &Verify,
                    request()
                ))
                .is_err()
        );
        assert_eq!(ledger.data.lock().unwrap().commits.len(), 1);
    });
    assert!(!rows.is_empty(), "unknown workload");
    println!(
        "{}",
        serde_json::to_string(&json!({"workloads":rows,
        "limitations":["In-memory adapters do not measure disk durability or power-loss safety.",
          "Setup is included where labeled; no cross-workload equivalence is asserted.",
          "p99 is the largest of 31 sample averages, not per-request tail latency.",
          "Current-head reads use a fixed in-memory adapter, without concurrent head movement.",
          "Cross-tenant trade planning uses symbolic transactions, not atomic durable writes.",
          "Checkpoint resume uses in-memory handles and 256 independent creates, not process crashes."]}))
        .unwrap()
    );
}
