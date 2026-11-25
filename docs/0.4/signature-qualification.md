# Signature agility qualification

The compatibility baseline is the published StateChronicle v0.3.0 source
at `9ed64a5a5ece51fa2cd5929119cbe21e1e34f3f4`. See the
[frozen wire and consumer evidence](../../ci/wire-0.3/README.md) and
[legacy signature measurements](../../ci/wire-0.3/signature-latency.md).

| Phase | Evidence and remaining qualification |
| --- | --- |
| F0 | Tagged-source wire fixtures, package and dependency inventory, frozen standalone consumer, and legacy signature measurements. Full workflow benchmark comparison is in progress. |
| F1 | ADR-009 specifies format dispatch, transcript fields, trust boundaries, hybrid composition, and migration. A separate source review checked those boundaries during qualification. |
| F2 | Shared signer and verifier contracts, strict Ed25519 adapters, and bounded buffers. Async providers share the synchronous path's transcript and validation. |
| F3 | Explicit legacy and v1 dispatch, canonical BCS, normalized v1 JSON, and standalone wire fixtures. |
| F4 | Active and Historical policy contexts bind the independently configured authority and exact key set. Deployments qualify their lifecycle and KMS adapters separately. |
| F5 | Protected mutation and batch signing use the existing atomic persistence and recovery path. Exact signed bytes and membership are checked during authoritative readback. |
| F6 | Current and historical claims, mixed trade and settlement proofs, checkpoint verification, portable verification, and mixed index rebuild use shared verification. The legacy wire fixtures exercise compatibility. |
| F7 | Two ML-DSA-65 implementations interoperate. The standalone provider tests cover selected NIST ACVP ML-DSA and SLH-DSA cases and a genuine Ed25519 plus ML-DSA-65 hybrid. The sanitizer-backed fuzz gate is in progress. |
| F8 | Paired v0 sign and verify results and five v1 provider measurements are recorded. The broader paired v0 workloads and allocation comparison are in progress. |
| F9 | The migration guide covers staged reader and writer deployment and rollback. The 0.4 package graph passed extracted-package validation. Final workspace and sanitizer checks are in progress. |

The source review found duplicate requested settlement facts were checked only
by the legacy wrapper and that the mixed verifier applied its body cap to the
full v1 envelope. Commit `4b40aea` shares request validation and checks the
full record bound. The trade and settlement integration suite and strict proof
Clippy passed after the fix.

RustSec audit completed with warnings denied against advisory database
`b9b8a3c2cb1f2a3c76ab17a551193568c9f8b7a4`. All seven active lockfiles in the 0.4 package graph passed.
Cargo Deny also passed its advisory, ban, license, and source checks. This is evidence for that dependency graph and advisory
snapshot. The standalone provider fixtures are test dependencies and carry
their own provider and platform limits.


## Extracted-package and external consumer checks

On 2026-11-25, source revision `f0c49bda42afe9238a8657bbfa991def69ba8c3c`
passed the isolated archive check with Rust 1.100.0. The checker created all 13
archives, verified the packaged original fixtures against their pinned sources,
and ran all-target tests, doctests, and strict Clippy in a fresh workspace
containing only the extracted archives. The registry dependencies resolved
within the original locked graph.

The standalone checks on `4af9576` passed 26 durable adapter conformance tests
and five release-mode PQ provider tests. On the 0.4 package graph the same two
frozen consumer tests compiled and passed in release mode against both the
published v0.3.0 packages and the local 0.4.0 packages.

