#![no_main]
use libfuzzer_sys::fuzz_target;
use statechronicle_core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle_domain::commit::ProfileId;
use statechronicle_domain::custom::CustomState;
use statechronicle_domain::resource_state::ResourceState;
use statechronicle_profiles::{StateProfile, custom::TypedProfile};
struct Counter;
impl StateProfile for Counter {
    type State = u64;
    type Command = u64;
    type Error = &'static str;
    const DOMAIN_ID: &'static str = "fuzz.counter";
    const SCHEMA_VERSION: u32 = 1;
    const MAX_PAYLOAD_BYTES: usize = 8;
    fn operation(&self, _command: &u64) -> &'static str {
        "counter.set"
    }
    fn validate(&self, _current: Option<&u64>, _command: &u64) -> Result<(), Self::Error> {
        Ok(())
    }
    fn apply(&self, _current: Option<&u64>, command: &u64) -> Result<u64, Self::Error> {
        Ok(*command)
    }
    fn validate_state(&self, _state: &u64) -> Result<(), Self::Error> {
        Ok(())
    }
}
fuzz_target!(|data: &[u8]| {
    if data.len() > 4096 {
        return;
    }
    let profile = TypedProfile::new(Counter, &[]);
    let mut state = ResourceState::Custom(CustomState {
        domain: ProfileId::new(String::from(Counter::DOMAIN_ID)).unwrap(),
        schema_version: 1,
        payload: data.to_vec(),
    });
    if let Ok(value) = profile.decode_state(&state) {
        assert_eq!(canonicalize(&value).unwrap(), data);
        let digest = canonicalize_and_digest(&state).unwrap();
        if let ResourceState::Custom(custom) = &mut state {
            custom.schema_version = 2;
        }
        assert!(profile.decode_state(&state).is_err());
        assert_ne!(digest, canonicalize_and_digest(&state).unwrap());
    }
    if let Ok(decoded) = bcs::from_bytes::<ResourceState>(data) {
        assert_eq!(
            bcs::from_bytes::<ResourceState>(&canonicalize(&decoded).unwrap()).unwrap(),
            decoded
        );
        let _ = profile.decode_state(&decoded);
    }
});
