# statechronicle-executor

## What it is

The protocol's "brain": the ordered execution pipeline (protocol §18.1) that
runs a validated intent through conflict gates, expected-version checks,
delegated-authority evaluation, and profile rules, producing a deterministic
after-state and the emitted event. The pipeline drives pure transition and
conflict logic through injected port traits. Plain execution returns events;
durable entry points invoke consumer-owned sinks and return success only after
the sink completes its write contract.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §18 | Execution Semantics | How intents become events |
| §18.1 | Validation Pipeline | Ordered checks: idempotency, actor auth, tenant scope, state load, expected version, authority evaluation, profile rules, after-state |
| §18.2 | Conflict Rules | Fail-closed conflict checks before any event is emitted |
| §18.3 | Atomicity | Multi-resource transactions commit all or none |
| §19 | Commit Authority | How the executor binds authority evidence into events |

## Key types

- `pipeline::Executor`: the injected-port execution engine.
- `pipeline::Ports`: the bundle of port trait objects the executor drives.
- `pipeline::TrustGrantPort`: the dyn-compatible delegated-authority adapter
  (see ADR-003).
- `transition`: compatibility wrappers around baseline profile reducers.
- `conflict`: fail-closed conflict checks.
- `atomicity`: compatibility wrappers around baseline batch invariants.
- `error::ExecutorError`.

## How it's used

The composition root wires a `Ports` bundle (intent store, state index, tenant
store, one or more delegated-authority evaluators, transaction manager) plus a
profile registry, an executor subject, a wall clock, an event-id generator, and
an intent-signature verifier into `Executor::new`, then calls
`executor.execute(&validated_intent).await` for pure planning or explicitly
trusted service work. It must not be exposed as a player ingress. Player
requests should use `execute_player_durable_with_key_registry`; multi-action
player requests should use `execute_player_batch_durable_with_key_registry`,
with a durable sink that uses verified commit persistence.

```rust
use statechronicle_executor::pipeline::{Executor, Ports};
// Wire ports, registry, clock, id generator, verifier...
let events = executor.execute(&validated_intent).await?; // planning/trusted work only
```

## See it run

Pass `ProfileRegistry::with_custom` to `Executor::builder().profiles(...)` to
execute native consumer domains. The generic pipeline delegates addressing,
access, routing and reduction to profile-owned policies. Custom commands mutate
one resource per intent and retain the same tenant, version, authority and
durable-persistence contracts. See the
[extension guide](../../docs/DESIGN/DOMAIN_EXTENSIONS.md).

`crates/statechronicle/examples/marketplace.rs` shows `execute_batch`
(all-or-nothing settlement), `cross_tenant.rs` shows `execute_cross_tenant`
(per-tenant atomic groups), and `access.rs` shows fail-closed rejections.

## Dependencies

`statechronicle-core`, `statechronicle-domain`, `statechronicle-intent`,
`statechronicle-ports`, `statechronicle-profiles`, `async-trait`, `chrono`,
`serde_json`. Dev-only: `proptest`, `tokio`.

## Tests

`tests/`: `common/` (in-memory fakes), `pipeline.rs`, `cross_tenant.rs`,
`property.rs`, plus inline unit tests for transition rules, conflict checks,
and atomicity.

## Where it fits

The second stage of the canonical pipeline, between `statechronicle-intent`
and `statechronicle-commit`. It consumes execution ports; commit persistence and
read-side services consume their own ports as well. The umbrella crate
re-exports `Executor` and `Ports`.
