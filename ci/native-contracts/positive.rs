#![allow(dead_code, unused_imports)]
mod domains;
use domains::{
    CONFIGURATION, Configuration, ConfigurationCommand, ConfigurationState, EQUIPMENT,
    EquipmentCommand, EquipmentState,
};
use statechronicle::commit::persist::SignedCommitVerifier;
use statechronicle::index::rebuild::verified::{HistoryPin, HistoryWitness, verify_history};
use statechronicle::mutation::{PreparedBatch, PreparedMutation};
use statechronicle::profiles::registry::ProfileRules;
use statechronicle::proof::claim::{ClaimProof, ClaimVerifier, ExpectedClaim};
use statechronicle::{Operation, StateProfile, TypedProfile, command_operations};

// Each negative probe replaces exactly one expression in this otherwise
// compiling application. Dead functions are type-checked too.
fn command_type() {
    let _ = CONFIGURATION.encode_command(&ConfigurationCommand::Create { enabled: true });
}
fn state_type() -> ConfigurationState {
    Configuration
        .apply(None, &ConfigurationCommand::Create { enabled: true })
        .unwrap()
}
fn batch_leg(batch: &PreparedBatch) -> &PreparedMutation {
    &batch.mutations()[0]
}
async fn historical_claim(
    verifier: &ClaimVerifier,
    proof: ClaimProof<'_>,
    expected: &ExpectedClaim,
    pin: &statechronicle::CommitId,
    key: &ed25519_dalek::VerifyingKey,
) {
    let _ = verifier.verify_historical(proof, expected, pin, key).await;
}
fn historical_source(
    witnesses: &[HistoryWitness<'_>],
    pin: &HistoryPin,
    verifier: &dyn SignedCommitVerifier,
) {
    let _ = verify_history(witnesses, pin, verifier);
}
enum Command {
    Create,
    Disable,
}
command_operations! {
    fn operation(command: &Command);
    static OPERATIONS: Operation;
    Command::Create => "configuration.create",
    Command::Disable => "configuration.disable",
}

// A normal native implementation must return its declared State, not an Event
// or an unrelated domain's state. This also exercises the portable trait's
// contract independently of the hosted serialization adapter.
struct Model;
impl StateProfile for Model {
    type State = ConfigurationState;
    type Command = ConfigurationCommand;
    type Error = &'static str;
    const DOMAIN_ID: &'static str = "example.configuration";
    const SCHEMA_VERSION: u32 = 1;
    const MAX_PAYLOAD_BYTES: usize = 64;
    fn operation(&self, command: &Self::Command) -> &'static str {
        Configuration.operation(command)
    }
    fn validate(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<(), Self::Error> {
        Configuration.validate(current, command)
    }
    fn apply(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<ConfigurationState, Self::Error> {
        Configuration.apply(current, command)
    }
    fn validate_state(&self, state: &Self::State) -> Result<(), Self::Error> {
        Configuration.validate_state(state)
    }
}

struct Schema<const VERSION: u32>;
impl<const VERSION: u32> StateProfile for Schema<VERSION> {
    type State = ConfigurationState;
    type Command = ConfigurationCommand;
    type Error = &'static str;
    const DOMAIN_ID: &'static str = "example.configuration";
    const SCHEMA_VERSION: u32 = VERSION;
    const MAX_PAYLOAD_BYTES: usize = 64;
    fn operation(&self, command: &Self::Command) -> &'static str {
        Configuration.operation(command)
    }
    fn validate(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<(), Self::Error> {
        Configuration.validate(current, command)
    }
    fn apply(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<Self::State, Self::Error> {
        Configuration.apply(current, command)
    }
    fn validate_state(&self, state: &Self::State) -> Result<(), Self::Error> {
        Configuration.validate_state(state)
    }
}

fn main() {
    let command = ConfigurationCommand::Create { enabled: true };
    let (operation, inputs) = CONFIGURATION.encode_command(&command).unwrap();
    let configuration = CONFIGURATION.evaluate(None, &operation, &inputs).unwrap();
    let (equipment_operation, equipment_inputs) = EQUIPMENT
        .encode_command(&EquipmentCommand::Install)
        .unwrap();
    let equipment = EQUIPMENT
        .evaluate(None, &equipment_operation, &equipment_inputs)
        .unwrap();
    assert_eq!(
        CONFIGURATION.decode_state(&configuration).unwrap(),
        ConfigurationState {
            revision: 1,
            enabled: true
        }
    );
    assert!(EQUIPMENT.decode_state(&equipment).is_ok());
    assert!(CONFIGURATION.decode_state(&equipment).is_err());
    assert!(EQUIPMENT.decode_state(&configuration).is_err());
    assert!(
        CONFIGURATION
            .evaluate(
                None,
                &Operation::from_static("configuration.set_enabled"),
                &inputs
            )
            .is_err()
    );
    let zero = TypedProfile::new(Schema::<0>, CONFIGURATION.allowed_operations());
    assert!(zero.validate_registration().is_err());
    assert!(zero.encode_command(&command).is_err());
    let v2 = TypedProfile::new(Schema::<2>, CONFIGURATION.allowed_operations());
    assert!(v2.validate_registration().is_ok());
    let (operation2, inputs2) = v2.encode_command(&command).unwrap();
    assert!(CONFIGURATION.evaluate(None, &operation2, &inputs2).is_err());
    assert!(v2.evaluate(None, &operation, &inputs).is_err());
    let state2 = v2.evaluate(None, &operation2, &inputs2).unwrap();
    assert!(v2.decode_state(&state2).is_ok());
    assert!(CONFIGURATION.decode_state(&state2).is_err());
    assert!(v2.decode_state(&configuration).is_err());
    println!("native contracts: typed domains, operation refusal and schema isolation verified");
}
