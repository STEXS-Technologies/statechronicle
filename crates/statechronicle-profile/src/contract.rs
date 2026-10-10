/// Consumer-defined deterministic domain semantics.
///
/// Implementations must depend only on their arguments: no clocks, randomness,
/// I/O or mutable global context. Schema changes require a new schema version.
/// The standard adapter encodes states and commands with BCS; custom Serialize
/// implementations must be deterministic and roundtrip through Deserialize.
/// State and command types can use fixed-capacity storage; this trait allocates
/// nothing and requires neither an allocator nor the standard library.
pub trait StateProfile: Sync {
    /// Consumer's native state model.
    type State;
    /// Consumer's native transition command.
    type Command;
    /// Consumer's validation/reduction failure.
    type Error;
    /// Stable globally namespaced domain identity.
    const DOMAIN_ID: &'static str;
    /// Explicit schema version, starting at one.
    const SCHEMA_VERSION: u32;
    /// Maximum canonical state and command payload size accepted by the adapter.
    const MAX_PAYLOAD_BYTES: usize;
    /// Public operation corresponding to this command's discriminator.
    /// The adapter checks this against the signed intent operation so authority
    /// permissions for one transition cannot authorize a different command.
    fn operation(&self, command: &Self::Command) -> &'static str;
    /// Validates a command and the state invariants before reduction.
    ///
    /// # Errors
    /// Returns the consumer's validation error for illegal transitions.
    fn validate(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<(), Self::Error>;
    /// Computes a new state without mutating the input.
    ///
    /// # Errors
    /// Returns the consumer's reduction error for illegal transitions.
    fn apply(
        &self,
        current: Option<&Self::State>,
        command: &Self::Command,
    ) -> Result<Self::State, Self::Error>;
    /// Validates the invariants of decoded and newly reduced states.
    ///
    /// # Errors
    /// Returns the consumer's error for an invalid state.
    fn validate_state(&self, state: &Self::State) -> Result<(), Self::Error>;
}
