# Canonical claim verifier fuzz checkpoint

Candidate `c8ff30543fe4070e09bac611b567a152ed665f17`; installed nightly
Rust 1.102.0-nightly (`5f4effd57`, 2026-11-13), host x86_64 Linux.
Production libraries, workspace dependency manifest/lock and stable toolchain
are byte-identical to the previous complete local driver checkpoint `5b1980c`.
This checkpoint adds dev-only fuzz code, fixed inputs, binary checkout attributes
and the campaign tool. The manifest now enumerates **26** fuzz targets.

The new target directly invokes the public `ClaimVerifier` through a minimal
read-only adapter. Each input signs an empty-root absence checkpoint and a
subsequent presence checkpoint using a data-derived Ed25519 key. Genuine current
presence and pinned historical absence are checked even for empty input. Up to
32 input bytes then select probes for valid claims, stale absence, independently
wrong resource/tenant/owner/key/pin, corrupt leaf evidence, missing or changed
head tuple members, advancement during commit lookup, historical verification
without head support, and immutable ownership of the returned verified payload.

The two positive ASAN campaigns started from independent copies of all 18 fixed
inputs. Every choice has a one-byte seed; forward/reverse full sequences exercise
all choices in a single invocation. Limits were one million executions or
60 seconds, with input length at most 64 bytes. libFuzzer completed normally at
its time limit:

| Seed | Executions | Reported seconds | Final counter coverage | Final feature count |
| --- | ---: | ---: | ---: | ---: |
| 7331 | 6,219 | 61 | 2,364 | 9,081 |
| 42 | 6,450 | 61 | 2,364 | 9,086 |

These are instrumentation counters, not a percentage of library coverage or
latency measurements. Both runs exited 0 with no crash artifact. Raw logs and
complete final corpora are retained. Seed inputs are in the committed source.
An early development run rejected the harness's invalid `claim_before` ID;
the committed fixture uses validated `cmt_`/`evt_` prefixes. That development
fixture error is not a product finding or a passing campaign.

Two real insecure-source controls were compiled and executed in the isolated
archive, with the live checkout untouched:

- Bypass the final canonical-head check. The head-advance probe exits **77** at
  `claim.current.head_advance`.
- Bypass the independent historical commit pin. The pin-substitution probe exits
  **77** at `claim.historical.pin`.

Both builds exit 0; a build failure cannot count as the intended control failure.
After each probe the exact original source is restored, and all archive source
and lock hashes must agree. Stable root formatting and strict Clippy for the new
target pass on the same source. Existing library tests are not presented as a
new run here; the production-source identity allows their prior scope to remain
traceable without implying new fuzz coverage from those tests.

Reproduce from the repository root:

```sh
python3 docs/0.3/evidence/fuzz/claim-verifier-c8ff305/runner.py --candidate c8ff30543fe4070e09bac611b567a152ed665f17 --output <new-directory>
python3 docs/0.3/evidence/fuzz/claim-verifier-c8ff305/verify.py
```

`runner.py` is byte-identical to the tool in the pinned commit. The manifest
records commands, compiler, environment, archive/input/seed/binary hashes,
actual exits, budget/counts, negative source hashes and artifact hashes. The
independent verifier checks the actual Git archive and runner, recomputes input
and seed maps, validates logs and final corpora, reconstructs both source
mutations, and checks stable results. `qualify.py` rechecks stable formatting and
strict target Clippy only when live sources match the campaign.

Scope is two signed checkpoints and a controlled reader/head-advance hook, not
a validated complete event history, OS concurrency, arbitrary backend consistency
or physical durability. Host signer authorization policy remains independently
supplied; wrong-key rejection does not qualify revocation policy. This closes a
specific missing public claim API fuzz lane. Protected durable receipt/recovery
fuzzing, longer all-target campaigns and the remaining final roadmap gates are
still open. No production dependencies, wire formats or library APIs change.
