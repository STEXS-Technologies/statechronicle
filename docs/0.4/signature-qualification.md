# Signature agility qualification

The compatibility baseline is the published StateChronicle v0.3.0 source
at `9ed64a5a5ece51fa2cd5929119cbe21e1e34f3f4`. See the
[frozen wire and consumer evidence](../../ci/wire-0.3/README.md) and
[legacy signature measurements](../../ci/wire-0.3/signature-latency.md).

| Phase | Evidence and remaining qualification |
| --- | --- |
| F0 | Tagged-source wire fixtures, package and dependency inventory, frozen standalone consumer, and legacy signature measurements. The exact-tag legacy workflow and allocation comparisons are recorded. |
| F1 | ADR-009 specifies format dispatch, transcript fields, trust boundaries, hybrid composition, and migration. A separate source review checked those boundaries during qualification. |
| F2 | Shared signer and verifier contracts, strict Ed25519 adapters, and bounded buffers. Async providers share the synchronous path's transcript and validation. |
| F3 | Explicit legacy and v1 dispatch, canonical BCS, normalized v1 JSON, and standalone wire fixtures. |
| F4 | Active and Historical policy contexts bind the independently configured authority and exact key set. Deployments qualify their lifecycle and KMS adapters separately. |
| F5 | Protected mutation and batch signing use the existing atomic persistence and recovery path. Exact signed bytes and membership are checked during authoritative readback. |
| F6 | Current and historical claims, mixed trade and settlement proofs, checkpoint verification, portable verification, and mixed index rebuild use shared verification. The legacy wire fixtures exercise compatibility. |
| F7 | Two ML-DSA-65 implementations interoperate. The standalone provider tests cover selected NIST ACVP ML-DSA and SLH-DSA cases and a genuine Ed25519 plus ML-DSA-65 hybrid. The sanitizer-backed fuzz gate is in progress. |
| F8 | Paired v0 sign and verify results and five v1 provider measurements are recorded. The 22 paired legacy workflows and allocation probes are recorded. Versioned durable and proof workflow measurements are in progress. |
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


## Legacy workflow comparison

The frozen [external benchmark](../../ci/compat-0.2/crates/consumer/examples/refactor_bench.rs)
was built against the exact v0.3.0 tag and `f0c49bda42afe9238a8657bbfa991def69ba8c3c`.
It used Rust 1.100.0, release optimization, and each revision's source lock.
The subsequent `c6e3a63` changes add the standalone v1 benchmark. Its
packaged Rust sources and workspace lock match `f0c49bd`.

The comparison uses seven pairs, alternating order on logical CPU 0.
For each workload the benchmark calculates percentiles over 31 sample
averages. The number of calls per sample varies by workload. p99 is the
largest sample average. Adapter benchmarks use in-memory services and
include setup only when the workload label specifies it. The host recorded
zero cgroup throttling during the paired runs, though workload scheduling
and shared-host variation still affect the tails. [All 22 workloads and
targeted reruns](paired-legacy-workflows-f0c49bd.csv)

Each row below reports the median of lane percentiles and the median
percentage change within pairs.

| Workload | v0.3 p50 / p95 / p99 (ns/op) | 0.4 p50 / p95 / p99 (ns/op) | Median paired change (%) |
| --- | --- | --- | --- |
| Commit signing | 70,205.0 / 88,770.3 / 100,406.4 | 69,918.3 / 88,261.1 / 97,649.1 | -1.8 / -4.7 / -2.1 |
| Historical state proof | 117,847.8 / 157,265.8 / 185,931.7 | 117,697.2 / 147,573.3 / 162,873.0 | -2.9 / -1.7 / -0.7 |
| Current state proof | 115,907.6 / 155,716.0 / 162,105.7 | 118,752.3 / 153,242.6 / 175,206.8 | 2.6 / 9.2 / 6.2 |
| Trade proof | 122,456.9 / 162,238.3 / 178,134.5 | 115,869.2 / 142,117.5 / 146,816.0 | -0.9 / -7.8 / -17.3 |
| Protected write | 232,811.2 / 331,756.6 / 344,451.4 | 236,095.8 / 346,560.4 / 393,645.0 | -0.5 / 2.5 / 6.6 |
| Exact retry | 124,894.6 / 174,522.8 / 196,762.0 | 124,119.0 / 164,306.0 / 207,609.8 | 8.7 / 3.6 / 2.5 |
| Ambiguous acknowledgement | 235,373.8 / 298,042.2 / 312,167.2 | 246,167.0 / 392,100.6 / 402,782.6 | 1.9 / 7.5 / 16.9 |


