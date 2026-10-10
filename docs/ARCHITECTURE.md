# StateChronicle: Architecture

**Status:** Implementation overview for 0.2.0
**Last Updated:** 2026-10-10
**Related:** [ADR-001](DESIGN/ADR/ADR-001-VERTICAL_SLICE_ARCHITECTURE.md),
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

StateChronicle is an infrastructure-agnostic library and protocol for recording,
verifying, and replaying consumer-defined and built-in resource state transitions.
It is **not** a blockchain: no mining, no global
consensus, no token. It is a Git-inspired, append-only, content-addressed history model
with strict transaction semantics.

This repository implements the protocol as a **Rust Cargo workspace** whose architecture
mirrors sibling workspace conventions (`shardline`, `trustgrant`):

- **Hexagonal architecture** (ports & adapters) so domain logic is pure and testable.
- **Vertical slices** (one crate per bounded context) so each feature is owned end-to-end.
- **Crate-boundary layering** where a dedicated ports crate defines contracts
  and consumers supply backend adapters.
- **Stage-separated protocol documents** (`Raw → Validated → Verified`) following the
  trustgrant cold-path/hot-path split.
- **A composition root** that owns all cross-slice wiring and dependency injection,
  following sibling conventions. The composition root is the consumer's, not a crate in
  this workspace.

StateChronicle is a **standalone, open-source workspace** built in the same style as
`shardline` and `trustgrant`. It has **no compile-time dependency on either** (see
ADR-003). TrustGrant is consumed only through `statechronicle-ports::TrustGrantEvaluator`;
the production adapter lives in the **consumer's composition root**, which is the single
place the three protocols are wired together. Shardline is entirely out-of-band:
StateChronicle events carry content digests as metadata, and byte
resolution/verification happens in another consumer system.

---

## 2. Design Goals Mapped to Architecture

| Protocol goal | Architecture consequence |
|---|---|
| Verifiable state | Pure protocol core produces proofs; verification is a library, not a server |
| Infrastructure independence | No transport/persistence inside domain; all I/O behind ports |
| Deterministic transitions | Transition rules live in pure, unit-tested domain services |
| High-throughput batching | Commit formation is a separate bounded context from per-intent validation |
| Authority-aware execution | Delegated-authority evaluator port; authority proofs bound into events |
| Content references | Digest + media_type metadata on events; no content-store dependency (resolved out-of-band) |
| Replayability | Append-only EventStore port; state is a projection |
| Conflict safety | Fail-closed conflict checks in the execution pipeline |
| Portable proof bundles | Proof assembly/verification is transport-agnostic |
| Domain extensibility | Native `StateProfile` contracts, typed BCS adapters and static domain/schema registration |
| Profile-based specialization | Profile-owned validation, reduction, addressing, access and event-group policies |
| Tenant isolation | `TenantStore`/scope resolution port; hard/logical isolation modes |
| Multiple state models | Built-in resource types plus native custom state/command types |
| Durable paid ownership | Paid-unique ownership and no-hard-delete invariants in the profile |
| Bounded portable core | Existing allocation-free profile/transition/codec/integrity crates; full standard engine requires `std` |

---

## 3. Tech Stack

- **Rust**, edition 2024, stable toolchain (`rust-toolchain.toml`)
- **Cargo workspace** (`resolver = "3"`), members under `crates/` plus `fuzz`
- **cargo-make** (`Makefile.toml`) task runner: `check`, `clippy`, `test`, `audit`, `deny`, `fuzz`
- **cargo-nextest** for tests; **cargo-fuzz** for fuzzing
- **cargo-deny** (`deny.toml`) for dependency/license policy
- Strict deny-by-default clippy lints at workspace level: no `unwrap`, no `expect`, no
  `panic`, no indexing in non-test code (same discipline as shardline/trustgrant)
- **Async:** host runtime supplied by consumers; standard adapters/ports use
  Send-capable async interfaces, with tokio used in examples and tests
- **Storage/HTTP:** none in this workspace. Consumers provide these through the ports
  at their own composition root.

---

## 4. Workspace Layout

