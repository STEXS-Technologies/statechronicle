# External native compiler and value contracts

Retained source `46a6950cf64752b7dd854356bceeafe582d8492a` adds consumer compiler
probes and a required local release-driver stage. Production library source,
dependencies, lockfiles, wire fixtures and GitHub workflows remain identical to
`437b481`. The compiler remains Rust 1.100.0. No new public API or production
runtime abstraction is introduced.

The [final exact-source campaign](20261116T122521Z-3288710/manifest.json) passes.
A fresh external application imports only the public umbrella, Serde and the
proof API's signing-key type. The canonical example's two domains are copied
byte-for-byte from the archive. Metadata resolves from the frozen root lock once,
then every build uses locked/offline Cargo. Registry packages must be a subset
of the root lock. Positive source, domain source, source archive and resolved
lock hashes remain unchanged after all probes.

The valid application checks, runs and passes strict Clippy. Seven deliberate
single-expression changes must produce Cargo exit 101 and only the intended
compiler error code. At least one matching diagnostic must originate at a
primary application span, directly or through its macro call-site expansion
chain. Restoring the exact valid source must compile again after every refusal.

| Misuse | Observed error |
| --- | --- |
| Configuration adapter receives an equipment command | `E0308` |
| Configuration reduction is returned as equipment state | `E0308` |
| Owned mutation is moved out of a borrowed prepared batch | `E0508` |
| Historical claim omits its independent commit pin | `E0061` |
| Verified historical source omits its independent history pin | `E0061` |
| An enum command has no explicit operation mapping | `E0004` |
| A native reducer returns an event instead of its declared state | `E0053` |

Dynamic schema values and external bytes are separate boundaries. The executed
positive application checks zero-schema registration/encoding refusal, valid
version-one/two execution, cross-version command/state refusal, cross-domain
state refusal and operation/command mismatch. These are value/semantic checks,
not cryptographic verification or migration authorization. The compiler probes
prove required pin inputs, not that a caller chose a trustworthy pin. The suite
uses native type identity where available and does not pretend numeric schema
values can all be rejected by Rust's type system.

The [final negative control](20261116T122526Z-3288675/manifest.json) deliberately
requires the valid program to fail compilation. Cargo returns 0; the runner exits
1 and retains `failed_or_interrupted`, with an explicit expected-101/actual-0
failure. This is the expected control result, not a passing compiler campaign.
Both final commands ran under optimized Python; manifests record optimization 1,
interpreter version and invocation. Explicit validation remains active rather
than relying on Python assertions. Raw Cargo output and terminal command exits
are retained even for the deliberate failure.

[Release-driver tests](release-driver-tests.json) pass all three orchestration
tests and require the real shell script to invoke the new tool. Every observed
external boundary is failed independently using fake tools, including the new
probe stage; success must not print after refusal. This proves orchestration,
not a new run of the entire actual release sequence. The real compiler/runtime
campaign above is separately retained. No new full-root test, coverage,
performance, package or dependency-audit claim is made for this dev-tool change.

Reproduce from the repository root:

```sh
PYTHONOPTIMIZE=1 python3 scripts/check_native_contracts.py --revision 46a6950cf64752b7dd854356bceeafe582d8492a
PYTHONOPTIMIZE=1 python3 scripts/check_native_contracts.py --revision 46a6950cf64752b7dd854356bceeafe582d8492a --negative-control-valid-program
python3 scripts/test_release_driver.py
```

The second command must fail. Evidence directories use timestamps/PIDs and retain
all attempts. The first `4ea437e` campaign refused the macro case despite the
correct E0004 because its location check did not follow expansion spans. The
`4ba6d4e` correction passed all probes. `206d9ae` integrated the local stage and
passed both the campaign and expected negative control. `da4854d` replaced
assertions with active validation and reduced noisy failure text; its campaign
passed and its optimized-Python control failed as intended. The retained source
then added explicit interpreter/invocation provenance and repeated the affected
checks. Earlier manifests remain, including their actual failed statuses.

This advances F3's external misuse and runtime schema qualification. Full schema
migration/authorization, frozen signed-history compatibility, built-in wrapper
review, whole F3/F8 completion and release readiness are separate requirements.
No PR, CI dispatch, tag or publication is authorized or created here.
