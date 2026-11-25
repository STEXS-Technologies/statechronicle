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
| F8 | Initial paired v0 sign and verify measurements are recorded. Final release latency, allocations, and workload coverage are in progress. |
| F9 | The migration guide covers staged reader and writer deployment and rollback. Final same-revision qualification, versioning, and release approval remain open. |

The source review found duplicate requested settlement facts were checked only
by the legacy wrapper and that the mixed verifier applied its body cap to the
full v1 envelope. Commit `4b40aea` shares request validation and checks the
full record bound. The trade and settlement integration suite and strict proof
Clippy passed after the fix.

RustSec audit completed with warnings denied against advisory database
`b9b8a3c2cb1f2a3c76ab17a551193568c9f8b7a4`. All six active lockfiles at
`7c764c1` passed. This is evidence for that dependency graph and advisory
snapshot. The standalone provider fixtures are test dependencies and carry
their own provider and platform limits.