```text
statechronicle/
├── Cargo.toml                       # workspace root, resolver=3, shared deps, deny-lints
├── Makefile.toml / deny.toml / clippy.toml / rust-toolchain.toml / rustfmt.toml
├── docs/
│   ├── ARCHITECTURE.md              # this file
│   └── DESIGN/ADR/                  # ADR-001..006 (+ README, TEMPLATE)
├── fuzz/                            # cargo-fuzz targets
└── crates/
    ├── statechronicle-core/         # pure protocol primitives: BCS canonicalization,
    │                                #   SHA-256 digests, Ed25519 signing, fixed-point amounts,
    │                                #   size/safety limits
    ├── statechronicle-domain/       # core domain types: Tenant, ResourceId, SubjectId,
    │                                #   StateType, Intent, Event, Commit, Proof, state projections
    ├── statechronicle-intent/       # intent parsing + validation (Raw → ValidatedIntent)
    ├── statechronicle-executor/     # execution pipeline §18: validate → transition → event;
    │                                #   profile dispatch and durable sink entry points
    ├── statechronicle-commit/       # commit batching, ordering, signing, checkpoint commits,
    │                                #   fork/failure semantics
    ├── statechronicle-accumulator/  # state-root accumulator (sparse Merkle tree baseline)
    ├── statechronicle-proof/        # proof bundles, inclusion/state/ownership proofs,
    │                                #   verification algorithm §29
    ├── statechronicle-profile/      # allocation-free no_std native domain contract
    ├── statechronicle-portable/     # bounded transitions, codecs, compatible hashes,
    │                                #   signature and sparse-Merkle verification
    ├── statechronicle-profiles/     # typed custom adapters, static registry, baseline reducers:
    │                                #   unique_asset,
    │                                #   paid_unique_asset, consumable_stack, fungible_balance,
    │                                #   entitlement, meter, listing/escrow (protocol §20)
    ├── statechronicle-index/        # ★ trade read-side vertical slice: deterministic index
    │                                #   builder, history reconstruction, proof serving
    ├── statechronicle-ports/        # ★ backend-agnostic contracts and policy types:
    │                                #   IntentStore, EventStore, CommitStore, StateIndex,
    │                                #   ProofIndex, TradeIndex, SnapshotStore, TenantStore,
    │                                #   TrustGrantEvaluator,
    │                                #   TransactionManager, EventPublisher
    └── statechronicle/              # ★ umbrella crate: namespaced re-exports + facade
```

The root `Cargo.toml` `[workspace.members]` lists every crate under `crates/` plus `fuzz`.
There is no `statechronicle-http`, `statechronicle-shared`, `statechronicle-shared-http`,
`slices/`, or `migrations/` in this workspace; consumers who want those own them.

---

## 5. Two Kinds of Crates

### 5.1 Pure protocol crates (core)

These crates contain **no transport, no persistence, no framework**:

- `statechronicle-core`, `-domain`, `-intent`, `-executor`, `-commit`,
  `-accumulator`, `-proof`, `-profiles`, `-index`, `-profile`, `-portable`

Pure reductions and integrity calculations operate on supplied data. Execution,
durable commit persistence and read-side services call injected ports; their
storage/authority implementations belong to the consumer. They are unit-tested
and integration-tested against in-memory adapters and injected faults.

### 5.2 The ports crate (`statechronicle-ports`)

A single crate holds backend-agnostic contracts and supporting policy types.
Backend adapters are the consumer's job and are wired at the composition root;
the crate does not supply a database or transport implementation.

### 5.3 The umbrella crate (`statechronicle`)

A thin facade that re-exports the protocol crates under collision-safe namespaces and
surfaces the most-used types directly. Consumers depend on this single crate and wire
their own port adapters at their composition root.

---

## 6. Protocol Flow (core)

The protocol's canonical pipeline maps directly onto core crates:

```text
submit_intent
  → statechronicle-intent      parse + schema-validate + intent_id idempotency (Raw→ValidatedIntent)
  → statechronicle-executor    §18.1 pipeline: auth, tenant scope, expected_version,
                               delegated-authority evaluation (via port), profile rules,
                               deterministic after-state, conflict fail-closed checks (§18.2)
  → emit Event(s)              form transition commitments; planning returns events
  → statechronicle-commit      batch, order deterministically, compute event Merkle root +
                               state roots, sign commit (Ed25519)
  → consumer durable sink     verified persistence through LedgerStore: events, commit,
                               projections, idempotency and outbox in one transaction
  → statechronicle-accumulator update state root (sparse Merkle tree)
  → statechronicle-proof      assemble proof bundles; verify (§29)
```

Durable atomicity requires an adapter satisfying the transaction contract.
Plain batch/cross-tenant methods perform all-or-nothing planning; durable sink
variants wait for persistence. Multi-resource work validates affected versions;
cross-tenant writes require one supported transaction across all legs.

For 0.2.0 custom domains, consumers define native state and command types,
deterministic reduction, invariants, operation binding and schema versions.
`TypedProfile` connects these models to the standard execution/history/proof
facilities; baseline resource semantics are supplied by profile policies.
See [domain extensions and migration](DESIGN/DOMAIN_EXTENSIONS.md) and the
[acceptance audit](DESIGN/0.2.0_ACCEPTANCE.md). Custom requests currently mutate
one resource per intent; schema changes use explicit consumer migrations.

---

## 7. Data Flow Through the Platform (consumer composition root)

The composition root is the consumer's. It wires the protocol
crates to the consumer's storage, authority, and transport via the ports:

```text
Client ──transport──▶ composition root (consumer-owned)
                        │  parses requests into raw intents
                        ▼
                  statechronicle-intent       parse + validate → ValidatedIntent
                        ▼
                  statechronicle-executor     §18.1 pipeline via injected Ports
                        │  events (returned, not persisted)
                        ▼
                  statechronicle-commit       form + sign commits
                        ▼
                  statechronicle-accumulator  update state root
                        ▼
                  statechronicle-proof        serve proofs / verify (§29)
                        ▼
   [consumer adapters: storage repositories, event publisher,
    delegated-authority evaluator, transaction manager]
```

