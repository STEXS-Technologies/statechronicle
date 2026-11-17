//! Combined evaluation retains the executor's legacy error categories.
#![allow(clippy::unwrap_used, clippy::panic)]
mod common;
use common::{Harness, mint};
use serde_json::Value;
use statechronicle_domain::{
    intent::Operation, resource_state::ResourceState, state::StateProjection, state_type::StateType,
};
use statechronicle_executor::error::ExecutorError;
use statechronicle_profiles::{
    error::ProfileError,
    registry::{ProfileRegistry, ProfileRules},
};
use std::collections::BTreeMap;
struct Refusal(bool);
impl ProfileRules for Refusal {
    fn state_type(&self) -> StateType {
        StateType::UniqueAsset
    }
    fn profile_id(&self) -> &'static str {
        "test.refusal"
    }
    fn allowed_operations(&self) -> &'static [Operation] {
        &[]
    }
    fn check(
        &self,
        _: &Operation,
        _: Option<&StateProjection>,
        _: &BTreeMap<String, Value>,
    ) -> Result<(), ProfileError> {
        if self.0 {
            Err(ProfileError::ActorMismatch {
                expected: "expected".into(),
                actual: "actual".into(),
            })
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
        Err(ProfileError::HardDeleteForbidden)
    }
}
static CHECK: Refusal = Refusal(true);
static APPLY: Refusal = Refusal(false);
#[tokio::test]
async fn check_uses_structured_mapping_and_apply_uses_transition_mapping() {
    let check =
        Harness::with_profiles_and_ports(ProfileRegistry::with_unique_asset(&CHECK), vec![]);
    assert!(
        matches!(check.executor.execute(&mint("check", "asset", "alice")).await,
        Err(ExecutorError::ActorMismatch { expected, actual }) if expected == "expected" && actual == "actual")
    );
    let apply =
        Harness::with_profiles_and_ports(ProfileRegistry::with_unique_asset(&APPLY), vec![]);
    assert!(
        matches!(apply.executor.execute(&mint("apply", "asset", "alice")).await,
        Err(ExecutorError::TransitionInvalid(message)) if message == "hard delete is forbidden")
    );
}
