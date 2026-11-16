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
boundaries. The compiler snapshot and feature inventory below supply full signatures and
trait declarations; the frozen external matrix exercises the promised legacy
imports and selected exhaustive enum matches independently.

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

The original dependency graph in
`evidence/costs/20261116T021527Z-2997189/dependency-graph.log.gz` records every
package dependency and activated feature. All 13 StateChronicle library crate
manifests declare no custom Cargo features (`features = {}`); default and
all-features library surfaces therefore have no package-defined alternatives.
Third-party feature unification is still recorded and independently excluded
from the bare-metal consumer builds. Cold facade build cost is measured with
only the external `statechronicle` dependency, not workspace dev dependencies.

A complete original-release rustdoc JSON snapshot is retained for all thirteen
library crates in `evidence/baseline/20261116T022002Z-3050762/`. Unlike the syntax
inventory, these compiler outputs include generated implementations and resolved
public declarations. The manifest pins nightly and rustdoc flags; every
compressed crate snapshot has a hash. Regenerate reference snapshots only from
the pinned original source, with `scripts/refactor_baseline.py --api-inventory`.

The additive `proof::claim` surface introduces `ExpectedClaim`, borrowed
`ClaimProof`, `VerificationPoint`, and private-field `VerifiedClaim`, with
`ProofService::verify_current` and `verify_historical`. No existing error enum
variants, wire structs or serialized tags change. Existing verification helpers
retain compatibility semantics. Candidate-only consumer overlays under
`ci/canonical-0.3/` compile the new surface against original proof records; their
hashes are recorded separately from the immutable original consumer manifest.

The hosted decode-once change adds the object-safe default
`ProfileRules::evaluate(current, operation, inputs)` returning the proposed
`ResourceState` or `ProfileEvaluationError::{Check, Apply}`. The default preserves
existing implementations' check/apply behavior. `TypedProfile` overrides it to
share native decoded values; its existing methods retain their contracts.
`StateProfile` and all existing exhaustive error enums remain unchanged. This
adds a method and a new enum, not a variant to a previously exhaustive enum.
External legacy consumers pass on Rust 1.85 and 1.100 at `d1550a9`.
