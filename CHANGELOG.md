# Changelog

All notable changes to StateChronicle are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

No unreleased changes.

## [0.1.0] - 2026-09-19

This release makes StateChronicle ready for production multiplayer backend
integration. It completes the protocol, durable adapter, proof, fuzzing, and
release hardening needed for verifiable resource state under concurrent load.

### Added

- Added typed resource states, canonical BCS serialization, SHA-256 content
  digests, Ed25519 signatures, deterministic state roots, sparse Merkle proofs,
  non-membership proofs, and trade proofs.
- Added durable SQLite and PostgreSQL ledger adapters with idempotency claims,
  canonical heads, transactional projections, outbox delivery, lease takeover,
  integrity scans, rebuild checkpoints, backups, and crash recovery tests.
- Added authenticated player and batch ingress, key-registry rotation and
  revocation, authorization boundaries, quota ports, retry classification,
  bounded metrics, and fail-closed durable write APIs.
- Added profile rules for unique assets, paid assets, balances, stacks,
  entitlements, meters, listings, and escrow with property and lifecycle tests.
- Added 21 adversarial fuzz targets, property tests, concurrency regressions,
  adapter load drills, chaos drills, and economy-shaped release benchmarks.
- Added a production release workflow with tag validation, reliability gates,
  coverage evidence, benchmark evidence, package verification, and optional
  crates.io publishing.
- Added a 92.4% protocol-surface LLVM line-coverage ratchet with retained JSON
  evidence and live PostgreSQL coverage execution.
- Added Shardline-style reusable Rust CI setup, coverage, and reliability-soak
  workflows with bounded artifact retention.

### Changed

- Durable persistence validates intent identity, tenant and actor scope, event
  integrity, event roots, state roots, projection derivation, outbox payloads,
  commit metadata, and schema versions before writes or idempotency claims.
- Cross-tenant settlement uses deterministic lock ordering and rejects
  unsupported multi-database atomicity rather than weakening guarantees.
- Projection rebuilds and outbox consumers are replay-safe, bounded, and
  resumable from verified canonical history.
- Public APIs distinguish pure planning from durable mutation and require an
  explicit verified persistence boundary for production writes.
- Release checks now use real nightly `cargo fuzz` instrumentation instead of
  running non-instrumented fuzz binaries.
- Parallel fuzz campaigns now run sanitizer-instrumented release targets and
  fail if any target exits abnormally.
- JSON serialization tests inspect decoded structures instead of matching
  serialized substrings.

### Fixed

- Closed malformed identifier, Unicode-limit, control-character, oversized
  payload, duplicate event, duplicate delivery, stale lease, forged projection,
  event-rewrite, fork, stale-root, and cross-tenant substitution paths.
- Fixed quota duplicate charging and cardinality-exhaustion cases.
- Fixed fuzz coverage tooling so release smoke tests exercise sanitizer
  coverage rather than ordinary binaries.

### Security

- Durable writes fail closed on missing authorization, invalid signatures,
  revoked keys, malformed trust metadata, digest mismatches, and unsupported
  schemas.
- Dependency advisories, licenses, bans, and sources are checked under the
  locked release gate.

### Reliability

- The release passed the locked workspace suite, strict Clippy,
  rustdoc warnings, dependency policy, live PostgreSQL integration, SQLite
  recovery/load drills, bounded instrumented fuzzing, and economy benchmarks.
- Fuzz status validation now measures Unicode character length consistently with
  the production identifier limit; a Unicode input previously exposed an
  incorrect harness assertion.
- Release verification retains separate live PostgreSQL and SQLite adapter
  gates alongside protocol-surface coverage.
- Measured protocol-surface coverage is ratcheted at 92.4% minimum; adapter
  behavior remains protected by dedicated live reliability suites.

[Unreleased]: https://github.com/STEXS-Technologies/statechronicle/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/STEXS-Technologies/statechronicle/releases/tag/v0.1.0
