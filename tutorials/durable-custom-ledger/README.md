# Durable custom ledger, recovery and proofs

This standalone application follows two native domains from authenticated
preparation through a signed atomic mutation, lost acknowledgement, file reopen,
verified recovery, exact replay and current/historical state proofs. It uses only
the public `statechronicle` dependency and ordinary application dependencies.
Its own workspace and lockfile permit building outside the repository.

## Run

With Rust 1.100 or newer, from the repository root:

```sh
cargo run --manifest-path tutorials/durable-custom-ledger/Cargo.toml --locked
cargo test --manifest-path tutorials/durable-custom-ledger/Cargo.toml --locked
```

The program creates and removes its own temporary directory. It writes four
commits and retains four events and outbox rows; it dispatches no notification.
The exact output is in [expected-output.txt](expected-output.txt). Append
`--offline` when dependencies are cached. Clear `RUSTC_WRAPPER` if your local
compiler wrapper is broken. The path dependency selects this refactoring
checkout; the separately qualified release has not been published.

## Follow the authority

* `src/domains.rs` defines configuration and equipment types and deterministic
  rules. Each domain declares schema identity and one exhaustive operation
  mapping. `TypedProfile` supplies canonical hosted encoding. Optional delegated
  authority is appropriate here because the restricted host policy owns permission.
* `src/host.rs` wires read-only preparation to one `FileLedger`. The host binds
  tenant, authenticated principal, resource, original identity and expected
  version before signing. The policy denies unspecified routes and rechecks key
  revocation during commit. Preparation produces no durable claim.
* `Application::plan` creates a signed commit and outbox plan from the prepared
  event and current head. `SignedMutation::commit` owns the protected ledger
  sequence and validates receipts. Stale preparation cannot bypass the adapter's
  head/version checks. A final commit error returns unknown.
* `src/ledger.rs` implements `LedgerStore`, `LedgerTransaction`, recovery,
  `StateIndex`, `TenantStore` and read-only `CommitReader`. Proof verification
  needs no commit writer, snapshot store or dummy proof-index implementation.
  Original mutation effects remain immutable when newer projections replace them.
* `src/main.rs` retains the pending signed body's digest independently before
  simulating acknowledgement loss. After dropping every connection it reopens
  actual files using independently retained allowed tips and verifies original
  recovery. The exact plan then returns replay without another reduction/write.
* `ClaimVerifier` receives an independent expected tenant/resource and trusted
  commit key. The old proof fails current-head verification; its independently
  chosen historical commit still verifies. The configuration proof at the latest
  equipment commit demonstrates unchanged state under a later global root.

The fixture actor key and commit-authority key are distinct. Signer verification
checks tenant, executor, profile and key identity as well as the signature.
These fixed development keys, session, clock and event counter require replacement
with real host authentication, current policy/time, key management and durable
identity allocation. Copying a credential string is not authentication. Signature
validity does not establish historical key authorization or physical truth.

## File contract and limits

This bounded reference adapter supports one tenant, resource-keyed custom state,
one event and one outbox row per mutation, at most 64 records and an 8 MiB JSON
journal. Subject-held state and multi-tenant transactions refuse. Batch persistence
is unsupported. JSON format version 1 is application storage, not protocol wire.
No automatic schema reinterpretation or migration is performed.

A single shared owner holds an exclusive `writer.lock`. Transactions stage effects
in memory; commit validates head/version and complete effects under one mutex,
writes a new full journal, syncs the file, renames it and syncs the directory.
Only then does the in-memory head advance. Injected response loss occurs after
that sequence. Every I/O failure makes the owner unavailable until exclusive
reopen; reads cannot turn uncertain state into false absence. Pending claims are
owner-lifetime reservations, with no lease expiry or takeover.

This uses blocking filesystem calls inside bounded async methods. It assumes a
trusted directory and local filesystem supporting atomic rename and directory
sync. It is not a throughput-oriented database adapter or a generic power-loss
guarantee. Tests exercise logical failures and graceful reopen, not process kills,
filesystem corruption recovery or hardware crashes. Signed history and independent
tip pins protect commit/event consistency; unsigned application metadata still
requires authoritative storage integrity. Do not treat JSON as fully authenticated.

## Recovery decisions

1. Retain the original intent, payload digest, signed plan, last trusted tip and
   pending tip before committing. Never invent a new identity after uncertainty.
2. On unknown acknowledgement, stop further writes and perform authoritative
   lookup with current authorization and trusted commit verification.
3. A verified committed result permits exact replay and use of its original
   receipt. Conflicting payloads refuse. In-progress requires reconciliation.
4. Not-found alone does not prove an outstanding transaction cannot commit.
   Establish exclusive ownership and resolve it before deciding an exact retry.
5. On file uncertainty, release every handle before reopening. Allow only
   independently retained old/pending tips; never trust a tip read from the
   journal merely because it verifies under a key.
6. An abrupt process death leaves `writer.lock`; reopen intentionally refuses.
   An operator must establish that no writer remains, preserve the directory,
   verify a backup/history against independent pins and reconcile the write
   before removing a stale lock or scratch `journal.next`. Do not automate that
   deletion or discard evidence to force a successful reopen. Missing/corrupt or
   unpinned history requires stop/repair or restoration from a verified backup.

Tests independently inspect disk records and cover replay without changed bytes,
rollback before commit, sticky uncertainty, key revocation after preparation,
contenders, exclusive scope, native transition replay, original-effect retention,
and malformed/missing/reordered/unpinned history. These are tutorial tests, not
qualification of an arbitrary consumer backend. Use the separate adapter contract
and backend-specific crash/concurrency campaigns before production use.

## Reproduce isolated qualification

After committing source, with Python 3.12+, cached dependencies and both toolchains:

```sh
python3 scripts/check_tutorials.py --tutorial durable-custom-ledger --toolchain stable
python3 scripts/check_tutorials.py --tutorial durable-custom-ledger --toolchain 1.100.0
```

The runner extracts committed sources, copies this application outside the
repository and changes only the umbrella dependency path. It retains exact
source/toolchain/log hashes, tests, output comparison, strict Clippy and formatting
results in `docs/0.3/evidence/tutorials/`. It checks lockfile stability and absence
of new registry dependencies relative to the root lockfile. It starts no GitHub
workflow and creates no PR. Broader migration/operator documentation and full
release qualification remain roadmap work.
