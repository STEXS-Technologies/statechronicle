# StateChronicle — Architecture

**Status:** Draft v0.1
**Last Updated:** 2026-08-03
**Related:** [STATECHRONICLE_PROTOCOL_UPDATED.md](../STATECHRONICLE_PROTOCOL_UPDATED.md),
[ADR-001](DESIGN/ADR/ADR-001-VERTICAL_SLICE_ARCHITECTURE.md),
[ADR-002](DESIGN/ADR/ADR-002-HEXAGONAL_ARCHITECTURE.md),
[ADR-003](DESIGN/ADR/ADR-003-TRUSTGRANT_PORTS_ONLY.md)

---

## 1. Positioning

StateChronicle is the third pillar of a three-protocol platform:

```text
Shardline       proves what the resource is          (content truth)
TrustGrant      proves who may act                    (authority truth)
StateChronicle  proves what happened and what state   (state truth)
```

StateChronicle is an infrastructure-agnostic protocol for recording, verifying, and
replaying resource state transitions. It is **not** a blockchain: no mining, no global
consensus, no token. It is a Git-inspired, append-only, content-addressed history model
with strict transaction semantics.

This repository implements the protocol as a **Rust Cargo workspace** whose architecture
mirrors the conventions already established in `stexs`, `shardline`, and `trustgrant`:

- **Hexagonal architecture** (ports & adapters) so domain logic is pure and testable.
- **Vertical slices** (one crate per bounded context) so each feature is owned end-to-end.
- **Crate-boundary layering** where ports are traits and adapters are impls, following the
  `trustgrant` convention of a dedicated ports crate with no implementations inside.
- **Stage-separated protocol documents** (`Raw → Validated → Verified`) following the
  trustgrant cold-path/hot-path split.
- **A composition root** (`api-http` equivalent) that owns all cross-slice wiring and
  dependency injection, following stexs.

StateChronicle is a **standalone, open-source workspace** built in the same style as
`shardline` and `trustgrant`. It has **no compile-time dependency on either** (see
ADR-003). TrustGrant is consumed only through `statechronicle-ports::TrustGrantEvaluator`;
the production adapter lives in the **stexs composition root**, which is the single place
the three protocols are wired together. Shardline is entirely out-of-band: StateChronicle
events carry content digests as metadata, and byte resolution/verification happens in
another stexs system.

---

## 2. Design Goals Mapped to Architecture

| Protocol goal | Architecture consequence |
|---|---|
| Verifiable state | Pure protocol core produces proofs; verification is a library, not a server |
| Infrastructure independence | No transport/persistence inside domain; all I/O behind ports |
| Deterministic transitions | Transition rules live in pure, unit-tested domain services |
| High-throughput batching | Commit formation is a separate bounded context from per-intent validation |
| TrustGrant-native authority | `TrustGrantEvaluator` port; authority proofs bound into events |
| Content references | Digest + media_type metadata on events; no content-store dependency (resolved out-of-band) |
| Replayability | Append-only EventStore port; state is a projection |
| Conflict safety | Fail-closed conflict checks in the execution pipeline |
| Portable proof bundles | Proof assembly/verification is transport-agnostic |
| Profile-based specialization | `statechronicle-profiles` crate + per-profile slices |
| Tenant isolation | `TenantStore`/scope resolution port; hard/logical isolation modes |
| Multiple state models | Profile types (unique, stack, fungible, entitlement, meter, escrow) |
| Durable paid ownership | Paid-unique invariants in the profile + no-hard-delete rule in executor |

---

## 3. Tech Stack

- **Rust**, edition 2024, stable toolchain (`rust-toolchain.toml`)
- **Cargo workspace** (`resolver = "3"`), members under `crates/`
- **cargo-make** (`Makefile.toml`) task runner: `check`, `clippy`, `test`, `audit`, `deny`, `fuzz`
- **cargo-nextest** for tests; **criterion** for benches; **cargo-fuzz** for fuzzing
- **cargo-deny** (`deny.toml`) for dependency/license policy
- Strict deny-by-default clippy lints at workspace level: no `unwrap`, no `expect`, no
  `panic`, no indexing in non-test code (same discipline as shardline/trustgrant/stexs)
- **Composition root:** axum-based HTTP binary (`statechronicle-http`)
- **Storage:** PostgreSQL via sqlx (driven adapters); per-slice migrations under
  `migrations/<slice>/`
- **Async:** tokio; async ports via `trait_variant::make(Send)` (stexs convention)

