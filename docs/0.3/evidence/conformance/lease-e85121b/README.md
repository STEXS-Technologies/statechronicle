# Single-reservation lease takeover qualification

Source: `e85121b`, extracted with `git archive` into an isolated directory.
The manifest records its archive digest, exact source/lock digests, toolchains,
commands, exit codes and compressed log digests. No edits occurred to that
archive during qualification. All five commands pass: formatting, Rust 1.85.0
and 1.100.0 all-target tests (nine integration functions each), strict Clippy
and strict rustdoc. The two added broken behaviors fail their intended named
oracles within the passing integration test.

Production `crates/` remains tree `0068b705d6ee332bcfd1b88a55fa99b75d67ec15`.
Root and isolated harness manifests/locks match the parent byte for byte.
Consequently this checkpoint changes no production footprint, dependency or
runtime performance. It adds a dev-only optional capability and schedule;
it is not full F5/F8 qualification.

The reference expiry hook marks a reservation expired without deleting it or
closing its old transaction. A normal claim then performs fenced replacement.
This deterministic model does not test database time, statement failures,
physical persistence, all interleavings or batch-member expiry. See the
[consumer capability instructions](../../../../../ci/conformance/README.md).
