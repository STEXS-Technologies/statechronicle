//! Compiler control: the generated discriminator versus its manual equivalent.
#[path = "../../crates/statechronicle-profiles/src/macros.rs"]
mod macros;
// The macro only needs this existing Operation constructor contract.
struct Operation(String);
impl Operation {
    fn from_static(value: &'static str) -> Self { Self(String::from(value)) }
}
pub enum Command { Create { enabled: bool }, SetEnabled { enabled: bool } }
#[unsafe(no_mangle)]
pub fn manual(command: &Command) -> &'static str {
    match command {
        Command::Create { .. } => "configuration.create",
        Command::SetEnabled { .. } => "configuration.set_enabled",
    }
}
command_operations! {
    #[unsafe(no_mangle)]
    pub fn generated(command: &Command);
    static OPERATIONS: Operation;
    Command::Create { .. } => "configuration.create",
    Command::SetEnabled { .. } => "configuration.set_enabled",
}
