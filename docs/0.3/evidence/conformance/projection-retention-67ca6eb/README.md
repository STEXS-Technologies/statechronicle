# Same-key projection retention qualification

Exact candidate `67ca6eb4dc8a4ca2437fb3f7abab2c78b6ac0984` passes all ten checks: stable and Rust 1.85.0 each run 15 integration functions, zero doctest examples, strict Clippy, strict rustdoc and formatting.

The dev-only reference now upserts current projections by tenant/resource and subject for subject-held states, with version checks before publishing committed rows. Immutable receipt projections stay separate. Native profile check/reduction produces a unique-asset transfer; independently signed sequential commits replace the current owner/version while preserving exact original receipts through later-head retry and outbox acknowledgement. Another signed pair creates balances for different holders of the same currency resource and retains both current rows. Explicit expected snapshots do not reuse the reference upsert helper. A deliberately invalid equal-version projection is refused with every original committed row and receipt unchanged. This does not establish pending-reservation cleanup after refusal.

Two additional faulty modes append projections or erase another holder; both fail `retention.effects`. Existing lookup/pruning/pending-outbox faults continue to fail `lookup.original`, including same-key replacement. Historical different-resource coverage remains.

Root production crates/manifests/lockfile/fuzz are unchanged from `4052146`; matching Git objects are recorded. No production source footprint, latency, workspace qualification or release-pass claim follows from this dev-only change. Batch retention/rebuild, long/corrupt/interrupted histories and real backend crash/reopen remain open.

`qualify.py` archives committed HEAD and runs both installed compilers against isolated source/target directories, offline with cached dependencies. Machine paths in `manifest.json` describe this actual run. Logs, compiler versions, commands, source and artifact hashes are retained. The initial unqualified compile failure is preserved alongside corrected worktree preflights and AST-only graph update.