An initial sequential run showed a 125% higher ambiguous-acknowledgement
p95 and a 104% higher p99. Alternating the same executables reduced those
differences. A focused second series alternated the same binaries again,
passing only one workload name per invocation. The largest replay tail
in the full run occurred in a single candidate process. Native validation
has a p50 near 2 ns, so a short interruption can produce a large percentage
change in the largest batch-average sample. The targeted results show
how much these high-percentile estimates varied by series.

| Targeted workload | p50 change | p95 change | p99 change |
| --- | ---: | ---: | ---: |
| Native validation | +2.1% | -2.4% | -2.0% |
| Exact retry | -9.9% | -22.5% | -29.7% |
| Ambiguous acknowledgement | +5.4% | -7.2% | -10.1% |
| Two-event replay | -1.1% | -7.9% | -12.8% |


The ambiguous-acknowledgement median increased in both paired series
by 1.9% and 5.4%. This path shares transaction classification and
readback code with v1. The exact-retry median moved in opposite directions
across the two series. These remain shared-runner estimates.

Allocation probes ran 1,000 calls per process and subtracted a setup-matched
control. The glibc interposer counts successful `malloc`, `calloc`, and
`realloc` requests. It measures request bytes rather than peak live memory.

| Workload | v0.3 events / bytes per call | 0.4 events / bytes per call |
| --- | --- | --- |
| Native validation | 0 / 0 | 0 / 0 |
| Portable validation and hash | 0 / 0 | 0 / 0 |
| Hosted typed check and apply | 23 / 362 | 23 / 362 |
| Legacy state proof verification | 29 / 41,128 | 18 / 1,223 |

The proof sizing check now counts serialized bytes directly. This removes
the temporary BCS vector while preserving the proof size limit.


## V1 durable workflow latency

The existing protected mutation and batch integration fixtures were
timed on source `c6e3a63` with the reproducible
[instrumentation patch](durable-benchmark.patch). The patch only adds test
timing and leaves release library files unchanged. Use a detached checkout
of the measured source.

```sh
git apply --unidiff-zero /path/to/durable-benchmark.patch
STATECHRONICLE_DURABLE_BENCH=1 cargo +1.100.0 test --release --locked \
  -p statechronicle --test durable_recovery \
  protected_v1_plan_binds_exact_intent_and_commit_records -- --nocapture
STATECHRONICLE_DURABLE_BENCH=1 cargo +1.100.0 test --release --locked \
  -p statechronicle --test durable_batch \
  v1_batch_recovers_the_full_exact_group_after_lost_ack -- --nocapture
```

Three runs on logical CPU 3 each timed 31 calls per operation with
nearest-rank percentiles. The reported values are medians of the three
run percentiles. [Per-call samples](durable-v1-c6e3a63.csv)

| Fixture | Operation | p50 µs | p95 µs | p99 µs |
| --- | --- | ---: | ---: | ---: |
| One-event mutation | Commit | 423.3 | 713.5 | 901.2 |
| One-event mutation | Exact recovery | 139.1 | 276.6 | 448.5 |
| Two-member batch | Commit | 823.6 | 1152.0 | 1412.1 |
| Two-member batch | Exact recovery | 251.1 | 459.1 | 501.1 |


Each timed commit starts with a prepared and signed plan. It includes
verification, writes, and readback using the reference in-memory ledger.
Recovery measures exact readback from that same ledger after the commit.
These measurements cover one- and two-event fixtures on the shared runner.
They exclude provider queueing, application preparation, production storage,
and the larger event counts.
