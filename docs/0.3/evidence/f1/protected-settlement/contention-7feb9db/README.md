# Contested canonical economic groups

Test/fixture source: `7feb9dbe356f26c98fe98ee6e5db1e98d2cbcec3`.
Production library source is unchanged from `f6cd75d`; this checkpoint changes
only integration tests and their in-memory ledger adapter.

## Scenario and assertions

The fixture creates and signs canonical genesis asset and per-buyer balance
records, then commits the asset lock as the next head. Preparation reads an
index populated only after those confirmed commits. Every contender is a
different independently authenticated seller/buyer request pair with distinct
intent and commit identities against that same locked asset and head. One atomic
group releases ownership and transfers 100 gold from the winning buyer to the
seller, including the companion destination balance event.

The positive run starts 2, 8 and 32 OS threads through a barrier, each with its
own current-thread async runtime and cloned backend handle. Exactly one group
wins. Other final-commit failures retain typed canonical-head conflicts inside
`Unknown`; lookup through a fresh handle returns `NotFound` for those exact
losing groups. Stale exact-group retries remain refused. Winner recovery and
retry return the complete original record without extra effects.

An independent reader inspects authoritative snapshots during contention. The
retained run has 8, 8 and 17 observations respectively; these counts depend on
scheduling and are not pass thresholds or performance measurements. Each
snapshot checks all buyers' balances, seller credit, asset owner, full effect and
identity counts, projection hashes and their source events, commit signatures,
event Merkle roots and an independently addressed full state root. The reader
accepts only the complete seeded state or one complete economic winner.

Thirteen injected boundaries cover begin, both claims, events, commit row,
all three projections, outbox, both finalizations, group retention and final
commit. Every failed attempt preserves the exact pre-existing events, commits,
projections, outbox, identities and retained records. Original-group recovery
and retry then produce exactly one winner. Modeled lost ACK preserves the full
original identity, recovers the original signed group and never adds effects.
A current deny policy blocks both retry and recovery before any ledger call,
without rewriting the canonical outcome. Authenticated cross-tenant group
preparation and the unsupported multi-tenant storage capability both refuse
without changing seeded canonical data.

## Evidence and negative controls

[Control manifest](manifest.json) pins the source archive, commands, real exits,
logs and isolated source changes. The positive run passes all nine protected
settlement tests. Three controls each fail a selected runtime test with exit 101:

- Bypassing head, observed-state and root guards permits a nonwinning buyer's
  balance to become 900 instead of 1000. The economic snapshot assertion is the
  primary failure; a later poisoned-mutex panic in a worker is secondary.
- Publishing a payment projection before its injected failure violates the exact
  rollback snapshot oracle.
- Reporting success for a modeled lost response violates the required unknown
  outcome. This is an intentionally incorrect fixture response, not a claim
  about network delivery detection by the library.

`classify-controls.py` separately verifies the retained assertion markers;
compiler failures are not accepted as negative runtime outcomes. The `.patch`
files are deliberately incorrect isolated archive edits, never production edits.
Run the retained `runner.py` from a checkout of the pinned source to reproduce;
it creates evidence in a new directory and refuses to overwrite an existing run.

[Immutable-source qualification](../qualification-7feb9db/manifest.json) passes
893 finite workspace tests, 16 doctests, formatting, strict Clippy and strict
rustdoc. It pins all archived source bytes and checks for drift after the run.
The earlier exact-source preparation compatibility campaign remains retained
in the parent directory; no production API, wire, dependency or lock changes
occur in this test checkpoint.

## Scope

The ledger now keys typed subject-held projections correctly, instead of
collapsing all holders of a currency into one resource row. That correction is
necessary for this economic scenario. It is a test adapter with one shared
mutex-backed authority, not a database. Fresh handles share the same process
memory. It does not model subject relocation, independent database connections,
process restart, power loss, lease expiry or arbitrary backend topology. The
reader is independent test logic over the shared authority; it is not a second
physical storage replica.

This advances roadmap Scenario D for the supported single-tenant canonical path.
It does not complete generic randomized adapter conformance, schema-migration
Scenario E, final coverage/fuzz/performance gates or release qualification.
