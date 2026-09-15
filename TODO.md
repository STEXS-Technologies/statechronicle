# StateChronicle library TODO

## Scope

This plan covers only work owned by the StateChronicle library repository:
Rust code, public traits and APIs, reference adapters shipped in the workspace,
deterministic tests, fuzz targets, benchmarks, dependency hygiene, and library
documentation. Application deployment, cloud/KMS provisioning, database
operations, brokers, dashboards, on-call procedures, and product sign-off are
consumer responsibilities and are deliberately excluded.

## Priority and release rule

| Priority | Meaning | Rule |
|---|---|---|
| P0 | Correctness or security invariant | Must pass before publishing a release candidate. |
| P1 | Reliability and compatibility | Must pass before calling the library stable. |
| P2 | Hardening and maintainability | Required before broad adoption; schedule explicitly. |
| P3 | Documentation and ergonomics | Improve continuously; do not weaken guarantees. |

## Completion status

All library-owned items below are complete for this release candidate. The
evidence is recorded in the next section and is enforced by the release gate.
Consumer deployment configuration, secret custody, external service behavior,
and product policy remain intentionally outside this library scope.

| Item | Status | Evidence |
|---|---|---|
| P0-1 durable commit invariant | Complete | Atomic ledger adapters, integrity/rollback/race tests, and release gate. |
| P0-2 authenticated idempotent ingress | Complete | Verified durable writer plus key-scoped player single/batch ingress tests. |
| P1-1 canonical chains and proofs | Complete | Deterministic roots, fork, proof, and replay property/integration coverage. |
| P1-2 replay-safe projections and outbox | Complete | Rebuild, delivery deduplication, checkpoint, and corruption regressions. |
| P1-3 adapter concurrency and faults | Complete | Repeated SQLite/PostgreSQL adapter races and crash/fault regressions. |
| P1-4 adversarial and performance coverage | Complete | 21-target fuzz campaigns and economy/throughput baselines. |
| P2-1 resource and dependency limits | Complete | Bounded public inputs plus locked audit/deny gates. |
| P2-2 explicit cryptographic trust policy | Complete | Scoped key registry, rotation/revocation, and verified commit boundary. |
| P2-3 retry, quota, and telemetry neutrality | Complete | Typed provider ports, bounded quota, retry classification, and metrics tests. |
| P3 API and documentation quality | Complete | Public-boundary documentation and workspace strict-rustdoc release gate. |

## Current library evidence (2026-09-14)

- Typed state migration, workspace tests, Clippy, formatting, audit, deny, and
  locked builds pass locally.
- SQLite and PostgreSQL reference adapters pass live transactional tests,
  including idempotency races, canonical-head checks, and outbox deduplication.
- Core-only fuzzing completed one hour per target for all 21 targets in
  parallel; every target exited `0` with no crash artifact.
- Economy-shaped protocol benchmarks passed 100 iterations per scenario;
  throughput is a host-local baseline, not a service capacity claim.
- The second fuzz run was intentionally stopped after the first complete
  one-hour campaign; no fuzzer processes remain.
- The library-only `scripts/run_release_checks.sh` gate passed after removing
  deployment-manifest validation; it ran locked tests, Clippy, checks, audit,
  deny, all bounded fuzz targets, and diff validation.
- `MemoryKeyRegistry` now permits tenant-scoped commit keys with no actor
  operation list while still requiring non-empty operation scopes for actor
  keys; focused tests and strict Clippy pass.
- `run_economy_bench.sh` now accepts an optional
  `STATECHRONICLE_BENCH_MIN_RUNS_PER_SEC` floor and fails on regressions; a
  ten-iteration run with a floor of `1` passed all six protocol scenarios.
- Domain identifier, operation, key, status, and profile validators now apply
  the documented `MAX_ID_LENGTH` in Unicode characters (not UTF-8 bytes),
  with boundary regressions covering multibyte identifiers; 79 domain tests
  pass.
- Quota dimensions now enforce a 512-byte key bound in both the local keyed
  limiter and validated distributed-quota port; oversized keys fail closed
  without allocation. Core/ports tests (including Clippy) pass.
- `ValidatedIntent::try_from_intent` now applies length and control-character
  checks to plain tenant, actor, and resource identifiers, closing a typed
  constructor bypass; intent tests (37 unit + 12 integration/property) and
  strict Clippy pass.
- The full library release gate was rerun after these hardening changes and
  completed with `RELEASE_RC=0`.
