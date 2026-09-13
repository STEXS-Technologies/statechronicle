# Changelog

All notable changes are recorded here. The project is pre-1.0; protocol or
adapter contracts may still require migration notes before a stable release.

## Unreleased

- Restricted the raw durable commit writer to the commit crate and made the
  verified commit-signature/trust API the public persistence boundary.
- Added key-registry-enforced durable player ingress for single requests and
  batches; each batch item now carries its own authenticated principal.

- Dependency policy now denies unknown registry and git sources through
  `cargo-deny`; new sources must be explicitly reviewed before CI passes.
- Added typed `ResourceState` payloads and canonical BCS hashing boundaries.
- Added durable `LedgerStore`/`LedgerTransaction` contracts and the reference
  SQLite adapter with idempotency, canonical heads, projections, and outbox.
- Added authenticated player ingress, key-registry, outbox dispatch, startup
  integrity verification, resource limits, and bounded fuzz CI coverage.
- Durable persistence now recomputes the canonical intent digest at the write
  boundary, rejecting mismatched caller-provided idempotency digests.
- Added direct regression coverage for durable intent/principal/commit scope
  binding before any transaction or idempotency reservation is opened.
- Added `persist_durable_verified_with_metrics` so signature verification and
  mutation telemetry are composed in one production ingress helper.
- Added durable consumer delivery deduplication (`ConsumerDedupStore` and
  `consume_once`) with SQLite lease takeover, applied markers, and retry tests.
- Added explicit outbox poison-message quarantine with durable SQLite metadata;
  corrupt payloads no longer have to retry indefinitely.
- CI fuzz smoke coverage now runs every checked-in libFuzzer target with a
  bounded budget instead of sampling only two targets.
- Enforced worker ownership for outbox completion, release, and quarantine to
  prevent stale workers from finalizing a lease taken over by another worker.
- Added `MemoryKeyRegistry` for fail-closed key registration, rotation,
  retirement, revocation, and historical commit trust checks; production KMS/
  HSM wiring remains a deployment responsibility.
- Added integrity-checked SQLite snapshots via `SqliteLedgerStore::backup_to`
  and backup/reopen regression coverage.
- SQLite backups now fail before snapshot creation when the destination path
  already exists, preventing accidental overwrite.
- Added bounded keyed local rate limiting for account, tenant, and operation
  dimensions with atomic weighted charges.
- Added atomic multi-dimension rate-limit acquisition so account, tenant, and
  operation buckets cannot be partially charged when one dimension denies.
- Zero-cost rate-limit requests no longer allocate keyed buckets, preventing
  cardinality exhaustion through free probes.
- Impossible-cost requests (above bucket capacity) are rejected before key
  allocation, preventing another cardinality-exhaustion vector.
- Added a 32-thread regression proving multi-dimension limiter budgets cannot
  be overspent concurrently.
- Added `statechronicle_ports::quota::DistributedQuota` for shared multi-worker
  quota enforcement with fail-closed request validation.
- Added `quota::try_acquire_validated` so untrusted dimensions pass one shared
  validation gate before any distributed adapter is called.
- Distributed quota providers now have an explicit atomic multi-key method;
  the default fails closed rather than partially charging dimensions.
- Unsupported quota capabilities now have a permanent error classification,
  distinct from transient provider outages.
- Added retry classification for distributed quota errors so outages can be
  retried safely while malformed requests remain permanent failures.
- Added `CommitSigner` and `sign_commit_with_signer` for KMS/HSM-backed
  canonical commit signing without passing private keys through the app.
- Added bounded `InMemoryMetrics` aggregation for mutation counters and
  latency summaries without retaining raw payloads.
- Added SQLite crash-before-commit coverage proving dropped reservations leave
  no partial event/commit rows and can be reclaimed after lease expiry.
- Added a subprocess crash regression that reopens the SQLite file and proves
  an expired reservation can be taken over exactly once.
- Added `statechronicle-postgres`, a `SERIALIZABLE` server-database adapter for
  the durable ledger contract with canonical-head locking and transactional
  event, projection, idempotency, and outbox writes. Cross-tenant manifest
  settlement now uses one shared transaction with deterministic head-lock
  ordering. The adapter fails closed on missing or unsupported
  `sc_schema_meta` migration versions.
- SQLite now exposes fail-closed projection rebuild progress tied to the
  canonical event stream and durable checkpoint metadata.
- Added a bounded SQLite `rebuild_projections` operator example that verifies,
  resumes, checks catch-up, and clears checkpoints only after success.
- SQLite rebuild-progress queries now perform their own integrity scan before
  reporting lag, preventing corrupted streams from appearing healthy.
- Durable executor batch, settlement, and cross-tenant trade wrappers now
  bypass the legacy symbolic transaction handle when a durable sink is
  supplied; failed sink writes cannot report success or leave a replay marker.
