# Requested Rust and dependency refresh

The maintainer requested latest Rust and all available dependency updates during
the refactor. This explicitly changes the earlier roadmap constraint to retain
Rust 1.85. The default toolchain is pinned to Rust 1.100.0, the latest stable
release checked on 2026-11-16, and all current library/isolated application
manifests declare minimum Rust 1.100. The CI setup and embedded default select
that toolchain. The installed nightly fuzz compiler was updated separately to
1.102.0-nightly (`5f4effd57`, compiler commit dated 2026-11-13).

| Direct dependency | Previous root lock | Updated root lock |
|---|---|---|
| async-trait | 0.1.91 | 0.1.92 |
| base64 | 0.22.1 | 0.23.1 |
| ed25519-dalek | 2.2.0 | 3.0.0 |
| proptest | 1.11.0 | 1.12.0 |
| serde_json | 1.0.151 | 1.0.154 |
| sha2 | 0.10.9 | 0.11.1 |
| thiserror | 2.0.19 | 2.0.21 |
| tokio | 1.53.1 | 1.54.1 |
| trait-variant | 0.1.3 | 0.1.3 |

Other direct requirements were refreshed to their latest stable releases too,
including unchanged latest versions. Full direct requirements and every
transitive version/checksum are recorded in manifests and lockfiles rather than
repeated here. The root, conformance harness and both standalone tutorials have
updated locks. Frozen original-release consumers/wire records and historical
measurement artifacts retain their original dependencies.

Rust 1.100 reported unused `tracing` in the core crate and `uuid` in the domain
crate. Both declarations were removed; no current crate required the workspace
`uuid` dependency, so it was removed too. SHA-2 0.11 no longer has the old
`compress` feature; the code uses the standard digest API, with default features
disabled. The portable crate keeps separate no-default-feature cryptographic
dependencies to preserve its bare-metal boundary.

No library Rust source changed. The portable fuzz input classifier uses
`as_chunks::<33>().0.iter()` instead of `chunks_exact(33)` with equivalent
remainder handling, to satisfy current Clippy at the raised MSRV. The file
ledger tutorial uses equivalent let-chain cleanup; it keeps attempt matching
before removing a reservation. No lint suppression was added for either change.

This is a dependency/source compatibility boundary: key types from Dalek 2 and
Dalek 3 are distinct. Consumers of interim version-2 checkpoints must align their
own dependency or use validated byte conversion. Original v0.2 used version 3.
Wire, signing and root compatibility are checked against frozen original records;
current resolved updates do not regenerate those expectations.

The retained qualification distinguishes the dependency candidate, the following
tutorial-only lint correction, and evidence commits. It covers workspace finite
and doc tests, strict lint/docs, external old-wire consumer/portable comparisons,
conformance, bare-metal builds, isolated tutorials and lockfile audit/update
checks. Current advisory and dependency policy results are time-scoped evidence,
not a promise about future advisories or every downstream application. Frozen
historical locks are deliberately outside current dependency-refresh claims.

Remaining full-roadmap performance, packaging, API review, extended fuzz/crash
campaigns and other F0–F8 gates are not declared complete by this refresh. Package
release version remains unchanged pending separately qualified release work.
