# StateChronicle

## What is this?

StateChronicle is a library for recording **who owns what, who changed it, and
why you can trust the record**. It is the kind of ledger you build game
inventory, marketplaces, in-game currency, licenses, and escrow flows on top
of: every change is appended (never overwritten), batched into a signed
commit, and given a cryptographic state root, so anyone can replay the whole
history and verify that the current state is exactly what the recorded events
produce.

It is a *pure-logic* engine: it ships no database, no HTTP server, and no
authorization system. Those are the consumer's job, supplied behind ten small
trait interfaces (`statechronicle-ports`) and wired together at the consumer's
composition root. What you get instead is a deterministic, fully testable core
that cannot silently diverge, lose a transaction, or round a balance.

## Use cases

- **Game inventory**: mint, transfer, lock, and burn unique assets (swords,
  skins, collectibles) with a provable ownership history that survives
  database restores, because state is recomputed from events, not stored.
- **Marketplaces**: list, buy, and escrow assets; prove a seller owns what
  they listed and a buyer can afford what they are buying.
- **In-game currency**: fungible balances (gold, credits, gems) with exact
  fixed-point arithmetic, so balances never drift due to floating-point
  rounding and debits never exceed available funds.
- **Entitlements and licenses**: grant, activate, suspend, and revoke access
  rights, with a history of who held what and when.
- **Paid items and ownership protection**: assets that were sold are protected
  from silent deletion or creator overreach; loss of utility (quarantine,
  legal hold) is distinct from loss of ownership.
- **Audits and proofs**: produce a compact proof that a player currently owns
  an item, or that an item never existed, verifiable against the signed commit
  chain without replaying all history.

## What you get (capabilities)

- **Append-only, signed history**: events are never edited or deleted; commits
  are Ed25519-signed, so the record is tamper-evident.
- **Deterministic replay**: the state root after any commit is a pure function
  of the events in that commit, so replay from genesis always reproduces the
  same state. There is no hidden mutable state.
- **Exact money math**: amounts are fixed-point integers (u128 mantissa with an
  explicit scale), and floating-point values are structurally impossible in
  the wire format. No rounding drift, no float bugs.
- **Fail-closed transitions**: optimistic concurrency (expected version),
  conflict rules, and per-profile invariants (no negative balances, no debit
  over available funds, transfers are atomic debit + credit) reject invalid
  changes before they are recorded.
- **Portable proofs**: state, ownership, and non-membership proofs anyone can
  verify against a signed commit, without the full history (a balance is proven
  as a state proof over a balance projection).
- **Atomic settlement**: `execute_batch` and `execute_cross_tenant` commit
  multi-resource (and multi-tenant) transactions all-or-nothing, so a purchase
  that spans an asset, a listing, an escrow, and two wallets either lands whole
  or not at all (protocol §18.3).
- **Delegated authority**: the executor checks, per operation, whether a
  delegated third party may act on a resource. The evaluator behind this check
  is a pluggable trait (TrustGrant is one option; your own policy engine is
  equally valid), and it is entirely separate from your platform's basic
  owner/actor authentication.

## Run the examples

The repository ships eight runnable examples in
`crates/statechronicle/examples/`. Each runs the real cross-crate pipeline
(submit → execute → commit → proof → verify) over in-memory port fakes, prints
a short narrative, asserts its outcome, and exits 0 only on success. Runs are
deterministic: every example uses a fixed clock, a fixed Ed25519 key, and a
counter-based event-id generator, so the same example produces the same output
every time.

| Example | Run it with | Capability demonstrated |
|---|---|---|
| `inventory` | `cargo run -p statechronicle --example inventory` | Unique asset lifecycle (mint → transfer → lock → unlock → restrict → restore → burn) with fail-closed rejections |
| `currency` | `cargo run -p statechronicle --example currency` | Fungible balance lifecycle with an atomic debit + credit transfer and exact amount math |
| `stack` | `cargo run -p statechronicle --example stack` | Consumable stack lifecycle |
| `access` | `cargo run -p statechronicle --example access` | Entitlement and meter lifecycles |
| `marketplace` | `cargo run -p statechronicle --example marketplace` | Atomic purchase settlement via `execute_batch` |
| `cross_tenant` | `cargo run -p statechronicle --example cross_tenant` | Cross-tenant atomic transaction via `execute_cross_tenant` |
| `proofs` | `cargo run -p statechronicle --example proofs` | State, ownership, and non-membership proofs |
| `paid_asset` | `cargo run -p statechronicle --example paid_asset` | Paid unique asset overlay: owner consent and hard delete |

The examples construct validated intents both ways: the typed path
(`Intent::new` → `ValidatedIntent::from_intent`) is the workhorse across most
examples, and the raw-wire path (`parse_intent` → `validate`, as if a payload
arrived over the wire) appears in `currency.rs` as an explicit callout and in
the intent section below.

## The flow

1. **Submit**: a transition request (intent) arrives, either as raw bytes or as
   already-typed data.
2. **Validate** (`statechronicle-intent`): turn it into a validated intent with
   a canonical body, an idempotency key, and an optional signature. You can
   skip parsing entirely if your data is already typed.
3. **Execute** (`statechronicle-executor`): the intent runs through the
   validation pipeline (conflict gates, version checks, delegated-authority
   evaluation, profile rules), producing a deterministic after-state and one
   or more events.
4. **Commit** (`statechronicle-commit`): events are batched, event and state
   Merkle roots are computed, and the commit is signed.
