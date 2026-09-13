# Relational adapter contract

The SQLite crate is a reference adapter for one durable file. A horizontally
scaled game backend should implement the same `LedgerStore` contract on one
server database (PostgreSQL or equivalent) and keep all effects of a mutation
inside one database transaction.

An executable PostgreSQL-oriented baseline is provided in
[`postgres_schema.sql`](postgres_schema.sql). Treat it as a migration starting
point, not a substitute for reviewing indexes, roles, retention, and capacity
against the deployment's workload.

The baseline schema applies inside a transaction-scoped advisory lock so
multiple workers starting simultaneously cannot race while creating catalog
objects. Production migration tooling should still run schema changes through
one reviewed migration process and fail closed on any migration error.

## Required tables and constraints

- `idempotency(tenant_id, intent_id)` primary key, canonical payload digest,
  attempt/lease state, and committed result reference.
- `events(tenant_id, event_id)` primary key, immutable BCS payload, and owning
  `commit_id`.
- `commits(tenant_id, commit_id)` primary key, unique `(tenant_id, sequence)`,
  immutable signed payload.
- `heads(tenant_id)` primary key with commit ID, sequence, and state root.
- `projections(tenant_id, resource_id)` primary key with monotonic version.
- `outbox(delivery_key)` primary key with payload digest, lease, and delivery
  state, including durable poison-message quarantine metadata.
- `consumer_deliveries(delivery_key)` primary key with in-progress/applied
  state, lease ownership, and bounded error metadata for downstream effect
  deduplication.

Foreign keys and `CHECK` constraints must reject empty identifiers, invalid
statuses, negative versions, and malformed lease transitions. Immutable rows
must not be updated after acceptance; corrections are new commits.

## Transaction and locking rules

Use a database transaction with a documented isolation level (serializable is
the safest default for settlement). Reserve idempotency after authentication,
lock/CAS all affected projections and tenant heads in canonical sorted key
order, append events and the signed commit, update projections, insert outbox
rows, finalize idempotency, and commit once. On serialization/deadlock errors,
rollback completely and return a bounded retryable conflict.

Outbox completion, release, and quarantine must be conditional on the current
lease owner. A worker whose lease expired must not be able to finalize a row
after another worker has taken it over.

For cross-tenant settlement, all tenant heads and resources must be in this
same database transaction. If that invariant cannot be guaranteed, reject the
operation; do not coordinate independent databases with best-effort writes.

## Required adapter tests

Run the 32+ concurrent idempotency, double-spend, listing-purchase, transfer,
and cross-tenant cases against the real server database. Inject failures after
each write and after commit-before-response, restart, and verify exactly-one
durable result. Verify isolation/lock behavior under hot shared-inventory and
market resources, then record p95/p99 latency, conflict rate, and lock-timeout
limits before launch.
