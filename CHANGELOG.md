# Changelog

All notable changes to StateChronicle are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.2.0] - Unreleased

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

[0.1.0]: https://github.com/STEXS-Technologies/statechronicle/releases/tag/v0.1.0