5. **Prove** (`statechronicle-proof`): state, ownership, and non-membership
   proofs are served from committed state and verified against the signed
   commit chain (a balance is proven as a state proof over a balance
   projection).

## Two ways to construct a validated intent

StateChronicle works with whatever shape your data is already in.

**Already-typed data (no parsing).** If your platform builds the `Intent`
itself (for example, a handler that already deserialized and validated the
request), construct the validated intent directly:

```rust
use statechronicle::domain::intent::{Intent, Operation, Nonce};
use statechronicle::intent::validated::ValidatedIntent;

let intent = Intent::new(
    tenant_id,            // TenantId
    intent_id,            // IntentId
    operation,            // Operation
    actor,                // SubjectId
    resource_id,          // ResourceId
    Some(state_type),     // Option<StateType>
    expected_version,     // u64
    inputs,               // BTreeMap<String, serde_json::Value>
    authority,            // Option<AuthorityProof>
    created_at,           // DateTime<Utc>
    expires_at,           // Option<DateTime<Utc>>
    nonce,                // Nonce
);
let validated = ValidatedIntent::from_intent(intent, None); // typed in, no parsing
```

A complete typed-path example lives in `crates/statechronicle/examples/currency.rs`.

**Raw wire bytes.** If you receive a payload over the wire, parse then
validate it:

```rust
use statechronicle::intent::parse::parse_intent;
use statechronicle::intent::validate::validate;

let raw = parse_intent(&bytes)?;      // cheap structural check + size limit
let validated = validate(&raw)?;      // schema, newtypes, expiry, signature
```

Both paths produce the same `ValidatedIntent` and feed the same executor.

## Example: a full lifecycle

The fastest way to see the whole pipeline wired is
`cargo run -p statechronicle --example inventory` (unique asset: mint →
transfer → lock → unlock → restrict → restore → burn, with fail-closed
rejections). For the tamper and non-membership proof variants, see the end-to-end
test in `crates/statechronicle/tests/e2e.rs` (run with `cargo test -p
statechronicle`). Both build the signed commit + state accumulator exactly as a
production composition root would.

## Crate map

| Crate | Role |
|---|---|
| `statechronicle` | Umbrella crate: namespaced re-exports + curated facade |
| `statechronicle-core` | Primitives: fixed-point amounts, digests, signatures, limits |
| `statechronicle-domain` | Canonical protocol objects: tenants, intents, events, commits, proofs |
| `statechronicle-intent` | Intent construction and validation (typed or raw) |
| `statechronicle-executor` | The validation pipeline through injected ports |
| `statechronicle-commit` | Commit formation, ordering, roots, and signing |
| `statechronicle-accumulator` | Sparse-Merkle state accumulator and state roots |
| `statechronicle-proof` | Proof serving and verification (incl. non-membership) |
| `statechronicle-profiles` | Baseline resource profiles and their rule sets |
| `statechronicle-ports` | The ten backend-agnostic port traits consumers implement |

Each crate carries a README with a "Protocol sections owned" table, so the
section numbers referenced throughout this workspace resolve to a concrete
owner.

## Authority model

StateChronicle separates two distinct concerns:

- **Platform basic authorization**: owner/actor identity and basic
  authorization are your platform's own auth system, applied before (or
  alongside) the execution pipeline. StateChronicle does not implement general
  authorization.
- **Delegated-authority evaluation**: the `TrustGrantEvaluator` port (in
  `statechronicle-ports`) is a **delegation-of-authority boundary**, not a
  general auth system. It is trait-only and dependency-free by construction: it
  references only `statechronicle-domain` types, so it is not coupled to any
  authority provider. The executor calls the port during execution and fails
  closed unless the evaluation is `allow` and fresh. Any evaluator that returns
  an `allow` result and passes the freshness check can be plugged in; TrustGrant
  is one option, not a requirement.

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

Implement these traits against your storage, authority, and transport
backends (no implementations live inside the `statechronicle-ports` crate),
then wire them into `Executor::new` and `ProofService`. The composition root
(where port adapters, key resolution, the wall clock, and the event-id
generator are assembled) is owned by the consuming platform, not by
StateChronicle.

## What's not included

StateChronicle ships no HTTP server, no database, no object store, no queue,
and no authority implementation. It is a pure-logic engine. Any such concerns
are the consumer's, supplied through the `statechronicle-ports` traits and
wired at the composition root.

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
| §28 | API Surface | `crates/statechronicle/README.md` |
| §29 | Verification Algorithm | `crates/statechronicle-proof/README.md` |
| §31 | Forks and Recovery | `crates/statechronicle-commit/README.md` |
| §33 | Example Full Stack Flow | `crates/statechronicle/README.md` |
| §37 | Glossary | `crates/statechronicle/README.md` |

## Verification

The workspace is fully test-locked (608 tests; check/test/clippy/fmt gates),
and every protocol decision is recorded in `docs/DESIGN/ADR/`, with ADR-006
resolving the open protocol questions.

## Where to go next

- `crates/statechronicle/examples/`: the eight runnable examples (start with
  `inventory`, then `currency` and `cross_tenant`).
- `crates/statechronicle/tests/e2e.rs`: the end-to-end lifecycle test with
  tamper and non-membership proof variants.
- `crates/statechronicle/README.md`: the umbrella crate and the full surface.
- `crates/statechronicle-ports/README.md`: the port traits and the authority
  model.
- `docs/ARCHITECTURE.md`: how the crates fit together.
- `docs/DESIGN/ADR/README.md`: the architecture decision record index.
