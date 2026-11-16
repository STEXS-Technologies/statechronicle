# Adapter conformance and recovery qualification

`LedgerStore`/`LedgerTransaction` own the atomic persistence boundary;
`LedgerRecovery` reads the same authority. A valid signature or a checked receipt
cannot establish that a database committed every effect atomically. The separate
unpublished test-support workspace at `ci/conformance/` now exercises that
contract through independently opened handles and raw storage observations.
It adds no published library crate, root runtime dependency, feature or port
method. Its resolved lockfile adds no third-party package/version/checksum beyond
the root lockfile. The local release-check script includes it explicitly because
root `cargo --workspace` does not traverse isolated test workspaces.

The reusable `LedgerTestBackend` has reset, connect and inspect operations. A
backend author supplies a disposable test database and an independent raw reader.
The harness never derives expected success from the adapter's own recovery row.
Snapshot comparisons preserve duplicate rows and cover events, signed commits,
projections, outbox rows, finalized identities and the actual head index. A second
oracle compares recovery with the complete original immutable mutation.

The reference integration uses a separately written in-memory transactional
backend, not the existing root integration-test fixture. Real signed genesis
vectors are constructed through public builders and verified independently of
storage. The reference models only bounded single-tenant genesis mutations;
new handles share its in-memory database and transactions stage private effects.
It is not a production adapter or disk-durability demonstration.

Current deterministic checks cover:

- No staged event, commit, projection, outbox or finalized row is visible before
  commit; successful commit publishes the complete expected set.
- Rollback after claim and every individual completed write leaves no effect or
  successful identity; the same identity can be reserved again.
- Dropping a pre-commit transaction at those boundaries exposes no committed
  effects or receipt. An unfinished lease may remain pending; this never grants
  permission to replace the original identity.
- Exact retry resolves the original commit; changed payload conflicts and leaves
  all effects unchanged. Coincident textual ids in different tenants stay scoped.
- Barrier-started 2/8/32-client races for one head produce one complete winner.
  Exact-identity races also produce one writer; pending/replay results resolve
  through later authoritative reads without duplicated effects.
- Finalization with a different attempt token refuses. Unsupported multi-tenant
  transactions refuse before any effect.
- Injected loss of a successful commit ACK produces `Unknown`. Reopening a handle
  and recovering verifies the complete original record; exact retry returns a
  verified replay and creates no second outbox, event or canonical commit.

Eight deliberately broken variants establish specific negative controls:

| Broken behavior | Required failing oracle |
| --- | --- |
| Publish events during staging | `atomic.visibility` |
| Lose outbox rows | `atomic.completeness` |
| Omit original outbox from recovery | `lookup.original` |
| Grant a new reservation for a committed key | `replay.identity` |
| Ignore tenant in identity lookup | `tenant.claim` |
| Ignore canonical-head comparison | `head.single_winner` |
| Finalize with a wrong attempt token | `fencing.attempt` |
| Acknowledge but discard the mutation | `ack.original` |

This is an initial reusable F5 contract harness, not completion of F5 or F8.
Backend-specific tests must still inject failures **inside** database statements,
exercise actual reopen across process termination, flush/power-loss behavior,
lease expiry and fencing races, full supported multi-tenant transactions,
non-genesis progression and immutable retention after projection/outbox changes,
corrupt/reordered/missing history, checkpointed projection replay, schema upgrades,
and any advertised distributed guarantees. The bounded reference does not prove
those properties. Generic logical ACK injection only covers the client-visible
uncertainty contract after a known successful underlying transaction.

See [consumer instructions](../../ci/conformance/README.md) for the trait,
fixture requirements, exact local command and per-check inputs. Run only against
disposable test databases: every check deliberately resets storage.

## Optional protected batch capability

