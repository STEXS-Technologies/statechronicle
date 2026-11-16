# Original configuration-history producer

The producer runs against an archive of the actual original release
`c418e98b91720188158c531cd5c2c4b8ab71cbcf`. It creates a separate new four-commit
configuration corpus; existing `ci/wire-0.2` and frozen consumers are untouched.
The [manifest](manifest.json) records the original archive and every original
source/lock hash, exact producer source, real Cargo command/exit, Rust compiler,
compressed build/output log and corpus SHA-256. All original files are checked
for drift after execution. The original lockfile is used with `--locked
--offline`; no dependency-patched comparator is substituted.

`migration_history.rs` is added only to the isolated archive as an example. Its
output contains original events/intents/signed commits and original canonical
BCS arrays. The fixed signing key, clock and tenant come from the original
release's unchanged example harness. This is test provenance, not a production
key-management or current-session authorization policy.

The retained `runner.py` refuses an existing corpus or evidence directory.
Generation is a one-time provenance action; ordinary candidate tests only read
the frozen corpus and compare its bytes/hash. See the [authorized migration
checkpoint](../authorized-migration-7489a5f/README.md) for consumers, controls and
scope limits.
