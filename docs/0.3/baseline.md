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

## F0 remaining requirements

- [ ] Complete finite baseline and original release checks with current tools.
- [ ] API/feature inventory including exhaustive-match obligations.
- [ ] Frozen external consumer matrix and one-command cross-version runner.
- [ ] Independent old-version protocol/history/receipt fixtures with provenance.
- [ ] Production-only AST footprint, semantic duplication and control-flow metrics.
- [ ] Matched build, allocation, latency and binary-size baseline measurements.
- [ ] Coverage, MSRV, portable, fuzz and recovery qualification evidence.
- [ ] Reproducers or explicit equivalence arguments for roadmap hypotheses.

F1–F8 remain pending. No new API, correctness fix, performance improvement or
release qualification is claimed by the baseline scaffolding.