Protected batches require `LedgerTransaction::retain_batch` and
`ledger_batch::LedgerBatchRecovery` on the same store. Retention is unsupported
by default. Every member claim must finalize atomically with one commit and the
complete immutable original group. Single and batch claims share the same
identity namespace; neither lookup may reinterpret the other's records as
independently committed subsets. See [durable-batches.md](durable-batches.md).
The optional `ci/conformance::batch` module checks complete atomic visibility,
rollback/cancellation, replay, lost-ACK recovery and distinct-group head CAS
through independent raw effects and retained membership observations. Its
separate reference adapter includes nine broken batch controls. The existing
single-mutation suite still runs. Additional generic checks cover tenant-isolated
coincident IDs, both directions of the shared single/batch namespace, rejection
of cross-kind fabricated receipts, and 2/8/32-client same-group contention.
Four namespace fault variants fail their intended oracles. Optional batch lease
schedules are described below; broader lease races and non-genesis
retention/rebuild remain open. Backend-specific crash and physical persistence
qualification are still required. See the consumer instructions for
exact capability and fixture requirements.

## Optional single-reservation lease takeover

The dev harness now provides `LedgerLeaseTestBackend` and `check_lease_takeover`.
For a backend advertising expiring concurrent reservations, an injected-clock
hook expires the old lease while leaving its transaction live. Normal claim
must allocate a different attempt token. Complete stale staging cannot commit;
stale rollback and drop cannot release the successor. Independent lookup and
raw effects verify the successor commits the exact original once. Two broken
reference controls fail `lease.stale_commit` and `lease.successor`.

This closes one bounded logical single-reservation schedule. The following batch
extension adds selected member-expiry schedules; other fencing interleavings,
real database clock/transaction behavior and process/power-loss qualification
remain open. Earlier references to open
lease testing should be read with this extension's precise scope.


## Optional batch lease takeover

The dev-only batch harness now checks whole-group takeover through the existing
lease hook: all member tokens change, stale commit/rollback/drop preserve the
successor, and only its complete original group can publish. Independent raw
observations include retained membership and all canonical effects. Shared
lease deadlines are supported. No runtime crate, port or transaction API changes.

A separate optional `LedgerBatchMemberLeaseTestBackend` configures independently
expirable member deadlines before ordinary claims. Its check replaces each
member in turn with the others still live. This detects a backend fencing only
the first member, which whole-group takeover alone cannot establish. Claim
replies must agree with independent pending lookup. Partial acquisition remains
pending, publishes nothing, and is rolled back. Shared-deadline-only adapters
need not implement this optional test capability.

The two-member reference exercises both members and all three stale cleanup
forms; five fault behaviors fail their named oracles. A shared-deadline model
passes the applicable whole-group check and rejects isolated configuration as
an unsupported setup. These are bounded logical schedules, not universal lease,
large-group, process-reopen or power-loss qualification. Broader later-head
retention/rebuild, interrupted/corrupt replay, real backend clocks/transactions
and broader F5/F8 work remain open. See the consumer instructions for exact
hook and fixture contracts.

## Retention after a later canonical head

Run `check_later_head_retention` with two independently constructed, signed,
consecutive mutations in one tenant, distinct intent/commit identities, and an
independent expected final `Snapshot`. The expected view must include retained
history/finalizations, current projections, pending outbox rows and only the new
canonical head. It must not be reconstructed from adapter lookups. Fresh
connections must retrieve both exact original records; retrying the first key
must identify its original commit and leave the final view unchanged.

Adapters with removable pending outbox rows can additionally implement
`LedgerOutboxTestBackend::acknowledge_outbox` and run
`check_delivered_retention`. Use the normal delivery acknowledgement path, not
receipt mutation. The check removes the first commit's pending rows from its
expected raw view and requires both immutable receipts to remain exact. This
is not an exactly-once delivery claim.

The bounded reference checks both different resources and an asset transfer
that replaces the current projection at the same resource key. Native profile
checks/reduction generate the transfer state; the commit builder recomputes
signed event/state roots. Both exact original receipts must survive the updated
owner/version and acknowledgement of the first outbox. A separate two-holder
currency fixture proves that subject-held projection keys preserve both holders
under the same tenant/resource. Owner-based keys exclude owner identity, while
subject-held keys include the subject. The reference upserts current rows under
these keys, checks monotonic versions before publishing any rows, and retains
immutable original projection sets separately. A deliberately invalid equal-
version projection is refused without changing any original committed effect.

