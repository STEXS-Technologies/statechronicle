# statechronicle-proof

## What it is

Portable proof bundles and verification: assembles inclusion, state, ownership,
and non-membership proofs and implements the deterministic verification
algorithm of protocol §29, independent of any transport or persistence layer.
The async `ProofService` composes the read-side proof ports; verification
itself is pure library code.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §16 | Proof Model | Portable proof types and their structure |
| §16.1 | Proof Types | State, ownership, inclusion, non-membership |
| §16.2 | Resource State Proof Bundle | The bundle envelope over state + inclusion + authority |
| §16.3 | Proof Verification | How bundles verify against the signed commit chain |
| §29 | Verification Algorithm | The deterministic verification procedure |

## Key types

- `bundle::{build_state_proof, build_non_membership_proof, build_snapshot_proof, derive_state_key}`.
- `verify::{verify_bundle, verify_non_membership_bundle, verify_ownership, verify_inclusion, verify_proof, verify_commit_signature_with_key}`.
- `service::{ProofService, ProofPorts}`: the async composition layer over the
  proof/state/commit/snapshot ports.
- `inclusion`, `state`, `ownership`, `error`.

## How it's used

Proofs are served from committed state and verified against the signed commit
chain. A resource-state proof bundle carries the current projection, its SMT
inclusion proof, and the signed commit; `verify_bundle` checks it all.

```rust
use statechronicle_proof::{bundle::build_state_proof, verify::verify_bundle};
let proof = build_state_proof(&projection, &signed, &inclusion, &op, None, key)?;
assert!(verify_bundle(&proof, &signed, &verifying_key, &key).is_ok());
```

## Dependencies

`statechronicle-core`, `statechronicle-domain`, `statechronicle-accumulator`,
`statechronicle-ports`. Dev-only: `proptest`, `tokio`.

## Tests

`tests/`: `verification_pipeline.rs`, `non_membership.rs`, `property.rs`, plus
inline unit tests for tamper fail-closed behavior and known-answer verification.

## Where it fits

The final stage of the canonical pipeline (`submit -> execute -> commit ->
proof -> verify`). It is what external verifiers and the e2e harness exercise
to prove state. The umbrella crate re-exports `ProofService` and `ProofPorts`.
