# Changelog

All notable changes to StateChronicle are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

StateChronicle 0.4 adds opt-in signature suites for state verification while
preserving the v0.3 APIs and wire bytes. See the
[signature migration guide](docs/0.4/signature-migration.md) for reader rollout,
key policy, and rollback.

### Added

- Add v1 signing and verification for actor intents, tenant commits, and global
  checkpoints. Deployments can select standalone ML-DSA-65 or SLH-DSA-SHA2-128s,
  or require both Ed25519 and ML-DSA-65 with the hybrid suite. Strict Ed25519
  remains the default for legacy callers.
- Support versioned durable writes and exact recovery for single mutations and
  batches. Verify current and historical claims, mixed-format trade and
  settlement evidence, and mixed-format index rebuilds. Add a borrowed v1
  verification hook for portable consumers.
- Add optional conformance tests for ML-DSA-65 interoperability, SLH-DSA
  verification, and the hybrid suite.

### Changed

- Count serialized proof bytes directly when checking the size limit, avoiding
  a temporary BCS buffer.

### Security

- Bind v1 signatures to the expected purpose, schema, suite, and key set.
  Resolve signer trust from deployment policy and reject mismatches before
  signature verification.
- Require independently pinned signed records for mixed-format historical
  evidence. Reject duplicate settlement facts and check complete v1 envelopes
  against their wire-size limit.

The [qualification record](docs/0.4/signature-qualification.md) documents the
compatibility checks, benchmark results, and deployment limits.

## [0.3.0] - 2026-11-17

### Added

- Protected read-only single, batch and single-tenant settlement preparation;
  immutable signed plans and independently checked original durable receipts.
- Exact-identity recovery/replay, explicit unknown acknowledgement outcomes,
  complete retained batch membership and default refusal of unsupported ports.
- Supplied-proof verification through a minimum read-only adapter, with independent
  expected tenant/resource/owner/trusted key and distinct current/historical modes.
- Verified pinned history, resumable projection rebuild and separately authorized
  custom-schema migration examples and adversarial fixtures.
- External native-domain and file-ledger tutorials, reusable dev-only adapter
  conformance checks, selected process-kill/reopen and concurrent history tests.
- Immutable-source qualification tooling for compiler API catalogs, frozen
  compatibility, isolated package contents, coverage, fuzzing and performance
  comparisons under matched test conditions.

### Changed

- Hosted native evaluation shares decoded state/command values; operation mappings,
  validated string trait implementations and equivalent profile/index checks
  reuse shared implementations. Legacy APIs retain their documented contracts.
- The minimum supported Rust version is 1.88; the repository development
  toolchain is pinned to 1.100. Dependencies use Ed25519 Dalek 3. Consumers
  exchanging direct dependency key types must align their major version. See
  [migration](docs/0.3/migration.md) and [Rust support](docs/0.3/rust-support.md).
- Consumer documentation recommends protected receipt-based composition and
  distinguishes legacy planning/sinks, historical evidence and current claims.
- Simplify the README and quick start; retain frozen compatibility fixtures and
  link archived qualification evidence instead of shipping generated work logs.

### Fixed

- Expected-address and canonical-head verification reject substituted/stale proof
  evidence; historical verification requires an independently supplied commit pin.
- Protected retry/recovery rejects changed payloads, corrupt original records and
  unverified receipts; lost acknowledgement does not fabricate confirmed failure.
- Trade-state verification binds each leg to its tenant, resolves trusted commits
  by tenant and commit ID, and rejects duplicate summary/proof coverage. Unsigned
  summary metadata is not promoted into authenticated settlement history.
- Projection rebuild keeps independent subject-held addresses and scopes event
  identity by tenant, preventing unrelated holders or tenants from colliding.
- Qualification drivers isolate archived fuzz and unpacked package builds from
  cached binaries of earlier or deliberately insecure source revisions.
- CI warms the complete locked dependency cache before isolated offline consumers
  and retains their diagnostic logs when verification fails.

### Security

- Temporarily except Chrono maintenance advisory `RUSTSEC-2026-0543`, with an
  enforced 2026-12-17 UTC expiry. All other advisory checks remain enabled. See
  the [dependency maintenance policy](DEPENDENCY_MAINTENANCE.md).

Original 0.2 wire fixtures remain frozen. See the [compatibility matrix](docs/0.3/release-compatibility.md)
and [qualification record](docs/0.3/qualification.md)
for exact tested scopes, accepted costs and retained limitations. Library packages
and their internal dependency requirements advance together to `0.3.0`.

## [0.2.0] - 2026-10-10

### Added

- Consumer-defined native state and command models through an allocation-free,
  dependency-free `no_std` `StateProfile` contract and a typed BCS adapter.
