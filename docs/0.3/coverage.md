# Hosted coverage qualification

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
