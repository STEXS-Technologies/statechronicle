# ADR-008: Durable ledger transaction boundary

*Status: Accepted (contract; adapter implementation required)*

## Context

StateChronicle's executor is pure logic: it validates an intent and returns
events. `execute_batch` currently wraps that work in a `TransactionHandle`, but
the handle cannot enlist the intent store, event store, commit store, state
projection, or publisher. The current wrapper is therefore symbolic and cannot
provide all-or-nothing settlement across a process crash or concurrent writers.

That distinction is unsafe for player-owned inventory, balances, listings,
escrow, and cross-tenant trades. A successful command must have one durable
meaning: either the entire mutation is committed and replayable, or no
mutation is visible. Notifications and search/proof indexes must not become
sources of truth.

## Decision

The contract is represented by `statechronicle_ports::ledger_store::{LedgerStore,
LedgerTransaction}`. The commit crate exposes `persist_durable_verified`, which
first verifies commit-signature trust and then performs principal binding,
authorization, canonical event validation, idempotency claim,
event/commit/projection/outbox writes, idempotency finalization, and one final
transaction commit. The raw writer is crate-private. Existing `persist`
remains a legacy non-transactional compatibility API and is not a production
settlement path.

Production integrations MUST provide one durable write transaction for each
single-tenant or supported multi-tenant ledger scope. The transaction owns all
write-side effects and commits exactly once. The recommended v0 deployment is a
single transactional relational database; distributed 2PC is not implied by
the protocol and must not be claimed by an adapter that does not implement it.

The durable operation order is:

```text
authenticate principal and bind actor
  -> parse/validate/expire intent
  -> atomically claim idempotency key
  -> lock or compare-and-swap every affected resource and scope head
  -> load and validate current state
  -> calculate deterministic after-state/events
  -> form and sign commit against the locked canonical head
  -> append immutable events and signed commit
  -> advance canonical head
  -> update conflict-check projections
  -> insert transactional outbox record
  -> commit
  -> publish asynchronously from outbox
```

The executor may retain a pure planning API, but a production command API MUST
use a durable transaction coordinator that can stage every operation above.
Calling `put_intent`, `append_events`, `put_commit`, or projection updates
outside that coordinator is not equivalent and must not be documented as
atomic.

## Required transaction contract

The port layer must expose a write-side transaction abstraction (the concrete
name is implementation-defined) with operations equivalent to:

1. `claim_or_get_intent(tenant, intent_id, payload_digest, attempt_id)` —
   atomically returns `New`, `Committed(existing_result)`, `InProgress`, or
   `ConflictDifferentPayload`.
2. `load_for_update` / versioned compare-and-swap for every resource projection
   and every affected tenant's canonical head.
3. `append_events` — rejects duplicate event IDs and immutable rewrites.
4. `append_commit_and_advance_head` — requires the exact expected parent ID,
   sequence, and previous state root; inserts the commit and advances the head
   as one operation.
5. `upsert_projection` — requires the expected prior version and state hash;
   projections remain rebuildable derived data.
6. `insert_outbox` — records commit/event delivery after all ledger rows are
   prepared and before transaction commit.
7. `commit` and `rollback` consuming the transaction exactly once.

The abstraction may be implemented as a database transaction object, session,
or adapter-specific unit of work. It must not expose a fake `commit` method
that merely records intent while writes happen through unrelated ports.

## Invariants

Every production adapter MUST enforce these constraints at the database or
equivalent serialization boundary:

- immutable event rows with unique `(scope, event_id)`;
- immutable signed commits with unique `(scope, commit_id)`;
- one accepted commit per `(scope, sequence)`;
- one canonical head per scope, advanced only by expected-parent CAS;
- `next.sequence = previous.sequence + 1` and
  `commit.previous_state_root = head.state_root`;
- unique idempotency key `(tenant_id, intent_id)` plus canonical payload digest;
- projection key uniqueness and strictly checked versions;
- outbox uniqueness by immutable commit/event delivery key;
- all resource keys and tenant head keys locked in deterministic sorted order
  for multi-resource operations.

If an adapter cannot atomically span all tenants in a requested operation, the
operation MUST be rejected as unsupported. It must not silently execute one
tenant at a time.

## Failure and retry semantics

The intent record is finalized as `Committed` in the same transaction as the
events and commit. An aborted transaction must not leave a permanent claimed
intent. A short-lived `InProgress` lease is allowed only if takeover after
process death is safe and two attempts cannot both commit.

After durable commit, a client response or publisher failure is retryable. A
retry with the same canonical payload returns the original committed result;
the command is never executed a second time. Outbox publication is at-least-
once; consumers deduplicate by immutable commit/event ID.

The adapter must classify serialization/deadlock conflicts as retryable and
authorization, schema, ownership, balance, and version failures as permanent.
Blindly retrying a mutation without its idempotency key is prohibited.

## Rejected alternatives

### Symbolic transaction manager

Rejected because a handle with only `commit()`/`rollback()` cannot enlist the
logical stores and gives false atomicity confidence.

### Persist intent before validation

Rejected because malformed or unauthorized client input can reserve an intent
ID, creating an availability attack and preventing legitimate retries.

### Synchronous publish as part of the database transaction

Rejected because a broker outage must not roll back an already durable ledger,
and a broker cannot provide the database's canonical commit guarantee.

### Per-store best-effort writes

Rejected because partial event/commit/projection writes produce unrecoverable
half-settlements under crashes.

## Consequences

The ports crate gains a more explicit write-side contract and production
adapters become responsible for isolation, uniqueness constraints, migrations,
and recovery. In-memory fakes remain useful for pure logic tests but cannot be
used as evidence of production atomicity.

Cross-tenant settlement may need to be disabled in deployments that use
separate databases. This is an intentional safe failure, not a feature gap to
hide behind documentation.

## Acceptance tests required before marking this ADR implemented

- 32 or more concurrent identical transfers produce one commit and stable
  replay responses.
- Concurrent buyers for one listing produce one winner and no negative balance
  or duplicate ownership.
- Process termination at every write boundary yields either the complete
  committed transaction or no transaction.
- Wrong parent, sequence, state-root, projection version, or payload digest is
  rejected atomically.
- Broker outage leaves the committed ledger intact and outbox delivery safely
  retryable.
- Projection deletion/rebuild from the event log reproduces the canonical
  state roots and versions.
