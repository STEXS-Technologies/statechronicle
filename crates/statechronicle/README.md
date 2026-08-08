# statechronicle

## What it is

The **umbrella crate**: the single dependency consumers add to use the whole
protocol surface. It re-exports the nine underlying protocol crates under
collision-safe namespaces and surfaces the most-used types directly at the top
level. It ships no storage, HTTP, or authority implementation.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §1 | Summary | What StateChronicle is and is not |
| §28 | API Surface | The public surface this facade exposes |
| §33 | Example Full Stack Flow | The end-to-end pipeline this crate wires |
| §37 | Glossary | Terminology used across the protocol |

## Key types

This crate re-exports every crate in the workspace plus a curated top-level
facade: `Amount`, `ContentDigest`, `Signature`, `Commit`, `CommitScope`,
`Event`, `Intent`, `Operation`, `Signed`, `StateProjection`, `StateType`,
`SubjectId`, `TenantId`, `ResourceId`, `Executor`, `Ports`,
`ValidatedIntent`, `StateAccumulator`, `StateRoot`, `ProfileRegistry`,
`ProfileRules`, `ProofService`, `ProofPorts`, and the authority types.

## How it's used

Add `statechronicle` as a dependency and use either the flat top-level names or
the namespaced `core::`, `domain::`, `executor::`, `commit::`, `proof::`,
`accumulator::`, `intent::`, `profiles::`, and `ports::` forms. See the root
`README.md` for a complete end-to-end consumption sketch modeled on
`crates/statechronicle/tests/e2e.rs`.

```rust
use statechronicle::{Amount, TenantId, executor::pipeline::Executor};
```

## See it run

Eight runnable examples live in `examples/` (shared harness in
`examples/common/`). Run any with `cargo run -p statechronicle --example <name>`;
each prints a narrative, asserts its outcome, and exits 0 only on success.

| Example | Demonstrates |
|---|---|
| `inventory` | Unique asset lifecycle (mint → transfer → lock → unlock → restrict → restore → burn) with fail-closed rejections |
| `currency` | Fungible balance lifecycle with an atomic debit + credit transfer and amount math |
| `stack` | Consumable stack lifecycle |
| `access` | Entitlement and meter lifecycles |
| `marketplace` | Atomic settlement via `execute_batch` |
| `cross_tenant` | Cross-tenant atomic transaction via `execute_cross_tenant` |
| `proofs` | State, ownership, and non-membership proofs |
| `paid_asset` | Paid unique asset overlay: owner consent and hard delete |

## Dependencies

The nine protocol crates: `statechronicle-core`, `statechronicle-domain`,
`statechronicle-intent`, `statechronicle-executor`, `statechronicle-commit`,
`statechronicle-accumulator`, `statechronicle-proof`, `statechronicle-profiles`,
`statechronicle-ports`. Dev-only: `tokio`, `serde_json`, `ed25519-dalek`,
`chrono`, `async-trait`.

## Tests

`tests/`: `e2e.rs` (full cross-crate lifecycle through real crates with
in-memory port fakes), `smoke.rs` (compile-only facade resolution), `common/`
(shared harness and fakes).

## Where it fits

The top of the crate stack: the entry point for consumers. It is the single
place the protocol surface is presented; the underlying crates remain usable
individually for finer-grained consumption.
