# statechronicle-ports

## What it is

Backend-agnostic port traits consumers implement to wire storage, authority,
transactions, and transport backends. The crate declares contracts only;
storage implementations remain application-owned.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §27 | Infra-Agnostic Storage Contract | The logical stores the protocol requires, expressed as ports |
| §28 | API Surface | The surfaces adapters expose to consumers |
| §19 | Commit Authority | The delegated-authority evaluator port (see ADR-003) |

## Key types

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
- `trade_index::TradeIndex`: keyed read access to accumulated trade records.
- `ledger_store::LedgerStore` / `LedgerTransaction`: atomic durable mutation
  boundary, including idempotency, signed commits, projections, and outbox.
- `commit_store::CommitStore::canonical_head`: typed canonical-chain anchor;
  proof-serving adapters must require this anchor rather than trusting an
  arbitrary signed fork.
- `authorization::Authorizer`: authenticated principal binding and policy.
- `outbox::OutboxStore`: lease-based post-commit delivery.
- `outbox::dispatch_once` / `dispatch_until_idle`: digest-verified bounded
  worker passes with retry-safe completion and shutdown draining.
- `outbox::PoisonPolicy` / `dispatch_once_with_policy`: explicit retry versus
  quarantine behavior for permanently corrupt payloads.
- `outbox::run_outbox_worker`: bounded supervisor ticks with shutdown,
  runtime-provided sleeping, and capped exponential backoff.
- `outbox::ConsumerDedupStore` / `consume_once`: durable downstream delivery
  deduplication with lease takeover and retry-safe application. The consumer
  effect must share the same transaction or use the delivery key as its own
  idempotency key.
- `key_registry::KeyRegistry`: tenant/actor-scoped key lifecycle and
  revocation-aware trust resolution.
- `key_registry::MemoryKeyRegistry`: thread-safe reference metadata registry
  for development; production should substitute an auditable KMS/HSM-backed
  implementation.
- `observability::MetricsSink`: privacy-safe mutation outcome telemetry.
- `observability::InMemoryMetrics`: bounded aggregate counters/latency sink
  for tests and small deployments; replace with a production exporter at the
  composition root.
- `quota::DistributedQuota`: shared rate-limit contract for multi-worker
  deployments; back it with Redis, an API gateway, or a database-side atomic
  procedure rather than treating the local limiter as globally authoritative.
  Use `try_acquire_many_validated` for account/tenant/operation dimensions;
  providers that do not implement atomic multi-key charging fail closed.

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
