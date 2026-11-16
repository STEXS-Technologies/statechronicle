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
large-group, process-reopen or power-loss qualification. Generic later-head
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

These are bounded two-commit schedules. They do not establish batch retention,
long-history projection rebuild, interrupted/corrupt replay, real database
reopen, process kill or power-loss guarantees. Callers must supply valid signed
fixtures; storage checks do not establish ingress or signer authorization. The
equal-version refusal test deliberately supplies invalid storage input.
