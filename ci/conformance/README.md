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
| `check_lost_ack` | Independent principals/policy/verifier; post-commit response loss, unknown complete identity, fresh-handle recovery and exact replay |

The reference integration includes nine deliberately broken batch behaviors:
early event publication, early membership publication, forgotten outbox,
forgotten membership, forgotten finalization, filtered single-leg lookup,
duplicate accepted identity, ignored head CAS, and discarded commits. Each must
fail its named oracle. All older single-mutation controls still run.

The batch extension remains bounded logical qualification. Same-group contention
is currently tested in the root protected API suite, not this generic module.
Batch-specific tenant isolation, cross-lane single/batch identity collisions,
lease expiry/takeover races, later-head retention/rebuild and physical database
crash/power-loss qualification still require additional adapter tests. Do not
claim those properties merely because these functions pass.
