# StateChronicle

## What it is

StateChronicle is a pure-logic, verifiable resource-state ledger protocol
engine: append-only events, signed commits, deterministic state transitions,
sparse-Merkle state roots, and portable proofs that anyone can verify. It is
the "brain" of a state protocol — it ships no storage, HTTP, or authority
implementation. Those are the consumer's, supplied behind the
`statechronicle-ports` traits and wired in at the composition root. The engine
stays deterministic and testable because every side effect is an injected port.

## The flow

1. **Submit** — a client submits a raw intent document describing the requested
   transition.
2. **Parse + validate** (`statechronicle-intent`) — the raw payload is parsed
   and validated into a `ValidatedIntent` with a canonical body, idempotency
   key, and optional detached signature.
3. **Execute through ports** (`statechronicle-executor`) — the §18.1 pipeline
   runs the intent through conflict gates, expected-version checks, TrustGrant
   authority evaluation with multi-authority aggregation, and profile rules,
   producing a deterministic after-state and the emitted event.
4. **Form + sign commits** (`statechronicle-commit`) — events are batched into
   commits, event/state Merkle roots are computed, and the commit body is signed
   with an Ed25519 commit key.
5. **Serve / verify proofs** (`statechronicle-proof`) — portable state,
   ownership, and inclusion proofs, including non-membership proofs, are served
   from committed state and verified against the signed commit chain.

## Crate map

| Crate | Role |
|---|---|
| `statechronicle` | Umbrella crate: namespaced re-exports + curated facade |
| `statechronicle-core` | Primitives: amounts, digests, signatures, limits, canonicalization |
| `statechronicle-domain` | Canonical protocol objects: tenants, intents, events, commits, proofs |
| `statechronicle-intent` | Intent parsing and validation into `ValidatedIntent` |
| `statechronicle-executor` | The §18.1 execution pipeline through injected ports |
| `statechronicle-commit` | Commit formation, ordering, roots, and signing |
| `statechronicle-accumulator` | Sparse-Merkle state accumulator and state roots |
| `statechronicle-proof` | Proof serving and verification (incl. non-membership) |
| `statechronicle-profiles` | Baseline resource profiles and their rule sets |
| `statechronicle-ports` | The ten backend-agnostic port traits consumers implement |

## Minimal consumption sketch

```rust
use statechronicle::{
    domain::intent::{Intent, Operation},
    core::amount::Amount,
    domain::signed::Signed,
};

// Build an intent, wrap it in a Signed envelope, then run it through
// executor::Executor (which needs your port adapters), form a Commit, sign it,
// and serve proofs via proof::ProofService. The storage / authority / transport
// behind the ports is yours — see "Implementing the ports".
```

This is illustrative, not runnable: the full wiring (port adapters, key
resolution, composition root) is platform-owned.

## Implementing the ports

| Port trait | What the consumer must provide |
|---|---|
| `IntentStore` | Dedup + idempotency storage for intents |
| `EventStore` | Append-only storage of validated events |
| `CommitStore` | Storage of signed commits (and snapshots) |
| `StateIndex` | Read access to current derived state projections |
| `ProofIndex` | Storage/query of served state, ownership, and inclusion proofs |
| `SnapshotStore` | Storage of opaque snapshot payloads |
| `TenantStore` | Tenant scope existence resolution |
| `TrustGrantEvaluator` | TrustGrant authority evaluation and freshness checks |
| `TransactionManager` | Atomic multi-store transaction coordination |
| `EventPublisher` | Delivery of committed events and signed commits |

Implement these traits against your storage, authority, and transport backends
(no implementations live inside the `statechronicle-ports` crate), then wire
them into `Executor::new` and `ProofService`. The composition root — where
port adapters, key resolution, the wall clock, and the event-id generator are
assembled — is owned by the platform (e.g. stexs), not by StateChronicle.

## Verification

The workspace is fully test-locked (599 tests; check/test/clippy/fmt/fuzz
gates), and every protocol decision is recorded in `docs/DESIGN/ADR/`, with
ADR-006 resolving the open protocol questions.

## Where to go next

- `STATECHRONICLE_PROTOCOL_UPDATED.md` — the protocol changelog and state.
- `docs/ARCHITECTURE.md` — how the crates fit together.
- `docs/DESIGN/ADR/README.md` — the architecture decision record index.
