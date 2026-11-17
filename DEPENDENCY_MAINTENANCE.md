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

## Temporary Chrono maintenance exception

The release maintainer owns the exception for
[RUSTSEC-2026-0543](https://github.com/RustSec/advisory-db/blob/main/crates/chrono/RUSTSEC-2026-0543.md),
which reports Chrono as unmaintained with no patched version. This is a
maintenance warning, not a reported vulnerability. Continuing to depend on it
carries the risk that future defects will not receive upstream fixes.

Chrono's `DateTime<Utc>` appears in the public API. Replacing it requires a
planned compatibility migration and wire-format validation. Until that
migration, only this advisory is ignored by Cargo Audit and Cargo Deny;
all other advisory checks remain enabled, including audit warnings.

The exception expires on **2026-12-17 (UTC)**. The release driver rejects the
exception on that date and later, and checks that both tool configurations
contain exactly the reviewed advisory. Before expiry, the release maintainer
must complete the migration or explicitly review and document a new deadline.
