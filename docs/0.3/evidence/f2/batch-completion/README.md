# Compatibility batch completion

Retained production: `1e995abc29af266b487654f32fcf321c4b335e7a`.
Pre-extraction control: `404dda5817702af2f7ca8700e1a4e2da812e7598`
(production identical to previous checkpoint `84287bd`). Actual original release:
`c418e98b91720188158c531cd5c2c4b8ab71cbcf`.

## Implementation and authority

Four compatibility coordinators formerly repeated completion: ordinary batch,
same-tenant settlement, inferred cross-tenant linkage and manifest-declared trade.
Each selected either a symbolic transaction handle or a compatibility durable
sink. They never held both; the sink-error branches that attempted handle rollback
were unreachable. The private `BatchCompletion` enum now represents that choice.

Its lazy `begin` calls the supplied transaction constructor only in the symbolic
route, preserving both callback and boxed-future allocation behavior. `succeed`
checks nonempty events before invoking a sink, or commits the symbolic handle.
`refuse` formats the primary error before rollback and preserves that error if
rollback fails, logging the same warning message/fields. Commit failures remain
store errors with no extra rollback. Sink failures remain direct store errors.
No complete event set is returned before its selected completion succeeds.

Coordinators keep ownership of their result values, explicit routing/linkage
validators and policy-specific refusal messages. Cross-tenant groups are flattened
and cloned at the same point, including on planning routes. Two private helpers
also share sorted tenant partitioning and settle-intent selection, preserving
input order, clones and allocation order. Existing marker writes remain inside
legacy leg execution and retain their historical partial-failure behavior.

This is compatibility orchestration. It does not replace LedgerStore, improve
sink atomicity, manufacture a canonical receipt or make symbolic rollback undo
independent markers. Protected ledger/recovery APIs and crypto/wire contracts
are unchanged. There is no new public API, dependency or trait framework.

## Independently frozen effects and errors

`ci/characterization/settlement.rs` runs **10,128 rows**: eight routes × six
structures × 211 masks (no fault, each of 20 faults, every pair). All eight normal
controls must emit their expected complete events/groups. Built-in scenarios
include real signed mints, same-tenant asset/value settlement, linked cross-tenant
transfers and declared asset/value trade. Structures include empty, truncated,
reversed, duplicated and collapsed-tenant inputs. Fixed projections are sentinel
observations, not a verified durable history.

Faults cover begin unavailability/abort, first/last crypto failure, index failure,
first/last marker replay/conflict/read/write, sink refusal, commit/rollback failure,
version mismatch/overflow, duplicate IDs, linkage/value mismatch and expiry.
First/last refer to input positions; cross-tenant execution remains sorted.
Each row freezes complete error Debug/Display or canonical events/group digest,
ordered reads/clock/IDs/marker writes/transaction/sink calls, claimed payloads and
consumed IDs. Rollback warning fields are collected with a fixed subscriber;
field names are sorted, while callback order and errors are not normalized.
Source-location tracing metadata is outside the comparison.

The [retained original/control/candidate matrix](
../executor-order/20261116T115404Z-2272634/manifest.json) passes all rows exactly;
all traces have SHA-256
`ee9b6315eb1beb0e46cb64ff33e5d990b6c65a0ecc6c561bbd4c2c2a3224a2ed`.
The first [pre-production comparison](
../executor-order/20261116T114010Z-1853989/manifest.json) already has this digest.
Source/archive, fixture, compiler, lock, binary and compressed output hashes are
recorded with commands and terminal exits. No new registry packages or mutated
source/locks are allowed.

The [negative control](../executor-order/20261116T115544Z-2324867/manifest.json)
changes the temporary candidate archive's one rollback call to commit. Both lanes
compile and finish; exact comparison fails on **2,556 rows**, covering all four
symbolic routes. Returned errors are unchanged in all those rows: the ordered
side-effect trace detects the defect. The worktree and original are unchanged.

Retained regression matrices also pass [23,544 executor rows](
../executor-order/20261116T115509Z-2307161/manifest.json) and [2,352 proof-service
rows](../../maintenance/proof-service/20261116T115532Z-2319150/manifest.json).
[Root qualification](qualification-1e995ab/manifest.json) passes formatting,
**884 finite tests, 16 doctests**, all-target strict Clippy, strict rustdoc and
source identity. [External original consumers](
../../compat/20261116T115404Z-2272546/manifest.json) pass frozen tests, portable
tests and bare-metal compilation. The package file listing includes both new
private modules; it does not replace final unpacked-crate qualification.

## Source footprint

