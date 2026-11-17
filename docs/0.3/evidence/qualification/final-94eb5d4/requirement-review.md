# Requirement review against the unchanged roadmap

This review uses the literal 138 clauses in `requirement-inventory.json`.
`supplemental-requirements.json` adds 84 success, design, invariant, phase-exit,
performance, risk, evidence-policy and maintainer clauses. The performance table
and phase exit criteria are therefore explicit audit inputs too.
`design-requirements.json` preserves another 85 literal H1–H8 work/acceptance,
file-map, authority-question, footprint and change/stop-discipline clauses.
All 307 inventory rows remain in scope; overlap is retained rather than treated
as independent proof.
It is an evidence interpretation, not an additional test result. Final closure
uses the terminal manifests and accepted performance disposition; this document
does not qualify queued work. The numbered roadmap remains the specification.

## F0: original reference and current conditions

The original producer is commit `c418e98b91720188158c531cd5c2c4b8ab71cbcf`,
not the interim dependency-refresh checkpoint. `v0.2.0` is an annotated tag:
its tag object and peeled commit have different hashes. `verify_baseline.py`
checks the peeled commit, complete original archive and source hashes, the
794-test log, the failed original advisory gate and separate instrumented fuzz
campaign. The policy failure is not waived or relabeled. The current local
release driver separately passes the current approved dependency policy.

Frozen `ci/wire-0.2` and `ci/compat-0.2` provenance, the original configuration
migration producer and final compatibility campaign supply original bytes and
signatures. New receipt APIs did not exist in 0.2: the old producer freezes old
records, while candidate canonical fixtures separately exercise receipt/recovery
contracts. Original legacy imports and exhaustive matches are compiled, rather
than asserting all conceivable downstream source compatibility.

The API catalogs cover thirteen crates and inherited/generated items; the
structural review distinguishes aliases, lifetime spelling and blanket impls
from removed contracts. No package defines a custom feature matrix; portable
consumers are compiled separately without hosted feature unification. The
approved toolchain/dependency decision supersedes the original 1.85 constraint.
Source metrics cover production syntax and retained duplicates, with macro and
branch-count limitations. Baseline cost and workload observations are replaced
for final comparison by matched four-lane measurements and repeated cold builds.

The reproduced hazards (supplied absence key/freshness, marker-only success,
trade tenant/cache scope and unsigned trade metadata) are recorded in
`findings.md`. Their intended replacements and residual legacy semantics are
explicit, rather than treating compatibility behavior as newly safe.

## F1 and correctness gate 7.2: independent authority

Claim tests include independent expected address/owner/signer/kind/pin,
current-head rechecking, swapped keys and metadata, failed reads, historical
absence and present claims. `claims_readonly.rs`, `ci/canonical-0.3/proof_claims.rs`
and proof crate tests run in the final driver. The current claim fuzz campaign
adds instrumented execution and detected head/pin bypass controls. Canonical
claims are observed-current at the final authoritative read, not an assertion
that state cannot change after return.

Trade verification now scopes commit lookup by tenant and verifies leg scope.
Historical settlement facts use explicit expectations and signed event roots;
unsigned summary metadata is never promoted to authenticated settlement history.
Old state proofs retain their narrower contract. Actual-original/current
proof-service and settlement traces preserve expected legacy behavior except the
separately documented intentional trust fixes; exact errors and callbacks are
compared without diagnostic normalization.

`durable_recovery.rs` and `durable_batch.rs` exercise authoritative original
receipts, exact membership, same/conflicting identities, signer and policy
refusals, all staged failure boundaries, cancellation, lost ACK, failed readback,
active/absent reservations and corrupt records. The signed-plan path verifies
authenticated scope and the exact event set before persistence. Preparation has
no marker/ledger mutation capability. Unknown never becomes safe failure or a
new identity. The two-seed current durable fuzz campaign and its ACK/signature
bypass controls supplement these deterministic assertions. Authenticity and
physical atomicity still depend on the injected host policy and conforming
backend; the library cannot make an adversarial port truthful.

## F2: ordered shared preparation

`preparation-pipeline.md` and `batch-preparation.md` define the ordered gates.
One private evaluator serves read-only preparation and the compatibility
executor; the legacy marker capability is explicit and absent in preparation.
Single, batch and settlement traces compare actual original and current outputs,
errors and callbacks. The six strict characterization campaigns together cover
348,621 distinct cases, with compiled library markers and archived source
verified by `verify_semantic_parity.py`.

The read-only composition tests check complete bundles, ingress-before-state
ordering, late batch refusal and unchanged effects. Native combined evaluation
decodes once but retains output-state validation. Typed model failures and
storage uncertainty retain their distinct contracts. Private modules follow
these owners; no new framework or published crate is required.