Cross-store flows (e.g. an atomic purchase spanning marketplace + economy + inventory +
ledger) are orchestrated by the consumer's composition root through the injected ports,
never by crates importing each other's internals.

---

## 8. Storage Contract (protocol §27)

Logical stores are expressed as ports in `statechronicle-ports`, with adapters wired by
the consumer at its composition root:

| Logical store | Port (trait) | Purpose |
|---|---|---|
| Intent store | `IntentStore` | Deduplication and idempotency |
| Event store | `EventStore` | Append-only validated transitions |
| Commit store | `CommitStore` | Signed batch commits |
| State index | `StateIndex` | Current state projection |
| Proof index | `ProofIndex` | Efficient proof generation |
| Trade index | `TradeIndex` | Trade read-side records, keyed by `trade_id` (Phase 2) |
| Snapshot store | `SnapshotStore` | Optional compact checkpoints |
| Tenant scope | `TenantStore` | Tenant roots, isolation modes |
| Authority | `TrustGrantEvaluator` | Delegated-authority evaluation (consumer-owned adapter) |
| Transaction | `TransactionManager` | Atomic multi-store boundaries |
| Events | `EventPublisher` | Outbox/event bus publication |

The backend may vary; the canonical objects and verification results must not.

---

## 9. Integration Boundaries (ADR-003)

- **TrustGrant:** `statechronicle-ports::TrustGrantEvaluator`: the executor calls the
  port during the pipeline (§18.1 step 8) and fails closed unless the result is `allow`
  and fresh; the evaluation itself runs behind the port. The port is trait-only and
  dependency-free by construction, so any evaluator that returns `allow` and passes the
  freshness check can be plugged in. TrustGrant is one option, not a requirement. The
  production adapter lives in the **consumer's composition root**; StateChronicle v0
  uses an in-memory fake.
- **Shardline:** no integration. Events carry `content: { kind, digest, media_type,
  size }` as pure metadata; resolution and byte verification happen out-of-band in
  another consumer system.
- **Commit authority:** a `service:statechronicle...` subject authorized via the
  platform's own auth system (checked at the consumer's root); commit keys Ed25519,
  rotated
  through platform-authorized key transition procedures.

---

## 10. Testing Strategy

- **Core crates:** inline `#[cfg(test)]` unit tests for deterministic rules (transitions,
  canonicalization, conflict checks, accumulator); integration tests against in-memory fake
  ports.
- **Umbrella crate:** `crates/statechronicle/tests/e2e.rs` runs the full cross-crate
  lifecycle (mint → transfer → lock → proof → verify, plus tamper-fail-closed and
  non-membership) through real crates with in-memory port fakes.
- **Conformance/property:** property tests for canonicalization, event replay, proof
  verification, and amount arithmetic.
- **Tooling:** nextest, cargo-fuzz, cargo-deny.

---

## 11. Conventions (aligned with sibling workspaces shardline/trustgrant)

- **Crates:** `statechronicle-<concern>`; `crates/statechronicle` is the umbrella facade.
- **Ports:** output `<Noun>Repository`, `<Noun>Gateway`, `<Noun>Cache`,
  `EventPublisher`, `Outbox`, `TransactionManager`, and the dedicated port traits in
  `statechronicle-ports`. Async via `trait_variant::make(Send)`.
- **Adapters:** prefixed by implementation role, for example `Durable*`, `Redis*`,
  `InMemory*`, and `Ed25519*`,
  `Hmac*` (consumer-owned).
- **Domain types:** validated newtypes (macro-generated), prefixed IDs
  (`stc_…`, `evt_…`, `cmt_…`, `int_…`), never loose strings.
- **Stage documents:** `Raw → Validated → Verified` where the pipeline benefits
  (intent parsing, proof verification).
- **Errors:** `thiserror` per crate; `statechronicle-core`/`domain` carry the shared
  envelope + safety `limits` (mirrors trustgrant-error).
- **lib.rs / mod.rs:** declaration + re-export shells only (shardline rule).
- **Docs:** `docs/ARCHITECTURE.md`, `docs/DESIGN/ADR/*`, per-crate READMEs.

---

## 12. Open Items

The protocol's open questions (§36) are resolved: ADR-006 records the formal v0
decisions. Any future open items are tracked in the ADR set.

---

## 13. v0 Scope (protocol §35 minimal implementation)

- BCS canonicalization (binary; JSON retained as the API logical view), SHA-256
  digests, Ed25519 commit signatures
- Tenant-scoped intent deduplication, append-only events, signed commits,
  current-state projection
- `asset.mint` / `asset.transfer` / `asset.burn` / `asset.lock` / `asset.unlock`
- Paid unique asset no-hard-delete rule
- `stack.credit` / `stack.debit` / `stack.consume`
- `balance.credit` / `balance.debit` / `balance.transfer`
- State proof (current owner) + balance/quantity proofs
- Delegated-authority evaluation binding (port; in-memory fake for v0, consumer-wired
  adapter in production)
- Content digest references as event metadata (out-of-band; no content-store dependency)

The protocol must not require any particular backend; storage, HTTP, and authority
adapters are the consumer's.
