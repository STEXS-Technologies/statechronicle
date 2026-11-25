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
