# StateChronicle

## What is this?

StateChronicle is a domain-extensible Rust library for **controlled state changes,
signed history, deterministic replay, and verifiable state**. Define your own
configuration, equipment, or application state and legal commands, or use the
built-in inventory, currency, access, marketplace, and escrow profiles. Accepted
changes become append-only events and signed commits with cryptographic roots.
Consumers can reconstruct state from history and verify committed claims under
their chosen signer and authority policies.

It is a *pure-logic* engine: it ships no database, no HTTP server, and no
authorization system. Those are the consumer's job, supplied through
trait interfaces (`statechronicle-ports`) and wired together at the consumer's
composition root. What you get instead is a deterministic, fully testable core;
durability and atomicity guarantees apply only when the consumer supplies an
adapter satisfying the durable `LedgerStore` contract.

## Define a model, commit a mutation, verify a claim

This branch contains the proposed coherence refactor. Workspace package versions
remain `0.2.0`; the APIs below are checkout APIs, not an announcement of a
published `0.3.0`. Library release qualification is complete; see the
[exact-candidate audit](docs/0.3/evidence/qualification/final-94eb5d4/README.md).

1. **Define your model.** Implement `StateProfile` with native state/command types,
   deterministic validation/reduction, domain identity and schema version.
   `TypedProfile` supplies hosted encoding and registration;
   `command_operations!` keeps operation mapping in one place. Start with the
   [standalone native-domain tutorial](tutorials/minimal-native-domain/README.md).
2. **Make an authenticated durable mutation.** Use `mutation::Preparer` with
   read-only state/tenant ports, independent authentication, key verification and
   default-deny policy. Build and sign a `mutation::SignedMutation`, then call its
   `commit` against a qualified `LedgerStore` with `LedgerRecovery`. Only checked
   `Committed`/`Replay` receipts establish success. Preserve the original identity
   and resolve `Unknown` with `mutation::recover` before any retry decision.
   Follow the [durable custom-ledger tutorial](tutorials/durable-custom-ledger/README.md).
3. **Verify the requested claim.** Use `proof::claim::ClaimVerifier` with an
   independent `ExpectedClaim` and trusted signer key. `verify_current` requires
   the authoritative canonical head; `verify_historical` requires an independent
   commit pin and asserts no freshness. A valid signature alone proves neither
   current state nor authorization. See [proof claims](docs/0.3/proof-claims.md).

With Rust 1.100 or newer, run from this checkout:

```sh
cargo run --manifest-path tutorials/minimal-native-domain/Cargo.toml --locked
cargo run --manifest-path tutorials/durable-custom-ledger/Cargo.toml --locked
```

Both applications have their own manifests and lockfiles and use the umbrella
crate without workspace test helpers. Their fixed development credentials,
clock and identity generators require replacement for external requests. The
second example writes a bounded temporary file journal and demonstrates reopen;
it does not qualify a production database or hardware crash durability.

The [consumer API map](docs/0.3/consumer-guide.md),
[migration guide](docs/0.3/migration.md) and
[recovery decisions](docs/OPERATIONS/RECOVERY.md) describe the boundaries.
Existing APIs remain available for compatibility and specialized composition.
The portable `StateProfile` contract is allocation-free and `no_std`;
`portable::` supplies bounded canonical transitions and integrity verification.
The hosted adapter and full engine require `std`.

## Use cases

- **Configuration lifecycles**: define revisioned configuration, permitted
  updates and authority requirements, with replayable changes and typed proofs.
- **Equipment-state tracking**: define operational modes, legal transitions and
  counters without ownership or financial fields. Recorded observations remain
  the consumer's responsibility.
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
  an item, or that a key is absent at a committed root, verifiable against the signed commit
  chain without replaying all history.

## What you get (capabilities)

- **Native custom domains**: consumer-owned state, command and error types;
  static profile registration; domain/schema isolation; canonical command binding;
  loaded/output state validation; and domain-aware replay and proof verification.
- **Append-only, signed history**: events are never edited or deleted; commits
  are Ed25519-signed, so the record is tamper-evident.
- **Deterministic replay**: applying committed events to the preceding state
  reconstructs projections and roots. Custom transition verification recomputes
  domain reduction; consumer reducers and serializers must obey the deterministic
  contract.
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
- **Atomic settlement planning**: legacy batch and cross-tenant execution
  validate complete plans. Protected single-tenant player batches use
  `Preparer::prepare_player_batch` → `mutation::SignedBatch` with an adapter
  supporting complete batch retention and recovery. Cross-tenant settlement
  remains specialized advanced composition: every leg must share one supported
  atomic ledger transaction. Unsupported topology must refuse without partial
  effects. See [batch contracts](docs/0.3/durable-batches.md) and the
  [consumer API map](docs/0.3/consumer-guide.md).
- **Delegated authority**: the executor checks, per operation, whether a
  delegated third party may act on a resource. The evaluator behind this check
  is a pluggable trait (TrustGrant is one option; your own policy engine is
  equally valid), and it is entirely separate from your platform's basic
  owner/actor authentication.

## Run the examples