---

## 4. Workspace Layout

```text
statechronicle/
├── Cargo.toml                       # workspace root, resolver=3, shared deps, deny-lints
├── Makefile.toml / deny.toml / clippy.toml / rust-toolchain.toml / rustfmt.toml
├── migrations/                      # per-slice SQL migrations (up/down pairs)
│   ├── ledger/  inventory/  economy/  entitlement/  marketplace/  tenant/
├── docs/
│   ├── ARCHITECTURE.md              # this file
│   └── DESIGN/ADR/                  # ADR-001..003 (+ README, TEMPLATE)
├── fuzz/                            # cargo-fuzz targets (optional, later)
└── crates/
    ├── statechronicle-core/         # pure protocol primitives: BCS canonicalization,
    │                                #   SHA-256 digests, Ed25519 signing, size/safety limits
    ├── statechronicle-domain/       # core domain types: Tenant, ResourceId, SubjectId,
    │                                #   StateType, Intent, Event, Commit, Proof, state projections
    ├── statechronicle-intent/       # intent parsing + validation (Raw → ValidatedIntent)
    ├── statechronicle-executor/     # execution pipeline §18: validate → transition → event;
    │                                #   deterministic transition rules per state type
    ├── statechronicle-commit/       # commit batching, ordering, signing, checkpoint commits,
    │                                #   fork/failure semantics
    ├── statechronicle-accumulator/  # state-root accumulator (sparse Merkle tree baseline)
    ├── statechronicle-proof/        # proof bundles, inclusion/state/ownership proofs,
    │                                #   verification algorithm §29
    ├── statechronicle-profiles/     # baseline profile registry: unique_asset,
    │                                #   paid_unique_asset, consumable_stack, fungible_balance,
    │                                #   entitlement, meter, listing/escrow (protocol §20)
    ├── statechronicle-ports/        # ★ backend-agnostic port traits ONLY (no impls):
    │                                #   IntentStore, EventStore, CommitStore, StateIndex,
    │                                #   ProofIndex, SnapshotStore, TenantStore,
    │                                #   TrustGrantEvaluator,
    │                                #   TransactionManager, EventPublisher
    ├── statechronicle-shared/       # transport-agnostic kernel: newtype macros, error
    │                                #   envelope, config, metrics (stexs shared analog)
    ├── statechronicle-shared-http/  # HTTP-only concerns: error envelope, headers,
    │                                #   idempotency, validated_json, test harness
    ├── statechronicle-http/         # ★ composition root + HTTP binary (axum): wires slices,
    │                                #   ports, and infra; owns DI
    └── slices/                      # ★ VERTICAL SLICES — one crate per bounded context
        ├── ledger/                  # commit authority, batching, snapshot publishing
        ├── inventory/               # unique assets, consumable stacks, paid-unique rules
        ├── economy/                 # fungible balances, currency transfers, reserves
        ├── entitlement/             # entitlements, meters, licenses
        ├── marketplace/             # listings, escrow, atomic purchase settlement
        └── tenant/                  # tenant provisioning, isolation modes, scope resolution
```

The root `Cargo.toml` `[workspace.members]` lists every crate under `crates/` plus `fuzz`.

---

## 5. Two Kinds of Crates

### 5.1 Pure protocol crates (core)

These crates contain **no transport, no persistence, no framework**:

- `statechronicle-core`, `-domain`, `-intent`, `-executor`, `-commit`,
  `-accumulator`, `-proof`, `-profiles`

They implement the protocol deterministically. They consume ports from
`statechronicle-ports` only as traits passed into functions (trustgrant pattern: the core
"works with already-assembled domain types directly" and never calls port traits itself).
They are unit-tested inline and integration-tested against in-memory fake ports.

### 5.2 Platform slices (vertical slices)

These crates are the **application layer** of the platform. Each slice is a bounded
context with the stexs per-slice hexagonal layout (ADR-001, ADR-002):

