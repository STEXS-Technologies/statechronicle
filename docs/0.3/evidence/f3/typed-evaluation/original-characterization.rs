#![allow(clippy::unwrap_used, clippy::panic)]

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_domain::{
    commit::ProfileId,
    custom::CustomState,
    ids::{CommitId, EventId},
    intent::Operation,
    resource::ResourceId,
    resource_state::ResourceState,
    state::StateProjection,
    state_type::StateType,
    tenant::TenantId,
};
use statechronicle_profiles::{StateProfile, custom::TypedProfile, registry::ProfileRules};
use std::{cell::RefCell, collections::BTreeMap, sync::LazyLock};

thread_local! { static TRACE: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) }; }
fn record(step: &'static str) {
    TRACE.with(|trace| trace.borrow_mut().push(step));
}
fn trace() -> Vec<&'static str> {
    TRACE.with(|trace| std::mem::take(&mut *trace.borrow_mut()))
}

#[derive(Serialize)]
struct State(u64);
impl<'de> Deserialize<'de> for State {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        record("decode state");
        u64::deserialize(decoder).map(Self)
    }
}
#[derive(Serialize)]
struct Command(u64);
impl<'de> Deserialize<'de> for Command {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        record("decode command");
        u64::deserialize(decoder).map(Self)
    }
}
struct Model;
impl StateProfile for Model {
    type State = State;
    type Command = Command;
    type Error = &'static str;
    const DOMAIN_ID: &'static str = "test.evaluation";
    const SCHEMA_VERSION: u32 = 1;
    const MAX_PAYLOAD_BYTES: usize = 8;
    fn operation(&self, _: &Command) -> &'static str {
        "evaluation.set"
    }
    fn validate(&self, _: Option<&State>, command: &Command) -> Result<(), Self::Error> {
        record("validate command");
        if command.0 == 99 {
            Err("command refused")
        } else {
            Ok(())
        }
    }
    fn apply(&self, _: Option<&State>, command: &Command) -> Result<State, Self::Error> {
        record("reduce");
        if command.0 == 98 {
            Err("reduction refused")
        } else {
            Ok(State(command.0))
        }
    }
    fn validate_state(&self, state: &State) -> Result<(), Self::Error> {
        record("validate state");
        if state.0 > 100 {
            Err("state refused")
        } else {
            Ok(())
        }
    }
}
static OPERATIONS: LazyLock<Vec<Operation>> =
    LazyLock::new(|| vec![Operation::new("evaluation.set".into()).unwrap()]);
fn profile() -> TypedProfile<Model> {
    TypedProfile::new(Model, &OPERATIONS)
}
fn inputs(command: u64) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("profile".into(), json!(Model::DOMAIN_ID)),
        ("schema_version".into(), json!(1)),
        ("command".into(), json!(command.to_le_bytes())),
    ])
}
fn state(value: u64) -> ResourceState {
    ResourceState::Custom(CustomState {
        domain: ProfileId::new(Model::DOMAIN_ID.into()).unwrap(),
        schema_version: 1,
        payload: value.to_le_bytes().to_vec(),
    })
}
fn current() -> StateProjection {
    let state = state(17);
    StateProjection {
        tenant_id: TenantId("acme.game.alpha".into()),
        resource_id: ResourceId("config".into()),
        state_type: StateType::Custom,
        version: 1,
        last_event_id: EventId::new("evt_original".into()).unwrap(),
        last_commit_id: CommitId::new("cmt_original".into()).unwrap(),
        state_hash: canonicalize_and_digest(&state).unwrap(),
        state,
    }
}

#[test]
fn original_hosted_check_apply_decodes_and_validates_twice() {
    let adapter = profile();
    let projection = current();
    trace();
    adapter
        .check(&OPERATIONS[0], Some(&projection), &inputs(23))
        .unwrap();
    let after = adapter
        .apply(Some(&projection), &OPERATIONS[0], &inputs(23))
        .unwrap();
    assert_eq!(after, state(23));
    assert_eq!(
        trace(),
        vec![
            "decode command",
            "decode state",
            "validate state",
            "validate command",
            "decode command",
            "decode state",
            "validate state",
            "validate command",
            "reduce",
            "validate state",
        ]
    );
}

#[test]
fn original_check_and_reduction_failures_have_distinct_stages() {
    let adapter = profile();
    let projection = current();
    assert_eq!(
        adapter
            .check(&OPERATIONS[0], Some(&projection), &inputs(99))
            .unwrap_err()
            .to_string(),
        "invalid input: command refused"
    );
    adapter
        .check(&OPERATIONS[0], Some(&projection), &inputs(98))
        .unwrap();
    assert_eq!(
        adapter
            .apply(Some(&projection), &OPERATIONS[0], &inputs(98))
            .unwrap_err()
            .to_string(),
        "invalid input: reduction refused"
    );
    adapter
        .check(&OPERATIONS[0], Some(&projection), &inputs(101))
        .unwrap();
    assert_eq!(
        adapter
            .apply(Some(&projection), &OPERATIONS[0], &inputs(101))
            .unwrap_err()
            .to_string(),
        "invalid input: state refused"
    );
}
