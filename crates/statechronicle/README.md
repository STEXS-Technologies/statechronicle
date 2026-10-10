# statechronicle

## What it is

The **umbrella crate**: the single dependency consumers add to use the whole
protocol surface. It re-exports the twelve underlying library crates under
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

This crate re-exports the underlying library crates plus a curated top-level
facade: `Amount`, `ContentDigest`, `Signature`, `Commit`, `CommitScope`,
`Event`, `Intent`, `Operation`, `Signed`, `StateProjection`, `StateType`,
`SubjectId`, `TenantId`, `ResourceId`, `Executor`, `Ports`,
`ValidatedIntent`, `StateAccumulator`, `StateRoot`, `ProfileRegistry`,
`ProfileRules`, `StateProfile`, `TypedProfile`, `CustomState`, `ProofService`,
`ProofPorts`, and the authority types. `portable::` exposes the existing bounded
transition, codec and integrity APIs; the complete standard engine requires `std`.

## How it's used

Add `statechronicle` as a dependency and use either the flat top-level names or
the namespaced `core::`, `domain::`, `executor::`, `commit::`, `proof::`,
`accumulator::`, `intent::`, `profiles::`, `portable::`, and `ports::` forms. See the root
`README.md` for a complete end-to-end consumption sketch modeled on
`crates/statechronicle/tests/e2e.rs`.

```rust
use statechronicle::{Amount, TenantId, executor::pipeline::Executor};
```

## See it run

Fifteen runnable examples live in `examples/`: twelve behavioral demonstrations
and three performance probes. The shared in-memory harness lives in
`examples/common/harness.rs`; custom native profiles live in
`examples/common/domains.rs`. Run behavioral examples with
`cargo run -p statechronicle --example <name>` and performance probes with
`--release`. Symbolic transactions demonstrate planning, not durable storage.

| Example | Demonstrates |
|---|---|
| `inventory` | Unique asset lifecycle (mint → transfer → lock → unlock → restrict → restore → burn) with fail-closed rejections |
| `currency` | Fungible balance lifecycle with an atomic debit + credit transfer and amount math |
| `stack` | Consumable stack lifecycle |
| `access` | Entitlement and meter lifecycles |
| `marketplace` | All-or-nothing purchase-settlement planning via `execute_batch` |
| `cross_tenant` | Cross-tenant planning via `execute_cross_tenant` |
| `proofs` | State, ownership, and non-membership proofs |
| `paid_asset` | Paid unique asset overlay: owner consent and hard delete |
| `trade_value` | Asset/payment settlement planning with declared value legs |
| `trade_cross_tenant` | Cross-tenant trade planning linked by a manifest |
| `trade_bundle` | Declared multi-asset bundle settlement planning |
| `custom_domains` | Static native profile registration and direct configuration reduction/decoding |
| `throughput` | Direct reduction and canonical hashing |
| `e2e_throughput` | Signed execution/indexing with optional commits and batching |
| `parallel_throughput` | Independent executor/adaptor scaling |

See the [domain extension guide](../../docs/DESIGN/DOMAIN_EXTENSIONS.md) for
consumer-defined state, command binding, authority, replay/proof verification and
schema migration. Custom-domain integration tests exercise both configuration
and equipment through the signed and durable paths.

## Dependencies

The twelve library crates: `statechronicle-core`, `statechronicle-domain`,
`statechronicle-intent`, `statechronicle-executor`, `statechronicle-commit`,
`statechronicle-accumulator`, `statechronicle-proof`, `statechronicle-profiles`,
`statechronicle-ports`, `statechronicle-index`, `statechronicle-profile`,
`statechronicle-portable`. Dev-only: `tokio`,
`serde_json`, `ed25519-dalek`, `chrono`, `async-trait`.

## Tests

`tests/`: `e2e.rs` (full cross-crate lifecycle through real crates with
in-memory port fakes), `smoke.rs` (compile-only facade resolution), `common/`
(shared harness and fakes), and `custom_domains.rs` (native domains, portability,
fault injection, cancellation, replay and reordered/concurrent recovery).

## Where it fits

The top of the crate stack: the entry point for consumers. It is the single
place the protocol surface is presented; the underlying crates remain usable
individually for finer-grained consumption.
