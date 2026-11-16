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

`check_native_contracts.py` additionally builds a fresh external application,
checks seven intended compiler refusals with exact diagnostic codes and source
attribution, restores the valid program after each probe, and executes dynamic
schema/domain/operation checks. See [the fixture contract](../../ci/native-contracts/README.md).
A mere compiler failure is insufficient. Its deliberate valid-program negative
control must fail and is a separate manual tool qualification command.

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
[isolated package checks](package-checks.md), registry qualification and public
API review remain separate gates.
Publishing/tagging still requires separate authorization. See [the roadmap](ROADMAP.md)
and retained exact-candidate manifests under `evidence/` for scope and limits.

A [complete local run on `f5e9b2c`](evidence/qualification/local-release-f5e9b2c/README.md)
passes with immutable input guards and independently checked nested evidence:
939 Rust test executions, both tutorial applications, native contracts, strict
static/portable checks, four lock audits, dependency policy and all 25 fuzz smoke
targets. The checkpoint explicitly records the remaining canonical API fuzz and
other full-roadmap gates; it is not a release-completion assertion.

Canonical claim fuzzing additionally has a dedicated campaign:

```sh
python3 scripts/check_claim_fuzz.py --candidate <commit> --output <new-directory>
```

It archives the named revision, builds `fuzz_claim_verifier` with nightly ASAN,
then runs seeds 7331 and 42 with independent copies of the 18-input fixed corpus,
60-second / million-run limits and 64-byte input bounds. Each input verifies
real signed absence/presence checkpoints through the public minimum reader,
then probes expected-address/owner/trust/pin substitutions, corrupt evidence,
missing or mismatched head tuples and head advancement during lookup. Positive
paths run even on empty input. The target also checks ownership of the verified
payload after its input changes. Two isolated insecure-source controls must
panic at the named head-advance and historical-pin assertions; compiler failure
cannot satisfy these controls. Sources/locks must remain unchanged or be exactly
restored after deliberate mutation. Raw logs, final corpora, seeds, budgets,
actual counts and hashes are retained. This exercises read-side verification;
it does not qualify durable storage, arbitrary histories or all interleavings.

The [first canonical claim campaign](evidence/fuzz/claim-verifier-c8ff305/README.md)
passes 12,669 executions across the two seeds. Both insecure-source controls
build and then fail at the intended runtime assertion. Stable formatting and
strict target Clippy pass. The checkpoint retains its limited read-side scope;
protected durable receipt fuzzing remains a separate gate.