| Crate archive | Bytes | SHA-256 |
| --- | ---: | --- |
| `statechronicle-0.4.0.crate` | 210152 | `22b3ceb432e662dee45980270e8356204821aec8a9e51b57cd0c353bac136e31` |
| `statechronicle-accumulator-0.4.0.crate` | 40426 | `e7e0ec1ea65267a25545199857719f7bcd35e1031b02efce3654d243e1bbeb2a` |
| `statechronicle-commit-0.4.0.crate` | 71066 | `a80d15437b2e2df89e356bbe7cf729ffdaea33348e8fead5535775b3a8cf082f` |
| `statechronicle-core-0.4.0.crate` | 41263 | `0667cb1527de94394d8fab1c12a276f9b91b0e32db737e71514a6f6e297914b7` |
| `statechronicle-domain-0.4.0.crate` | 44146 | `f2e4c6158302688a1e171cfc8f6099c6a06a7833b0e77e3bbe16a1e5b7c0e9c2` |
| `statechronicle-executor-0.4.0.crate` | 73509 | `10eb96b55c4837d1a3f885bc6de78c678f5023c2f0c445f364d3270af46b8a14` |
| `statechronicle-index-0.4.0.crate` | 37873 | `12b2539ab27c37dab8b1ddf5fbe5d066985168539dbd13975e2ee1dc98f83dbd` |
| `statechronicle-intent-0.4.0.crate` | 27227 | `c0dad9766ec709ecbb29c17dcb854df9add5eafcbd9d202209e6ddf83176dfd5` |
| `statechronicle-portable-0.4.0.crate` | 18321 | `1212e7373fa9b8ef4fb9459f460046b42dc018b3a67c77ba2eca989d51fa63bf` |
| `statechronicle-ports-0.4.0.crate` | 39805 | `d189ceefd582c2ae1dcd458fa685de931a228a75d594f6d12a9d0c2a6af96cf2` |
| `statechronicle-profile-0.4.0.crate` | 2821 | `eb427b98daa52dccfdc8ece511f6d283ce774440334d2c8b8208da36a9329304` |
| `statechronicle-profiles-0.4.0.crate` | 80058 | `cd1cdba7becdc13d3cfc8740e2f529ae1e6b6a310a0206e971b0e370a1da3f01` |
| `statechronicle-proof-0.4.0.crate` | 69793 | `44e1d880508bc9bbc01d2abc1f89e589a5b212270d00c97440cb7f0bcfda990e` |


## V1 release envelope measurements

The [benchmark](../../ci/signature-conformance/README.md) ran on
`c6e3a63e2020d2c3c6d648e80fd62d9a6107d4eb` with Rust 1.100.0, the locked
release profile, and a Linux x86-64 Intel Xeon Platinum 8573C runner.
The test executable was pinned to logical CPU 4 with `taskset`. Each of
three runs measured 100 calls per operation after 10 warmups. The runner
shared a four-core CPU quota with compilation during these runs, so the
tail results include scheduler variation. The table gives the median
of each run percentile in microseconds. [Per-run values](signature-envelope-c6e3a63.csv)
are in nanoseconds.

| Provider | Operation | p50 µs | p95 µs | p99 µs | Signature bytes | Envelope bytes |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Ed25519 | Sign | 18.5 | 29.0 | 94.8 | 64 | 176 |
| Ed25519 | Decode and verify | 43.3 | 64.2 | 116.7 | 64 | 176 |
| ML-DSA-65 (RustCrypto) | Sign | 664.5 | 955.8 | 1170.1 | 3309 | 3424 |
| ML-DSA-65 (RustCrypto) | Decode and verify | 106.4 | 158.5 | 250.1 | 3309 | 3424 |
| ML-DSA-65 (AWS-LC) | Sign | 143.0 | 421.9 | 576.1 | 3309 | 3424 |
| ML-DSA-65 (AWS-LC) | Decode and verify | 52.4 | 95.6 | 116.2 | 3309 | 3424 |
| SLH-DSA-SHA2-128s (RustCrypto) | Sign | 218258.0 | 284182.2 | 331258.7 | 7856 | 7979 |
| SLH-DSA-SHA2-128s (RustCrypto) | Decode and verify | 196.4 | 306.4 | 344.2 | 7856 | 7979 |
| Ed25519 + ML-DSA-65 (AWS-LC) | Sign | 170.0 | 527.7 | 708.5 | 3380 | 3513 |
| Ed25519 + ML-DSA-65 (AWS-LC) | Decode and verify | 100.0 | 162.4 | 243.3 | 3380 | 3513 |


Each sample used the same 45-byte body. Signing includes transcript creation,
provider execution, and envelope encoding. Verification includes envelope
decoding, transcript creation, and verification of a prebuilt valid envelope.
SLH-DSA signing dominated the measured provider cost. AWS-LC and RustCrypto
timings reflect their paired implementations and build features. These measurements report latency and encoded bytes. Allocation instrumentation is separate.


## Deployment boundaries

The default normal dependency graph contains no PQ provider dependencies.
The optional conformance crate uses AWS-LC and RustCrypto to check
interoperability. The tested AWS-LC build uses its default features, and the
SLH-DSA RustCrypto package is a release candidate. Deployments select and
qualify their own crypto providers and key registry.

