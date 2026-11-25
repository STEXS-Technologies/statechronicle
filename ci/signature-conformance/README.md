# External post-quantum provider interoperability

This standalone consumer uses the public StateChronicle provider traits.
It exercises pure ML-DSA-65 through AWS-LC and RustCrypto, pure
SLH-DSA-SHA2-128s through RustCrypto, and hybrid verification with AWS-LC
ML-DSA-65 and the existing strict Ed25519 verifier. Algorithms use the
complete StateChronicle transcript and the empty algorithm context.

```sh
cargo test --release --locked --manifest-path ci/signature-conformance/Cargo.toml
```

The provider crates are confined to this consumer's dependency graph.
The AWS-LC test uses its default non-FIPS build. RustCrypto documents
both implementations as independently unaudited, and the SLH-DSA
dependency is a release candidate. These tests cover API interoperability
cross-verification between the two ML-DSA providers, and refusal of modified signatures. Independent standards vectors,
dependency auditing, and deployment validation are separate requirements.

## Release microbenchmarks

The ignored release benchmark measures the complete public v1 envelope path
for Ed25519, ML-DSA-65 with RustCrypto, ML-DSA-65 with AWS-LC, SLH-DSA-SHA2-128s
with RustCrypto, and the Ed25519 + AWS-LC ML-DSA-65 hybrid. It reports p50,
p95 and p99 nanoseconds for signing and for envelope decode plus verification,
along with the exact sample count and body, signature and envelope byte sizes.
The default is 100 measured samples after 10 warmups. Key generation and
fixture signing are outside the timed region. Set `SC_BENCH_SAMPLES` and
`SC_BENCH_WARMUP` to positive integers to change those counts.

```sh
rustc +1.100.0 -Vv
uname -a
SC_BENCH_SAMPLES=1000 SC_BENCH_WARMUP=50 \
  cargo +1.100.0 test --release --locked \
  --manifest-path ci/signature-conformance/Cargo.toml \
  --test providers -- --ignored --nocapture
```

The reported operations include transcript construction, envelope encode or
decode, provider dispatch, and the cryptographic operation; verification uses
the same prebuilt valid envelope for every sample. AWS-LC ML-DSA operations
execute in its native backend, while RustCrypto ML-DSA and SLH-DSA operations
are implemented by RustCrypto. The hybrid runs both Ed25519 and AWS-LC ML-DSA
for every sample. All output allocations are part of the measured public path.
Allocation-event counts require instrumentation for both the Rust allocator
and native AWS-LC allocations. Use an external allocator profiler for that
measurement. Record the host, toolchain, power mode, and competing system load
when comparing microbenchmark results.
