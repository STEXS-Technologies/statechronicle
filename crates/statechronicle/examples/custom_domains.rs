//! Consumer-defined configuration and equipment state using the common engine contracts.
#[path = "common/domains.rs"]
mod domains;
use domains::{CONFIGURATION, Configuration, ConfigurationCommand, registry};
use serde_json::json;
use statechronicle::core::canonicalize::canonicalize;
use statechronicle::{Operation, StateProfile, StateType};
use std::collections::BTreeMap;
use std::error::Error;
fn main() -> Result<(), Box<dyn Error>> {
    let registry = registry()?;
    let command = ConfigurationCommand::Create { enabled: true };
    let inputs = BTreeMap::from([
        (String::from("profile"), json!(Configuration::DOMAIN_ID)),
        (
            String::from("schema_version"),
            json!(Configuration::SCHEMA_VERSION),
        ),
        (String::from("command"), json!(canonicalize(&command)?)),
    ]);
    let operation = Operation::new(String::from(Configuration.operation(&command)))?;
    let rules = registry.resolve(StateType::Custom, &inputs)?;
    let state = rules.apply(None, &operation, &inputs)?;
    println!("{:?}", CONFIGURATION.decode_state(&state)?);
    Ok(())
}
