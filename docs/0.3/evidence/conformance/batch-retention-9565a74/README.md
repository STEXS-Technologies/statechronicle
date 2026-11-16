# Complete batch retention qualification

Exact candidate `9565a74bb5b33429b4e40a8a9b96c12aab2b1c06` passes all ten checks: stable and Rust 1.85.0 each run 16 integration functions, zero doctest examples, strict Clippy, strict rustdoc and formatting.

The optional dev harness checks two consecutive signed single-tenant batches against an independently supplied final raw snapshot and every separately inspected retained member row. Both resource projections are replaced by native unique-asset transfers, while original group projections/outbox stay immutable. Every member of each group must resolve to the complete original group and refuse standalone lookup. Exact retries identify the original commit; changed nonce retries conflict. All observations are repeated after acknowledging only the first group's pending outbox rows, including original lookup, namespace, retry, raw membership and effects. Shared private helpers own membership comparison and exact/conflicting member retries.

Seven faulty retention/retry behaviors fail their named oracles in the manifest. One corrupts only the second old member after head advance, and the test explicitly asserts `member 1`; another drops only that raw membership row. These controls distinguish membership storage, lookup reconstruction, retry identity and current projection failures.

This is a bounded in-memory logical schedule with fresh connections. It does not establish arbitrary replay/rebuild, mixed single/batch histories, large groups, real database restart/crash/power-loss, full F5/F8 or release readiness. Fixture signature/native reduction checks are separate from ingress and signer authorization. Root crates/manifests/lockfile/fuzz remain byte-identical to `7e311a1`; Git objects are recorded.

Run `qualify.py` from the repository root at the candidate with installed compilers and cached dependencies. It archives committed HEAD and uses isolated source/target directories. The manifest retains actual machine paths, compiler versions, commands, source/archive/log hashes and scope. Worktree preflights and AST-only graph update are retained separately and are not exact-archive qualification.
