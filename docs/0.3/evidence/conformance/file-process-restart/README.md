# File-backed process termination and restart

Exact source `574fcede921e9ea09dfae949ac49a58181bcbb50` advances F5's
backend-specific subprocess qualification for the bounded durable tutorial.
It changes tutorial code only: library crates, manifests, dependencies, lockfiles,
wire representations and GitHub workflows remain identical to `3ea6d31`.
Rust remains pinned to 1.100.0 with the already refreshed dependencies.

The adapter now uses `File::try_lock` on a persistent inode. The OS releases the
exclusive lock on process death; unlinking the lock file is forbidden. Once the
authoritative journal verifies against independently retained allowed tips,
reopen can quarantine an interrupted scratch file using a non-overwriting hard
link. It never promotes scratch content. An occupied evidence slot or non-file
scratch refuses safely. The tutorial README describes operator reconciliation.

A Unix subprocess test starts from one real committed mutation. Parent and child
independently reconstruct the same pending signed update and compare its digest
before writing. The child pauses at six explicit test-only boundaries: before
write, partial write, after write, after file sync, after rename and after directory
sync. An atomic readiness-file rename synchronizes the parent; a deadline and
child watchdog bound failures. The parent verifies that a second opener refuses,
uses actual SIGKILL, waits for signal 9, and reopens with only old/pending pins.
The child guard kills and reaps on parent-test failure as well.

Before rename, authoritative bytes remain unchanged; after rename, the pending
record is present. Reopen preserves scratch bytes separately when applicable.
The test verifies original identity lookup, the original committed receipt or
exact-plan retry, two retained complete effect sets, subsequent replay without
changed bytes, conflicting-payload refusal, and current/historical proof behavior.
Another test proves incorrect pins leave scratch untouched and occupied evidence
is never overwritten. Existing corruption/history tests still run.

[Isolated exact-source qualification](../../tutorials/durable-custom-ledger-20261116T120608Z-2582327/manifest.json)
passes locked/offline metadata, **12 tests** (including a worker entry that returns
immediately outside child mode), exact demo output, strict all-target/all-feature
Clippy and package formatting on Rust 1.100.0. The application is built outside
the checkout with only its declared dependencies. Tutorial/root lockfile identity,
registry dependency containment and unchanged archived library sources are checked
by the existing runner. No new root-workspace test run is claimed for this
tutorial-only change. Local exploratory tests/Clippy also passed before commit.

The [negative control](lock-negative.json) extracts the same committed source
outside the checkout and removes only `lock.try_lock()`. Running the unchanged
subprocess test fails with exit 101 at its live-owner exclusion assertion. Thus
the test detects a concrete unsafe two-writer backend behavior. It does not prove
that every possible backend fault is detected. The [runner](lock-negative.py)
and compressed raw log retain the mutation, source/archive hashes and command;
run the script from the repository root to reproduce. Child cleanup remains in
place on the deliberately failing assertion. The checkout was never mutated.

These results concern process termination on the test host's local filesystem,
not loss of the kernel/page cache, physical power-loss durability, corruption
repair, arbitrary filesystems, production keys, throughput, multi-tenant batches,
randomized mixed-history concurrency or schema migration. The test hooks and
partial-write injection compile only into the unit-test binary. No production
environment-triggered pause is added. F5/F8 and the broader roadmap remain open;
no PR, CI dispatch, release tag or package publication is authorized here.
