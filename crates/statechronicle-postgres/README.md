# statechronicle-postgres

`statechronicle-postgres` is the server-database adapter for the durable
`LedgerStore` contract. It opens one dedicated PostgreSQL connection per
ledger transaction, starts `SERIALIZABLE` isolation, locks the canonical head,
and atomically writes commits, events, projections, idempotency state, and
outbox rows. It also implements `CommitStore`, `OutboxStore`, and
`ConsumerDedupStore` so proof serving and post-commit delivery can use the same
database.

Apply and review [`docs/OPERATIONS/postgres_schema.sql`](../../docs/OPERATIONS/postgres_schema.sql)
with the deployment migration system before use. The adapter deliberately
supports `begin_multi` with one shared transaction and deterministic head-lock
ordering; callers must still provide a manifest that binds each tenant's
commit and event set. It never pretends that independent databases provide
atomic settlement.

The migration creates `sc_schema_meta` at version 1. The adapter checks this
metadata before every transaction and fails closed when it is missing or newer
than the binary, so migrations must be reviewed and rolled out ahead of the
application.

Call `PostgresLedgerStore::verify_integrity` during startup and restore before
unfreezing writes; it validates commit/event roots, chain continuity, the
canonical head, outbox payload digests, and projection payload/index bindings.
It also rejects orphaned event and outbox rows and non-contiguous event
indexes.

The constructor accepts a libpq connection string:

```rust,no_run
use statechronicle_postgres::PostgresLedgerStore;
let ledger = PostgresLedgerStore::new("host=localhost user=game dbname=ledger");
```

Connection pooling, credentials, migrations, and operational retry policy
belong at the consuming service's composition root. The default build uses
`tokio-postgres::NoTls` for local/private deployments. Enable the crate's
`tls` feature and construct with `PostgresLedgerStore::new_with_tls` for
OpenSSL peer-verified connections in production; provision and review the CA
file as deployment configuration. Serialization and deadlock errors are
returned as retryable ledger conflicts; callers must keep the original
idempotency key when retrying.

Use `verify_all_integrity` for a startup/restore gate when the service cannot
rely on a separately maintained tenant list.
`PostgresLedgerStore::new_verified` combines construction with this fail-closed
all-tenant scan for mutation-serving startup paths.
TLS deployments can use `new_with_tls_verified` for the same startup gate.
Use `verify_all_integrity_with_verifier` when every tenant must also pass
cryptographic commit-signature trust checks.
An operator command is available with
`STATECHRONICLE_POSTGRES_URL='...' cargo run -p statechronicle-postgres --example verify_integrity`;
it exits nonzero on any integrity failure.
After a verified restore, `canonical_events` exposes the ordered stream for
projection rebuilding with `statechronicle_index`.
`rebuild_projections_from_canonical` performs the rebuild in one serializable,
head-locked transaction and can clear stale projections first.
The checked-in `rebuild_projections` example verifies one tenant and invokes
this operation, exiting nonzero on integrity or projection conflicts.
Transactions apply 5-second lock and 30-second statement timeouts by default;
use `with_timeouts` to choose stricter deployment-specific bounds for both
ledger and outbox transactions.
The same bounds apply to read and integrity connections, preventing large
recovery scans from running indefinitely.
Serialization failures, deadlocks, lock timeouts, and statement cancellations
are surfaced as retryable ledger conflicts; preserve the idempotency key when
retrying.
Malformed outbox metadata is leased as poison and must be handled through the
dispatcher quarantine policy rather than silently dropped.
Claims are capped at 1,024 rows per pass; drain workers by repeating bounded
claims rather than requesting an unbounded batch.
Transient release failures are retained in the bounded `sc_outbox.last_error`
column, while `quarantine_error` is reserved for permanent poison rows; the
startup schema check refuses deployments missing this migration.
Integrity verification also checks idempotency payload canonical digests and
committed reservation-to-commit references; rows without canonical intent
payloads fail closed.
Direct transaction callers receive the same event-count and event/batch-byte
limits as the higher-level durable persistence helper.
Transactions fail closed when required schema foreign keys are missing, even
if the reported schema version matches.
The migration also installs and startup validation requires append-only
history triggers, a monotonic projection trigger, and an idempotency state
guard.
Use `verify_integrity_with_verifier` with the deployment's KMS/HSM-backed
`SignedCommitVerifier` when cryptographic signature trust must be part of the
startup/restore gate.
