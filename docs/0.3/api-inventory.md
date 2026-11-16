# API inventory work map

Status: initial owner map, not a complete generated public API snapshot.
Reference: original release `c418e98b91720188158c531cd5c2c4b8ab71cbcf`.

| Consumer contract | Authoritative source | Compatibility evidence needed |
| --- | --- | --- |
| Native state/command semantics | `statechronicle-profile/src/contract.rs` | Downstream implementation, associated types, default methods, no-std |
| Hosted native adaptation | `statechronicle-profiles/src/custom.rs` | TypedProfile construction and operation registration |
| Domain routing/policy | `statechronicle-profiles/src/registry.rs` | Existing ProfileRules implementations and dispatch errors |
| Planning and authenticated execution | `statechronicle-executor/src/pipeline.rs` | Every public execute variant, sinks, builders, error precedence |
| Signed durable persistence | `statechronicle-commit/src/persist.rs` | Ledger validation, all receipt/retry/failure paths |
| Atomic transaction authority | `statechronicle-ports/src/ledger_store.rs` | External adapter implementations and transaction method contracts |
| Symbolic planning lifecycle | `statechronicle-ports/src/transaction_manager.rs` | Existing adapters; no implied durable authority |
| Proof trust/freshness | `statechronicle-proof/src/service.rs` | Old verifier semantics and explicit supplied signer keys |
| Pure proof verification | `statechronicle-proof/src/verify.rs` | Exact wire/root/signature vectors |
| Replay and projections | `statechronicle-index/src/{build,rebuild,history}.rs` | Original histories and interrupted rebuild |
| Umbrella imports | `statechronicle/src/lib.rs` | Namespaced and flat imports in frozen external consumer |

The umbrella re-exports its underlying protocol crates and selected flat names.
A new curated facade must preserve promised imports while documenting authority
boundaries. This map does not yet inventory all signatures, enum variants,
feature combinations or exhaustive matches. F0 remains open until those are
captured and the external consumer matrix executes independently.

The baseline [metrics.json](metrics.json) now includes a parser-derived syntax
inventory of public declarations and trait member signatures, with source file,
line and source hash. It does not resolve re-export reachability or macros.
Frozen external exhaustive matches cover `StateType`, `ProofType`, `DomainError`,
`IntentError`, `ExecutorError`, `CommitError`, `ProofError`, and `ProfileError`.
A new variant in these enums will require an explicit source-compatibility
decision instead of silently passing a wildcard match.

The pre-refactor starting point also changes the public Ed25519 dependency
major version from 3 to 2. The external consumer declares `>=2, <4` and uses each
revision's existing lockfile; exact package-version/checksum checks forbid
unrecorded drift. The original signature corpus is verified across this change.
This does not promise that downstream crates declaring only major version 3
can directly exchange key objects with major version 2.