```text
crates/slices/<name>/src/
├── lib.rs               # exports public contract: V1 DTOs, domain VOs, error, output ports
├── domain/              # PURE business logic — no axum/sqlx
│   ├── aggregates/  entities/  events/  value_objects/  services/  specifications/
├── application/         # orchestration (CQRS)
│   ├── commands/        # <UseCase>CommandHandlerV1 (generic, static dispatch)
│   ├── queries/         # <UseCase>QueryHandlerV1
│   └── view.rs          # read-model DTOs (<X>ViewV1)
├── ports/
│   ├── input/           # port traits the slice exposes: <UseCase>CommandPort / QueryPort
│   └── output/          # ports the slice needs: Repository, Gateway, Cache, Publisher, Outbox, TransactionManager
├── adapters/
│   ├── driving/         # entry adapters: rest/ (axum handlers, router.rs, openapi.rs)
│   └── driven/          # impl adapters: postgres/ (sqlx repos), redis/, in_memory/ (test fakes)
├── error/               # slice-specific errors: domain.rs / infra.rs / rest.rs
└── tests/               # optional integration tests
```

Dependency rule (enforced): `domain → nothing`; `application → domain`;
`adapters → application/domain`. Slices never import each other's internals; cross-slice
communication happens at the composition root through ports/gateways, or through the
shared kernel. `shared` must stay transport-agnostic; HTTP-only types live in
`shared-http`.

### 5.3 Ports crate (`statechronicle-ports`)

Following trustgrant-ports: a single crate holding all backend-agnostic port traits,
**with no implementations inside**. Adapters are the consumer's job and are wired at the
composition root. This is what keeps core and slices free of persistence/transport.

---

## 6. Protocol Flow (core)

The protocol's canonical pipeline maps directly onto core crates:

```text
submit_intent
  → statechronicle-intent      parse + schema-validate + intent_id idempotency (Raw→ValidatedIntent)
  → statechronicle-executor    §18.1 pipeline: auth, tenant scope, expected_version,
                               TrustGrant evaluation (via port), profile rules,
                               deterministic after-state, conflict fail-closed checks (§18.2)
  → emit Event(s)              append to EventStore (port)
  → statechronicle-commit      batch, order deterministically, compute event Merkle root +
                               state roots, sign commit (Ed25519), persist CommitStore (port)
  → statechronicle-accumulator update state root (sparse Merkle tree)
  → statechronicle-proof      assemble proof bundles; verify (§29)
```

Atomicity (§18.3): a transaction commits all affected state transitions or none.
Multi-resource transactions are validated against every affected state record's
`expected_version`.

---

## 7. Data Flow Through the Platform (slices + composition root)

```text
Client ──HTTP──▶ statechronicle-http (axum)
                  │  routes /v1/... (ledger, inventory, economy, entitlement, marketplace, tenant)
                  ▼
            driving adapter (slice)        e.g. InventoryRestState
                  ▼
            ports::input::CommandPort      e.g. TransferAssetCommandPort
                  ▼
            application::CommandHandlerV1  e.g. TransferAssetCommandHandlerV1
                  ▼
            ports::output::Gateway/Repo    e.g. AssetRepository, EventPublisher
                  ▼
   [composed at stexs root: PostgresAssetRepository, OutboxEventPublisher,
    TrustGrantEvaluatorAdapter → trustgrant (stexs-owned),
    TransactionManager → postgres]
```

Cross-slice flows (e.g. an atomic purchase: marketplace + economy + inventory + ledger)
are orchestrated by **higher-layer gateways** implemented at the composition root, which
call multiple slices' input ports — never by slices importing each other.

---

## 8. Storage Contract (protocol §27)

Logical stores are expressed as ports in `statechronicle-ports`, with Postgres/object-store
adapters wired at the composition root:

| Logical store | Port (trait) | Purpose |
|---|---|---|
| Intent store | `IntentStore` | Deduplication and idempotency |
| Event store | `EventStore` | Append-only validated transitions |
| Commit store | `CommitStore` | Signed batch commits |
| State index | `StateIndex` | Current state projection |
| Proof index | `ProofIndex` | Efficient proof generation |
| Snapshot store | `SnapshotStore` | Optional compact checkpoints |
| Tenant scope | `TenantStore` | Tenant roots, isolation modes |
| Authority | `TrustGrantEvaluator` | TrustGrant evaluation binding (stexs-owned adapter) |
| Transaction | `TransactionManager` | Atomic multi-store boundaries |
| Events | `EventPublisher` | Outbox/event bus publication |

The backend may vary; the canonical objects and verification results must not.

---

## 9. Integration Boundaries (ADR-003)

- **TrustGrant:** `statechronicle-ports::TrustGrantEvaluator` — the executor calls the
  port during the pipeline (§18.1 step 8) and fails closed unless the result is `allow`
  and fresh; the trustgrant evaluation itself runs behind the port. The
  `TrustGrantEvaluatorAdapter` lives in the **stexs composition root** (extending the
  existing stexs `infra/trustgrant/` pattern); StateChronicle v0 uses an in-memory fake.
