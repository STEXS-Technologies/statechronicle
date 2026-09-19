# Changelog

All notable changes to StateChronicle are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-09-19

First production release of StateChronicle, a verifiable state and transaction foundation for multiplayer game backends. This release completes the protocol, durable adapter, proof, fuzzing, coverage, and release hardening needed for concurrent player-facing mutations.

### Added

- Added typed resource states, canonical BCS serialization, SHA-256 content digests, Ed25519 signatures, deterministic state roots, sparse Merkle proofs, non-membership proofs, and trade proofs.
- Added durable SQLite and PostgreSQL ledger adapters with idempotency claims, canonical heads, transactional projections, outbox delivery, lease takeover, integrity scans, rebuild checkpoints, backups, and crash recovery coverage.
- Added authenticated player and batch ingress, key-registry rotation and revocation, authorization boundaries, quota ports, retry classification, bounded metrics, and fail-closed durable write APIs.
- Added profile rules for unique assets, paid assets, balances, stacks, entitlements, meters, listings, and escrow with lifecycle, property, concurrency, and rollback coverage.
- Added 21 adversarial fuzz targets, in-process protocol throughput benchmarks, adapter load drills, PostgreSQL chaos drills, and a production release workflow with coverage and package evidence.

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

- The release passed the locked workspace suite, strict Clippy, rustdoc warnings, dependency policy, live PostgreSQL integration, SQLite recovery and load drills, bounded instrumented fuzzing, and economy correctness benchmarks.
- Measured protocol-surface LLVM line coverage is ratcheted at a 92.4% minimum, with separate live PostgreSQL and SQLite reliability gates protecting adapter behavior.
- The release workflow validates the tag, package graph, coverage evidence, load evidence, package contents, release notes, and published crate availability before completing publication.

[0.1.0]: https://github.com/STEXS-Technologies/statechronicle/releases/tag/v0.1.0