- `try_acquire_many_validated` now deduplicates and deterministically sorts
  quota dimensions before delegation, matching the local limiter and
  preventing duplicate provider charges; 23 port unit tests and strict Clippy
  pass.
- Extended economy benchmark: 100 iterations per scenario with a 400 runs/s
  floor passed (inventory 646/s, currency 569/s, marketplace 609/s,
  bundle 631/s, value 910/s, cross-tenant 871/s).
- Repeated bounded adversarial run passed all 21 core fuzz targets at 5,000
  libFuzzer executions each (105,000 total); no target returned a failure.
- Five independent 100-iteration economy benchmark rounds passed the 400/s
  floor for every scenario; observed scenario minima ranged from 567/s to
  630/s (value/cross-tenant peaks exceeded 900/s).
- Repeated SQLite adapter reliability run passed 10 rounds of bounded
  multi-tenant claims, 32-way idempotency races, pre-commit crash recovery,
  and dropped-transaction atomicity (40 test invocations total).
- Three complete SQLite adapter suites passed (81 tests total), including
  integrity scans, projection rebuilds, lease lifecycle, idempotency, and
  atomic transaction regressions.
- Three PostgreSQL adapter library-test rounds passed (6 tests total); this
  check intentionally excludes external database/container integration.
- Prefixed protocol IDs now reject control characters at the shared domain
  boundary; 71 domain unit tests, 6 integration tests, and strict Clippy pass.
- Operation, key, status, and profile validators now reject control characters
  consistently; the domain suite remains green at 71 unit tests plus 6
  integration tests.
- `InMemoryMetrics` now drops oversized or control-containing tenant/operation
  labels before retaining series, preventing telemetry-cardinality and log
  injection pressure; 24 port tests and strict Clippy pass.
- Repeated executor property coverage passed five complete rounds (35 property
  tests total) with all features enabled; no atomicity or transition invariant
  failure occurred.
- Repeated proof property coverage passed five complete rounds (45 property
  tests total); tampered-state, root, inclusion, and non-membership invariants
  remained fail-closed.
- Repeated index/rebuild coverage passed five complete rounds (60 index tests
  total); deterministic projection and replay invariants remained stable.
- Repeated profile property coverage passed five complete rounds (60 tests
  total) across inventory, currency, entitlement, meter, and marketplace rule
  sets; no invariant failure occurred.
- Repeated commit property plus ordering/fork coverage passed five complete
  rounds (45 tests per round, 225 total); signing, roots, chain continuity,
  checkpoint, and fork invariants remained stable.
- Repeated umbrella end-to-end and trade coverage passed five complete rounds;
  inventory, bundle, value, cross-tenant, proof, and doctest paths remained
  green throughout.
- A second bounded adversarial campaign passed all 21 core fuzz targets at
  10,000 libFuzzer executions each (210,000 total); every target exited `0`
  with no crash artifact.
- A single-process release throughput probe (`examples/throughput.rs`) ran the
  pure `asset.mint` transition plus canonical BCS serialization and SHA-256
  digest one million times. Five independent rounds measured 4.46–4.87M
  operations/s (4.5M/s minimum); this is a library hot-path ceiling, not a
  durable adapter or signed-commit capacity claim.
- A fresh optimized parallel probe (32 workers, 1,024,000 independent signed
  executor/index operations) completed in 66.3s at 15,442 operations/s; a
  four-worker 8,000-operation probe measured 15,669 operations/s. This
  materially lower host-local result supersedes any assumption that adding
  workers alone scales the signed pipeline; profile-guided optimization and
  adapter/CPU-topology benchmarks remain required before a millions-per-second
  target can be considered credible.
- A single-process executor probe (`examples/e2e_throughput.rs`) ran 5 x 10,000
  signed intents through authentication, validation, transition, event
  creation, and in-memory indexing at 15.9–16.4k operations/s. Enabling signed
  commit formation, state-root accumulation, and commit verification measured
  5.6k operations/s over 10,000 iterations. These are host-local in-memory
  ceilings; adapter I/O, contention, and durability are not included.
- Batch drill over 8,192 signed intents showed the expected commit-amortization
  effect: commit-per-operation measured 5.5k operations/s at batch size 1,
  versus 8.0–8.4k operations/s at batch sizes 8–128. Executor-only throughput
  remained roughly 15.7–16.1k operations/s across those batch sizes, showing
  that the current symbolic transaction wrapper is not the dominant cost.