## F3 and usability gate 7.3: native consumers

The minimal native tutorial is an independent application with ordinary state
and command types, static typed operation registration and an explicit schema.
It imports the public umbrella, not private implementation modules. The durable
file tutorial independently composes preparation, signing, receipts, recovery
and expected current/historical claims. Both are copied and built with their
own locks by the final driver; the final run contains four native and twelve
file-tutorial tests plus execution, formatting and strict lint checks.

The native-contract suite checks mixed command/state/event types, non-exhaustive
operation registration, forbidden ownership of a prepared batch leg, and missing
historical pins through intended compiler diagnostics. Runtime schema/domain/
operation checks remain where string identities cannot be proved by Rust types.
Neither a compile error from an unrelated cause nor a runtime control that
compiles incorrectly counts as success. The consumer guide gives one short
profile-responsibility diagram and an API decision table. The portable custom
consumer has independent fixed buffers and a bare-metal target; allocation
measurements independently confirm its zero-allocation requirement.

## F4 and F7: canonical path and documentation

The root README, consumer guide and migration map consistently lead through
native domain, protected preparation, signed atomic commit, authoritative
receipt/recovery and explicitly current or pinned-historical proof. Public
ports are selected capabilities, not a universal write-capable read service.
The final external benchmark implements these ports using only public APIs;
`api-review.md` documents the resulting consumer review and outcome decisions.
This is an agent consumer review, not a human maintainer endorsement.

`TransactionManager` and legacy sink/intent shortcuts remain labeled advanced
planning/compatibility. They are not aliases for ledger transactions. A successful
canonical receipt binds signed original records and retained complete membership;
a prepared/signed plan does not. Unsupported cross-tenant storage fails before
partial writes. Separate advanced coordination is not presented as a generic
multi-database transaction.

Migration documents the approved Rust/Dalek type boundary, unchanged old wire
meaning, method/import mapping and explicit schema policy. Operator recovery
covers lookup and exact-identity retry, reconciliation, stop/repair and verified
restore. Historical ADRs and dated evidence remain traceable; current links
supersede their old pending statements instead of silently rewriting results.
The changelog and release compatibility table describe the actual branch,
without implying a version bump or registry release.

## F5 and scenarios A–E: bounded backend and history checks

The unpublished conformance package provides independent handles, failure
injection, reopen/inspection and deterministic single/batch vectors. It exercises
2/8/32 contenders, same/conflicting identities, tenant separation, rollback,
canonical-head conflicts, retention/lease fencing and bounded mixed histories.
Eight seeded schedules and one serialized-writer schedule inspect raw authority
independently of returned receipts. Four broken backend modes violate named
oracles. The ineffective first deadlock mutation remains failed; the corrected
commit barrier produces a runtime timeout after a fresh build.

The file tutorial checks real process termination at six persistence boundaries,
reopen, original/current history, corruption, incomplete trailing records and
cross-process locking. Removing its OS lock is detected by the fresh runtime
control. These process-kill checks are not power-loss or arbitrary database
qualification.

Scenario A is covered by the native/domain and protected durable consumer tests:
one state/command definition, explicit registry/schema, pure preparation, atomic
effects, exact duplicate, conflict and independently expected current proof.
Scenario B uses original signed absent/present evidence and independent expected
keys, subjects, signers and historical pins; later canonical heads refuse stale
current claims. Scenario C tests both sides of commit/ACK loss, fresh handles,
exact original recovery, one head advance and one outbox effect.

Scenario D combines protected settlement tests with contention and whole-group
membership assertions. Unsupported multi-tenant topology refuses. Penelope is
an optional external orchestrator; this library adds no override of canonical
results. Scenario E consumes actual original four-commit configuration history,
verifies its old bytes/signatures/root and restores under a separate policy.
Migration to schema two requires a separately bound host grant, preserves the old
resource and rejects reinterpretation. All 44 before/after-checkpoint interruption
schedules resume to the signed expected roots. The migration grant deliberately
pins historical source; live-source freshness would be a further backend policy.

## F6 and maintenance gate 7.4

`maintenance-review.md`, exact source metrics and `source-growth-accounting.json`
report net growth, not a fictitious whole-project shrink. Original-path files
shrink by 3,671 syntax tokens while added files contain 17,007; total growth is
13,336 tokens. The added owners enforce protected preparation, authenticated
receipts, complete batches, independent claims and verified source/checkpoints.
Specific equivalent extractions retain deletion, parity and negative-control
evidence. Four remaining exact duplicate function groups have explicit retention
reasons. Macro syntax reduction does not imply fewer generated runtime branches.

