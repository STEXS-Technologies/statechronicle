#![no_main]
//! Differential native transition/replay fuzzing over two independent domains.

#[path = "../../crates/statechronicle/examples/common/domains.rs"]
#[allow(dead_code)]
mod domains;

use chrono::{DateTime, Utc};
use libfuzzer_sys::fuzz_target;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use statechronicle::core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle::domain::event::StateCommitment;
use statechronicle::domain::intent::Nonce;
use statechronicle::domain::resource_state::ResourceState;
use statechronicle::portable::codec::CanonicalCodec;
use statechronicle::portable::transition::{EncodedState, TransitionRequest, execute};
use statechronicle::profiles::registry::ProfileRules;
use statechronicle::{
    CommitId, Event, EventId, Intent, IntentId, Operation, ResourceId, StateProjection, StateType,
    SubjectId, TenantId,
};
use statechronicle::{StateProfile, TypedProfile};
use std::collections::BTreeMap;
use std::fmt::{Debug, Display};

fn exercise<P>(
    profile: &P,
    adapter: &TypedProfile<P>,
    bytes: &[u8],
    command_for: fn(u8) -> P::Command,
) where
    P: StateProfile,
    P::State: Serialize + DeserializeOwned + CanonicalCodec,
    P::Command: Serialize + DeserializeOwned + CanonicalCodec,
    P::Error: Display + Debug,
{
    let mut current: Option<StateProjection> = None;
    let now = DateTime::<Utc>::from_timestamp(0, 0).unwrap();
    for (index, byte) in bytes.iter().take(32).enumerate() {
        let command = command_for(*byte);
        let command_bytes = canonicalize(&command).unwrap();
        let operation = Operation::from_static(profile.operation(&command));
        let inputs = BTreeMap::from([
            (String::from("profile"), json!(P::DOMAIN_ID)),
            (String::from("schema_version"), json!(P::SCHEMA_VERSION)),
            (String::from("command"), json!(command_bytes)),
        ]);
        let version = current.as_ref().map_or(0, |state| state.version);
        let native_bytes = current
            .as_ref()
            .map(|state| canonicalize(&adapter.decode_state(&state.state).unwrap()).unwrap());
        let request = TransitionRequest {
            domain: P::DOMAIN_ID,
            schema: P::SCHEMA_VERSION,
            expected_version: version,
            operation: operation.as_str(),
            command: &command_bytes,
            current: native_bytes.as_ref().map(|payload| EncodedState {
                domain: P::DOMAIN_ID,
                schema: P::SCHEMA_VERSION,
                version,
                payload,
            }),
        };
        let mut output = [0_u8; 64];
        let mut scratch = [0_u8; 64];
        let portable = execute(profile, &request, &mut output, &mut scratch);
        let standard = adapter.apply(current.as_ref(), &operation, &inputs);
        assert_eq!(portable.is_ok(), standard.is_ok());
        let Ok(after) = standard else {
            continue;
        };
        assert_eq!(
            after,
            adapter
                .apply(current.as_ref(), &operation, &inputs)
                .unwrap()
        );
        let reduced = portable.unwrap();
        assert_eq!(
            reduced.state_hash,
            *canonicalize_and_digest(&after).unwrap().as_bytes()
        );
        assert_eq!(
            &output[..reduced.payload_len],
            canonicalize(&adapter.decode_state(&after).unwrap()).unwrap()
        );
        let intent = Intent::new(
            TenantId(String::from("fuzz.domain")),
            IntentId::new(format!("int_{index}")).unwrap(),
            operation,
            SubjectId(String::from("operator")),
            ResourceId(String::from("native")),
            Some(StateType::Custom),
            version,
            inputs,
            None,
            now,
            None,
            Nonce::from_b64u_str("b64u:AA").unwrap(),
        );
        let before = current.as_ref().map_or_else(
            || {
                let ResourceState::Custom(mut state) = after.clone() else {
                    unreachable!()
                };
                state.payload.clear();
                ResourceState::Custom(state)
            },
            |state| state.state.clone(),
        );
        let event = Event::new(
            intent.tenant_id.clone(),
            EventId::new(format!("evt_{index}")).unwrap(),
            intent.intent_id.clone(),
            intent.operation.clone(),
            intent.resource_id.clone(),
            intent.actor.clone(),
            StateCommitment {
                version,
                state_hash: canonicalize_and_digest(&before).unwrap(),
                state: before,
            },
            StateCommitment {
                version: reduced.version,
                state_hash: canonicalize_and_digest(&after).unwrap(),
                state: after,
            },
            None,
            SubjectId(String::from("executor")),
            now,
        );
        adapter
            .verify_transition(current.as_ref(), &intent, &event)
            .unwrap();
        let mut tampered = event.clone();
        tampered.schema = String::from("statechronicle.event.future");
        assert!(
            adapter
                .verify_transition(current.as_ref(), &intent, &tampered)
                .is_err()
        );
        tampered.schema.clone_from(&event.schema);
        tampered.after.version = tampered.after.version.wrapping_add(1);
        assert!(
            adapter
                .verify_transition(current.as_ref(), &intent, &tampered)
                .is_err()
        );
        let mut substituted = event.clone();
        if let ResourceState::Custom(state) = &mut substituted.after.state {
            state.schema_version = state.schema_version.wrapping_add(1);
        }
        substituted.after.state_hash = canonicalize_and_digest(&substituted.after.state).unwrap();
        assert!(
            adapter
                .verify_transition(current.as_ref(), &intent, &substituted)
                .is_err()
        );
        let mut wrong_actor = intent.clone();
        wrong_actor.actor = SubjectId(String::from("other"));
        assert!(
            adapter
                .verify_transition(current.as_ref(), &wrong_actor, &event)
                .is_err()
        );
        current = Some(StateProjection {
            tenant_id: intent.tenant_id,
            resource_id: intent.resource_id,
            state_type: StateType::Custom,
            version: event.after.version,
            last_event_id: event.event_id,
            last_commit_id: CommitId::new(String::from("cmt_fuzz")).unwrap(),
            state_hash: event.after.state_hash,
            state: event.after.state,
        });
    }
}

fuzz_target!(|data: &[u8]| {
    exercise(
        &domains::Configuration,
        &domains::CONFIGURATION,
        data,
        |byte| {
            if byte & 1 == 0 {
                domains::ConfigurationCommand::Create {
                    enabled: byte & 2 != 0,
                }
            } else {
                domains::ConfigurationCommand::SetEnabled {
                    enabled: byte & 2 != 0,
                }
            }
        },
    );
    exercise(
        &domains::Equipment,
        &domains::EQUIPMENT,
        data,
        |byte| match byte % 5 {
            0 => domains::EquipmentCommand::Install,
            1 => domains::EquipmentCommand::Start,
            2 => domains::EquipmentCommand::Stop,
            3 => domains::EquipmentCommand::Fail,
            _ => domains::EquipmentCommand::Recover,
        },
    );
});
