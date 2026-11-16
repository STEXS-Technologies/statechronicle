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
fn operation() -> &'static Operation {
    OPERATIONS.first().unwrap()
}
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
        .check(operation(), Some(&projection), &inputs(23))
        .unwrap();
    let after = adapter
        .apply(Some(&projection), operation(), &inputs(23))
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
            .check(operation(), Some(&projection), &inputs(99))
            .unwrap_err()
            .to_string(),
        "invalid input: command refused"
    );
    adapter
        .check(operation(), Some(&projection), &inputs(98))
        .unwrap();
    assert_eq!(
        adapter
            .apply(Some(&projection), operation(), &inputs(98))
            .unwrap_err()
            .to_string(),
        "invalid input: reduction refused"
    );
    adapter
        .check(operation(), Some(&projection), &inputs(101))
        .unwrap();
    assert_eq!(
        adapter
            .apply(Some(&projection), operation(), &inputs(101))
            .unwrap_err()
            .to_string(),
        "invalid input: state refused"
    );
}

#[test]
fn combined_typed_evaluation_decodes_once_and_keeps_output_validation() {
    let adapter = profile();
    let projection = current();
    trace();
    let after = adapter
        .evaluate(Some(&projection), operation(), &inputs(23))
        .unwrap();
    assert_eq!(after, state(23));
    assert_eq!(
        trace(),
        vec![
            "decode command",
            "decode state",
            "validate state",
            "validate command",
            "reduce",
            "validate state",
        ]
    );
    trace();
    adapter.evaluate(None, operation(), &inputs(23)).unwrap();
    assert_eq!(
        trace(),
        vec![
            "decode command",
            "validate command",
            "reduce",
            "validate state"
        ]
    );
}

#[test]
fn combined_evaluation_matches_legacy_states_and_failure_stages() {
    use statechronicle_profiles::error::ProfileEvaluationError::{Apply, Check};
    let adapter = profile();
    let mut commands = vec![
        inputs(23),
        inputs(98),
        inputs(99),
        inputs(101),
        inputs(u64::MAX),
    ];
    for (key, value) in [
        ("profile", json!("other.domain")),
        ("schema_version", json!(2)),
        ("command", json!([0])),
        ("command", json!([0, 0, 0, 0, 0, 0, 0, 0, 0])),
        ("command", json!([256])),
        ("command", json!("bytes")),
    ] {
        let mut input = inputs(23);
        input.insert(key.into(), value);
        commands.push(input);
    }
    for key in ["profile", "schema_version", "command"] {
        let mut input = inputs(23);
        input.remove(key);
        commands.push(input);
    }
    let mut projections = vec![None, Some(current())];
    let mut bad = current();
    bad.state_type = StateType::UniqueAsset;
    projections.push(Some(bad));
    for (domain, schema, payload) in [
        (Model::DOMAIN_ID, 1, 101u64.to_le_bytes().to_vec()),
        (Model::DOMAIN_ID, 2, 17u64.to_le_bytes().to_vec()),
        ("other.domain", 1, 17u64.to_le_bytes().to_vec()),
        (Model::DOMAIN_ID, 1, vec![0]),
        (Model::DOMAIN_ID, 1, vec![0; 9]),
    ] {
        let mut projection = current();
        projection.state = ResourceState::Custom(CustomState {
            domain: ProfileId::new(domain.into()).unwrap(),
            schema_version: schema,
            payload,
        });
        projections.push(Some(projection));
    }
    let unknown = Operation::new("evaluation.unknown".into()).unwrap();
    for operation in [operation(), &unknown] {
        for projection in &projections {
            for input in &commands {
                let legacy = adapter
                    .check(operation, projection.as_ref(), input)
                    .map_err(Check)
                    .and_then(|()| {
                        adapter
                            .apply(projection.as_ref(), operation, input)
                            .map_err(Apply)
                    });
                assert_eq!(
                    adapter.evaluate(projection.as_ref(), operation, input),
                    legacy,
                    "operation={operation:?}, projection={projection:?}, inputs={input:?}"
                );
            }
        }
    }
}

#[test]
fn default_evaluation_preserves_custom_rule_order_and_refusal() {
    use statechronicle_profiles::error::{ProfileError, ProfileEvaluationError};
    struct Legacy {
        fail_check: bool,
    }
    impl ProfileRules for Legacy {
        fn state_type(&self) -> StateType {
            StateType::Custom
        }
        fn profile_id(&self) -> &'static str {
            Model::DOMAIN_ID
        }
        fn allowed_operations(&self) -> &'static [Operation] {
            &OPERATIONS
        }
        fn check(
            &self,
            _: &Operation,
            _: Option<&StateProjection>,
            _: &BTreeMap<String, Value>,
        ) -> Result<(), ProfileError> {
            record("legacy check");
            if self.fail_check {
                Err(ProfileError::NotTransferable)
            } else {
                Ok(())
            }
        }
        fn apply(
            &self,
            _: Option<&StateProjection>,
            _: &Operation,
            _: &BTreeMap<String, Value>,
        ) -> Result<ResourceState, ProfileError> {
            record("legacy apply");
            Err(ProfileError::HardDeleteForbidden)
        }
    }
    trace();
    assert_eq!(
        Legacy { fail_check: true }.evaluate(None, operation(), &inputs(23)),
        Err(ProfileEvaluationError::Check(ProfileError::NotTransferable))
    );
    assert_eq!(trace(), vec!["legacy check"]);
    assert_eq!(
        Legacy { fail_check: false }.evaluate(None, operation(), &inputs(23)),
        Err(ProfileEvaluationError::Apply(
            ProfileError::HardDeleteForbidden
        ))
    );
    assert_eq!(trace(), vec!["legacy check", "legacy apply"]);
}
