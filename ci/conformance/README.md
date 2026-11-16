# StateChronicle adapter conformance support

This unpublished **dev-only** package checks `LedgerStore`/`LedgerRecovery`
implementations. It is an isolated workspace, adds no root library dependency,
and is never linked into the normal StateChronicle facade.

Run the reference and negative controls locally:

```sh
RUSTC_WRAPPER= CARGO_BUILD_JOBS=8 cargo +1.85.0 test \
  --manifest-path ci/conformance/Cargo.toml --all-targets --locked --offline
```

For an adapter's test crate, add this checkout's `ci/conformance` directory as a
path dev dependency named `statechronicle-conformance`. Implement
`LedgerTestBackend` with three operations:

- `reset`: clear a dedicated disposable test database, including reservations.
- `connect`: open a fresh independent connection to that same database.
- `inspect`: read the raw committed event/commit/projection/outbox/finalization
  rows and the actual canonical-head index through an independent reader.

Do not implement inspection by calling `lookup_mutation` or reconstructing a
snapshot from a cached receipt. Return every row, including duplicates. The
harness compares canonical multisets, so row ordering does not matter. It never
deduplicates an observation before checking it.

Use well-formed, genuinely signed immutable `LedgerMutation` fixtures. The basic
checks use genesis mutations; the head-contention fixtures need distinct intent
and commit ids in one tenant, all prepared against the same genesis head. The
tenant check needs the same textual intent/commit ids in distinct tenants. See
`tests/reference.rs` for a complete external integration using only public
StateChronicle and harness APIs; it generates fixtures separately from its
storage implementation and verifies their signatures.

Call the check functions independently on an isolated backend instance:

| Function | Required input |
| --- | --- |
| `check_atomic_visibility` | One genesis mutation |
| `check_rollback` | Same mutation; every individual write prefix |
| `check_cancellation` | Same mutation; drop after each completed stage |
| `check_replay` | Exact retry and changed-payload conflict |
| `check_tenant_isolation` | Two independent tenant genesis mutations with coincident ids |
| `check_head_contention` | 2, 8, then 32 distinct competing genesis mutations |
| `check_idempotency_contention` | One mutation, 2, 8, then 32 exact concurrent requests |
| `check_attempt_fencing` | Wrong reservation token must refuse |
| `check_lease_takeover` | Optional `LedgerLeaseTestBackend`; live lease stays pending, expired lease gets a new token, stale commit/rollback/drop cannot invalidate successor |
| `check_lost_ack` | Mutation plus independent principal, authorizer and signer verifier |
| `check_unsupported_multi` | Only when the backend declares multi-tenant atomicity unsupported |

Every check resets the test database. Wrap the async call in a backend-appropriate
timeout so a deadlock fails qualification. Head and identity races use a barrier
and separate handles; assertions inspect the winner and every committed effect,
not timing. Pending exact requests are not counted as committed receipts; new
reads after the winner finishes must resolve the original record.

`Violation.check` identifies the assertion that failed. Operation/setup errors
are reported separately as `adapter.operation`. Broken variants in the reference
integration must fail their *specific intended* check, proving the suite has an
active oracle rather than merely compiling a mock.

The fixed signing key, fixture authorizer and storage code in `tests/reference.rs`
are test-only. They are not deployment adapters or trust-policy examples. The
reference supports bounded single-tenant genesis transactions, not a general
storage engine. `faults::LoseCommitAck` injects response loss after an underlying
successful commit; it does not kill a process or simulate disk flush behavior.

The [adapter contract and remaining qualification](../../docs/0.3/adapter-contract.md)
lists the precise scope and backend-specific work still required.

## Optional batch tests

For an adapter that supports `LedgerTransaction::retain_batch`, its connection
must also implement `LedgerBatchRecovery`. Implement `batch::LedgerBatchTestBackend`
in addition to `LedgerTestBackend`. Its `inspect_batches` reads actual retained
membership rows through an independent connection and returns a `RetainedBatch`
for **each** member, including any duplicates. Do not build these observations
from `lookup_batch`, an expected fixture or a cached receipt. Each row includes
the exact ordered original intent bodies and complete immutable effects.

Use well-formed, genuinely signed `LedgerBatch` fixtures with at least two
members, distinct member identities, complete deterministic projections and
outbox, all in one tenant. The independent fixture builder in `tests/reference.rs`
combines two intent/event pairs under a newly signed commit. It does not import
the root mutation-test adapter.

| Function in `batch` | Checks |
| --- | --- |
| `check_atomic_visibility` | Every write prefix, retained metadata and all member finalizations remain private; complete publication and reopened lookup |
| `check_rollback` | Roll back each prefix, then retry every exact member successfully without resetting first |
| `check_cancellation` | Drop each completed prefix; no effects or committed group receipt; unfinished leases may remain |
| `check_replay` | Each member replays the original commit; changed payload conflicts; original complete group remains unchanged |
| `check_head_contention` | 2, 8, then 32 distinct complete groups against one genesis head; exactly one whole winner |
| `check_idempotency_contention` | 2, 8, then 32 exact concurrent groups; one writer and only complete replay or pending results |
| `check_tenant_isolation` | Distinct tenant genesis groups with coincident member/event/resource/commit IDs; no lookup leak or identity conflict |
| `check_single_namespace` | A standalone mutation whose full intent exactly matches one batch member; shared claim status in both directions and no cross-kind receipt fabrication |
| `check_lost_ack` | Independent principals/policy/verifier; post-commit response loss, unknown complete identity, fresh-handle recovery and exact replay |