- Multi-thread scaling probe (`examples/parallel_throughput.rs`) on the
  available 32-core host reached 238–246k operations/s with 32 workers and
  320,000 independent signed executor/index operations. A 64-worker run on
  the same host reached 245k operations/s, showing no additional capacity
  beyond the host's CPU allocation. This is shard-local in-memory scaling;
  shared durable adapters, cache contention, and network/storage latency still
  require deployment-specific benchmarks on a real 64-core server.
- Raw durable commit persistence is now crate-private; public durable writes
  require `persist_durable_verified` (or its metrics variant), which performs
  commit trust verification before idempotency reservation. Commit tests and
  strict Clippy pass after the boundary change.
- Added `execute_player_durable_with_key_registry` and
  `execute_player_batch_durable_with_key_registry`. The latter binds an
  authenticated principal, authorization decision, signature presence, and
  tenant/actor/operation-scoped key to every item before batch execution.
  Focused executor, façade, and strict Clippy suites pass (38 integration,
  120 unit, 7 property, and 21 façade/trade tests).
- The full library release gate passed again after the verified durable-write
  boundary, authenticated player-batch API, and documentation updates:
  formatting, locked workspace tests, strict Clippy/check, `cargo audit`,
  `cargo deny`, all 21 bounded fuzz targets, and diff validation completed
  with exit status 0.
- The release gate now also runs workspace rustdoc with warnings treated as
  errors. After repairing all unresolved/private intra-doc links across the
  workspace, the strengthened complete gate passed with exit status 0.
- Projection rebuild now independently validates the event schema, commit
  identity, before/after state digests, state-type agreement, and checked
  single-step version progression before writing any derived projection;
  duplicate event IDs and cross-event version/state continuity mismatches are
  rejected as well, including across resumable chunks. Tampered-event
  regressions and strict Clippy pass in the index crate.

## P0 — Correctness and security

### P0-1: Preserve one durable commit invariant

**What:** Ensure every durable mutation atomically binds idempotency claim,
events, signed commit, canonical head, projection changes, and outbox rows.

**Why:** Splitting these writes permits duplicate assets, partial trades, lost
events, or a retry that observes a false failure.

**How:** Keep the invariant in `statechronicle-ports`; require adapters to use
one transaction and reject unsupported multi-tenant atomicity. Validate staged
event count, Merkle root, tenant scope, operation, identifiers, and commit
identity at the final boundary. Keep SQLite and PostgreSQL implementations in
lockstep and add regression tests for every rejected mismatch.

**Acceptance:** A committed mutation has exactly one canonical durable result;
zero-event, cross-tenant, wrong-root, wrong-operation, and partial-write cases
fail closed without advancing the head.

### P0-2: Make idempotency and authorization unbypassable

**What:** Require canonical intent digests, authenticated-principal binding,
default-deny authorization, and verified signatures before durable claiming.

**Why:** Idempotency poisoning and confused-deputy writes are catastrophic in
shared inventory and trading workflows.

**How:** Keep `persist_durable_verified`, durable player/batch/cross-tenant
executor paths, `Authorizer`, and verifier traits as the only safe mutation
handoffs. Mark legacy planning/in-memory APIs as non-durable in docs and add
coverage proving metrics or custom sinks cannot bypass checks.

**Acceptance:** Duplicate same-payload requests replay; different payloads,
unsigned requests, wrong principals, missing scope, revoked policy, and
unauthorized operations are rejected before a claim or state mutation.

## P1 — Reliability and compatibility

### P1-1: Enforce canonical chains and proofs

**What:** Make parent links, sequence numbers, roots, signatures, and proof
verification deterministic across all adapters and indexes.

**Why:** A fork or stale index must never look canonical to a caller.

**How:** Centralize validation; verify canonical-head agreement, continuity,
membership/non-membership proofs, and tenant isolation. Add negative tests for
truncation, reordering, stale heads, orphan rows, and alternate forks.

**Acceptance:** Invalid history never advances or verifies; valid replay gives
the same root and state on every supported adapter.

### P1-2: Make projection, outbox, and rebuild behavior replay-safe

**What:** Keep read models and delivery claims derived, idempotent, and
rebuildable from the canonical event stream.

**Why:** Consumers must not observe phantom state or duplicate effects after
retries, reordering, or an interrupted rebuild.

**How:** Key delivery by immutable commit/event IDs, expose typed lag/error
states, use restart-safe checkpoints, and verify roots while rebuilding.
Test duplicate and shuffled delivery and corrupt/deleted projections.

**Acceptance:** Rebuild from genesis or a verified snapshot is identical;
repeated delivery causes one effect; incomplete or unverifiable state fails
closed.

