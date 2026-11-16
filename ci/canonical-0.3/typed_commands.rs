//! New command authoring through only the external public facade.
#[path = "../examples/common/domains.rs"]
mod domains;
use domains::{
    CONFIGURATION, Configuration, ConfigurationCommand, EQUIPMENT, Equipment, EquipmentCommand,
};
use serde::Serialize;
use serde_json::json;
use statechronicle::core::canonicalize::canonicalize;
use statechronicle::{StateProfile, profiles::custom::TypedProfile};
use std::collections::BTreeMap;

fn matches_legacy<P>(adapter: &TypedProfile<P>, profile: &P, command: &P::Command)
where
    P: StateProfile,
    P::State: Serialize + serde::de::DeserializeOwned,
    P::Command: Serialize + serde::de::DeserializeOwned,
    P::Error: std::fmt::Display,
{
    let (operation, inputs) = adapter.encode_command(command).unwrap();
    assert_eq!(operation.as_str(), profile.operation(command));
    assert_eq!(
        inputs,
        BTreeMap::from([
            ("profile".into(), json!(P::DOMAIN_ID)),
            ("schema_version".into(), json!(P::SCHEMA_VERSION)),
            ("command".into(), json!(canonicalize(command).unwrap())),
        ])
    );
}
#[test]
fn independent_native_domains_encode_the_original_command_envelopes() {
    matches_legacy(
        &CONFIGURATION,
        &Configuration,
        &ConfigurationCommand::Create { enabled: true },
    );
    matches_legacy(
        &CONFIGURATION,
        &Configuration,
        &ConfigurationCommand::SetEnabled { enabled: false },
    );
    matches_legacy(&EQUIPMENT, &Equipment, &EquipmentCommand::Start);
    matches_legacy(&EQUIPMENT, &Equipment, &EquipmentCommand::Stop);
}
