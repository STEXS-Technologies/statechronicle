# Hosted coverage qualification

Current qualification is tracked in the [final release audit](evidence/qualification/final-94eb5d4/README.md). Dated results and pending statements below describe their original checkpoints; the final audit records subsequent evidence and remaining work.

Run `RUSTC_WRAPPER= CARGO_BUILD_JOBS=8 bash scripts/run_coverage_json.sh`,
then `bash scripts/check_coverage_ratchet.sh`. Requires cargo-llvm-cov,
LLVM tools, Python 3 and jq. The collection driver runs its parser tests,
uses the committed lockfile, and excludes the unbounded fuzz binaries.

The driver exports unfiltered `target/coverage-full.json` and historical
filtered `target/coverage.json`. The ratchet uses weighted covered/total
line counts and rejects malformed records and invalid baselines. Zero-line
records are allowed alongside measured records. The 92.4% baseline, tolerance,
and thirteen-file exclusion expression are unchanged.

Retained evidence under `evidence/coverage/` identifies the exact measured
commit, compiler, reports, commands and logs. Negative-control logs retain
failures against the preceding parser; these are expected failures, not a
passing qualification of that implementation.

These are hosted instrumented line counts. They do not establish branch or
MC/DC coverage, bare-metal execution, extended fuzz/crash qualification, or
completion of the roadmap. Filtered coverage must be reported alongside the
unfiltered scope. Historical measurements with different source layouts,
compilers or dependencies are not matched comparisons.

## Current retained source measurement

[Source `8faaa14`](evidence/coverage/hosted-8faaa14/README.md) passes collection
and the unchanged 92.4% ratchet: full hosted line coverage is **92.8701%**
(18,913/20,365), filtered coverage **94.4119%** (15,493/16,410). The isolated
archive runs all four parser tests and 897 finite workspace tests with source
and lockfile drift guards. A separate 95% test baseline is rejected with exit 1.

The shared string-trait macro reports 21/21 lines in its new source module;
LLVM line attribution does not establish per-instantiation branch coverage.
The exclusion expression is unchanged. Source-layout and test changes prevent
using the historical percentage difference as a matched improvement claim.
The remaining release gates still require their own exact-candidate evidence.
