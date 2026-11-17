//! Pure planning with a native command; no authentication or durable commit.
#[path = "common/domains.rs"]
mod domains;
use domains::{CONFIGURATION, ConfigurationCommand, registry};
use statechronicle::StateType;
use std::error::Error;
fn main() -> Result<(), Box<dyn Error>> {
    let registry = registry()?;
    let command = ConfigurationCommand::Create { enabled: true };
    let (operation, inputs) = CONFIGURATION.encode_command(&command)?;
    let rules = registry.resolve(StateType::Custom, &inputs)?;
    let state = rules.evaluate(None, &operation, &inputs)?;
    println!("{:?}", CONFIGURATION.decode_state(&state)?);
    Ok(())
}
