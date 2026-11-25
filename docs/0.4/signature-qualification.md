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
`4af9576` passed. This is evidence for that dependency graph and advisory
snapshot. The standalone provider fixtures are test dependencies and carry
their own provider and platform limits.


## Extracted-package and external consumer checks

On 2026-11-25, source revision `4af95763e80c365d398172a9d87a8e0e240c8a08`
passed the isolated archive check with Rust 1.100.0. The checker created all 13
archives, verified the packaged original fixtures against their pinned sources,
and ran all-target tests, doctests, and strict Clippy in a fresh workspace
containing only the extracted archives. The registry dependencies resolved
within the original locked graph.

The standalone checks on the same source passed 26 durable adapter conformance
tests, five release-mode PQ provider tests, and two release-mode tests against
the published v0.3.0 signing APIs.

| Crate archive | Bytes | SHA-256 |
| --- | ---: | --- |
| `statechronicle-0.3.0.crate` | 209787 | `eb845835790bb9fd997607be5fd01861fc465f8a89a1301ecfa8624ed5dd1cca` |
| `statechronicle-accumulator-0.3.0.crate` | 40430 | `b9fd98cbc1daf7f177ccb7d3ab8c7c7612de7c4dac7657b3bb6e7ceae4ddc5c5` |
| `statechronicle-commit-0.3.0.crate` | 71069 | `a0d1d7f2fb0ff5ff94a3b54fed7bb19af6979bbdc0f5fdeb5e8f7ca0f6a37313` |
| `statechronicle-core-0.3.0.crate` | 41262 | `08cdbdd525fb76d534210eb2f5cf2996bb0e59b57b783203adee1db67f1743a7` |
| `statechronicle-domain-0.3.0.crate` | 43470 | `ed22d75ecff6864655223e930f3d2f21e151e14ddc868d38a4718692f9d5e35d` |
| `statechronicle-executor-0.3.0.crate` | 73509 | `538ab9fd1c0330cbae8836375c6928570fc97d7e51afd73ffcadd7820338a118` |
| `statechronicle-index-0.3.0.crate` | 37870 | `e4c469d73a062d2dc3ca4b58593f83ac311f81ddb34f39698b19870911fb65b5` |
| `statechronicle-intent-0.3.0.crate` | 27223 | `a138d61058c5841607a264f3e57ec796e72787d169df20109c6e5d6601d06a69` |
| `statechronicle-portable-0.3.0.crate` | 18326 | `133ba6cf9ea358271f7508a9eab307bc4d427af5ef31cfdf6445a0ac1233cbee` |
| `statechronicle-ports-0.3.0.crate` | 39811 | `86664621e1b5a797be32a127e85891030eadc52d995f38f3035a9851a9d4ba8c` |
| `statechronicle-profile-0.3.0.crate` | 2819 | `2625098456a5020a2684714e1db1145c35d1d3f631e3a1c93f41f6b4e81b67b1` |
| `statechronicle-profiles-0.3.0.crate` | 80064 | `3f7e6ad424effebd44e70cbf780992f7f24a335faaf8d9f5fdda883114a3d6b0` |
| `statechronicle-proof-0.3.0.crate` | 69794 | `9485f42bc9a401a56e6609e088daa3cedb9e077c8fa95a65498a66f78d3da226` |