Five deliberately faulty retention/projection modes exercise the oracles:

| Fault | Required oracle |
| --- | --- |
| Discard the old receipt after another commit | `lookup.original` |
| Substitute current projections into the original receipt | `lookup.original` |
| Reconstruct original outbox from remaining pending rows | `lookup.original` |
| Append rather than replace a current projection | `retention.effects` |
| Key subject-held projections only by resource | `retention.effects` |

These are bounded two-commit schedules. They do not establish
long-history projection rebuild, interrupted/corrupt replay, real database
reopen, process kill or power-loss guarantees. Callers must supply valid signed
fixtures; storage checks do not establish ingress or signer authorization. The
equal-version refusal test deliberately supplies invalid storage input.

## Batch retention after head advance and delivery

Use `batch::check_later_head_retention` for two consecutive signed single-tenant
batches with distinct commit/member identities and an independently supplied
final `Snapshot`. The final snapshot contains both append-only histories and all
finalizations, current projections, pending outbox rows, and only the later head.
The harness separately inspects every raw retained membership row and compares
its complete original group. Neither raw inspection nor expected effects may be
reconstructed from lookup results.

Every member in both groups must resolve to its complete original batch;
standalone lookup must still refuse the batch namespace. Each exact member
retry must return its original commit, while a changed nonce must conflict.
Fresh independent connections and repeated raw observations check that retries
change no effects or retained membership. These checks reuse the existing exact/
conflicting-retry logic, rather than introducing another interpretation.

If the adapter implements the optional `LedgerOutboxTestBackend`, also run
`batch::check_delivered_retention`. It acknowledges only the first group's
pending rows, then repeats all original lookup, namespace, exact retry,
conflicting retry and raw membership/effect checks. Original immutable receipt
outbox and projection sets must stay intact. Acknowledgement is not proof of
exactly-once external delivery or physical durability.

The reference uses two two-member groups in one tenant. Native profile checks
and reduction produce two asset transfers, replacing both current projections
while the original group remains exact. Seven fault behaviors fail named checks:

| Fault | Required oracle |
| --- | --- |
| Drop the old group | `batch.retention.membership` |
| Drop only its second member row | `batch.retention.membership` |
| Substitute current projections into a historical lookup | `batch.lookup.original` |
| Corrupt only the second old member lookup after head advance | `batch.lookup.original` with `member 1` |
| Reconstruct original outbox from pending rows | `batch.lookup.original` |
| Return the latest head for an original exact retry | `batch.replay.identity` |
| Append rather than replace current projections | `batch.retention.effects` |

This closes selected logical batch-retention schedules. Larger groups, mixed
single/batch histories, interrupted/corrupt replay, verified projection rebuild,
real database reopen/process-kill/power-loss behavior and broader F5/F8 gates
remain open. Fixture signature validity is checked during reference generation;
consumer ingress and signer authorization remain separate authorities.

## Projection rebuild addressing correction

The canonical `index::rebuild` implementation now keys projection selection and
continuity by tenant/resource plus subject for subject-held state. Owner-based
state retains its resource key across owner changes. Duplicate event detection
uses tenant/event identity, matching scoped event storage. No wire format,
cryptographic computation, public signature or dependency changes.

The prior implementation demonstrably rejected valid independent balance
holders as a continuity mismatch and coincident event IDs in distinct tenants
as duplicates. Retained failing regressions reproduce both. New negative
controls still reject wrong-holder continuity, reordered same-holder history
and same-tenant duplicates before any projection write. Logical restart tests
inject a checkpoint save failure after projection writes at each checkpoint for
chunk sizes one, two and three, then resume the same pinned four-event source
against an idempotent monotonic sink and verify both exact final holder rows.

