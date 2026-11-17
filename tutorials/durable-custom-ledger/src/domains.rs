//! Application-owned native models; no portable codec or JSON schema required.
use serde::{Deserialize, Serialize};
use statechronicle::profiles::{error::ProfileError, registry::ProfileRules};
use statechronicle::{Operation, ProfileRegistry, StateProfile, TypedProfile, command_operations};
use std::sync::LazyLock;

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
        configuration_operation(command)
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
        equipment_operation(command)
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
command_operations! {
    fn configuration_operation(command: &ConfigurationCommand);
    static CONFIG_OPERATIONS: Operation;
    ConfigurationCommand::Create { .. } => "configuration.create",
    ConfigurationCommand::SetEnabled { .. } => "configuration.set_enabled",
}
command_operations! {
    fn equipment_operation(command: &EquipmentCommand);
    static EQUIPMENT_OPERATIONS: Operation;
    EquipmentCommand::Install => "equipment.install",
    EquipmentCommand::Start => "equipment.start",
    EquipmentCommand::Stop => "equipment.stop",
    EquipmentCommand::Fail => "equipment.fail",
    EquipmentCommand::Recover => "equipment.recover",
}
// This application uses host principal/key/operation policy instead of delegated grants.
// Optional delegated authority does not disable protected ingress or signature checks.
pub static CONFIGURATION: LazyLock<TypedProfile<Configuration>> = LazyLock::new(|| {
    TypedProfile::new(Configuration, &*CONFIG_OPERATIONS).with_optional_authority()
});
pub static EQUIPMENT: LazyLock<TypedProfile<Equipment>> = LazyLock::new(|| {
    TypedProfile::new(Equipment, &*EQUIPMENT_OPERATIONS).with_optional_authority()
});
static CUSTOM: LazyLock<[&'static dyn ProfileRules; 2]> =
    LazyLock::new(|| [&*CONFIGURATION, &*EQUIPMENT]);
pub fn registry() -> Result<ProfileRegistry, ProfileError> {
    ProfileRegistry::with_custom(&*CUSTOM)
}