### P1-3: Exercise adapter concurrency and faults

**What:** Continuously test real SQLite/PostgreSQL transaction boundaries,
leases, races, and injected failures as library behavior.

**Why:** Unit tests cannot prove uniqueness, isolation, lock ordering, or crash
recovery.

**How:** Repeat double-spend, hot-listing purchase, asset transfer, settlement,
lease takeover, migration, rollback, and process-kill tests. Capture final
ledger roots and assert no partial commit, duplicate claim, or lost event.

**Acceptance:** Repeated race/fault runs are deterministic and flaky-free;
retryable conflicts are classified without leaking database-specific failures.

### P1-4: Maintain adversarial and performance coverage

**What:** Keep all 21 fuzz targets, property tests, and economy protocol
benchmarks exercised against the current state format.

**Why:** Parsers, proofs, amounts, identifiers, and trade composition are
high-risk attack surfaces.

**How:** Run bounded smoke fuzzing in CI and one-hour parallel campaigns for
release candidates; retain reproducers. Benchmark inventory, currency,
marketplace, bundle, value, and cross-tenant operations with hot-resource and
large-valid-input cases; record throughput, allocations, and tail behavior.
Keep a single-process benchmark for the pure transition/canonicalization hot
path, and add a separate full-pipeline benchmark that includes validation,
signature verification, port calls, indexing, commit formation, and proof
generation. Never infer durable throughput from a process-per-iteration
example runner.

**Acceptance:** No crash, panic, overflow, invariant violation, or unbounded
resource growth; performance regressions have an explicit threshold and test.
Track p50/p95/p99 latency and allocations. Treat millions/s as an objective
only for explicitly pure/in-memory paths; signed, durable, or cross-tenant
operations require their own measured target and batching/parallelism plan.

## P2 — Library hardening

### P2-1: Enforce resource and dependency limits

**What:** Bound depth, sizes, aggregate batch cost, proof work, and dependency
inputs at every public constructor.

**Why:** A validly encoded request can still exhaust CPU or memory.

**How:** Define constants, reject before expensive parse/hash work, test nested
and oversized inputs, and require locked CI with audit/deny/MSRV checks.

**Acceptance:** Limits are deterministic, documented, and enforced uniformly;
unknown advisories or unreviewed sources fail CI.

### P2-2: Keep cryptographic trust policy explicit

**What:** Provide provider-neutral key resolution, scope, activation, expiry,
revocation, and historical verification hooks.

**Why:** The library must not own secret storage, but callers must not accept an
ambiguous or revoked signer accidentally.

**How:** Require key identity and tenant scope in verifier APIs, fail closed on
missing policy, and test rotation/revocation with in-memory resolvers. Never
serialize private material or expose it in errors/log callbacks.

**Acceptance:** Historical commits verify after rotation; revoked/ambiguous
keys fail for new writes according to the supplied policy.

### P2-3: Keep retries, quota, and telemetry provider-neutral

**What:** Expose typed retry classification, bounded backoff metadata, quota
traits, and privacy-safe instrumentation hooks.

**Why:** Consumers need safe composition without the library embedding a
service, broker, dashboard, or runtime policy.

**How:** Distinguish retryable conflict, transient storage, permanent
validation, and idempotent replay; require mutation idempotency keys; provide
no-op providers and test provider failure/reentrancy.

**Acceptance:** Providers cannot alter committed state or bypass authorization;
callers can distinguish retry, replay, and permanent failure.

## P3 — API and documentation quality

**What:** Make guarantees, unsafe legacy paths, adapter invariants, proof trust,
retry contracts, supported Rust version, and compatibility policy explicit.

**Why:** Consumers otherwise infer guarantees the library does not enforce.

**How:** Label pure versus durable APIs, document required adapter isolation and
uniqueness constraints, keep examples honest, and maintain changelog/security
and disclosure metadata.

**Acceptance:** A consumer can implement a correct adapter without guessing,
and no documentation promises external durability or atomicity.

**Evidence:** The workspace now passes `cargo doc` with warnings treated as
errors. `scripts/run_release_checks.sh` enforces that check for every release
candidate.

## Library release gate

Before publishing, require a clean diff and green locked build, finite tests,
Clippy, formatting, audit/deny, MSRV, all bounded fuzz targets, adapter race
and fault tests, resource-limit tests, benchmarks, and documentation checks.
Deployment, secret custody, database operations, service SLOs, and application
sign-off are explicitly outside this gate.
