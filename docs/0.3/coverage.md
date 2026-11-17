# Hosted coverage

From the repository root:

```sh
RUSTC_WRAPPER= CARGO_BUILD_JOBS=8 bash scripts/run_coverage_json.sh
bash scripts/check_coverage_ratchet.sh
```

Requires cargo-llvm-cov, LLVM tools, Python 3 and jq. The collection driver runs
its parser tests, uses the committed lockfile and excludes unbounded fuzz
binaries. It exports unfiltered `target/coverage-full.json` and filtered
`target/coverage.json`.

The ratchet uses weighted covered/total line counts and rejects malformed
records and invalid baselines. Zero-line records are allowed alongside measured
records. The 92.4% baseline, tolerance and thirteen-file exclusion expression
remain unchanged. Report filtered coverage alongside the unfiltered scope.

The [qualified source snapshot](qualification.md) measured full hosted line
coverage of 92.8701% (18,913/20,365) and filtered coverage of 94.4119%
(15,493/16,410). These are historical results, not measurements of every later
commit. Changed source layouts, compilers and dependencies prevent unmatched
percentage comparisons from establishing an improvement.

Line counts do not establish branch or MC/DC coverage, bare-metal execution,
extended fuzzing or crash safety. Shared macro line attribution also does not
prove every expansion's branch coverage. Retain new reports and logs as
build/release artifacts.
