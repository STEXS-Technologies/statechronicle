# Refactor baseline and qualification status

The authoritative plan is [ROADMAP.md](ROADMAP.md). This file tracks execution;
unchecked work is not release evidence. Publishing, tags, GitHub CI dispatch and
pull requests require separate approval. Local checks are permitted.

## Reference revisions

- Original release: `v0.2.0`, `c418e98b91720188158c531cd5c2c4b8ab71cbcf`.
- Refactor starting point: `ae3d94bc22c7835eaa3fdcd766e3c117929c12bb`.
- Working branch: `refactor/coherence`.

The starting point differs from the release in `Cargo.lock`, workspace and
portable dependency declarations, and the release-notes workflow. Production
Rust sources are identical. Dependency changes must be accounted for separately
when comparing performance and compatibility.

## Reproduce the original finite baseline

```sh
RUSTC_WRAPPER= CARGO_BUILD_JOBS=8 python3 scripts/refactor_baseline.py
```

The runner verifies the release tag against the pinned SHA, exports that exact
Git archive into a new local directory, and runs original formatting and
all-target/all-feature workspace tests with the original lockfile. Every attempt
has its own manifest and compressed logs under `evidence/baseline/`. It neither
updates the lockfile nor regenerates original fixtures. An optional
`--release-checks` additionally runs the original release script, including its
instrumented fuzz smoke. It requires the tools that script invokes.

The extracted source lives under ignored `target/refactor-baseline/`. This is a
reference snapshot, not a candidate checkout. Test build output is local and not
committed. Manifests hash every original tracked file and the runner, identify the
installed compiler and command, and retain actual exit codes. Tracked Rust
file/byte counts include comments and tests and must not be presented as
production-only LOC or complexity measurements.

The first attempt failed before compilation because the configured `sccache`
wrapper could not start. It remains recorded as failed. A separate attempt with `RUSTC_WRAPPER` empty passed formatting and
794 tests across 62 test binaries. See
[`20261116T014840Z-2747068/manifest.json`](evidence/baseline/20261116T014840Z-2747068/manifest.json).
This does not establish doctest, lint, coverage, fuzz or performance gates.

## F0 execution status

- [x] Original finite checks captured; original advisory failure is explicitly retained.
- [x] Frozen external consumer matrix and one-command cross-version runner.
- [x] Original-version signed history/proof/canonical-byte corpus and frozen recovery/receipt tests.
- [x] Parser-derived syntax footprint and initial invariant/duplication ownership map.
- [x] Coverage, doctests, stable/MSRV portable builds, bounded fuzz and recovery evidence.
- [x] Original economy and cold facade build/binary/dependency-graph observations.
- [x] Reproducers for absence expectations, marker-only durable replay and trade tenant substitution.
- [x] Complete rustdoc API snapshot, including generated items and public trait signatures.
- [x] Workload-specific latency/allocation baseline and initial variance/dependency-control measurements.

F0 baseline capture is complete, with the explicitly recorded original advisory
failures and measurement limitations below. F1 is in progress; F2–F8 remain
pending. This is not
release qualification: final paired performance disposition and exact-candidate
gates remain required. F1 now binds trade commits to tenant scope and adds
independent current/historical expected-claim verification; see
[findings](findings.md) for remaining durability and trade work.

## Additional local baseline evidence

The original release-check script passed formatting, all 794 tests, strict
Clippy, workspace check, both independent embedded crate checks, and strict
rustdoc. It stopped at cargo-deny's advisory gate for original `rustix 1.1.4`
(`RUSTSEC-2026-0486`) and `tokio 1.53.2` (`RUSTSEC-2026-0386`). The failed gate is
preserved in `evidence/baseline/20261116T015301Z-2774962/`. The original lockfile
has not been patched and the original script therefore did not reach fuzz.
Separate fuzz evidence must be evaluated independently.

The external matrix lives in `ci/compat-0.2/`; the original-history corpus is in
`ci/wire-0.2/`. Run the full external hosted/portable matrix with:

```sh
RUSTC_WRAPPER= CARGO_BUILD_JOBS=8 python3 scripts/refactor_compat.py --portable
```

The runtime corpus has an explicit original-release producer, independent
reader and pinned hash. Frozen original tests are byte-for-byte checked against
Git, not just their local provenance manifest. Original and candidate
third-party packages must retain their respective locked versions/checksums.
The external portable consumer uses its own no-std domain, bounded buffers and
bare-metal compile, with no umbrella or hosted feature unification.

## Reproducible source metrics