`layer-review.json` confirms thirteen publishable crates, no custom feature
coupling and no new internal normal dependency graph edge. The production SHA-2
edge moved to dev scope where only an independent test oracle used it. Portable
runtime/storage service dependencies are not added. Build time/RSS and binary
size require the final repeated observations and explicit measured disposition.
Timing reviews must separate approved dependency changes from equivalent-code
refactoring and additional trust work; a universal speedup is not a goal claim.

## F8 and definition of done

`candidate-equivalence.json` binds the full unchanged library/test/example tree,
locks, fuzz, conformance and tutorials across the tested commits. Two checker-only
fixes have affected gates rerun: isolated package build directories and separation
of Cargo stderr from JSON stdout. Documentation and evidence changes do not
silently qualify changed runtime source.

The final audit inspects each terminal local release, package, compatibility,
API, semantic, coverage, property, fuzz, crash/conformance and performance record.
The actual observations, distributions, full allocation probes and three cold
builds per lane have been reviewed and accepted in `performance-review.md`.
The additional signed-history campaign and all seventeen final evidence checks
pass, including preserved timeout/interruption provenance. The all-path
extension records actual individual timings for every one of the 22 compatibility
workloads, supplementing the larger heavy-path sample sets and thirteen canonical
facade workloads including eight-thread exact retries.

F8-9 is a separate release action: the authorized goal is qualification, commit
and branch push. Updating versions, tagging and publishing require their own
instruction. A completed qualification audit does not imply physical backend
certification, all possible interleavings, universal downstream compatibility,
zero future advisories or a gap-free product.

## Supplemental design, metrics and risk review

The non-goals are checked against the compiler/package dependency graph and API
review: no new published crate, runtime database, serializer/runtime requirement
on portable profiles, generic transaction/proof super-trait, or mandatory external
coordinator was introduced. Cryptography and sparse-Merkle algorithms remain the
original protocol owners; approved dependency refresh is explicitly separated
from equivalent-code refactoring. Typed identities and explicit schema migration
remain intact. Signer policy, live head, expected claim, durable authority and
physical truth stay separate decisions.

Metrics now include exact pinned-parser syntax/tokens, approximate physical/body
lines and per-function complexity (`complexity-metrics.json`), semantic/exact
owner review, compiler API catalogs and concept/setup accounting (`api-review.md`),
fresh compiled dependency/build/RSS/binary costs and matched portable RISC-V core
archive sections (`portable-size-verification.json`). Instruction-level attribution
is not claimed: the retained instrumentation measures allocations and paired
latency, not isolated CPU instruction causes. Single-action nominal rate can be
derived as 1e9/latency_ns; concurrent service throughput is not inferred from that
reciprocal. Macro-expanded runtime complexity is explicitly outside source-token
metrics. Both whole-project growth and path-local reductions are reported.

`finding-disposition.md` accounts for the original observed trust hazards and
remaining scope. It distinguishes fixed canonical defects, intentionally retained
legacy semantics with a migration path, and consumer/backend qualification duties.
The roadmap's risk table is addressed by specific negative controls, strict
original/current semantics and independent trust/receipt checks, rather than by
an assertion that all theoretical risks disappeared. There is no known unresolved
P0/P1 canonical-path defect or unnamed P2 implementation backlog being waived.
This is a bounded implementation/consumer review, not a human security sign-off.

Maintainer evidence remains chronological: original before-fix probes, exact
revision/source/lock hashes, per-extraction deletions and parity controls,
compiled public consumers, measured deltas, and retained failed attempts are
linked by the inventories. Final qualification adds evidence and checker fixes;
it does not silently reclassify failed original policy or ineffective controls
as passing work, change protocol source during measurement, or authorize release
publication.


The additional H1–H8 work/acceptance clauses map to the same tested invariant
owners above and `finding-disposition.md`; their literal wording remains in the
third inventory. The file-by-file work map is an owner/task map, not a demand
to edit protected core source: unchanged portable, profile and accumulator
implementations remain verified through original vectors and separate builds.
Provisional API names, conceptual flow snippets and suggested PR slices do not
supersede the actual compiled equivalent API or authorize PR/merge/publication.
The roadmap's 5–10 minute setup is a usability target, not a human-timed claim.
Native authoring LOC/steps are measured separately in `consumer-setup-metrics.json`.
The conditional hotspot target is assessed using actual per-extraction deletions;
whole-project growth is published with exact ownership benefits, never hidden by
path moves. Measurement failures trigger investigation: the discovered mismatched
allocation setup control is explicitly unqualified and replaced with matched
zero/1000-action controls rather than accepting invalid subtraction.