The repository ships fifteen runnable examples: twelve behavioral demonstrations
and three performance probes. The lifecycle and settlement examples exercise
real library APIs over in-memory adapters with fixed keys, clocks and event-ID
generators. Their symbolic transactions demonstrate planning. Authenticated
durable applications should use `Preparer` and `SignedMutation`/`SignedBatch`
with one qualified `LedgerStore`, as shown in the standalone tutorials. Legacy
durable sinks require consumer-enforced atomicity and original-identity recovery;
their event-vector results do not establish the protected receipt contract.
`custom_domains` demonstrates
registration and direct reduction, while `tests/custom_domains.rs` exercises the
full signed execution, durable persistence, replay and proof paths for both
configuration and equipment domains. Performance timings depend on the machine.

| Example | Run it with | Capability demonstrated |
|---|---|---|
| `inventory` | `cargo run -p statechronicle --example inventory` | Unique asset lifecycle (mint → transfer → lock → unlock → restrict → restore → burn) with fail-closed rejections |
| `currency` | `cargo run -p statechronicle --example currency` | Fungible balance lifecycle with coordinated debit/credit planning and exact amount math |
| `stack` | `cargo run -p statechronicle --example stack` | Consumable stack lifecycle |
| `access` | `cargo run -p statechronicle --example access` | Entitlement and meter lifecycles |
| `marketplace` | `cargo run -p statechronicle --example marketplace` | In-memory purchase planning via `execute_batch`; see [protected groups](docs/0.3/durable-batches.md) for durable composition |
| `cross_tenant` | `cargo run -p statechronicle --example cross_tenant` | Advanced cross-tenant planning; persistence requires consumer-qualified multi-tenant atomic coordination |
| `proofs` | `cargo run -p statechronicle --example proofs` | State, ownership, and non-membership proofs |
| `paid_asset` | `cargo run -p statechronicle --example paid_asset` | Paid unique asset overlay: owner consent and hard delete |
| `trade_value` | `cargo run -p statechronicle --example trade_value` | In-memory asset-for-gold planning; see [protected single-tenant settlement](docs/0.3/batch-preparation.md#protected-settlement-preparation) |
| `trade_cross_tenant` | `cargo run -p statechronicle --example trade_cross_tenant` | Advanced cross-tenant trade planning; persistence requires consumer-qualified multi-tenant atomic coordination |
| `trade_bundle` | `cargo run -p statechronicle --example trade_bundle` | In-memory bundle planning; [protected settlement](docs/0.3/batch-preparation.md#protected-settlement-preparation) validates the complete single-tenant group |
| `custom_domains` | `cargo run -p statechronicle --example custom_domains` | Register configuration/equipment profiles, create a configuration state and decode it through the typed adapter |
| `throughput` | `cargo run --release -p statechronicle --example throughput` | Direct reduction and canonical hashing; excludes authentication and storage |
| `e2e_throughput` | `cargo run --release -p statechronicle --example e2e_throughput` | Signed intent execution and in-memory indexing; optional commit formation/verification and batching |
| `parallel_throughput` | `cargo run --release -p statechronicle --example parallel_throughput` | Independent workers with separate executors/adapters; measures partitionable workload scaling |

The examples construct validated intents both ways: the typed path
(`Intent::new` → `ValidatedIntent::from_intent`) is the workhorse across most
examples, and the raw-wire path (`parse_intent` → `validate`, as if a payload
arrived over the wire) appears in `currency.rs` as an explicit callout and in
the intent section below.

## The flow

1. Construct or parse an intent, retaining its original identity and body.
   Structural validation does not authenticate the actor or verify a signature.
2. Authenticate the host session independently. Protected preparation verifies
   signed ingress, principal/key/policy scope, expected state and domain rules.
   The resulting event plan is uncommitted.
3. Build and sign a commit plan from the complete events and independently
   obtained state/history context. Signing does not make that context authoritative.
4. Commit through one qualified ledger transaction. It atomically binds
   idempotency, events, signed commit, head, projections and outbox; the protected
   API checks retained original readback before returning a receipt.
5. Resolve unknown acknowledgements by original-identity recovery. Dispatch from
   the post-commit outbox with delivery-key deduplication.
6. Verify proofs against independently expected tenant/resource/holder and
   signer trust. Choose current-head or independently pinned historical semantics.

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
let validated = validate(&raw)?;      // structural fields and signature-block parsing
```

Both paths produce `ValidatedIntent`. This is structural validation, not an
authenticated principal or cryptographic signature verdict. The protected
preparer rechecks structure, key policy, signature and authorization using the
injected trusted services. Durable success requires the subsequent ledger path.

## Example: a full lifecycle

A specialized legacy planning demonstration is
`cargo run -p statechronicle --example inventory` (unique asset: mint →
transfer → lock → unlock → restrict → restore → burn, with fail-closed
rejections). For the tamper and non-membership proof variants, see the end-to-end
test in `crates/statechronicle/tests/e2e.rs` (run with `cargo test -p
statechronicle`). These examples demonstrate deterministic planning and commit
formation; production deployments must add authenticated ingress and the
durable adapter/recovery controls in the
[consumer guide](docs/0.3/consumer-guide.md). Start with the standalone tutorials
above for the recommended protected consumer journey.

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
| `statechronicle-profile` | Portable, allocation-free consumer transition contracts |
| `statechronicle-portable` | Bounded no_std transitions, canonical codecs, hashes, signatures and proof verification |
| `statechronicle-profiles` | Baseline reducers, typed custom adapters, and static profile composition |
| `statechronicle-ports` | Backend-agnostic storage, transaction, authorization, and delivery port traits |

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
| `IntentStore` | Legacy planning markers; not authoritative durable receipts |
| `EventStore` | Append-only storage of validated events |
| `CommitStore` | Broader signed-commit storage; existing adapters bridge to `CommitReader` |
| `CommitReader` | Immutable commit reads and linearizable canonical heads for supplied claim verification |
| `StateIndex` | Read access to current derived state projections |
| `ProofIndex` | Storage/query of served state, ownership, and inclusion proofs |
| `SnapshotStore` | Storage of opaque snapshot payloads |
| `TenantStore` | Tenant scope existence resolution |
| `TrustGrantEvaluator` | Delegated-authority evaluation and freshness checks (trait-only; TrustGrant is one option) |
| `TransactionManager` | Legacy planning lifecycle; cannot enlist separate authoritative stores by itself |
| `LedgerStore` / `LedgerTransaction` | One durable mutation boundary for idempotency, events, signed commits, projections, and outbox |
| `Authorizer` | Authenticated principal binding and default-deny policy |
| `OutboxStore` | Lease-based post-commit delivery and retry |
| `EventPublisher` | Delivery of committed events and signed commits |
| `TradeIndex` | Keyed read access to accumulated trade records (`trade_id` → `TradeRecord`) |

Implement only the capabilities your workflow requires. Preparation needs
read ports; supplied-proof verification needs `CommitReader`; durable writes
need one atomic ledger authority and recovery. The table is a capability
catalogue, not a requirement to implement every trait. Legacy `Executor` and
proof-serving compositions retain their broader port sets. The composition root
(where port adapters, key resolution, the wall clock, and the event-id
generator are assembled) is owned by the consuming platform, not by
StateChronicle.

## Durable storage

StateChronicle defines durable storage contracts but does not ship a storage
adapter. Implement `LedgerStore`, `LedgerTransaction`, projection sinks, and
outbox delivery in the application composition root. The application owns
transaction boundaries, migrations, backup and restore, connection management,
integrity verification, and provider-specific failure handling.
For supplied proof evidence, use `ClaimVerifier::verify_current` with an
independent expectation and trusted signer key, or `verify_historical` with an
independent commit pin. Compatibility `ProofService::verify_canonical` helpers
retain their contracts but do not require the new independent expected claim.

The `statechronicle_ports::outbox::dispatch_once` helper supplies a bounded
worker pass: it claims leased rows, verifies payload digests, publishes through
the consumer's broker adapter, and releases failed deliveries for retry.
Operators should follow [the recovery runbook](docs/OPERATIONS/RECOVERY.md)
for startup integrity checks, restore promotion, projection rebuilds, outbox
incidents, and signer compromise.

For provider-supplied signed history, prefer
[`index::rebuild::verified`](docs/0.3/verified-rebuild.md) with an independently
retained history pin and historical signer policy. The compatibility
`statechronicle_index::rebuild_projections` function assumes its caller already
verified the `(event, commit_id)` stream; it selects the latest version per scoped
resource, rejects conflicting equal versions, and writes through an injected
projection sink.
Large histories can use `rebuild_projections_chunk` and persist its returned
checkpoint between bounded passes.
Operator checkpoint keys are tenant-scoped; clear them with
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

Protocol decisions are recorded in `docs/DESIGN/ADR/`. Original release evidence
is retained in `docs/DESIGN/0.2.0_ACCEPTANCE.md`; exact candidate refactor checks
and their limitations are retained in `docs/0.3/evidence/`. Neither historical
evidence nor a tutorial pass establishes full candidate release qualification.
Run `scripts/run_release_checks.sh` for workspace tests/doctests, standalone
tutorials, conformance, security/dependency policy and bounded fuzz smoke.
See [local release checks](docs/0.3/release-checks.md) for prerequisites, failure
semantics and the remaining separate qualification gates.

## Where to go next

- `crates/statechronicle/examples/`: fifteen runnable examples (start with
  `custom_domains` for native models or `inventory` for built-in resource profiles).
- `docs/DESIGN/DOMAIN_EXTENSIONS.md`: custom profiles, authority, replay, proofs,
  schema migration and the existing portable-core boundary.
- `docs/DESIGN/0.2.0_ACCEPTANCE.md`: release gates and verified audit evidence.
- `crates/statechronicle/tests/e2e.rs`: the end-to-end lifecycle test with
  tamper and non-membership proof variants.
- `crates/statechronicle/README.md`: the umbrella crate and the full surface.
- `crates/statechronicle-ports/README.md`: the port traits and the authority
  model.
- `docs/ARCHITECTURE.md`: how the crates fit together.
- `docs/DESIGN/ADR/README.md`: the architecture decision record index.
