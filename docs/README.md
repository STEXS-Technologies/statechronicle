# Documentation

Start with the [consumer guide](0.3/consumer-guide.md) for the 0.3 prepare,
commit, recover and verify workflow. Workspace packages are versioned `0.3.0`;
tagging and registry publication follow release review.

## Build an application

- [Native-domain tutorial](../tutorials/minimal-native-domain/README.md): define typed state and commands, then authenticate and prepare a batch.
- [Durable-ledger tutorial](../tutorials/durable-custom-ledger/README.md): commit signed mutations, recover uncertain writes and verify claims.
- [Adapter contract](0.3/adapter-contract.md): storage atomicity, recovery and conformance responsibilities.
- [Proof claims](0.3/proof-claims.md): current state versus independently pinned historical evidence.
- [Verified rebuild](0.3/verified-rebuild.md): verify signed history and resume projection rebuilds.
- [Migration](0.3/migration.md) and [compatibility](0.3/release-compatibility.md): upgrading from 0.2 to 0.3.

## Understand and operate the library

- [Architecture](ARCHITECTURE.md), [design decisions](DESIGN/ADR/README.md) and [domain extensions](DESIGN/DOMAIN_EXTENSIONS.md).
- [Deployment](OPERATIONS/DEPLOYMENT.md), [recovery](OPERATIONS/RECOVERY.md) and [observability](OPERATIONS/OBSERVABILITY.md).
- [Crate API overview](../crates/statechronicle/README.md) and [port traits](../crates/statechronicle-ports/README.md).

## Contribute and verify

- [Rust support](0.3/rust-support.md): supported application compiler versus development tooling.
- [Code standards](GOVERNANCE/CODE_STANDARDS.md) and [dependency maintenance](../DEPENDENCY_MAINTENANCE.md).
- [Local release checks](0.3/release-checks.md), [coverage](0.3/coverage.md) and [package checks](0.3/package-checks.md).
- [Qualification summary](0.3/qualification.md), [changelog](../CHANGELOG.md) and [security policy](../SECURITY.md).

Generated run logs, build archives and completed engineering work notes are
excluded from the current tree. The qualification summary links their immutable
historical record. Frozen compatibility fixtures remain versioned where checks
require them.
