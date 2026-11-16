# Mixed ordered ledger history

Candidate: `c99333a` (full SHA in `manifest.json`), Rust 1.100.0.
The retained runner archives committed source before qualification; every
archived source file and retained artifact is hashed in the manifest.

All five archived checks passed: package formatting, all-target/all-feature
conformance tests (19 integration functions), doctest command (zero examples),
strict Clippy and strict rustdoc. The suite preserves every preceding conformance
check. It is an isolated unpublished dev package; production source, manifests,
lockfiles and frozen old-version fixtures are unchanged from `d829c7c`.

The new reference history has 32 commits, 16 in each of two interleaved tenants.
It deliberately reuses textual IDs across tenants. Each tenant repeatedly
transfers two assets and credits two holders of the same currency resource.
Both full signed histories are independently verified before storage use.
Expected raw prefix snapshots come from the fixture builder, not the adapter.
The passing schedule checks 224 staged/rolled-back prefixes and 528 retained
original identities with exact replay and changed-payload conflict.

Three faulty variants first pass the existing genesis rollback and two-commit
retention checks, then fail the stronger schedule with named oracles:

| Fault | Oracle |
| --- | --- |
| Prune old tenant receipts from the fourth head | `lookup.original` |
| Replace an old receipt's signed commit with the current head from the fourth head | `lookup.original` |
| Clear previously committed events during a reserved transaction rollback | `history.rollback_effects` |

Another test supplies malformed fixture lengths, broken chains and duplicate
scoped identities; setup refuses without resetting existing test history.

Reproduce from a separate checkout of the candidate: copy `runner.py` to
`target/qualify_mixed_history.py` there, then run
`python3 target/qualify_mixed_history.py`. Requires installed Rust 1.100.0,
Python 3.12+ and the dependencies cached for locked offline execution. The
runner records the checked-out commit and will refuse an existing output folder.
It does not push, publish or invoke GitHub workflows.

Development preflight logs are retained separately and are not exact-candidate
qualification. The initial compile attempt failed because TenantId is not Ord;
the initial strict Clippy attempt failed on nested if syntax. Corrected preflight
and final archived commands passed. No lint allowance or test was removed.

These results qualify a bounded logical in-memory schedule. They do not prove
real database reopen, process/power-loss durability, randomized concurrency,
mixed single/batch histories, live proof serving, production signer policy or
completion of F5/F8/the release roadmap. No library footprint or performance
improvement is claimed for added dev-only tests.
