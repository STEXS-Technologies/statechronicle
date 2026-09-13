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
authorization system. Those are the consumer's job, supplied behind eleven small
trait interfaces (`statechronicle-ports`) and wired together at the consumer's
composition root. What you get instead is a deterministic, fully testable core;
durability and atomicity guarantees apply only when the consumer supplies an
adapter satisfying the durable `LedgerStore` contract.

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
- **Atomic settlement planning**: `execute_batch` and
  `execute_cross_tenant` validate multi-resource (and multi-tenant) work
  all-or-nothing in memory. To make the result durable, route the events
  through `statechronicle_commit::persist_durable_verified` and a
  `LedgerStore`; the legacy executor methods alone do not provide a durable
  commit boundary.
  Player-facing code should use `Executor::execute_player_durable` with a
  `DurableMutationSink`, which refuses success until the composition root has
  completed the verified durable write.
  Player-driven shared-inventory, marketplace, and settlement batches should
  use `Executor::execute_player_batch_durable_with_key_registry`, which binds
  a principal and registered key to every item. `execute_batch_durable` is for
  explicitly trusted service jobs; plain batch methods remain planning-only.
  Value-leg settlements have the equivalent `execute_settle_durable` route for
  trusted service jobs.
  Generic cross-tenant batches should use `Executor::execute_cross_tenant_durable`
  with the same single-database sink; the plain method remains planning-only.
  Cross-tenant trades use `execute_cross_tenant_trade_durable`; the sink must
  persist all tenant legs in one supported database transaction.
- **Delegated authority**: the executor checks, per operation, whether a
  delegated third party may act on a resource. The evaluator behind this check
  is a pluggable trait (TrustGrant is one option; your own policy engine is
  equally valid), and it is entirely separate from your platform's basic
  owner/actor authentication.

## Run the examples

The repository ships eleven runnable examples in
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
| `marketplace` | `cargo run -p statechronicle --example marketplace` | In-memory purchase-settlement planning via `execute_batch` (player ingress uses the authenticated durable batch route in production) |
| `cross_tenant` | `cargo run -p statechronicle --example cross_tenant` | In-memory cross-tenant validation via `execute_cross_tenant` (use `execute_cross_tenant_durable` in production) |
| `proofs` | `cargo run -p statechronicle --example proofs` | State, ownership, and non-membership proofs |
| `paid_asset` | `cargo run -p statechronicle --example paid_asset` | Paid unique asset overlay: owner consent and hard delete |
| `trade_value` | `cargo run -p statechronicle --example trade_value` | In-memory asset-for-gold settlement planning via `execute_settle` (use `execute_settle_durable` in production) |
| `trade_cross_tenant` | `cargo run -p statechronicle --example trade_cross_tenant` | In-memory cross-tenant trade planning via `execute_cross_tenant_trade` (use the durable variant in production) |
| `trade_bundle` | `cargo run -p statechronicle --example trade_bundle` | In-memory multi-asset bundle planning (persist through a durable batch sink in production) |

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
request), the intended DX is the fluent `Intent::builder()`, which sets only
the fields you care about and fills the rest with safe defaults:

```rust
use statechronicle::domain::intent::{Intent, Nonce, Operation};
use statechronicle::intent::validated::ValidatedIntent;

let intent = Intent::builder()
    .tenant(tenant_id)          // TenantId
    .intent_id(intent_id)       // IntentId
    .operation(operation)       // Operation
    .actor(actor)               // SubjectId
    .resource(resource_id)      // ResourceId
    .state_type(state_type)     // StateType
    .expected_version(expected_version) // u64 (defaults to 0)
    .input("to_owner", serde_json::json!("alice")) // append one input
    .created_at(now)            // DateTime<Utc>
    .nonce(nonce)               // Nonce
    .build()?;
let validated = ValidatedIntent::from_intent(intent, None); // typed in, no parsing
```

The positional constructor `Intent::new(...)` (twelve required fields) is also
available when you have every field at hand; the builder is recommended for
clarity and defaults.

A complete typed-path example lives in `crates/statechronicle/examples/currency.rs`.

**Raw wire bytes.** If you receive a payload over the wire, parse then
validate it:

```rust
use statechronicle::intent::parse::parse_intent;
use statechronicle::intent::validate::validate;

let raw = parse_intent(&bytes)?;      // cheap structural check + size limit
let validated = validate(&raw)?;      // schema, newtypes, expiry, signature
```

Both paths produce the same `ValidatedIntent` and feed the same executor. The
executor is a planning/validation layer; persistence is an explicit subsequent
step through the durable commit API.

## Example: a full lifecycle

The fastest way to see the whole pipeline wired is
`cargo run -p statechronicle --example inventory` (unique asset: mint →
transfer → lock → unlock → restrict → restore → burn, with fail-closed
rejections). For the tamper and non-membership proof variants, see the end-to-end
test in `crates/statechronicle/tests/e2e.rs` (run with `cargo test -p
statechronicle`). These examples demonstrate deterministic planning and commit
formation; production deployments must add authenticated ingress and the
durable adapter/recovery controls described in `TODO.md`.

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
| `statechronicle-ports` | Backend-agnostic storage, transaction, authorization, and delivery port traits |
| `statechronicle-sqlite` | SQLite durable ledger transaction adapter for single-database deployments |
| `statechronicle-postgres` | PostgreSQL `SERIALIZABLE` durable ledger adapter for server-database deployments |

