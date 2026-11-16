#![allow(clippy::unwrap_used, clippy::panic, dead_code)]
#[path = "../../../ci/compat-0.2/crates/consumer/examples/common/domains.rs"]
mod original;

#[path = "../examples/common/domains.rs"]
mod registered;
use registered::{CONFIGURATION, Configuration, ConfigurationCommand, EQUIPMENT, EquipmentCommand};
use serde::{Serialize, de::DeserializeOwned};
use statechronicle::profiles::registry::ProfileRules;
use statechronicle::{
    Operation, ProfileRegistry, StateProfile, StateType, TypedProfile, command_operations,
};
use std::{fmt::Display, sync::LazyLock};

fn compare<P, Q>(new: &TypedProfile<P>, old: &TypedProfile<Q>, command: &P::Command)
where
    P: StateProfile,
    Q: StateProfile,
    Q::State: Serialize + DeserializeOwned,
    Q::Command: Serialize + DeserializeOwned,
    P::State: Serialize + DeserializeOwned,
    P::Command: Serialize + DeserializeOwned,
    P::Error: Display,
    Q::Error: Display,
{
    assert_eq!(new.allowed_operations(), old.allowed_operations());
    let (operation, inputs) = new.encode_command(command).unwrap();
    let old_command: Q::Command =
        serde_json::from_value(serde_json::to_value(command).unwrap()).unwrap();
    assert_eq!(
        (operation.clone(), inputs.clone()),
        old.encode_command(&old_command).unwrap()
    );
    assert_eq!(
        new.evaluate(None, &operation, &inputs),
        old.evaluate(None, &operation, &inputs)
    );
    let registry = registered::registry().unwrap();
    let rules = registry.resolve(StateType::Custom, &inputs).unwrap();
    assert_eq!(rules.profile_id(), P::DOMAIN_ID);
    assert_eq!(rules.allowed_operations(), new.allowed_operations());
    let mut wrong_schema = inputs.clone();
    wrong_schema.insert("schema_version".into(), serde_json::json!(2));
    assert!(registry.resolve(StateType::Custom, &wrong_schema).is_err());
    let mut wrong_domain = inputs.clone();
    wrong_domain.insert("profile".into(), serde_json::json!("other.domain"));
    assert!(registry.resolve(StateType::Custom, &wrong_domain).is_err());
    for other in new
        .allowed_operations()
        .iter()
        .filter(|other| **other != operation)
    {
        assert!(new.evaluate(None, other, &inputs).is_err());
    }
}
#[test]
fn explicit_variant_mappings_preserve_both_original_domains() {
    for enabled in [true, false] {
        compare(
            &CONFIGURATION,
            &original::CONFIGURATION,
            &ConfigurationCommand::Create { enabled },
        );
        compare(
            &CONFIGURATION,
            &original::CONFIGURATION,
            &ConfigurationCommand::SetEnabled { enabled },
        );
    }
    for command in [
        EquipmentCommand::Install,
        EquipmentCommand::Start,
        EquipmentCommand::Stop,
        EquipmentCommand::Fail,
        EquipmentCommand::Recover,
    ] {
        compare(&EQUIPMENT, &original::EQUIPMENT, &command);
    }
}

command_operations! {
    fn malformed_operation(command: &ConfigurationCommand);
    static MALFORMED_OPERATIONS: Operation;
    ConfigurationCommand::Create { .. } => "",
    ConfigurationCommand::SetEnabled { .. } => "configuration.set_enabled",
}
command_operations! {
    fn reserved_operation(command: &ConfigurationCommand);
    static RESERVED_OPERATIONS: Operation;
    ConfigurationCommand::Create { .. } => "asset.transfer",
    ConfigurationCommand::SetEnabled { .. } => "configuration.set_enabled",
}
static MALFORMED: LazyLock<TypedProfile<Configuration>> =
    LazyLock::new(|| TypedProfile::new(Configuration, &*MALFORMED_OPERATIONS));
static RESERVED: LazyLock<TypedProfile<Configuration>> =
    LazyLock::new(|| TypedProfile::new(Configuration, &*RESERVED_OPERATIONS));
static MALFORMED_CUSTOM: LazyLock<[&'static dyn ProfileRules; 1]> = LazyLock::new(|| [&*MALFORMED]);
static RESERVED_CUSTOM: LazyLock<[&'static dyn ProfileRules; 1]> = LazyLock::new(|| [&*RESERVED]);
#[test]
fn generated_lists_still_require_runtime_registration_checks() {
    assert!(ProfileRegistry::with_custom(&*MALFORMED_CUSTOM).is_err());
    assert!(ProfileRegistry::with_custom(&*RESERVED_CUSTOM).is_err());
}
