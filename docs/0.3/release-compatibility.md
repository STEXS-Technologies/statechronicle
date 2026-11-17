# Release candidate compatibility matrix

The release candidate uses package version `0.3.0`. This matrix describes the
0.3 APIs, which are unavailable in the released 0.2 registry packages. Tagging
and publication follow release review. Exact evidence is linked from the
[final qualification record](qualification.md).

| Contract | Candidate decision | Evidence |
|---|---|---|
| Old BCS enum indices/envelopes/signature payloads/digests/roots | Preserve frozen original records and semantics | Original/current external consumer and packaged frozen fixtures |
| Old proof/history verification | Verify original independent signed corpus | Frozen consumer, minimum-reader claim campaign and tutorial |
| Existing exhaustive public error/type matches | Preserve promised original variants | Unchanged frozen external exhaustive-match fixture |
| Old native `StateProfile` implementations | Preserve original semantic contract | Both hosted/portable original/current external implementers |
| Hosted `ProfileRules` | Add default evaluation; native adapter shares decoded values | Compiler method catalogs, parity/negative fixtures and old implementers |
| Preparation and authoritative durability | Add protected APIs; legacy sinks remain planning/advanced compatibility | External tutorials, canonical consumer, durable/conformance tests |
| Batch support | Complete single-tenant retained group; optional capability defaults to refusal | Batch conformance and protected batch integration tests |
| Coordinated settlement | Explicit single-tenant protected preparation; incomplete bundle routes refuse | Settlement contention/invariant tests and migration guide |
| Current versus historical claims | Current live full head check; historical independent pin | Expected-claim integration tests, runtime bypass controls |
| Projection rebuild identities | Preserve independent holder addresses and tenant-scoped event IDs; reject same-address forks and same-tenant duplicates | Rebuild holder/continuity/duplicate regressions and original root vectors |
| Custom schema migration | Explicit separately authorized transition; no reinterpretation | Original-produced history, protected migration/rebuild tests |
| Rust compiler | Minimum Rust 1.88; development toolchain and original qualification compiler Rust 1.100.0 | [Minimum-version validation](rust-support.md), root/isolated builds |
| Direct Ed25519 key dependency types | Use Dalek 3; interim Dalek 2 consumers must align direct key types | Dependency-refresh/migration notes and frozen signature verification |
| Portable supported profiles | Allocation-free fixed buffers; supported bare-metal compile | Independent no-std consumer and embedded release check |
| Crate/default feature topology | Thirteen library crates; no new runtime backend/service dependency | Cargo manifests, package archives and dependency/cost catalogs |
| Physical backend/key/service guarantees | Consumer-owned implementation and deployment qualification | Explicit ports, adapter contract and bounded file tutorial limits |

The complete source catalogs contain 86 added, zero removed and 22 changed named
export records versus original 0.2, not 86 independent new concepts. The current
catalogs equal the previously reviewed `64c5350` catalogs. This comparison is
supplemented by real external implementers; it is not a promise that every possible
macro invocation, dependency feature unification or downstream name collision is
source-compatible.

The [qualification summary](qualification.md) records the completed source
snapshot and accepted costs. New changes require checks appropriate to their
scope; an older passing checkpoint does not qualify later changes.
