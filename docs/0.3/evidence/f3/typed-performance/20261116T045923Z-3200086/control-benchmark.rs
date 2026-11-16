//! Same inputs and assertions for legacy check/apply and combined evaluation.
#![deny(unsafe_code)]
#[path = "common/domains.rs"]
mod domains;
use domains::{CONFIGURATION, Configuration, ConfigurationCommand, ConfigurationState};
use serde_json::{Value, json};
use statechronicle::{CommitId, EventId, Operation, ResourceId, StateProfile, StateProjection, StateType, TenantId};
use statechronicle::core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle::domain::{custom::CustomState, resource_state::ResourceState};
use statechronicle::profiles::registry::ProfileRules;
use std::{collections::BTreeMap, hint::black_box, time::Instant};

fn legacy(rules: &dyn ProfileRules, current: Option<&StateProjection>, operation: &Operation, inputs: &BTreeMap<String, Value>) -> Result<ResourceState, String> {
    rules.check(operation, current, inputs).map_err(|error| error.to_string())?;
    rules.apply(current, operation, inputs).map_err(|error| error.to_string())
}
fn combined(rules: &dyn ProfileRules, current: Option<&StateProjection>, operation: &Operation, inputs: &BTreeMap<String, Value>) -> Result<ResourceState, String> {
    legacy(rules, current, operation, inputs)
}
fn state(revision: u32, enabled: bool) -> ResourceState {
    ResourceState::Custom(CustomState { domain: statechronicle::ProfileId::new(Configuration::DOMAIN_ID.into()).unwrap(), schema_version: 1,
        payload: canonicalize(&ConfigurationState { revision, enabled }).unwrap() })
}
fn inputs(command: &ConfigurationCommand) -> BTreeMap<String, Value> {
    BTreeMap::from([("profile".into(), json!(Configuration::DOMAIN_ID)), ("schema_version".into(), json!(1)),
        ("command".into(), json!(canonicalize(command).unwrap()))])
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let mode = args.get(1).unwrap().trim_end_matches('_');
    let kind = args.get(2).map(|value| value.trim_end_matches('_')).unwrap_or("latency");
    assert!(["legacy", "combined", "control"].contains(&mode));
    let prior_state = state(1, false);
    let prior = StateProjection { tenant_id: TenantId("acme.game.alpha".into()), resource_id: ResourceId("configuration".into()),
        state_type: StateType::Custom, version: 1, last_event_id: EventId::new("evt_bench".into()).unwrap(),
        last_commit_id: CommitId::new("cmt_bench".into()).unwrap(), state_hash: canonicalize_and_digest(&prior_state).unwrap(), state: prior_state };
    let create = inputs(&ConfigurationCommand::Create { enabled: true });
    let update = inputs(&ConfigurationCommand::SetEnabled { enabled: true });
    let mut malformed = update.clone();
    malformed.insert("command".into(), json!([]));
    let cases = [
        ("hosted_create", None, Operation::from_static("configuration.create"), create, Ok(state(1, true))),
        ("hosted_update", Some(&prior), Operation::from_static("configuration.set_enabled"), update, Ok(state(2, true))),
        ("hosted_malformed", Some(&prior), Operation::from_static("configuration.set_enabled"), malformed, Err(())),
    ];
    let mut workloads = Vec::new();
    for (name, current, operation, input, expected) in &cases {
        if kind != "latency" && kind != *name { continue; }
        let mut action = || {
            if mode == "control" { black_box(()); return; }
            let rules: &dyn ProfileRules = black_box(&*CONFIGURATION);
            let result = if mode == "legacy" { legacy(rules, black_box(*current), black_box(operation), black_box(input)) }
                else { combined(rules, black_box(*current), black_box(operation), black_box(input)) };
            match expected {
                Ok(state) => assert_eq!(result.unwrap(), *state),
                Err(()) => assert!(result.is_err()),
            }
        };
        if kind != "latency" { for _ in 0..1000 { action(); } continue; }
        for _ in 0..100 { action(); }
        let mut samples = Vec::with_capacity(31);
        for _ in 0..31 {
            let start = Instant::now();
            for _ in 0..10000 { action(); }
            samples.push(start.elapsed().as_nanos() as f64 / 10000.0);
        }
        samples.sort_by(f64::total_cmp);
        workloads.push(json!({"workload":name, "iterations_per_sample":10000, "samples_ns_per_operation":samples,
            "median_ns": samples[15], "p95_ns":samples[29], "p99_ns":samples[30]}));
    }
    if kind == "latency" { println!("{}", json!({"workloads": workloads})); } else { println!("ok"); }
}