- Added `execute_cross_tenant_durable` for generic multi-tenant batches,
  requiring the same single-database durable sink boundary as trade settlement.
- Added optional OpenSSL TLS support (`statechronicle-postgres/tls`) with
  peer-verified CA-file connections for production database deployments.
- PostgreSQL integrity scans now reject orphaned events/outbox rows,
  non-contiguous event indexes, and direct-adapter event-root mismatches.
- Added PostgreSQL `verify_all_integrity` tenant enumeration for complete
  startup/restore scans without a manually maintained scope list.
- Added verified PostgreSQL `canonical_events` loading for projection rebuilds
  after restore.
- Added PostgreSQL head-locked `rebuild_projections_from_canonical` with an
  integration test that deletes and reconstructs a projection.
- SQLite projection-rebuild checkpoints are now tenant-scoped, and recovery
  tooling clears them through `clear_rebuild_checkpoint_for_tenant`, preventing
  same-key progress or cleanup collisions across tenants.
- PostgreSQL idempotency contention now fails closed as a deterministic
  in-progress result, and expired lease takeover uses the correct schema
  column; the live six-test integration suite passes against PostgreSQL 16.
- PostgreSQL baseline schema installation now serializes concurrent migration
  attempts with a transaction-scoped advisory lock.
- Added a 16-worker concurrent schema-installation integration regression.
- PostgreSQL projection rebuilds now read canonical events on the same locked
  transaction connection used for projection replacement.
- Added `PostgresLedgerStore::new_verified` for fail-closed all-tenant startup
  verification.
- Added `new_with_tls_verified` for the equivalent TLS startup gate.
- Added `scripts/run_postgres_integration.sh` for repeatable isolated live
  PostgreSQL gate execution.
- Repeated the scripted PostgreSQL gate across three fresh databases; all runs
  passed without concurrency or migration flakes.
- Removed unused cargo-deny license allowances; the supply-chain gate is now
  warning-free.
- Added `scripts/run_release_checks.sh` to run all bounded repository release
  gates from one command.
- CI-equivalent 100-run smoke coverage passed across all 21 fuzz targets.
- PostgreSQL projection rebuild now acquires the canonical-head lock before
  reading the event stream, preventing a concurrent commit from being omitted
  or its projection accidentally removed during recovery.
- Added a PostgreSQL `verify_integrity` operator example for all-tenant
  startup/restore gates.
- Added bounded PostgreSQL lock/statement timeouts with deployment-level
  configuration to limit hot-key contention and hung transactions.
- Custom PostgreSQL timeout settings now apply consistently to outbox claims as
  well as ledger transactions.
- PostgreSQL read/integrity sessions now receive the same bounded timeout
  policy, covering recovery and canonical-stream queries.
- PostgreSQL outbox claiming defensively preserves malformed commit identifiers
  from legacy databases for deterministic poison quarantine instead of
  aborting the whole claim batch.
- PostgreSQL serialization/deadlock/lock-timeout errors are now classified as
  retryable ledger conflicts for safe bounded retries.
- PostgreSQL idempotency claims now safely take over expired reservations via
  compare-and-swap ownership checks.
- PostgreSQL transactions now require every claimed idempotency reservation to
  be finalized before commit, including shared multi-tenant transactions.
- Added live integration coverage for expired-reservation takeover and
  unexpired lease exclusion.
- PostgreSQL direct event staging now binds event actors to the claimed intent
  actor and operation, preventing cross-actor or cross-operation substitutions.
- Live PostgreSQL integration now covers rejection of cross-actor event
  substitution.
- SQLite direct event staging now enforces the same claimed intent and actor
  and operation binding.
- Added SQLite regression coverage for cross-actor event substitution.
- Added `Executor::execute_player_durable` and `DurableMutationSink` so player
  ingress requires an explicit durable persistence handoff before success.
- Added `Executor::execute_batch_durable` and `DurableBatchSink` for atomic
  multi-resource inventory, marketplace, and settlement routing.
- Added `Executor::execute_settle_durable` for durable value-leg settlement
  routing.
- Added `Executor::execute_cross_tenant_trade_durable` for durable
  manifest-validated multi-tenant trade routing.
- Durable executor routes now defer legacy intent-store replay markers until
  the durable sink succeeds, preventing lost-mutation replays.
- Added executor coverage proving a failed durable player sink leaves no
  legacy replay marker.
- Expanded SQLite crash-boundary regression coverage to stage events,
  projections, and outbox rows before an abandoned transaction and verify that
  none become visible after restart.
- Added an observability/runbook contract covering privacy-safe dashboards,
  alert thresholds, and broker/database failure drills.