This correction does not authenticate an untrusted event stream: callers must
verify signatures, signer authorization, complete canonical order and roots
before rebuilding. Numeric checkpoint keys must remain pinned by the caller to
the same source. There is no new physical-crash, schema migration, arbitrary
missing-prefix or changed-source checkpoint guarantee.

## Verified source and checkpoint binding

The opt-in `index::rebuild::verified` path now composes existing commit signature,
chain-link, event-integrity and replay-root owners before exposing an owned
`VerifiedHistory`. It requires an independently selected historical tip digest
and signer policy. Its resumable method derives checkpoint keys from the entire
verified signed source plus job name; changed sources do not reuse an old offset.
The event-only APIs retain their caller-verified-source contracts. See
[verified rebuild](verified-rebuild.md) for the external consumer, sink/key
requirements, historical trust boundaries and remaining physical/performance
limits. This does not close arbitrary history, database crash or full F5/F8
qualification.


## Mixed ordered history qualification

The dev harness additionally exposes `check_history` for a supplied sequence and
an independent raw snapshot at every committed prefix. It resets once, checks
every staged/rolled-back write prefix against the preceding history, commits
through fresh connections, and then verifies every original receipt, original
retry commit and changed-payload conflict. Incorrect setup is refused before
resetting the test database. This extends the two-commit retention checks; their
scope limits remain applicable.

The reference's 32 signed mutations interleave two tenants with coincident
textual IDs, update two owner-based asset keys and two subject-held balance
keys per tenant, and independently verify both complete signed histories.
Delayed receipt pruning and latest-commit substitution at the fourth head, and
rollback that deletes existing history, pass the previous smaller schedules but
fail named oracles in this one. The reference checks 224 rollback prefixes and
528 retained original identities across the growing history. The input snapshots
are constructed before backend use; no lookup or adapter key helper supplies
the expected rows.

Use backend-specific history fixtures and an outer timeout. This does not add
production dependencies or change ledger authority. Mixed single/batch history,
randomized concurrent histories, process/database reopen, proof serving and
physical crash/power-loss qualification remain separate requirements. See the
[conformance instructions](../../ci/conformance/README.md#ordered-mixed-histories).

## Alternating single/batch histories

For adapters supporting both APIs over shared storage, use
`batch::history::check` with owned `HistoryEntry` values and independently
constructed raw snapshots. Each tenant has one consecutive head chain across
single and complete batch transactions. Scoped member/commit identities must be
distinct, including across kinds. Every rollback prefix preserves earlier raw
effects and membership; every committed prefix rechecks every original receipt,
exact/conflicting claim and namespace through fresh connections. Invalid input
shape or chain refuses before destructive reset. Fixtures must already be valid
and signed; this dev harness is not an authentication or signer-policy layer.

The reference supplies 22 transactions (12 singles, 10 two-member batches), 32
intents/events, two tenants with coincident textual IDs, 8 current projection
keys and 64 outbox rows. It checks 224 rollback prefixes and 368 cumulative
original identities. Two new cross-kind receipt-loss backends pass the existing
single history and genesis batch visibility/replay checks, then fail this mixed
schedule. Both signed per-tenant fixture histories verify before backend use.
See [consumer setup](../../ci/conformance/README.md#alternating-single-and-batch-histories).
Randomized concurrency, database crash recovery and full F5/F8 remain open.

## Canonical economic contention

The protected integration fixture now canonically seeds an asset lock and buyer
balances, then races distinct asset/payment groups on 2, 8 and 32 OS threads.
Its reader checks complete economic snapshots, signatures, event/projection
binding and independently addressed full state roots. Loser lookup and retry,
13 economic failure boundaries, lost ACK/current access checks and unsupported
cross-tenant scope are covered. Three isolated broken-adapter controls fail the
intended runtime oracles. See [exact source and limits](evidence/f1/protected-settlement/contention-7feb9db/README.md).

This test adapter shares one in-memory authority across fresh handles. Its typed
subject-held addressing covers the tested balance transfers; it does not model
subject relocation, physical durability or independent database connections.
The reusable generic conformance interface and backend-specific crash campaigns
remain separate from this protected end-to-end scenario.
