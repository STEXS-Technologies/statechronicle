# Seeded concurrent mixed histories

Source: `04a1bcd59c57478cdf2673be3ed8facbc5a61088`.
Parent: `65e9af706e109a8c777a9060d09542c3a59fe651`.
The production libraries, root lockfile and toolchain are byte-identical to the
parent. This checkpoint changes the isolated unpublished dev conformance package
and documentation; it introduces no production dependency, API or wire change.

## Actual qualification

[The archive campaign](manifest.json) retains exact source/archive hashes,
commands, environment, compiler identity, compressed raw logs and real exit codes.
Its first five checks passed: formatting, all 26 tests, doctests, strict Clippy,
and strict rustdoc. All targets/features were checked using the retained isolated
conformance lock. Existing deterministic single/batch, rollback, contention,
retention, lease and negative-control tests also ran.

The campaign's **overall status is failed**, because its sixth check attempted
an ineffective deadlock mutation. Delaying collection of completed commit tasks
does not stop those tasks from committing. That mutation's test exited 0, while
the driver expected 101; the driver exited 1 and rejected the attempted negative
control. This failed attempt is retained without reclassifying it as a pass.

[The corrected isolated deadlock control](deadlock-control/manifest.json) inserts
an actual commit barrier requiring all wave writers. A serialized-writer adapter
then cannot release its first write lease before another writer stages. The
existing serialized-writer test detects the deadlock through its outer timeout:
the patched test uses a two-second timeout, fails at runtime with `Elapsed(())`,
and returns 101. Its archive and pre-mutation source hashes match the positive
campaign. The production harness has no such barrier and passes the same test
with its normal thirty-second bound. No production mutation is retained.

## Coverage and boundaries

Eight positive reference seeds are `0`, `1`, `7`, `42`, `7331`, `65537`,
`18446744073709551614`, and `18446744073709551615`. They generate respectively
15, 13, 16, 15, 14, 15, 15, and 16 waves. Each schedule contains 22 transactions,
12 singles and 10 two-member batches, over two interleaved signed tenant chains:
32 intents/events, 64 outbox rows, eight final projection keys and two heads.
The fixture independently verifies signed chains/roots before adapter use.
Snapshots come from those fixture histories, not adapter receipts or inspection.
A ninth positive schedule uses an exclusive single-writer semaphore.

Wave participants start together through a barrier. Seeded cooperative yields
occur after claims and staged writes, before commits. Each completed staged
writer is inspected independently and released promptly; other writers may
remain blocked. Staged effects must remain private within that tenant, receipt
lookups must not claim commitment, and current group membership must remain
unpublished. Independent raw observations after commit completions admit each
tenant's complete before or after state, never a partial set or foreign rows.
Each completed wave checks its exact expected authority and membership, then
every original identity, conflicting retry and single/batch namespace boundary.

Four real broken backend modes fail their specific checks, asserted by the
reference test and retained in `all-tests.log.gz`:

| Broken behavior | Required violation |
| --- | --- |
| Publish events before commit | `mixed.concurrent.staged_effects` |
| Batch commit discards single receipts | `lookup.original` |
| Single commit discards batch membership | `mixed.membership` |
| Append duplicate current projections | `mixed.concurrent.visibility` |

Four invalid schedules (snapshot length mismatch, empty input, duplicate scoped
identity, and competing entries for one tenant within a wave) refuse before
reset, preserving an independently seeded anchor's events, commit, projections,
outbox, finalization, head, original lookup and empty membership.

The seed reproduces fixture grouping and yield counts, not the exact executor,
OS or database interleaving. The reference uses async tasks and fresh handles
over one in-memory authority; the serialized variant uses a semaphore, not a
physical database. Individual commit implementations already yield before their
atomic publication. Reader observations are consistent backend snapshots and
are not a timing oracle or a guarantee of observing every in-flight state.
These campaigns supplement separate 2/8/32 same-tenant races and deterministic
failure-prefix checks. They do not prove all interleavings, multi-tenant atomic
transactions, process/database restart, power loss, or production signer policy.

[Checkpoint verifier](verify-checkpoint.py) checks raw log hashes, all test/seed/
negative-control markers, the failed initial control, the corrected runtime
failure, matching immutable source/archive hashes and the unchanged libraries.
No performance improvement or full F5/F8 completion is claimed.