Each crate carries a README with a "Protocol sections owned" table, so the
section numbers referenced throughout this workspace resolve to a concrete
owner.

## Authority model

StateChronicle separates two distinct concerns:

- **Platform basic authorization**: owner/actor identity and basic
  authorization must be supplied through `statechronicle-ports::authorization`
  and applied before the durable execution path. `DenyAllAuthorizer` is the
  safe default; the executor's legacy pure-planning API is not a public auth
  boundary.
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
| `LedgerStore` / `LedgerTransaction` | One durable mutation boundary for idempotency, events, signed commits, projections, and outbox |
| `Authorizer` | Authenticated principal binding and default-deny policy |
| `OutboxStore` | Lease-based post-commit delivery and retry |
| `EventPublisher` | Delivery of committed events and signed commits |
| `TradeIndex` | Keyed read access to accumulated trade records (`trade_id` → `TradeRecord`) |

Implement these traits against your storage, authority, and transport
backends (no implementations live inside the `statechronicle-ports` crate),
then wire them into `Executor::new` and `ProofService`. The composition root
(where port adapters, key resolution, the wall clock, and the event-id
generator are assembled) is owned by the consuming platform, not by
StateChronicle.

## Durable adapter

The workspace includes `statechronicle-sqlite`, a reference SQLite adapter for
the durable ledger contract. It enforces unique event/commit/idempotency keys,
canonical head continuity, monotonic projections, and transactional outbox
writes. Call `SqliteLedgerStore::verify_all_integrity` during startup/restore
and block writes on any reported chain or orphan-event violation. Use a server
database adapter for horizontally scaled writers; do not use the legacy
non-transactional `persist` API for valuable mutations.
`SqliteLedgerStore::open_verified` combines opening and the fail-closed scan
for startup code that must not expose a writable store before verification.
Use `persist_durable_verified` when the composition root has a commit
signature/KMS verifier; `Ed25519CommitVerifier` adapts a tenant-scoped key
resolver and verifies trust before reserving idempotency state.
For horizontally scaled writers, use `statechronicle-postgres` with the
reviewed PostgreSQL schema and migration process; it provides `SERIALIZABLE`
transactions, deterministic multi-tenant head locking, and canonical-head
locking. Cross-tenant settlement is safe only when all tenant rows share this
single database transaction and the caller validates a tenant/commit manifest;
independent databases are not atomic.
Use `PostgresLedgerStore::new_verified` to require an all-tenant integrity scan
before exposing the store to mutation traffic.
TLS deployments can use `new_with_tls_verified` for the same fail-closed gate.
Enable the adapter's `tls` feature and use `PostgresLedgerStore::new_with_tls`
with a reviewed CA file when database traffic crosses a trust boundary.
The required relational schema, locking order, isolation, and fault-injection
test contract for that adapter is documented in
[the relational adapter contract](docs/OPERATIONS/RELATIONAL_ADAPTER.md).
Run `scripts/run_postgres_integration.sh` for a reproducible local PostgreSQL
16 integration gate; it removes its temporary container on exit.
Proof endpoints should use `ProofService::verify_canonical` (or
`verify_with_key_canonical`) so verification fails closed when the requested
commit is not the tenant's current canonical head.

The `statechronicle_ports::outbox::dispatch_once` helper supplies a bounded
worker pass: it claims leased rows, verifies payload digests, publishes through
the consumer's broker adapter, and releases failed deliveries for retry.
Operators should follow [the recovery runbook](docs/OPERATIONS/RECOVERY.md)
for startup integrity checks, restore promotion, projection rebuilds, outbox
incidents, and signer compromise.

For derived read models, `statechronicle_index::rebuild_projections` replays a
verified `(event, commit_id)` stream, selects the latest version per scoped
resource, rejects conflicting equal versions, and writes through an injected
projection sink.
Large histories can use `rebuild_projections_chunk` and persist its returned
checkpoint between bounded passes.
The SQLite adapter additionally exposes `canonical_events` and
`rebuild_projections_from_canonical`, which verify the tenant chain before
replaying its durable event stream with persisted checkpoints. Operator
checkpoint keys are tenant-scoped; clear them with
`clear_rebuild_checkpoint_for_tenant` after promotion.

## What's not included

StateChronicle ships no HTTP server, object store, queue worker, or authority
policy implementation. Those concerns remain the consumer's, supplied through
the ports and wired at the composition root.

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

The workspace is fully test-locked (workspace test suite; check/test/clippy/fmt gates),
and every protocol decision is recorded in `docs/DESIGN/ADR/`, with ADR-006
resolving the open protocol questions.
Run `scripts/run_release_checks.sh` to execute the finite workspace, security,
dependency, and bounded fuzz gates in one reproducible command.

## Where to go next

- `crates/statechronicle/examples/`: the eleven runnable examples (start with
  `inventory`, then `currency` and `cross_tenant`).
- `crates/statechronicle/tests/e2e.rs`: the end-to-end lifecycle test with
  tamper and non-membership proof variants.
- `crates/statechronicle/README.md`: the umbrella crate and the full surface.
- `crates/statechronicle-ports/README.md`: the port traits and the authority
  model.
- `docs/ARCHITECTURE.md`: how the crates fit together.
- `docs/DESIGN/ADR/README.md`: the architecture decision record index.
