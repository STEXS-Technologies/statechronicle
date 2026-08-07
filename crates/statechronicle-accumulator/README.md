# statechronicle-accumulator

## What it is

The state-root accumulator (ADR-005): maintains the current state root over a
fixed 256-bit-depth sparse Merkle tree baseline, one per tenant. It enables
compact inclusion and non-membership proofs for state, and composes tenant
roots into optional logical-isolation checkpoint roots.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §14 | State Root Model | The state root as a pure function of the key->leaf set |
| §16.2 | Resource State Proof Bundle | The SMT inclusion/non-membership path behind proof bundles |
| ADR-005 | Baseline State Accumulator | Fixed 256-bit SMT, one per tenant |

## Key types

- `sparse_merkle::{StateAccumulator, StateRoot, StateUpdate}`: the SMT and its
  root/update types.
- `key::StateKey`: domain-separated state key derivation.
- `proof`: level-tagged inclusion and non-membership proofs.
- `checkpoint::CheckpointRoot`: logical-isolation composition over tenant roots.
- `error`: accumulator error type.

## How it's used

The commit crate computes state-root updates from events and feeds them to an
accumulator; the proof crate uses the accumulator to prove inclusion or
non-membership of a state key against a signed commit root.

```rust
use statechronicle_accumulator::sparse_merkle::StateAccumulator;
let mut acc = StateAccumulator::empty();
acc.insert_batch(&updates)?;
let root = acc.root();
```

## Dependencies

`statechronicle-core`, `statechronicle-domain`. Dev-only: `proptest`.

## Tests

`tests/`: `accumulator.rs`, `known_answers.rs`, `property.rs`, plus inline unit
tests for known-answer vectors, insertion order determinism, and non-membership
proofs.

## Where it fits

Between `statechronicle-commit` (which produces root updates) and
`statechronicle-proof` (which consumes the accumulator for proofs). The
umbrella crate re-exports `StateAccumulator`, `StateRoot`, `StateUpdate`, and
`StateKey`.
