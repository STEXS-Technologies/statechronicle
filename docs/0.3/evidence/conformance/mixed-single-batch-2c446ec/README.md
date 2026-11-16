# Growing mixed single/batch authority

Exact source `2c446ec6b1cfb8c4ce4378e4f3ff40f995cfb33f` adds the dev-only
`batch::history` conformance interface and three integration tests. Production
library code, dependencies, all lockfiles, wire data and GitHub workflows remain
identical to preceding checkpoint `a04c70c`. Rust remains pinned to 1.100.0.

The reference independently constructs a signed history before adapter use:
22 transactions, comprising 12 singles and 10 two-member batches, over 32
intents/events. Two tenants interleave with coincident textual identities and
one consecutive canonical head chain each. Both tenant histories verify against
independent tip digests and the fixture signer. Asset owner changes and
subject-held balance changes share 8 current projection keys; 64 original outbox
rows remain. Signing and fixture construction do not consult the backend.

The generic harness consumes independent raw snapshots for each committed prefix.
It checks 224 staged rollback prefixes against earlier retained effects and raw
group membership. After each commit it reopens connection handles and checks all
original receipts, exact original commit IDs, changed-payload conflicts and
single/batch namespace separation: 368 cumulative original member identities.
Expected effects include every event, signed commit, latest projection, outbox,
finalized identity and actual head index. Expected membership includes every
member's complete original group, not just its own event slice. Existing single
and batch stage/retry/normalization owners are reused.

Two new broken adapters demonstrate the added coverage:

| Deliberate backend fault | Existing controls that pass | Mixed-history refusal |
| --- | --- | --- |
| Batch commit discards single receipt authority | 32-single history; genesis batch visibility and replay | `lookup.original` |
| Single commit discards batch membership authority | Same homogeneous controls | `mixed.membership` |

The existing control calls execute on those same faulty implementations before
the mixed check, rather than merely asserting they would pass. The new setup
negative test supplies missing snapshots, a homogeneous schedule, a broken chain,
a cross-kind duplicate identity, an empty group and a foreign-tenant member.
Each refuses before destructive reset and leaves an existing anchor mutation
readable with its original raw events.

[Manifest](manifest.json) records the committed archive, compiler, source and
artifact hashes. All archived checks pass on Rust 1.100.0: package formatting,
all-target/all-feature **22 integration tests**, doctests (zero present), strict
Clippy and strict rustdoc. Raw compressed test output retains the two named
negative-control refusals and all earlier negative controls. The runner enforces
the expected integration-test count and unchanged archived source/locks after
checks. No root-workspace rerun or performance improvement is claimed for this
dev-only change. Run `python3 <path-to-this-directory>/runner.py` from repository
root to reproduce; archive/rename the existing evidence output directory first.
The runner pins the immutable source revision, uses an external temporary source
directory, and records a timeout or nonzero child exit as failure.

Development preflights passed the mixed cases before commit. Strict Clippy first
rejected the unboxed fixture enum's size; boxing its values then required fixing
a `Vec<&Box<LedgerBatch>>` coercion to `Vec<&LedgerBatch>`. Both were corrected
before the exact-source checks. These exploratory attempts are not represented
as green exact-candidate runs; their raw output was not retained as artifacts.

This advances F5's shared single/batch history coverage. It is deterministic
logical in-memory conformance, not randomized concurrency, a database/process
restart, power-loss safety, full projection-rebuild/serving qualification or
production authentication/key-policy validation. Backend consumers still supply
valid signed vectors and authoritative raw inspection. F5/F8 and the broader
0.3.0 release remain open. No PR, CI dispatch, tag or publication is included.
