# statechronicle-core

## What it is

The transport- and persistence-free foundation of the protocol: BCS canonical
serialization (ADR-004), SHA-256 content digests, Ed25519 signatures, exact
fixed-point amounts, and the size/safety limits that bound every protocol
payload. Every other crate in the workspace depends on this one. It ships no
I/O of any kind: pure functions and newtypes over bytes.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §17 | Canonicalization and Hashing | Deterministic BCS serialization of every hashed/signed object; `sha256:<hex>` content digests |
| §30 | Security Considerations | Size/safety limits enforced here before canonicalization |
| ADR-004 | Canonical Serialization, Hashing, Signature | The BCS baseline this crate implements |

## Key types

- `Amount` (+ `MAX_MANTISSA_DIGITS`, `MAX_SCALE`): exact unsigned fixed-point
  economic value (u128 mantissa x 10^-scale, scale <= 18, never floats).
- `ContentDigest`: a `sha256:<lowercase-hex>` content digest.
- `Signature`: an Ed25519 signature over canonicalized content.
- `canonicalize` / `canonicalize_and_digest`: BCS canonical bytes and their digest.
- `digest::hash_bytes`: raw SHA-256 over arbitrary bytes.
- `signature::{sign, verify}`: Ed25519 over canonical bytes.
- `limits::{MAX_INTENT_BYTES, check_size}`: bounded input sizes for
  intent/event/commit payloads.
- `error`: the shared protocol error type.

## How it's used

Consumers call the pure functions directly (parse an intent, canonicalize it,
sign the body, compute a digest). `statechronicle-domain` builds its objects on
top of these primitives, and the executor uses `Amount` for all economic
arithmetic.

```rust
use statechronicle_core::amount::Amount;
use statechronicle_core::digest::{ContentDigest, hash_bytes};

let amount = Amount::from_u64(125_000);
let digest = ContentDigest::new(hash_bytes(b"bytes"));
assert_eq!(amount.to_canonical_string(), "125000");
```

## Dependencies

`bcs`, `sha2`, `ed25519-dalek`, `hex`, `serde`, `serde_json`,
`thiserror`. Dev-only: `proptest`.

## Tests

`tests/`: `amount_property.rs`, `canonicalize.rs`, `digest.rs`, `limits.rs`,
`property.rs`, `signature.rs`, plus inline unit tests. Property tests lock the
determinism of canonicalization and the fail-closed behavior of amount
arithmetic.

## Where it fits

This is the bottom layer of the crate stack. `statechronicle-domain`,
`statechronicle-intent`, `statechronicle-executor`, `statechronicle-commit`,
`statechronicle-accumulator`, `statechronicle-proof`, and
`statechronicle-profiles` all depend on it. The umbrella crate re-exports its
primitives directly at the top level.
