# Rust and dependency refresh qualification

The maintainer requested latest Rust and dependencies. Production/dependency
candidate `f7cd78941febb4e3157fa3047c7f74c3ae68e380` pins Rust 1.100.0 and raises
MSRV to 1.100, updates current manifests/locks (including Dalek 3, SHA-2 0.11,
Base64 0.23 and Tokio 1.54), removes two unused dependency declarations and
updates a fuzz classifier for current Clippy. No library Rust source changes.
Tutorial-only `0dfd6ea8f54c92b954479cd9049462dcd400f1d0` then collapses nested
reservation cleanup into an equivalent let chain. Hashes and Git comparisons
bind the unchanged library/fuzz/manifests between these two revisions.

At the dependency candidate, 884 finite workspace tests and 16 doctests pass,
as do strict all-target Clippy (including fuzz compilation), strict rustdoc,
formatting and the native model example. The isolated dev conformance harness
passes finite tests, Clippy/docs/formatting. Both portable crates compile for
riscv32imafc-unknown-none-elf. The linked frozen-consumer compatibility manifest
passes candidate tests, original wire/proof expectations, hosted/portable
comparisons and its external bare-metal consumer.

The native tutorial passes all isolated checks at f7cd789. The durable tutorial
passes its tests and CLI output there but fails strict lint because the raised
MSRV enables let-chain recommendations. The entire durable isolated sequence
passes at 0dfd6ea after the cleanup. Both manifests and the failed attempt are
retained separately. No failed command is relabelled as a pass.

Audit JSON for each of the four active locks has zero vulnerabilities and no
warnings. Cargo-deny advisory/license/ban/source checks pass. Cargo-outdated
reports no dependency updates in each current resolved workspace; latest direct
versions were also checked against crates.io because inherited workspace
requirements are not comprehensively reported by older cargo-outdated tooling.
Frozen original consumers and historical evidence locks are outside that claim.
The prior two unsoundness advisories are absent from the refreshed active locks.

All 25 coverage-instrumented ASAN fuzz binaries build on updated nightly
1.102.0-nightly (5f4effd57, 2026-11-13), then pass bounded smoke with seed 7331,
1000-run and 5-second limits per target, max length 4096 and four workers. Logs
retain actual counts, coverage and exits. Initial local corpora were copied,
never mutated in place; both initial/final corpus archives and file hashes are
retained. This is not the full fuzz, performance, packaging or release campaign.

Reproduce at the pinned candidate with installed toolchains/populated caches:
`qualify.py` checks the workspace, `support.py` checks the auxiliary surfaces,
and `fuzz-smoke.py` runs built binaries after the recorded instrumented build.
The scripts use repository-relative paths and write ignored target logs.
`collect.py` is the evidence collector tied to the original qualification's
local paths/stamps; it is provenance, not a portable orchestration command.
Exact commands/environments/compiler versions are in the linked manifests.
Initial sccache fetch failures and the preflight Clippy failure are retained.
No benchmark, whole-path allocation equivalence or final release claim follows.
