# StateChronicle

## What it is

StateChronicle is a pure-logic, verifiable resource-state ledger protocol
engine: append-only events, signed commits, deterministic state transitions,
sparse-Merkle state roots, and portable proofs that anyone can verify. It is
the "brain" of a state protocol: it ships no storage, HTTP, or authority
implementation. Those are the consumer's, supplied behind the
`statechronicle-ports` traits and wired in at the composition root. The engine
stays deterministic and testable because every side effect is an injected port.

## The flow

1. **Submit**: a client submits a raw intent document describing the requested
   transition.
2. **Parse + validate** (`statechronicle-intent`): the raw payload is parsed
   and validated into a `ValidatedIntent` with a canonical body, idempotency
   key, and optional detached signature.
3. **Execute through ports** (`statechronicle-executor`): the §18.1 pipeline
   runs the intent through conflict gates, expected-version checks,
   delegated-authority evaluation with multi-authority aggregation, and profile
   rules, producing a deterministic after-state and the emitted event.
4. **Form + sign commits** (`statechronicle-commit`): events are batched into
   commits, event/state Merkle roots are computed, and the commit body is signed
   with an Ed25519 commit key.
5. **Serve / verify proofs** (`statechronicle-proof`): portable state,
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

Each crate carries a README with a "Protocol sections owned" table, so the
section numbers referenced throughout this workspace resolve to a concrete
owner even without a monolithic protocol document.

## Minimal consumption sketch

The following is a real, runnable lifecycle: parse and validate an
`asset.transfer` intent, sign its canonical body, run it through an `Executor`
wired to in-memory port fakes, form and sign the enclosing commit, and verify
a state proof end to end. It mirrors `crates/statechronicle/tests/e2e.rs` and
the fakes in `crates/statechronicle/tests/common/mod.rs`; every symbol below
exists in the public API.

```rust
use std::collections::BTreeMap;

use serde_json::{Value, json};

use statechronicle::accumulator::sparse_merkle::StateAccumulator;
use statechronicle::commit::builder::CommitBuilder;
use statechronicle::commit::sign::{sign_commit, verify_commit};
use statechronicle::core::canonicalize::canonicalize;
use statechronicle::core::digest::ContentDigest;
use statechronicle::core::signature::sign;
use statechronicle::domain::commit::CommitScope;
use statechronicle::domain::intent::{INTENT_SCHEMA, Operation, SignatureAlg, SignatureBlock};
use statechronicle::domain::ids::CommitId;
use statechronicle::domain::signed::Signed;
use statechronicle::domain::state_type::StateType;
use statechronicle::executor::pipeline::{Executor, Ports};
use statechronicle::intent::parse::parse_intent;
use statechronicle::intent::validate::validate;
use statechronicle::profiles::registry::ProfileRegistry;
use statechronicle::proof::verify::verify_bundle;

// A canonical raw-intent payload (protocol §11.1).
let payload = json!({
    "schema": INTENT_SCHEMA,
    "tenant_id": "stexs.game.alpha",
    "intent_id": "int_transfer_001",
    "operation": "asset.transfer",
    "actor": "account:stexs:player_123",
    "resource_id": "asset:sword_001",
    "state_type": "unique_asset",
    "expected_version": 0,
    "inputs": { "from_owner": "account:stexs:player_123",
                "to_owner": "account:stexs:player_456" },
    "created_at": "2026-07-14T00:00:00Z",
    "expires_at": "2026-07-14T00:05:00Z",
    "nonce": "b64u:AAME",
});

// 1. Parse + validate into a ValidatedIntent.
let raw = parse_intent(&serde_json::to_vec(&payload).unwrap()).unwrap();
let mut validated = validate(&raw).unwrap();

// 2. Sign the canonical intent body with your Ed25519 key (composition root).
let body_bytes = canonicalize(&validated.intent).unwrap();
validated.signature = Some(SignatureBlock {
    alg: SignatureAlg::Ed25519,
    key_id: key_id(),                       // a KeyId you resolve to your key
    sig: sign(&body_bytes, &fixed_key()),   // ed25519-dalek SigningKey
});

// 3. Execute through an Executor wired to your port adapters (in-memory fakes
//    here). Ports bundles intent store, state index, tenant store, one or more
//    delegated-authority evaluators, and a transaction manager.
let executor = Executor::new(
    Ports::new(intent_store, state_index, tenant_store, authority_ports, tx_manager),
    ProfileRegistry::baseline(),
    executor_subject(),
    Box::new(fixed_now),
    Box::new(event_id_gen),
    intent_verifier,                        // resolves key_id -> verifying key
);
let events = executor.execute(&validated).await.unwrap();

// 4. Form the commit and sign it. The next state root is a pure function of
//    the emitted events' after-state set.
let batch = /* ordered CommitBatch from the events */;
let commit = CommitBuilder::new(CommitScope::tenant(tenant()), 1,
    executor_subject(), profile(), now, None)
    .build(&batch, previous_root, &[], commit_id)
    .unwrap();
let signed: Signed<Commit> = sign_commit(&commit, &fixed_key(), key_id()).unwrap();
verify_commit(&signed, &fixed_key().verifying_key()).unwrap();

// 5. Build a resource-state proof and verify it against the signed commit.
let inclusion = accumulator.prove_inclusion(&key).unwrap();
let proof = build_state_proof(&projection, &signed, &inclusion, &op, None, key).unwrap();
assert!(verify_bundle(&proof, &signed, &fixed_key().verifying_key(), &key).is_ok());
```