- Static custom profile composition with domain identity, schema versioning,
  command/operation binding, validation, authority requirements and payload bounds.
- Domain-aware signed proof verification and deterministic transition replay.
- Independent configuration and equipment examples, compatibility vectors,
  custom-domain property/fuzz coverage, and transactional persistence regressions.
- Allocation-free portable transitions, bounded BCS codecs, compatible hashes,
  strict Ed25519 and sparse-Merkle verification, with stable-toolchain bare-metal checks.
- Transaction-boundary fault injection, cancellation and ambiguous-commit recovery
  tests, seeded reordered/concurrent outage schedules, and retained fuzz campaign evidence.

### Changed

- Execution dispatches reduction through `ProfileRules::apply`; baseline reducers
  live in the profiles crate with executor compatibility wrappers.
- Profile-owned execution policies define addressing, access gates, routing,
  unborn commitments and event groups; economic coordinators are isolated from
  the generic execution path.
- Appended custom variants to state enums while preserving baseline BCS indices,
  commit/proof envelopes, signing, root and key formats. Exhaustive Rust matches
  require a new custom arm. See [migration details](docs/DESIGN/DOMAIN_EXTENSIONS.md).
- Durable persistence rejects custom domain/schema substitutions before writes.
- Failed idempotency claims explicitly roll back the opened ledger transaction.
- Durable events must match their authenticated intent's state type in both
  directions. Custom replay rejects unsupported event schemas and stored
  version-zero projections; domain-id parsing has dedicated fuzz coverage.

## [0.1.0] - 2026-09-20

First production release of StateChronicle, a verifiable state and transaction foundation for multiplayer game backends. This release makes the protocol, proof, fuzzing, coverage, and release gates ready for concurrent player-facing mutations while leaving durable storage ownership with the integrating application.

### Added

- Added typed resource states, canonical BCS serialization, SHA-256 content digests, Ed25519 signatures, deterministic state roots, sparse Merkle proofs, non-membership proofs, and trade proofs.
- Added backend-agnostic storage, transaction, projection, outbox, authorization, quota, key-registry, and observability ports so integrating applications can provide their own durable implementations.
- Added authenticated player and batch ingress, key-registry rotation and revocation, authorization boundaries, quota ports, retry classification, bounded metrics, and fail-closed mutation APIs.
- Added profile rules for unique assets, paid assets, balances, stacks, entitlements, meters, listings, and escrow with lifecycle, property, concurrency, and rollback coverage.
- Added 21 adversarial fuzz targets, property tests, concurrency regressions, in-process protocol throughput benchmarks, and a production release workflow with coverage, fuzzing, policy, and package evidence.

### Changed

- Durable persistence now validates intent identity, tenant and actor scope, event integrity, event roots, state roots, projection derivation, outbox payloads, commit metadata, and schema versions before writes or idempotency claims.
- Cross-tenant settlement uses deterministic lock ordering and rejects unsupported multi-database atomicity instead of weakening transaction guarantees.
- Projection rebuilds and outbox consumers are replay-safe, bounded, and resumable from verified canonical history.
- Release verification now uses real sanitizer-instrumented cargo-fuzz campaigns, a metadata-derived crates.io publish order, a 92.4% protocol-surface coverage ratchet, and in-process hot-path performance evidence.

### Fixed

- Closed malformed identifier, Unicode-limit, control-character, oversized payload, duplicate event, duplicate delivery, stale lease, forged projection, event-rewrite, fork, stale-root, and cross-tenant substitution paths.
- Fixed quota duplicate charging and cardinality-exhaustion cases, including duplicate dimensions in validated distributed quota requests.
- Fixed fuzz coverage and benchmark gates so release checks exercise instrumented targets and measure protocol work without process-startup noise.

### Security

- Durable writes fail closed on missing authorization, invalid signatures, revoked keys, malformed trust metadata, digest mismatches, unsupported schemas, forged projections, and invalid persistence metadata.
- Locked release gates check dependency advisories, licenses, bans, and sources before packaging or publication.

### Reliability

- The release passed the locked workspace suite, strict Clippy, rustdoc warnings, dependency policy, bounded instrumented fuzzing, and economy correctness benchmarks.
- Measured protocol-surface LLVM line coverage is ratcheted at a 92.4% minimum, with property, concurrency, and replay tests protecting protocol behavior.
- The release workflow validates the tag, package graph, coverage evidence, benchmark evidence, package contents, release notes, and published crate availability before completing publication.

[Unreleased]: https://github.com/STEXS-Technologies/statechronicle/compare/v0.3.0...main
[0.3.0]: https://github.com/STEXS-Technologies/statechronicle/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/STEXS-Technologies/statechronicle/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/STEXS-Technologies/statechronicle/releases/tag/v0.1.0
