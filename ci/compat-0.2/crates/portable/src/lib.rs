//! External fixed-capacity domain; no std, allocator, serializer or runtime.
#![no_std]
#![forbid(unsafe_code)]
use statechronicle_portable::codec::{CanonicalCodec, CodecError, Decoder, Encoder};
use statechronicle_portable::transition::{
    TransitionError, TransitionRequest, TransitionResult, execute,
};
use statechronicle_profile::StateProfile;

pub struct Counter(pub u64);
impl CanonicalCodec for Counter {
    fn encode(&self, encoder: &mut Encoder<'_>) -> Result<(), CodecError> {
        encoder.u64(self.0)
    }
    fn decode(decoder: &mut Decoder<'_>) -> Result<Self, CodecError> {
        decoder.u64().map(Self)
    }
}
pub struct Model;
impl StateProfile for Model {
    type State = Counter;
    type Command = Counter;
    type Error = ();
    const DOMAIN_ID: &'static str = "consumer.counter";
    const SCHEMA_VERSION: u32 = 1;
    const MAX_PAYLOAD_BYTES: usize = 8;
    fn operation(&self, _: &Counter) -> &'static str {
        "counter.add"
    }
    fn validate(&self, current: Option<&Counter>, command: &Counter) -> Result<(), ()> {
        current
            .map_or(0, |s| s.0)
            .checked_add(command.0)
            .map(|_| ())
            .ok_or(())
    }
    fn apply(&self, current: Option<&Counter>, command: &Counter) -> Result<Counter, ()> {
        current
            .map_or(0, |s| s.0)
            .checked_add(command.0)
            .map(Counter)
            .ok_or(())
    }
    fn validate_state(&self, _: &Counter) -> Result<(), ()> {
        Ok(())
    }
}

pub fn prepare(
    request: &TransitionRequest<'_>,
    output: &mut [u8; 8],
) -> Result<TransitionResult, TransitionError<()>> {
    execute(&Model, request, output, &mut [0; 8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use statechronicle_portable::transition::EncodedState;

    #[test]
    fn transition_refusal_preserves_input_and_version() {
        let command = 3_u64.to_le_bytes();
        let current = 4_u64.to_le_bytes();
        let request = TransitionRequest {
            domain: Model::DOMAIN_ID,
            schema: 1,
            expected_version: 1,
            operation: "counter.add",
            command: &command,
            current: Some(EncodedState {
                domain: Model::DOMAIN_ID,
                schema: 1,
                version: 1,
                payload: &current,
            }),
        };
        let mut output = [0; 8];
        let result = prepare(&request, &mut output).unwrap();
        assert_eq!(result.version, 2);
        assert_eq!(output, 7_u64.to_le_bytes());
        let stale = TransitionRequest {
            expected_version: 0,
            ..request
        };
        assert_eq!(prepare(&stale, &mut output), Err(TransitionError::Version));
        assert_eq!(current, 4_u64.to_le_bytes());
    }
}