- **Shardline:** no integration. Events carry `content: { kind, digest, media_type,
  size }` as pure metadata; resolution and byte verification happen out-of-band in
  another stexs system.
- **Commit authority:** a `service:statechronicle...` subject authorized via TrustGrant
  (checked at the stexs root); commit keys Ed25519, rotated through TrustGrant-authorized
  key transition procedures.

---

## 10. Testing Strategy

- **Core crates:** inline `#[cfg(test)]` unit tests for deterministic rules (transitions,
  canonicalization, conflict checks, accumulator); integration tests against in-memory fake
  ports.
- **Slices:** inline unit tests + hand-rolled fake adapters (`RecordingXGateway`,
  `InMemoryXRepository`); integration tests under `crates/slices/<name>/tests/`.
- **Composition root:** `statechronicle-http/tests/e2e/` end-to-end flows (mint → transfer →
  proof → verify) against Postgres + the stexs-wired trustgrant adapter.
- **Interop/conformance:** language-agnostic JSON test vectors (trustgrant convention) for
  canonicalization, event replay, and proof verification.
- **Tooling:** nextest, criterion benches, cargo-fuzz (later), cargo-deny.

---

## 11. Conventions (aligned with stexs/shardline/trustgrant)

- **Crates:** `statechronicle-<concern>` for core; `slices/<name>` for slices.
- **DTOs/contracts:** `…V1` suffix — `<UseCase>CommandV1`, `<UseCase>QueryV1`,
  `<UseCase>ViewV1`.
- **Ports:** input `<UseCase>CommandPort` / `<UseCase>QueryPort`; output
  `<Noun>Repository`, `<Noun>Gateway`, `<Noun>Cache`, `EventPublisher`, `Outbox`,
  `TransactionManager`. Async via `trait_variant::make(Send)`.
- **Handlers:** `<UseCase>CommandHandlerV1<G: Gateway>` — generic, static dispatch, no
  trait objects; `Arc`-injected deps.
- **Adapters:** prefixed by impl tech — `Postgres*`, `Redis*`, `InMemory*`, `Ed25519*`,
  `Hmac*`.
- **Domain types:** validated newtypes (macro-generated), prefixed IDs
  (`stc_…`, `evt_…`, `cmt_…`, `int_…`), never loose strings.
- **Stage documents:** `Raw → Validated → Verified` where the pipeline benefits
  (intent parsing, proof verification).
- **Errors:** `thiserror` per crate/slice; `statechronicle-core`/`shared` carry the
  shared envelope + safety `limits` (mirrors trustgrant-error).
- **lib.rs / mod.rs:** declaration + re-export shells only (shardline rule).
- **Docs:** `docs/ARCHITECTURE.md`, `docs/DESIGN/ADR/*`, per-crate READMEs.

---

## 12. Open Items (protocol §36 as architecture decisions)

1. Baseline state accumulator: sparse Merkle tree (recommended) vs ordered Merkle map —
   ADR pending once `statechronicle-accumulator` is specced.
2. Whether commit authority is always represented through TrustGrant — default yes for v0.
3. Minimum revocation freshness window for authority-bound transitions.
4. Global operation registries vs local namespaces for profiles.
5. Whether tenant roots are mandatory even for single-tenant deployments.
6. Whether proof bundles embed full TrustGrant chains or digests + resolvable references
   (default: digests + references).

---

## 13. v0 Scope (protocol §35 minimal implementation)

- BCS canonicalization (binary; JSON retained as the HTTP API logical view), SHA-256
  digests, Ed25519 commit signatures
- Tenant-scoped intent deduplication, append-only events, signed commits,
  current-state projection
- `asset.mint` / `asset.transfer` / `asset.burn` / `asset.lock` / `asset.unlock`
- Paid unique asset no-hard-delete rule
- `stack.credit` / `stack.debit` / `stack.consume`
- `balance.credit` / `balance.debit` / `balance.transfer`
- State proof (current owner) + balance/quantity proofs
- TrustGrant evaluation binding (port; in-memory fake for v0, stexs-wired adapter in production)
- Content digest references as event metadata (out-of-band; no content-store dependency)

Backend for first implementation: PostgreSQL for transactional execution and projections;
object storage for immutable commit and snapshot objects; TrustGrant for authority via the
`TrustGrantEvaluator` port (adapter owned by the stexs root). The protocol must not
require this backend.
