# Dependency maintenance policy

This project treats the locked dependency graph as a release input. The
maintainer responsible for each release is the owner of dependency updates;
the owner must review both direct and transitive changes before merging.

## Routine updates

Dependabot opens grouped Cargo updates weekly and GitHub Actions updates
monthly. A reviewer must run the required CI workflow, inspect the generated
`Cargo.lock` diff, and confirm that public protocol formats and MSRV remain
unchanged. Updates are merged only when tests, Clippy, formatting, `cargo
audit`, and `cargo deny` pass.

## Security updates

An advisory affecting a deployed release is triaged within one business day.
The maintainer either upgrades the dependency and publishes a patch release,
or records a time-bounded justification and mitigation in the security issue.
Emergency fixes may bypass the normal weekly grouping, but still require a
reviewed lockfile diff and the full validation workflow before release.

## Release traceability

Release artifacts record the repository revision, Rust toolchain, and exact
`Cargo.lock` checksum. Consumers must build with `--locked` and retain the
artifact metadata with their deployment record.

## Ownership and escalation

The repository release maintainer owns routine updates and coordinates with
security/operations for advisories affecting signing, parsing, persistence, or
authorization. If no maintainer is available, freeze releases except for a
security patch approved by two reviewers.
