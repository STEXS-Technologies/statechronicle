#![no_main]

use libfuzzer_sys::fuzz_target;
use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{StateAccumulator, StateUpdate};
use statechronicle_portable::codec::{CodecError, Decoder, Encoder};
use statechronicle_portable::integrity::{
    PathStep, verify_inclusion, verify_non_membership, verify_signature,
};

fuzz_target!(|data: &[u8]| {
    if data.len() > 8192 {
        return;
    }
    let mut reader = Decoder::new(data);
    if let Ok(value) = reader.uleb() {
        let mut buffer = [0_u8; 5];
        let mut writer = Encoder::new(&mut buffer);
        writer.uleb(value).unwrap();
        let consumed = writer.written().len();
        assert_eq!(data.get(..consumed).unwrap(), writer.written());
    }
    let mut reader = Decoder::new(data);
    let _ = reader.boolean();
    let _ = reader.u32();
    let _ = reader.u64();
    let _ = reader.string(64);
    let _ = reader.finish();
    let mut storage = [0x55_u8; 128];
    let mut writer = Encoder::new(&mut storage);
    let result = writer.bytes(data);
    if data.len() > 127 {
        assert_eq!(result, Err(CodecError::Capacity));
        assert!(writer.written().is_empty());
    } else {
        result.unwrap();
        let mut reader = Decoder::new(writer.written());
        assert_eq!(reader.bytes(127).unwrap(), data);
        reader.finish().unwrap();
    }
    if data.len() >= 32 {
        let key_bytes: [u8; 32] = data[..32].try_into().unwrap();
        let key = StateKey::new(key_bytes);
        let state = statechronicle_portable::integrity::hash(data);
        // Keep the companion distinct even when a fuzzer discovers a fixture
        // key through comparison instrumentation.
        let mut other_key = key_bytes;
        other_key[0] ^= 1;
        let mut tree = StateAccumulator::empty();
        tree.insert_batch(&[
            StateUpdate::new(key, state),
            StateUpdate::new(StateKey::new(other_key), [1; 32]),
        ])
        .unwrap();
        let proof = tree.prove_inclusion(&key).unwrap();
        let mut steps: Vec<PathStep> = proof
            .steps
            .iter()
            .map(|step| PathStep {
                level: step.level.try_into().unwrap(),
                sibling: step.sibling,
            })
            .collect();
        assert!(verify_inclusion(
            tree.root().as_bytes(),
            &key_bytes,
            &state,
            &steps
        ));
        if let Some(first) = steps.first_mut() {
            first.sibling[0] ^= 1;
            assert!(!verify_inclusion(
                tree.root().as_bytes(),
                &key_bytes,
                &state,
                &steps
            ));
        }
        let arbitrary: Vec<PathStep> = data
            .as_chunks::<33>()
            .0
            .iter()
            .take(256)
            .map(|chunk| PathStep {
                level: chunk[0],
                sibling: chunk[1..].try_into().unwrap(),
            })
            .collect();
        let _ = verify_inclusion(&state, &key_bytes, &state, &arbitrary);
        let _ = verify_non_membership(&state, &key_bytes, &arbitrary);
        if data.len() >= 96 {
            let signature = data[32..96].try_into().unwrap();
            let _ = verify_signature(&data[96..], &key_bytes, &signature);
        }
    }
});
