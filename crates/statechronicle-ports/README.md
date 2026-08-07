# statechronicle-ports

## What it is

The ten backend-agnostic port traits consumers implement to wire their own
storage, authority, and transport backends. Following the trustgrant-ports
convention, this crate declares port traits only: there are no implementations
inside. Driven adapters implement these traits and are wired at the consumer's
composition root.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §27 | Infra-Agnostic Storage Contract | The logical stores the protocol requires, expressed as ports |
| §28 | API Surface | The surfaces adapters expose to consumers |
| §19 | Commit Authority | The delegated-authority evaluator port (see ADR-003) |

## Key types (the ten port traits)

- `intent_store::IntentStore`: dedup + idempotency for intents.
- `event_store::EventStore`: append-only storage of validated events.
- `commit_store::CommitStore`: signed commits and snapshots.
- `state_index::StateIndex`: read access to derived state projections.
- `proof_index::ProofIndex`: storage/query of served proofs.
- `snapshot_store::SnapshotStore`: opaque snapshot payloads.
- `tenant_store::TenantStore`: tenant scope existence resolution.
- `trustgrant_evaluator::TrustGrantEvaluator`: delegated-authority evaluation
  and freshness checks (trait-only, dependency-free by construction).
- `transaction_manager::TransactionManager`: atomic multi-store coordination.
- `event_publisher::EventPublisher`: delivery of committed events and commits.

## How it's used

The consumer implements these traits against its own storage, authority, and
transport backends, then wires them into `Executor::new` and `ProofService` at
its composition root. No implementations live inside this crate.

```rust
use statechronicle_ports::intent_store::IntentStore;
// impl IntentStore for MyIntentStore { ... }
```

## Authority model

This crate declares the `TrustGrantEvaluator` **port** only. It is
`trait-only and dependency-free by construction`: it references only
`statechronicle-domain` types, so it is not coupled to any particular authority
provider. The port is a **delegation-of-authority boundary**, not a general
platform authorization system. Owner/actor identity and basic authorization are
the platform's own auth system, applied before or alongside this port. Any
evaluator that returns an `allow` result and passes the freshness check can be
plugged in; TrustGrant is **one option, not a requirement**.

## Dependencies

`statechronicle-domain`, `thiserror`, `trait-variant`.

## Tests

`tests/`: `ports_conformance.rs`, plus inline unit tests for error types and
trait contracts.

## Where it fits

The outer boundary of the architecture: the traits `statechronicle-executor`
and `statechronicle-proof` consume, implemented by the consumer. The umbrella
crate re-exports this module as `ports`.