The mixed-history rebuild requires an independently selected ordered pin
for every record. The caller controls the history length. Individual
commit records and event batches have bounds, while retained replay data
scales with the caller-selected history. Remote signing limits and deadlines
remain adapter responsibilities. Local reference and test adapters exercise
the transaction and recovery contracts.


## Paired legacy signature latency

The baseline binary was built from the exact v0.3.0 tag and the candidate
binary from `c6e3a63e2020d2c3c6d648e80fd62d9a6107d4eb`. Both used the
same [benchmark source](../../crates/statechronicle-commit/examples/signature_baseline.rs),
Rust 1.100.0, the release profile and each revision's locked dependency graph.
Seven pairs alternated the lane order on logical CPU 4 of the same Xeon
runner. Each operation had 100 warmups and 10,000 individually timed
calls per run. Background work shared the machine during both lanes.
[Per-run values](paired-legacy-signature-c6e3a63.csv)

| Workload | v0.3 p50 / p95 / p99 (ns) | 0.4 p50 / p95 / p99 (ns) | Median change within pairs |
| --- | --- | --- | --- |
| `ed25519_sign_1024b` | 21092 / 34516 / 97416 | 21140 / 39069 / 107564 | +0.2% / +13.2% / +11.6% |
| `ed25519_verify_1024b` | 43686 / 94159 / 191667 | 43688 / 104980 / 196238 | -0.0% / +11.5% / +9.6% |
| `commit_sign` | 18991 / 25564 / 96537 | 19072 / 26513 / 95687 | +0.5% / +15.8% / +3.9% |
| `commit_verify` | 42573 / 94606 / 161919 | 42569 / 98730 / 189118 | +0.0% / +9.3% / +21.1% |


Absolute values are medians of the seven per-run percentiles. The change
column is the median of the percentage differences calculated within each
pair. Median p50 changes were within 0.5%. The initial p95 and p99
changes prompted a repeat after the stable unit and integration tests completed. Legacy signing, verification, canonicalization,
and their domain source files match the tag. Registry package versions and
checksums match the tag's lockfile. These checks narrow the investigation,
and the second series showed much smaller tail changes.

The repeat used the same binaries, reversed the order of the first pair,
and also alternated order across seven pairs. The reported percentages are
the median changes within the repeat pairs. The shared cgroup recorded
about 0.80 seconds of throttling during the repeat.

| Workload | Repeat p50 change | Repeat p95 change | Repeat p99 change |
| --- | ---: | ---: | ---: |
| Ed25519 sign | +0.09% | −0.03% | +1.94% |
| Ed25519 verify | +0.00% | +2.80% | +4.33% |
| Commit sign | +0.52% | −1.81% | +2.40% |
| Commit verify | −0.02% | −0.57% | −0.07% |

The repeat supports similar median latency for the four operations on this
runner. The spread across the two series limits claims about tail latency
under production load.

- Baseline executable SHA-256 `2c76ae8672464929779db12a8d0c507d992041c598863cb3940a58e6f7423faf`
- Candidate executable SHA-256 `775d6f2b994bff702461d740b3a5ea6e513e606f424df8cceab0e3b48a9a19b7`


The raw benchmark executables were 513,600 bytes at v0.3 and 513,648
bytes at 0.4. Their combined text, data, and BSS size was 514,074 bytes
in both lanes, as reported by `size`.

## Paired legacy signature allocations

Both revisions ran a release build of the same DHAT 0.3.3 harness. The
scratch builds retained all pre-existing registry lock entries. Each row
uses a fresh process, 1,000 calls, and subtraction of a setup-matched
control. All three process repetitions per lane produced identical totals.
The profiler was run separately from latency measurements.

| Workload | v0.3 allocations per call | 0.4 allocations per call | v0.3 bytes per call | 0.4 bytes per call |
| --- | ---: | ---: | ---: | ---: |
| `ed25519_sign_1024b` | 0 | 0 | 0.011 | 0.011 |
| `ed25519_verify_1024b` | 0 | 0 | 0.013 | 0.013 |
| `commit_sign_canonicalize_envelope` | 20 | 20 | 2056.026 | 2056.026 |
| `commit_verify_canonicalize` | 10 | 10 | 1639.019 | 1639.019 |


The raw signature rows show zero additional allocation events. Their
sub-byte differences reflect process and profiler overhead in the
control-subtracted totals.