- Added a bounded concurrent multi-tenant SQLite claim-load regression to
  verify tenant isolation under parallel traffic.
- Key revocation is now terminal in the reference registry; attempts to
  downgrade a revoked key fail closed.
- Durable persistence now validates event-operation binding before delegating
  to any custom ledger adapter.
- Durable commit boundaries now reject zero-event commits, preventing empty
  entries from advancing canonical history.
- PostgreSQL outbox completion is now idempotent on retries after successful
  delivery, matching SQLite behavior.
- Deprecated ownerless outbox completion, release, and quarantine methods so
  new integrations use lease-owner checks by default.
- Ownerless outbox completion and release now have fail-closed defaults for
  newly implemented adapters.
- SQLite and PostgreSQL ownerless outbox overrides now fail closed as well,
  eliminating unconditional stale-worker updates.
- Ownerless outbox quarantine overrides now fail closed too; poison handling
  must include the claiming worker identity.
- Added SQLite regression coverage for fail-closed ownerless completion,
  release, and quarantine calls.
- PostgreSQL integrity scans now validate idempotency payload digests and
  committed reservation-to-commit references.
- PostgreSQL direct event staging now enforces durable event-count and
  per-event/aggregate byte limits.
- PostgreSQL commit validation now preserves staged events after a rejected
  count or Merkle-root check, allowing safe same-transaction retries.
- Direct projection and outbox staging now reject empty identifiers in both
  durable adapters, including when running against legacy schemas.
- Idempotency finalization in both durable adapters now requires the exact
  tenant-scoped commit staged in the same transaction.
- SQLite idempotency finalization now updates by tenant, intent, and attempt
  together, strengthening reservation ownership under retries.
- PostgreSQL schema migrations now enforce canonical-head and committed-
  idempotency foreign keys to immutable commits.
- PostgreSQL startup now verifies required foreign-key constraints exist before
  opening ledger, commit, or outbox operations.
- Added fresh-schema integration coverage for canonical-head and idempotency
  foreign-key migration constraints.
- Added PostgreSQL projection monotonicity triggers and startup checks for all
  required history/projection triggers.
- Added PostgreSQL idempotency state-guard trigger preventing committed-row
  rewrites and payload/status poisoning.
- Added regression coverage for direct committed-reservation digest rewrites.
- Capped PostgreSQL outbox claims at 1,024 rows per pass to bound worker
  memory and broker fan-out.
- Applied the same 1,024-row outbox claim cap to the SQLite adapter.
- SQLite and PostgreSQL outbox claims now reject lease expirations that are not
  in the future.
- Consumer deduplication claims enforce the same future-lease invariant in
  SQLite and PostgreSQL.
- Shared `consume_once` now rejects expired leases before calling custom dedup
  adapters.
- SQLite outbox retries now retain a bounded `last_error` diagnostic with an
  automatic legacy-schema migration.
- SQLite ownership-aware outbox release now rejects stale workers when the
  conditional lease update affects no row.
- Expanded cross-connection lease regression coverage to stale release and
  stale completion attempts.
- SQLite quarantine diagnostics are bounded as well, limiting poison-message
  metadata growth.
- PostgreSQL now records transient outbox release errors in a dedicated
  bounded `last_error` column, with a migration for existing schemas.
- PostgreSQL startup validation now fails closed when the `last_error` column
  migration is missing.
- Fresh durable schemas cap outbox diagnostic text at 4 KiB, including direct
  SQL writes.
- Added SQLite regression coverage for the bounded outbox claim limit.
- Added a shared 1 MiB outbox payload limit enforced by SQLite and PostgreSQL
  durable enqueue paths.
- Added PostgreSQL integration coverage for oversized outbox payload rejection.
- Added SQLite integration coverage for oversized outbox payload rejection.
- Missing PostgreSQL idempotency intent payloads now fail startup/restore
  verification instead of remaining unverifiable.
- Added PostgreSQL `verify_integrity_with_verifier` for cryptographic commit
  signature trust checks during startup/restore.
- Added negative integration coverage proving rejected commit signatures block
  PostgreSQL restore verification.
- Added all-tenant PostgreSQL signature verification for KMS/HSM-backed
  startup and restore gates.
- Added a live PostgreSQL multi-tenant transaction test covering deterministic
  head locking, two commit append, and both canonical heads.
- Added a live same-head concurrency test proving competing canonical commits
  produce exactly one winner without a partial loser chain.
- Added PostgreSQL outbox lease takeover coverage, including stale-worker
  completion rejection.
- Added PostgreSQL append-only triggers for accepted commit and event history,
  with integration coverage proving historical updates are rejected.
- Added typed SQLite canonical-head inspection for recovery and proof
  composition roots.
- Production launch remains gated by the P0/P1 evidence in `TODO.md`.
