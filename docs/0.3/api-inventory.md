# API inventory work map

Status: owner map plus original/candidate compiler catalogs; final release API review remains open.
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

`TypedProfile::encode_command` adds a public native-to-wire authoring helper:
it returns `(Operation, BTreeMap<String, Value>)` from the adapter's identity,
schema, operation mapping and bounded BCS encoder. The pair carries no trust
claim. No existing trait signature, error variant or protocol type changes.


`command_operations!` is an additive hosted declarative macro, reachable through
`statechronicle::command_operations!` and the profiles crate. It emits the
existing static operation list and an exhaustive discriminator match from one
mapping; it introduces no trait or wire schema. Existing profile/adapter methods
and manual registration remain unchanged. Macro-generated consumer API is not
captured by the syntax metric. In the subsequent MSRV compatibility correction,
three immutable receipt accessors become `const fn` and `projections_for` elides
an equivalent lifetime; existing calls and lifetime relationships are unchanged.


The later maintainer-requested [dependency refresh](dependency-refresh.md) pins
Rust 1.100.0, raises the declared minimum to Rust 1.100 and returns the current
public key types to Ed25519 Dalek 3. Earlier version-2 and Rust-1.85 evidence
above describes those exact historical checkpoints, not the refreshed tree.
Frozen original sources and measurement artifacts remain unchanged. Consumers
of interim version-2 checkpoints must align their direct key-type dependency.

`Preparer::prepare_player_settlement` and the matching `Executor` forwarding
method add explicit protected single-tenant settlement composition. Both return
the existing private-construction `PreparedBatch`; `SignedBatch` remains the
canonical binding/commit owner. They reuse per-member ingress checks and legacy
settlement validators. Resource-profile bundle declarations now require a
coordinator, an intentional validation tightening described in migration.md.
See [settlement checkpoint](evidence/f1/protected-settlement/README.md) for exact
source, passing consumer checks and remaining canonical contention gates.

## Current compiler comparison

The [compiler checkpoint](evidence/api/README.md) compares all thirteen original
and candidate libraries with the same nightly compiler, including hidden public
items and compiler-expanded implementations. It retains raw and normalized JSON,
source/lock guards, reproducible collection and independent verification. There
are 86 added export records, no removed records and 22 changed records at
`64c5350`; nested methods and impls are reviewed separately from export counts.
This complements the frozen external implementer matrix and does not establish
SemVer compatibility or close the public-concept budget by itself.
