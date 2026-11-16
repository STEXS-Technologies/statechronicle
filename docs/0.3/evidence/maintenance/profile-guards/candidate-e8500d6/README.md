# Shared profile guard extraction

Exact candidate `e8500d6816c5f9d58abe830c37a983d057fed743`, immediate control
`89e2494cce934a1ad7adec47255fb4fccc44cc52`. The control freezes the original
behavior oracle before production changes. The parent directory contains its
original-release provenance, source producer and immutable trace digest.

This extraction shares positive-availability and status-only guards, and merges
three identical decrease checks within each economic profile. It preserves
operation-specific error names and all domain-specific gate positions. Stack
zero-credit acceptance and balance positive-credit requirements stay distinct.
No exported API, dependency version, wire encoding, reduction or portable
implementation changes. Root and conformance manifests/locks match the control.

Stable 1.100.0 and MSRV 1.85.0 each pass 863 finite workspace tests plus nine
doctests, including the full 14,364-case original error trace. The isolated
adapter harness passes nine integration functions per compiler. Strict stable
Clippy and rustdoc pass; Cargo emits advisory unused-dependency manifest
warnings, retained in the logs. MSRV external compatibility independently archives
the candidate and checks frozen consumers/wire plus the canonical overlay,
portable tests and bare-metal build. Its manifest is linked from `manifest.json`.
These are targeted qualification evidence, not a completed F8 campaign.

Initial full-suite/Clippy attempts failed because moving a production Status
import also removed a transitive test import. The fix imports Status directly in
three test modules. Failed logs remain explicitly unqualified. Successful
workspace checks ran against the clean production worktree matching the commit;
the external consumer and cost harness use independently archived source.

`metrics-summary.json` compares the same pinned parser/method on both commits.
Production syntax decreases by 72 lines, 517 tokens and 18 branch nodes. Across
the six touched files this is 5.49%, 5.56% and 7.17%; across the full workspace
it is 0.44%, 0.44% and 0.91%. Raw tracked bytes decrease 4,126 but include
comments and tests. The three additional public syntax items are crate-private
helpers, not new exported API. Source bytes alone are not a production-code
metric; syntax counts are not compiled binary size or cyclomatic complexity.
Whole-roadmap footprint targets are not met.

[Performance evidence](performance/README.md) retains seven-pair source
comparisons, a repeat after local builds and an identical-binary noise control.
Allocation calls and requested bytes match for every case and pair. Timing is
inconclusive for tail performance; possible instruction-layout overhead and
host interference remain for the final isolated campaign. No speedup or full
performance pass is asserted. All samples, source hashes and reproducible
runners remain retained.