Pinned metrics and runner are retained here. Versus `84287bd`, production loses
**3,412 raw bytes, 70 syntax lines, 375 tokens and 27 parsed branches**. The three
affected files, including the new owner, move from 6,196 to 5,821 tokens (-6.05%)
and 107 to 80 parsed branches (-25.23%). Four additional public-syntax declarations
are `pub(super)` items in a private module, not exported API. Whole-production
totals are 1,500,803 raw bytes, 16,531 syntax lines, 121,251 tokens, 1,958 parsed
branches and 1,452 public-syntax items. Counts exclude exact test-only items and
comments; they do not measure compiled complexity or macro-expanded API. The
broader flat-footprint/hotspot target is not proved by this extraction.

## Matched observations and retained design

The generic result-owning prototype (`9c3cb10`, following initial `e8d4c8f`)
passes semantics but shows a cross-trade p95 concern: +15.20% in its [first
campaign](bench-9c3cb10-20261116T114658Z/manifest.json), +2.87% in its [repeat](
repeat-9c3cb10-20261116T114907Z/manifest.json), with all seven repeat tail deltas
positive. It is not retained. The current design keeps results in coordinators
and shares only unit success/refusal completion, removing generic Cow/result
closures from the async path.

[Retained campaign](bench-1e995ab-20261116T115040Z/manifest.json) and [reverse-first
repeat](repeat-1e995ab-20261116T115202Z/manifest.json) use the same respective
binaries, byte-identical frozen workloads, compiler and resolved locks, a fixed
CPU, seven alternating pairs each and no other qualification jobs from this
session during measurement. The repeat records system conditions. No samples or
outliers are discarded. Percentiles are of 31 batch averages, not request tails.

| Workload | Median change, first / repeat | p95 change, first / repeat |
| --- | ---: | ---: |
| Single-intent control including setup | +0.586% / -0.078% | +0.640% / -0.448% |
| Two-intent authenticated batch to refusing sink | +0.842% / +0.162% | +2.587% / +3.804% |
| Two-intent planning including setup | -0.039% / +0.033% | -0.901% / -0.484% |
| Cross-tenant trade including seeds | -0.150% / -0.195% | -3.617% / -5.219% |

The generic prototype's trade-tail concern does not repeat in these retained
observations. **Refusing-sink batch p95 remains higher in both retained campaigns**;
this remains an open performance concern for the release matrix. Small medians,
large shared-host outliers and setup costs do not establish speedup or universal
neutrality. The retained consumer binary grows 4,088,064 → 4,102,024 bytes
(+13,960, about 0.341%); hashes and build paths are recorded. These observations
are not a complete performance-gate pass.

[Valgrind full-process totals](allocations-1e995ab/manifest.json) match exactly:
101,835 allocations / 8,401,657 bytes; 130,381 / 10,449,594; 122,625 / 9,638,986;
and 144,907 / 11,584,203, respectively. Processes include fixed startup/setup,
ten warmups and 31 samples (five actions each, two for trade). Equal-length copied
binary paths and hashes are checked. This is not a per-operation or arbitrary-input
allocation guarantee.

## Development and qualification history

All evidence is retained. Initial fixture attempts refused dependency drift from
unneeded tracing attributes ([manifest](../executor-order/20261116T113715Z-1771292/manifest.json)),
then exposed a missing Result annotation ([manifest](../executor-order/20261116T113739Z-1782371/manifest.json)),
three fixture-only collapsible ifs ([manifest](../executor-order/20261116T113817Z-1799839/manifest.json)),
and invalid expiry fixture construction ([manifest](../executor-order/20261116T113926Z-1834371/manifest.json)).
The corrected expired request has creation before expiry, both before fixed now;
its actual signature is valid. No production or frozen golden was altered to pass.

`qualification-e8d4c8f` and `qualification-9c3cb10` qualify their own prototype
sources, not the retained source. Earlier matrices, external consumers and
negative controls are separately retained. The retained parity wrapper initially
asserted because its broad new-manifest search also saw unrelated manifests
created by parallel qualification. The child settlement matrix had already passed.
`parities-1e995ab/runner-recovery.py` scopes discovery to the fixture directory and
finishes the remaining lanes without rerunning the successful first lane. The
original wrapper source is retained; this is an evidence-orchestration correction,
not a semantic comparison failure.

Full F2 mutation/settlement composition audit, broader F5 histories/backends,
F6 whole-repository budgets and F8 extended properties/fuzz/crash, performance,
packaging/version/release gates remain open.

## Later performance checkpoint

The [four-lane and individual-request campaign](../../performance/README.md) retains the earlier batch-average concern and adds a separate observable. Individual refusing-batch p95 does not show a consistent slowdown against the pre-completion control; planning tails and shared-host variation remain unresolved. This does not close the full performance gate.
