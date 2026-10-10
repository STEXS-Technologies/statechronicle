//! Two independent consumer-defined profiles, with no changes to the engine.
use serde::{Deserialize, Serialize};
use statechronicle::portable::codec::{CanonicalCodec, CodecError, Decoder, Encoder};
use statechronicle::profiles::error::ProfileError;
use statechronicle::profiles::registry::ProfileRules;
use statechronicle::{Operation, ProfileRegistry, StateProfile, TypedProfile};
use std::sync::LazyLock;

impl CanonicalCodec for ConfigurationState {
    fn encode(&self, output: &mut Encoder<'_>) -> Result<(), CodecError> {
        output.u32(self.revision)?;
        output.boolean(self.enabled)
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            revision: input.u32()?,
            enabled: input.boolean()?,
        })
    }
}
impl CanonicalCodec for ConfigurationCommand {
    fn encode(&self, output: &mut Encoder<'_>) -> Result<(), CodecError> {
        match self {
            Self::Create { enabled } => {
                output.uleb(0)?;
                output.boolean(*enabled)
            }
            Self::SetEnabled { enabled } => {
                output.uleb(1)?;
                output.boolean(*enabled)
            }
        }
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CodecError> {
        match input.uleb()? {
            0 => Ok(Self::Create {
                enabled: input.boolean()?,
            }),
            1 => Ok(Self::SetEnabled {
                enabled: input.boolean()?,
            }),
            _ => Err(CodecError::NonCanonical),
        }
    }
}
impl CanonicalCodec for EquipmentState {
    fn encode(&self, output: &mut Encoder<'_>) -> Result<(), CodecError> {
        let mode = match self.mode {
            EquipmentMode::Idle => 0,
            EquipmentMode::Running => 1,
            EquipmentMode::Failed => 2,
        };
        output.uleb(mode)?;
        output.u32(self.cycles)
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CodecError> {
        let mode = match input.uleb()? {
            0 => EquipmentMode::Idle,
            1 => EquipmentMode::Running,
            2 => EquipmentMode::Failed,
            _ => return Err(CodecError::NonCanonical),
        };
        Ok(Self {
            mode,
            cycles: input.u32()?,
        })
    }
}
impl CanonicalCodec for EquipmentCommand {
    fn encode(&self, output: &mut Encoder<'_>) -> Result<(), CodecError> {
        output.uleb(match self {
            Self::Install => 0,
            Self::Start => 1,
            Self::Stop => 2,
            Self::Fail => 3,
            Self::Recover => 4,
        })
    }
    fn decode(input: &mut Decoder<'_>) -> Result<Self, CodecError> {
        match input.uleb()? {
            0 => Ok(Self::Install),
            1 => Ok(Self::Start),
            2 => Ok(Self::Stop),
            3 => Ok(Self::Fail),
            4 => Ok(Self::Recover),
            _ => Err(CodecError::NonCanonical),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationState {
    pub revision: u32,
    pub enabled: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfigurationCommand {
    Create { enabled: bool },
    SetEnabled { enabled: bool },
}
pub struct Configuration;
impl StateProfile for Configuration {
    type State = ConfigurationState;
    type Command = ConfigurationCommand;
    type Error = &'static str;
    const DOMAIN_ID: &'static str = "example.configuration";
    const SCHEMA_VERSION: u32 = 1;
    const MAX_PAYLOAD_BYTES: usize = 64;
    fn operation(&self, command: &Self::Command) -> &'static str {
        match command {
            ConfigurationCommand::Create { .. } => "configuration.create",
            ConfigurationCommand::SetEnabled { .. } => "configuration.set_enabled",
        }
    }
    fn validate(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<(), Self::Error> {
        match (current, command) {
            (None, ConfigurationCommand::Create { .. })
            | (Some(_), ConfigurationCommand::SetEnabled { .. }) => Ok(()),
            _ => Err("configuration lifecycle violation"),
        }
    }
    fn apply(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<Self::State, Self::Error> {
        self.validate(current, command)?;
        let revision = current
            .map_or(0, |state| state.revision)
            .checked_add(1)
            .ok_or("revision overflow")?;
        let enabled = match command {
            ConfigurationCommand::Create { enabled }
            | ConfigurationCommand::SetEnabled { enabled } => *enabled,
        };
        Ok(ConfigurationState { revision, enabled })
    }
    fn validate_state(&self, state: &Self::State) -> Result<(), Self::Error> {
        if state.revision == 0 {
            Err("revision must be positive")
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EquipmentMode {
    Idle,
    Running,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipmentState {
    pub mode: EquipmentMode,
    pub cycles: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EquipmentCommand {
    Install,
    Start,
    Stop,
    Fail,
    Recover,
}
pub struct Equipment;
impl StateProfile for Equipment {
    type State = EquipmentState;
    type Command = EquipmentCommand;
    type Error = &'static str;
    const DOMAIN_ID: &'static str = "example.equipment";
    const SCHEMA_VERSION: u32 = 1;
    const MAX_PAYLOAD_BYTES: usize = 64;
    fn operation(&self, command: &Self::Command) -> &'static str {
        match command {
            EquipmentCommand::Install => "equipment.install",
            EquipmentCommand::Start => "equipment.start",
            EquipmentCommand::Stop => "equipment.stop",
            EquipmentCommand::Fail => "equipment.fail",
            EquipmentCommand::Recover => "equipment.recover",
        }
    }
    fn validate(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<(), Self::Error> {
        match (current.map(|state| state.mode), command) {
            (None, EquipmentCommand::Install)
            | (Some(EquipmentMode::Idle), EquipmentCommand::Start)
            | (Some(EquipmentMode::Running), EquipmentCommand::Stop | EquipmentCommand::Fail)
            | (Some(EquipmentMode::Failed), EquipmentCommand::Recover) => Ok(()),
            _ => Err("equipment lifecycle violation"),
        }
    }
    fn apply(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<Self::State, Self::Error> {
        self.validate(current, command)?;
        let cycles = current.map_or(0, |state| state.cycles);
        let (mode, cycles) = match command {
            EquipmentCommand::Install | EquipmentCommand::Stop | EquipmentCommand::Recover => {
                (EquipmentMode::Idle, cycles)
            }
            EquipmentCommand::Start => (
                EquipmentMode::Running,
                cycles.checked_add(1).ok_or("cycle overflow")?,
            ),
            EquipmentCommand::Fail => (EquipmentMode::Failed, cycles),
        };
        Ok(EquipmentState { mode, cycles })
    }
    fn validate_state(&self, state: &Self::State) -> Result<(), Self::Error> {
        if state.mode != EquipmentMode::Idle && state.cycles == 0 {
            Err("equipment has never started")
        } else {
            Ok(())
        }
    }
}
static CONFIG_OPERATIONS: LazyLock<[Operation; 2]> = LazyLock::new(|| {
    [
        Operation::from_static("configuration.create"),
        Operation::from_static("configuration.set_enabled"),
    ]
});
static EQUIPMENT_OPERATIONS: LazyLock<[Operation; 5]> = LazyLock::new(|| {
    [
        Operation::from_static("equipment.install"),
        Operation::from_static("equipment.start"),
        Operation::from_static("equipment.stop"),
        Operation::from_static("equipment.fail"),
        Operation::from_static("equipment.recover"),
    ]
});
pub static CONFIGURATION: LazyLock<TypedProfile<Configuration>> =
    LazyLock::new(|| TypedProfile::new(Configuration, &*CONFIG_OPERATIONS));
pub static EQUIPMENT: LazyLock<TypedProfile<Equipment>> =
    LazyLock::new(|| TypedProfile::new(Equipment, &*EQUIPMENT_OPERATIONS));
static CUSTOM: LazyLock<[&'static dyn ProfileRules; 2]> =
    LazyLock::new(|| [&*CONFIGURATION, &*EQUIPMENT]);
pub fn registry() -> Result<ProfileRegistry, ProfileError> {
    ProfileRegistry::with_custom(&*CUSTOM)
}
