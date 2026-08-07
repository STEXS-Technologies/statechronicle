#![no_main]

use libfuzzer_sys::fuzz_target;

use ed25519_dalek::SigningKey;

use statechronicle_core::signature::{sign, verify};

// Ed25519 strict verification must accept exactly the signed bytes: a signature
// produced over `data` verifies under the same key, and a single flipped byte
// in the message must fail verification.
fuzz_target!(|data: &[u8]| {
    if data.len() < 32 {
        return;
    }

    let mut seed = [0u8; 32];
    seed.copy_from_slice(&data[..32]);
    let key = SigningKey::from_bytes(&seed);

    let signature = sign(data, &key);
    assert!(verify(data, &key.verifying_key(), &signature).is_ok());

    // A flipped byte in the message must fail verification.
    let mut tampered = data.to_vec();
    let last = tampered.len().saturating_sub(1);
    tampered[last] ^= 0x01;
    assert!(verify(&tampered, &key.verifying_key(), &signature).is_err());
});