The storage / authority / transport behind the ports is yours; the composition
root (where port adapters, key resolution, the wall clock, and the event-id
generator are assembled) is platform-owned. Run the full lifecycle (mint,
transfer, lock, tamper-fail-closed, non-membership) with
`cargo test -p statechronicle`.

## Authority model

StateChronicle separates two distinct concerns:

- **Platform basic authorization**: owner/actor identity and basic authorization
  are the platform's own auth system, applied before (or alongside) the
  execution pipeline. StateChronicle does not implement general authorization.
- **Delegated-authority evaluation**: the `TrustGrantEvaluator` port (in
  `statechronicle-ports`) is a **delegation-of-authority boundary**, not a
  general auth system. It is `trait-only and dependency-free by construction`:
  it references only `statechronicle-domain` types, so it is not coupled to any
  authority provider. The executor calls the port during §18.1 and fails closed
  unless the evaluation is `allow` and fresh. Any evaluator that returns an
  `allow` result and passes the freshness check can be plugged in; TrustGrant
  is **one option, not a requirement**.

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
| `TrustGrantEvaluator` | Delegated-authority evaluation and freshness checks (trait-only; TrustGrant is one option) |
| `TransactionManager` | Atomic multi-store transaction coordination |
| `EventPublisher` | Delivery of committed events and signed commits |

Implement these traits against your storage, authority, and transport backends
(no implementations live inside the `statechronicle-ports` crate), then wire
them into `Executor::new` and `ProofService`. The composition root (where
port adapters, key resolution, the wall clock, and the event-id generator are
assembled) is owned by the platform (e.g. stexs), not by StateChronicle.

## What's not included

StateChronicle ships no HTTP server, no database, no object store, no queue, and
no authority implementation. It is a pure-logic engine. Any such concerns are
the consumer's, supplied through the `statechronicle-ports` traits and wired at
the composition root. There is no `statechronicle-http`, `statechronicle-shared`,
`statechronicle-shared-http`, `slices/`, or `migrations/` directory in this
workspace; consumers who want those own them.

## Protocol section index

Section numbers are load-bearing across this workspace (crate docs, ADRs,
tests). Each crate README carries a "Protocol sections owned" table; the index
below maps every section to its owning crate README.

| § | Title | Owner README |
|---|---|---|
| §1 | Summary | `crates/statechronicle/README.md` |
| §5–§9 | Conceptual, Resource, Subject, Tenant, State | `crates/statechronicle-domain/README.md` |
| §10 | Resource State Types | `crates/statechronicle-domain/README.md` |
| §11 | Intent Model | `crates/statechronicle-intent/README.md` |
| §12 | Event Model | `crates/statechronicle-domain/README.md` |
| §13 | Commit Model | `crates/statechronicle-commit/README.md` |
| §14 | State Root Model | `crates/statechronicle-commit/README.md`, `crates/statechronicle-accumulator/README.md` |
| §16 | Proof Model | `crates/statechronicle-proof/README.md` |
| §17 | Canonicalization and Hashing | `crates/statechronicle-core/README.md` |
| §18 | Execution Semantics | `crates/statechronicle-executor/README.md` |
| §19 | Commit Authority | `crates/statechronicle-executor/README.md`, `crates/statechronicle-commit/README.md` |
| §20 | Profiles | `crates/statechronicle-profiles/README.md` |
| §27 | Infra-Agnostic Storage Contract | `crates/statechronicle-ports/README.md` |
| §29 | Verification Algorithm | `crates/statechronicle-proof/README.md` |
| §31 | Forks and Recovery | `crates/statechronicle-commit/README.md` |

## Verification

The workspace is fully test-locked (607 tests; check/test/clippy/fmt gates),
and every protocol decision is recorded in `docs/DESIGN/ADR/`, with ADR-006
resolving the open protocol questions.

## Where to go next

- `crates/statechronicle/README.md`: the umbrella crate and the full surface.
- `crates/statechronicle-ports/README.md`: the port traits and the authority
  model.
- `docs/ARCHITECTURE.md`: how the crates fit together.
- `docs/DESIGN/ADR/README.md`: the architecture decision record index.