```sh
uv run --python 3.13 --with tree-sitter==0.25.2 --with tree-sitter-rust==0.24.2 \
  python scripts/test_refactor_metrics.py
uv run --python 3.13 --with tree-sitter==0.25.2 --with tree-sitter-rust==0.24.2 \
  python scripts/refactor_metrics.py --revision v0.2.0 --output docs/0.3/metrics.json
```

The baseline has 113 tracked Rust source files (1,348,105 bytes). After excluding
comments, attributes and exact test-only items, the parser measures 14,137
syntax-bearing lines, 105,588 lexical tokens, 1,676 branch syntax nodes and 1,221
public syntax items. These are syntax metrics, not executable LOC, effective
public API size or cyclomatic complexity. Conditional production alternatives
are all counted; macros and complex test cfg expressions require further audit.
Source hashes and parser versions accompany the per-file inventory. Eight exact
function duplicate groups are detected, mostly newtype trait boilerplate plus
profile quantity and holder helpers; semantic duplication is not yet measured.

The refactor starting production source is byte-identical to the release.
`evidence/starting-metrics.json` records the comparison. The system Python 3.14 /
tree-sitter 0.26 combination produced a crash and invalid source locations; its
output was discarded. Only the pinned Python 3.13 parser output is retained.

The full hosted/portable external matrix passed for both original and starting
candidate revisions in `evidence/compat/20261116T020221Z-2835838/`: all ten
resolution, hosted test, portable test and bare-metal checks passed. The
exhaustive enum matches compiled for both revisions. The separate original
wire-producer run and cross-version readers passed in
`evidence/compat/20261116T015951Z-2814374/`. Earlier harness failures are retained
and are not counted as passing qualification.

All 25 original fuzz targets completed a separate instrumented 60-second-per-
target campaign with zero exit codes in
`evidence/baseline/20261116T020259Z-2851508/`. The manifest records nightly,
budget and exact source hashes; the compressed artifact retains individual logs,
statuses and corpora. All original tracked source hashes were rechecked after
the campaign and remained unchanged. This is bounded exercised-input evidence,
not proof of universal correctness.

## Remaining baseline qualification and build cost

`evidence/baseline/20261116T020832Z-2906802/` passed the original coverage
ratchet, unfiltered report generation, doctests, Rust 1.85 independent portable
crate checks and economy benchmark. Observed protocol-scope line coverage is
94.29%; unfiltered workspace line coverage is 90.93%. These current compiler/
tool observations differ from historical release numbers; the original
exclusions and ratchet were retained. The benchmark observed 4,695,042 ops/s
and ten correctness runs for each of the six original economy scenarios. It is
a shared-host observation, not a noise-qualified performance comparison.

`ci/cost-consumer/` supplies a fixed external public-facade workload with only
one StateChronicle dependency. `scripts/refactor_costs.py --revision <sha>`
builds it in a fresh target directory using the revision's exact dependencies,
records the dependency/feature graph, compiler artifacts, CPU affinity, elapsed
time, maximum compiler resident memory and final binary size/hash. The original
release observation in `evidence/costs/20261116T021527Z-2997189/` compiled 74
packages in 6.61 seconds, with maximum resident memory of 297,848 KiB and a
333,392-byte stripped binary. Registry/filesystem caches were warm; this is one
shared-host sample. It cannot establish a regression threshold.

The initial [findings](findings.md) and [maintenance inventory](maintenance-inventory.md)
identify verified trust-boundary failures and the owners of repeated flows.
They distinguish observed behavior from remaining hypotheses. Test overlays are
recorded separately when probing original production code.


The initial [workload cost model](performance-baseline.md) now records three
pinned-CPU rounds for fifteen fixed workloads and external allocation probes.
Native and portable transitions added zero allocations relative to identical
setup. Hosted typed check/apply added 23 allocations / 362 bytes per operation.
See the raw reports and explicit remaining comparison scope; these observations
are not full performance qualification or a universal regression threshold.

The expanded pre-refactor run in
`evidence/compat/20261116T024032Z-3384369/` passed all twenty measurement checks
for twenty-two workloads in each lane, including current/head-mismatch proofs,
authenticated batch preparation, cross-tenant trade planning/verification, and
a 256-event checkpoint/resume/root workload. The [cost model](performance-baseline.md)
records precise setup and authority limitations and the final paired-run policy.

The expanded fixture set also passed the complete ten-check hosted/portable
external matrix in `evidence/compat/20261116T024127Z-3400784/`, including original
wire readers, legacy consumer tests, independent portable tests and bare-metal
compilation for both revisions.

Hosted Rust 1.85 qualification was additionally attempted during F1 and found
pre-existing unsupported let-chain syntax in the unchanged core rate limiter.
The failed log and original-source identity proof are retained in the F1
evidence. Portable MSRV success does not cover that hosted path; this remains
a final release gate, with no approved MSRV increase.
