# Release candidate compatibility matrix

The candidate remains on branch `refactor/coherence`, with package versions
`0.2.0`. This matrix describes the additive refactoring APIs, not availability
in the released 0.2 registry packages. Version bump, tag and publication require
separate release actions. Exact evidence is linked from the
[final qualification record](evidence/qualification/final-94eb5d4/README.md).

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
| Rust compiler | Approved minimum and pinned qualification compiler are Rust 1.100.0 | Root/isolated builds and dependency-refresh decision |
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

Current closure requires every final qualification result to pass and every
performance trigger to be resolved or explicitly justified. A successful narrow
test or an older checkpoint does not itself establish the full candidate.
