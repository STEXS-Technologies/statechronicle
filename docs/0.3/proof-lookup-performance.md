# Proof lookup future investigation

The previous cumulative candidate showed a repeated stale-head refusal cost
relative to the original release (+33 ns median paired change). Refusal runs
before cryptographic verification, so small orchestration costs are visible.
The initial measurement used different original/candidate dependency lockfiles.

Three isolated experiments keep the candidate lockfile, frozen 22-workload
consumer, compiler and pinned CPU unchanged. Each runs seven alternating pairs
against the exact pre-fix candidate binary. Patches, binary/source hashes,
setup data and every raw sample are in `evidence/f2/proof-lookup-future/`.

| Experiment | Median paired delta | Absolute delta | Disposition |
| --- | ---: | ---: | --- |
| Add `#[inline]` to the async helper | +2.65% | +7 ns | No consistent benefit; not applied |
| Restore direct lookup at the measured caller | -7.39% | -20 ns | Supports the async-layer hypothesis; duplicate implementation not adopted |
| Return the adapter future directly; normalize errors synchronously | -12.67% | -39.1 ns | Improved six of seven pairs; production form selected for qualification |

These noisy shared-host samples support the selected design, not a universal
speedup or final release acceptance. The third variant retains one authoritative
lookup call and one error/absence interpretation, shared by all six consumers.
It returns the adapter's existing future rather than wrapping it in another
async state machine. `require_commit` interprets its result synchronously.
Store failures remain `Store`, absent rows remain `NotFound`, and all identity,
expected-claim, signature, Merkle and head checks retain their original order.
No cryptographic or authorization check was removed and no dependency was added.

The selected implementation is private. A new `pub(crate)` result-normalization
helper may appear in parser syntax counts, but it adds no exported consumer API.
Rust 1.85 passes all 103 proof tests before the full combined workspace campaign.
A development lockfile-resolution failure and strict Clippy test-fixture naming
failure are retained as unqualified attempts. Final exact-commit consumer and
performance evidence follows separately; prototype results alone do not close
the original finding or the final F8 performance gate.

## Combined candidate qualification

Exact production candidate `abd7245db0ab262a429bf01f1952c4c87531ccff`
passes five external hosted/portable/bare-metal checks on Rust 1.85
(`evidence/compat/20261116T044250Z-2789154/`) and ten measurement checks
on Rust 1.100 (`evidence/compat/20261116T044250Z-2789155/`).
The latter retains all 22 frozen workloads and allocation probes.

Two additional seven-pair campaigns retain every raw sample and assertion.
Against the pre-fix candidate with byte-identical resolved consumer lockfiles,
stale-head refusal changes by -10 ns / -3.71% median paired delta and -5.02%
median paired p95 delta; six of seven median pairs improve. Against the frozen
original (different dependency versions), it changes by +12 ns / +4.88% and
+1.14% paired p95. The earlier +33 ns / +13.78% observation is mitigated in
this checkpoint, but final F8 performance acceptance remains open.
Individual original-comparison median deltas range from -35.09% to +66.33%;
large changes in unrelated workloads also show shared-host noise. These data
do not establish a universal speedup or isolate all cumulative refactor costs.

Prototype manifests retain an old runner's incorrect generic caveat about
original/candidate lockfile differences. The enclosing prototype manifest now
records byte-identical full resolved consumer lock hashes for all three
experiments and the control. The reusable paired runner reads dependency scope
from its input manifest; built-input verification is distinct from completed
measurement status.
