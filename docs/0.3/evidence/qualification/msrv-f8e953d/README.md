# MSRV and fuzz-tool compatibility correction

Exact candidate `f8e953d6e01a862b569f7fdae110aec53efb9211`. Production Rust is
identical to `d20a5cc`; the final follow-up declares `rust-version.workspace = true`
for the unpublished fuzz package. No root dependency/lock/default-feature/MSRV
change or supported wire change. The existing library MSRV remains Rust 1.85.

An additional strict MSRV Clippy run after operation registration exposed issues
that stable Clippy did not report. The corrections are separate from the macro:

- Three immutable receipt accessors become `const fn`; `projections_for` elides
  an equivalent named lifetime. Existing call semantics remain unchanged.
- The settle-count conditional collapses without changing predicate/error order.
- Value-pair classification uses `Amount::cmp` once, with greater/less/equal arms
  matching the previous comparisons. `Amount::partial_cmp` delegates to this same
  total ordering. Checked subtraction/addition, error details and actor lookup
  remain in the same semantic branches. This removes a repeated comparison on
  credit/equal paths; no speedup is claimed.
- Compile-time test helpers become const. A proof test borrows its genuine root
  instead of cloning it; the inequality assertion remains identical. Golden hex
  output is written into one String instead of collecting temporary strings.
- Unsigned divisibility conditions use `rem_euclid` with the same positive
  constants. This retains test/fuzz input classification and removes allowances
  for a lint unknown to MSRV. No broad lint suppression is added.
- The portable fuzz harness uses `chunks_exact(33)` instead of `as_chunks::<33>`:
  both produce complete 33-byte groups and ignore the remainder, but the latter
  API is newer than MSRV. No library/portable proof algorithm is changed.
- Declaring the fuzz package MSRV also prevents stable Clippy recommending that
  newer API again. This package is unpublished dev tooling.

The final source passes 865 finite workspace tests and 13 doctests on both
stable 1.100.0 and MSRV 1.85.0. Strict whole-workspace/all-target/all-feature
Clippy (including compilation of fuzz harnesses), strict rustdoc and running the
native-domain example pass on both compilers. Tests deliberately exclude fuzz
binaries to avoid unbounded execution. The archived MSRV external consumer/wire,
portable tests and bare-metal compilation also pass; see the manifest link.
No tests, assertions or authority checks are removed or weakened.

Four modified harnesses receive a separate instrumented cargo-fuzz campaign
with the default address sanitizer, fixed seed 20261116, 22 generated initial
files (including empty input), a 4,096-run cap and a 30-second cap per target.
Input lengths include codec/proof chunk and capacity boundaries. The final run
manifests record actual counts, not promised budgets. Build/run logs retain
coverage counter loading; the optimized binaries are stripped, so symbol-table
inspection was inconclusive; separate bounded initialization probes record `AddressSanitizer Init done`. Generated final
corpora are retained as compressed tar files, with deterministic seed generators.
The earlier successful pre-MSRV-declaration campaign is also retained. These are
bounded regression smokes for four harnesses, not the complete F8 fuzz campaign.

`unqualified-attempts` preserves failed incremental strict-lint attempts,
including the stable `chunks_exact_to_as_chunks` conflict before the fuzz
manifest inherited MSRV. Those logs do not qualify any source. Exact candidate
checks are in `manifest.json`; reproducible runners write under `target/` and
should be run from repository root in a fresh disposable checkout so previous
corpora/logs cannot contaminate a rerun. The external matrix has its own archive
and provenance checks.

Relative to the operation-registration commit, production syntax increases by
three lines/28 tokens/one parsed branch node; raw tracked source increases 179
bytes (including embedded tests). This is compatibility cleanup, not a code
reduction claim. Full totals and parser limitations are in `metrics-summary.json`.
F1–F7 completion, all-target fuzz budgets/properties, real-backend crash and
concurrency qualification, coverage, security advisory resolution, isolated
performance, API review and package/release gates remain open.