The reference integration includes nine deliberately broken batch behaviors:
early event publication, early membership publication, forgotten outbox,
forgotten membership, forgotten finalization, filtered single-leg lookup,
duplicate accepted identity, ignored head CAS, and discarded commits. Each must
fail its named oracle. All older single-mutation controls still run.

Namespace checks add four deliberate fault cases: global tenant keys, fabricated
singleton groups, group members returned as standalone mutations, and ignored
committed batch identities. Wrong-kind lookup must return an invariant error;
returning absence or a plausible record is not sufficient. The exact-group race
uses independently supplied principals and `Arc`-owned authorizer/verifier
policies so each task can call the actual verified coordinator.

The batch extension remains bounded logical qualification. The optional lease
checks below add selected expiry/takeover schedules. Broader fencing races,
later-head retention/rebuild and physical database crash/power-loss qualification
still require additional adapter tests. Do not
claim those properties merely because these functions pass.

## Optional expiring-lease tests

Implement `LedgerLeaseTestBackend::expire_reservation` using an injected clock
and the backend's normal expiry machinery. It must not delete the reservation,
replace its token, or release the original transaction. The next ordinary claim
must perform the takeover. `check_lease_takeover` keeps two independent write
transactions alive, stages the complete mutation in each, and independently
checks stale commit rejection, stale rollback and stale drop. Each scenario
requires the successor reservation and complete committed original to survive.
Two deliberately broken controls accept a superseded commit or release the
successor during stale cleanup; both must fail their specific assertion.

This optional test assumes concurrent visible reservations and expiring leases.
Adapters with transaction-local reservations or database locking need their own
schedule and backend-specific timeouts; they must not simulate this capability
by deleting locks. The reference uses deterministic logical expiry, not elapsed
wall time. The batch extension below supplies separate optional schedules;
transaction timeout, physical recovery and all lease interleavings remain open.


## Optional batch lease schedules

`batch::check_lease_takeover` uses the existing `LedgerLeaseTestBackend` hook
plus raw `LedgerBatchTestBackend` observations and `LedgerBatchRecovery`. Supply
a well-formed batch with at least two members and a signed commit. The check
stages a complete old group, verifies every live reservation, expires every
member, and acquires/stages the same group through a new connection. Each
member must have a new nonempty token and the original payload digest.
Stale commit, rollback and drop are separate schedules. None may publish old
effects or erase any successor claim. The successor must publish exactly the
original complete group, including every retained member, projection, outbox,
finalization and canonical head. Reopened member lookups must resolve the group.

This works with shared group deadlines. When one injected-clock advance expires
several siblings, `expire_reservation` must tolerate a later callback for an
already-expired sibling. This is test-hook behavior, not an extra production
storage method. No callback may manufacture replacement tokens or delete locks.

For adapters that also support independently expirable member leases, implement
`batch::LedgerBatchMemberLeaseTestBackend::configure_member_expiry`. It is called
after reset and before any claim. Configure the normal test clock/lease policy
so the chosen member expires while every other member remains live; do not edit
reservations or tokens to fake takeover. Shared-deadline-only adapters need not
implement this capability. Setup errors are `adapter.operation`, distinct from
a failed contract assertion.

Then run `batch::check_member_lease_takeover`. It replaces each member in turn
while the others stay live, checks the returned claim token against independent
pending lookup, and applies stale commit/rollback/drop separately. The partial
successor remains pending and is rolled back: claiming one member never counts
as a committed group. Raw effects and retained rows stay empty. The whole-group
check alone does not establish this independently expired-member property.

The reference exercises a two-member signed-commit fixture. Its shared-expiry
model passes the whole-group check and explicitly refuses unsupported isolated
configuration. Five deliberate lease fault behaviors fail named oracles:

| Fault | Required oracle |
| --- | --- |
| Accept a superseded whole commit | `batch.lease.stale_commit` |
| Fence only the first member | `batch.lease.stale_member_commit` for member 1 |
| Release successor during stale cleanup | `batch.lease.successor` |
| Reuse the old token on takeover | `batch.lease.token` |
| Return a claim token different from pending lookup | `batch.lease.claim_token` |

A first-member-only fault deliberately passes whole-group takeover but fails
isolated takeover of member 1. This is an explicit negative-control boundary,
not a claim that the smaller check detects every broken adapter. These are
bounded logical genesis schedules with independent readers, not real database
clock, process-kill or power-loss qualification. Wrap calls in backend-specific
timeouts. Broader histories, larger groups, further interleavings and physical
backend recovery still need qualification.
