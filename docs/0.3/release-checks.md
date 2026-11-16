# Local release-check orchestration

Run `scripts/run_release_checks.sh` from a committed checkout with Rust 1.100.0,
Python 3.12+, rustup and the Cargo audit/deny/fuzz tools available. Rustup installs
the selected embedded core and current nightly compiler; Cargo/advisory checks
may require network access. The script executes local commands only; it does not
create a PR, push a branch or dispatch a GitHub workflow. Existing workflows may
call it when their separately configured triggers run.

The script now includes both finite workspace tests and doctests (including
private-construction compile-fail examples), strict all-target Clippy, root
compilation/rustdoc/formatting, isolated conformance checks and embedded core
compilation. Fuzz binaries are excluded from finite test execution and compiled
by the lint/check stages instead.

Both standalone tutorials run through `check_tutorials.py`, which archives the
committed source, copies the application outside the repository and checks its
locked dependencies, finite tests, exact CLI output, strict Clippy and formatting.
Source changes must be committed before this stage. Each tutorial produces its
own exact-revision evidence manifest.

All four active lockfiles are audited with `--deny warnings`, so unsoundness,
unmaintained/yanked warnings and vulnerabilities cannot be silently accepted.
The separate cargo-deny stage enforces advisories, licenses, bans and sources.
Original-release and historical evidence locks are deliberately excluded from
current dependency-policy claims. Do not edit those to make a candidate green.

Fuzz smoke enumerates the Cargo manifest's targets. A failed or empty listing
stops the gate before success. Every target runs through cargo-fuzz with ASAN
instrumentation and 100-run / 5-second libFuzzer limits. These are libFuzzer stop
conditions, not a hard external wall-clock deadline; startup and builds can take
longer. Initial corpus replay can exceed the requested run count; some targets
finish after corpus initialization without generating a new mutation. Actual
seeds/counts remain in logs. This is smoke, not the extended final fuzz campaign. A final tracked-lock
diff check rejects dependency graph changes during the run, and whitespace checks
must pass before the script prints success.

`python3 scripts/test_release_driver.py` exercises orchestration in a temporary
repository layout using fake external tools. It verifies that the real shell
script requests required stages, that failures at every observed external
boundary stop execution before later commands/success, and that an empty
manifest refuses. This catches omissions and hidden shell exit codes; it does
not substitute for running the real Rust tests, advisory scanner or fuzzers.
The suite is itself part of the local release-check sequence.

This script is one qualification component. A successful run does not establish
the full roadmap: frozen independent compatibility, full fuzz/property/crash
campaigns, coverage ratchets, matched performance/source-cost measurements,
package/registry qualification and public API review remain separate gates.
Publishing/tagging still requires separate authorization. See [the roadmap](ROADMAP.md)
and retained exact-candidate manifests under `evidence/` for scope and limits.
