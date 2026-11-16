# Baseline trust-boundary findings

Status: reproduced on original production source. The trade tenant/cache fix
is implemented, and independent current/historical claims are implemented.
Protected single-intent durability and structural trade coverage are implemented.
Batch composition and independent expected trade/history claims remain open.
These are release-blocking inputs to F1. No deployed consumer or real database was tested.
The probes use deterministic local fixtures and consumer-owned in-memory ports.

## H1: absence verification confirms the supplied key, without current freshness

`ProofService::verify_non_membership` derives its state key from the supplied
bundle. The new
`legacy_absence_service_does_not_bind_independent_key_or_current_head` probe
accepts a genuine absence bundle even though the store exposes no canonical
head; the pure verifier rejects the same bundle when passed the independently
requested present key. This confirms the documented integration hazard, not a
cryptographic break. Add independent expected tenant/resource/key and explicit
current versus pinned-historical entry points; preserve old compatibility
semantics with clear documentation.

## H2: a planning marker can bypass the durable sink

`legacy_intent_marker_bypasses_durable_sink_without_commit_evidence` first calls
legacy `execute` with a signed intent. No ledger commit occurs. Calling
`execute_player_durable` with the same intent and a sink that always rejects
then returns `Ok([])`: the independent intent-store marker short-circuits the
sink. The canonical API must use authoritative ledger lookup and typed receipts,
never infer committed success from this empty planning result. The old route
needs explicit migration guidance; this probe freezes its existing semantics.

## H1 trade audit: tenant substitution and commit-cache reuse

`legacy_trade_verifier_accepts_state_proof_under_a_different_leg_tenant` uses a
cryptographically valid state proof in tenant alpha, labels its trade leg and
summary side as beta, and observes success from `verify_trade_proof`. The
verifier checks the state proof against the signed alpha commit but never
compares the proof tenant with the leg tenant.

Adding a genuine alpha leg first also makes `ProofService::verify_trade`
accept both legs. Its cache is keyed only by commit ID text: the beta leg reuses
the alpha entry, and the signer resolver is called only for alpha. Commit IDs
are not globally enforced: `CommitStore` lookups and the reference adapters
scope them by tenant. This is an observed cross-tenant proof acceptance problem,
not merely an identifier collision hypothesis.

F1 must bind proof tenant to leg tenant before lookup/verification and use
`(tenant, commit_id)` for authoritative commit/key context. Existing pure
commit-ID-map callers need source compatibility or an explicitly scoped new
entry point. Add correct same-ID/different-tenant positive coverage as well as
negative substitution tests; do not merely reject all identifier collisions.

The probe also confirms that `latest_event.operation` is editable proof
metadata: changing it from `asset.transfer` to `trade.settle` does not invalidate
the signed state/root. Consequently the old trade verifier cannot establish
that an actual settlement event occurred solely from this field. Audit the
remaining summary fields, side cardinality, distinct tenant/resource coverage,
per-asset commit references and event-history provenance before advertising a
stronger trade guarantee. A canonical state claim must not be presented as
proof of unauthenticated summary/history metadata.

## Reproduce

```sh
RUSTC_WRAPPER= CARGO_BUILD_JOBS=8 python3 scripts/refactor_baseline.py --audit
```

The runner overlays only the three probe test files onto a fresh original
release archive. Its manifest identifies their hashes separately from the
original archive; production source and the original lockfile remain unchanged.
The original-release probe run passed in
`evidence/baseline/20261116T021414Z-2990104/`. Passing here means the documented
old behavior was reproduced. It does not mean that behavior is safe. The root trade probe is now a rejection regression; original acceptance
characterization remains pinned to the pre-fix probe revision `bc76ce4`. The
runner reads that revision rather than current tests. The frozen probes were
rerun successfully in `evidence/baseline/20261116T024515Z-3522012/`.

## F1 trade tenant/cache fix

Both pure verification and the service now reject a state proof whose tenant
differs from its trade leg. The service loads and resolves commits/signers under
`(tenant_id, commit_id)` and delegates to `verify_trade_proof_scoped`. The old
`verify_trade_proof` signature remains available through the same verification
implementation, but its commit-ID-only input cannot express coincident IDs in
different tenants; multi-tenant callers should use the scoped entry point.
No wire schema, signed bytes, old error variants or valid historical semantics
changed. Previously accepted tenant substitution intentionally fails.

`trade_service_resolves_coincident_commit_ids_in_each_tenant` verifies genuine
signed state for two tenants sharing an ID with different roots and signing
keys. Both tenant policies are consulted. Swapped keys, swapped signed commits,
and missing scoped entries fail; the old global map fails closed when it cannot
represent both commits. The substitution probe rejects in pure and service
routes, including the formerly cache-priming alpha leg.

This closes the tenant/cache finding only. Unsigned operation/summary provenance,
distinct side/resource coverage and independent expected trade claims still
need their F1 audit. These APIs do not prove current canonicality, settlement
history authenticity, or durable cross-tenant atomicity from signed asset state.

Exact candidate `260bb1e` passed the five-check external hosted/portable lane in
`evidence/compat/20261116T024726Z-3615854/`, including original wire verification,
legacy exhaustive matches and bare-metal compilation. Parser-derived production
metrics in `evidence/f1/trade-scope-metrics.json` report +27 syntax-bearing lines,
+240 lexical tokens, +2 branch nodes and +1 public syntax item over the original
production source. The shared private verification implementation avoids copying
the legacy trade pipeline for the new scoped entry point. This is a measured
safety-related increase; later simplification targets remain unchanged.

The post-fix twenty-two-workload run passed all ten checks in
`evidence/compat/20261116T025005Z-3704572/`; portable allocation deltas remain
zero. See the performance baseline for its limits. F1 remains open for explicit
expected claims/current-vs-historical APIs, authoritative durability/recovery
receipts, and the remaining trade metadata/coverage audit.

## F1 independent expected claims and verification point

The new `proof::claim` API provides independent resource/holder/owner/absence
expectations and separate `ProofService::verify_current` / `verify_historical`
methods. `ExpectedClaim` derives keys from the independently supplied address;
`ClaimProof` distinguishes present and absent evidence. Historical calls require
a separately selected commit pin. Current calls perform one final canonical
head read after claim and cryptographic verification, and return a private-field
`VerifiedClaim` pinned to that exact observed head. The commit-store contract
now explicitly requires immutable lookups and linearizable, atomic head tuples.

Tests reject swapped absent keys, relabeled resources, wrong tenants, holders,
owners, signers, proof kinds, roots and historical pins. A genuine pre-creation
absence remains valid historically after a signed creating commit but fails as
current absence. A head advance during commit lookup is rejected; a successful
result retains its explicit commit pin if the head advances afterward. Missing
head support fails closed. A compile-fail doctest protects the result's private
constructor. Legacy APIs retain their original contracts and error order.

The audit also found that legacy pure verification authenticates the stored
commit signature, without comparing the redundant signature in the proof's
reference. The canonical route requires an exact reference signature match and
constructs its result reference from the verified stored commit. It also rejects
a malformed global-checkpoint scope carrying a tenant. No unverified reference
metadata is promoted into a `VerifiedClaim`; no old wire representation changes.

The [claim guide](proof-claims.md) documents exact trust and temporal semantics.
Candidate-only external tests use original-release signed proof bytes with
independently fixed expectations and the public umbrella API.

Rust 1.100 strict all-target Clippy additionally exposed redundant clones in the
trade index and custom-domain test. The production ownership-only move was
committed separately as `ae625a4`; index tests and strict package Clippy passed.
The root test now moves its last-use proof without weakening any assertion.
Original frozen consumer sources and all prior baseline evidence remain intact.

F1 is not complete: canonical authoritative durability, ambiguous-ACK lookup and
verified replay receipts, and trade summary/coverage provenance still require
implementation and qualification. F2–F8 remain pending.

The final worktree checks in `evidence/f1/expected-claims-20261116T031019Z/`
passed full all-target/all-feature workspace tests (excluding fuzz binaries),
strict workspace Clippy, strict rustdoc, formatting and the proof compile-fail
doctest. Exact source hashes identify the tested worktree. The earlier failed
strict-Clippy attempt remains in `expected-claims-20261116T030639Z/`. Rustc, Cargo
and benchmark manifests remain revision-specific: early finite baseline checks
used Rust 1.99, while the expanded workloads and this checkpoint use 1.100.

Exact candidate `6be864f` passed the ten-check original/candidate external
hosted/portable matrix in `evidence/compat/20261116T031203Z-184496/`. The log
confirms execution of the new public consumer against original-release proof
bytes, in addition to legacy imports, exhaustive matches, wire tests and both
bare-metal builds. `evidence/f1/expected-claims-metrics.json` records source
metrics: +215 syntax-bearing lines, +1,353 lexical tokens, +30 branch nodes and
+24 public syntax items since the trade-scope checkpoint (including the separate
clone repair). These additions enforce independent claims and unforgeable result
construction. Shared commit lookup and identity checks replace repeated logic;
no dependencies, published crates or old wire types were added.

## Hosted MSRV gate discovered during F1 qualification

`cargo +1.85.0 check -p statechronicle-proof --all-features --locked --offline`
fails before compiling the new claim module: the original
`statechronicle-core/src/rate_limit.rs` uses let-chain syntax unsupported on the
declared Rust 1.85 minimum. This file is byte-identical to the original release,
as recorded with the failed log in
`evidence/f1/expected-claims-20261116T031019Z/hosted-msrv.json`. The successful
portable MSRV baseline excludes this hosted code and does not establish hosted
MSRV compatibility. Preserve the declared minimum and fix the unsupported
hosted syntax before final qualification; do not raise MSRV or label this gate
as passing. The new hosted API's MSRV qualification is currently unproven.

A follow-up hardens the result's data boundary: `VerifiedClaim::state()` returns
an owned copy of the verified payload (or `None` for absence). Mutating the
original proof after verification cannot alter that payload. A regression test
changes the original owner and confirms the result remains unchanged; the
public consumer reads the result's verified state directly. This adds no wire
format or serialization constructor.

The owned-payload follow-up at exact commit `8f05cce` passed the candidate
external hosted/portable lane in `evidence/compat/20261116T031940Z-496125/`,
including the canonical consumer's `verified.state()` assertion and bare-metal
compilation. `evidence/f1/verified-state-payload/metrics.json` reports its exact
source footprint. Hosted Rust 1.85 remains explicitly unqualified as noted above.

## Authenticated preparation without replay markers

`Executor::prepare_player` now provides the uncommitted part of the canonical
flow. It reuses the existing execution core under a typed intent-handling mode
that bypasses both lookup and writes in the independent intent store. Legacy
lanes preserve their marker behavior and error order. `PreparedMutation` owns
its request, principal, body digest and complete event set behind private fields;
it cannot be publicly constructed or deserialized. Public `ValidatedIntent`
fields are structurally rechecked and its cached idempotency tuple reconstructed
by the existing checked constructor.

Six new integration tests cover legacy-marker isolation, signature revalidation,
owned input isolation, malformed public request fields, refused policy/key/scope,
and no intent-store calls, claims, projection writes or symbolic transaction.
The original false-durable-success characterization remains active for legacy
compatibility. The new canonical external consumer uses real signature checking
and the umbrella API; it does not change frozen original consumer sources.

Worktree evidence in `evidence/f1/mutation-preparation/` records 811 passing
workspace tests in 62 binaries, strict workspace Clippy, strict package rustdoc
and the private-constructor compile-fail test. Failed development builds are
retained and explicitly unqualified. See `mutation-preparation.md` for callback
trust, local id-counter effects, stale-state/policy caveats and retry identity.
The plan adds owned data and structural/digest checks; its cost is not established
by the legacy benchmark. No protocol bytes or dependency versions change.
This prepares the durability work but does not yet implement authoritative
receipt lookup, structured unknown acknowledgements or atomic durable replay.
F1 remains incomplete; the remaining F2–F8 gates are also open.

Exact production commit `0fb6179` passed all five candidate external hosted,
portable and bare-metal checks in
`evidence/compat/20261116T033222Z-889194/`. The retained log confirms execution of
`public_preparation_never_treats_a_planning_marker_as_a_receipt`; the runner also
verifies original frozen fixture provenance and unchanged wire bytes. No new
third-party lock entries were admitted. The source metrics at that exact commit
add 106 production syntax lines, 502 lexical tokens, eight branch nodes and nine
public syntax items relative to the owned-proof-payload checkpoint. The increase
owns preparation authority and input isolation; it is not a claim of F6 net
simplification, F8 performance qualification or completed durable recovery.

## Authoritative durable outcomes and recovery

The advanced `commit::durable` path now distinguishes confirmed committed/replay
receipts, payload conflict, active reservation, typed authorization causes,
pre-commit failures, corrupt evidence and unknown completion. Every final commit
error is unknown; failed rollback or missing/inconsistent/unavailable readback
cannot become safely failed or committed success. Unknown errors retain original
identity and adapter/rollback causes. `recover` uses the original intent/digest
and independently authenticated principal, checks access before reading, and
loads immutable original effects through the additive `LedgerRecovery` port.
It constructs no events or new identities and performs no claim or dispatch.
A bare finalized commit-id claim is insufficient for a receipt.

`CommittedMutation` privately owns the original verified record. Historical
receipts remain valid after later canonical commits; current-state freshness is
not implied. Original event root, event bindings, projections and outbox row
integrity are checked. Newly acknowledged writes additionally compare exact
signed commit, events and outbox readback. Canonical history membership, original
outbox completeness and physical persistence remain adapter guarantees; no
signature is presented as independent proof of database atomicity or physical
truth. The port requires retention across projection advancement/outbox delivery.
See `durable-outcomes.md` for the exact contract and recovery decisions.

Legacy persistence now shares one preflight/effect-validation and transaction
owner with the new path. Its public signatures, enum variants, gate/error order
and string mapping remain unchanged; the new surface retains typed authorization
and transaction-stage/rollback causes instead. No dependencies or protocol wire
types change. The root in-memory adapter retains the original mutation atomically
with its existing effects; frozen original consumer sources remain untouched.

Worktree qualification in `evidence/f1/durable-recovery/` records 822 passing
workspace tests in 63 binaries, strict workspace Clippy/rustdoc, formatting and
a receipt-constructor compile-fail test. Eleven new integration tests cover ACK
loss and reopened lookup, exact retry, conflicting payload, pre-commit fault and
rollback matrix, failed readback, reservation/absence semantics, partial/corrupt
records, bare replay claims, mismatched claimed commit, omitted new-write outbox,
historical replay after a genuine later commit, signer/principal/policy refusal
and typed policy unavailability. Development failures remain retained and marked
unqualified. The external canonical overlay reuses exact archived public-API
scenarios rather than duplicating the new test/adapter implementation.

This completes a single-intent advanced durability slice, not F1 or the release.
The canonical facade must still bind protected prepared events to signing and
storage; canonical batches, independent adapter conformance/crash/restart gates,
trade coverage/provenance and F2–F8 remain open. The new readback/verification cost
requires its own matched measurements; legacy timings are not equivalent work.

Exact durable implementation `ea988b6` passed all ten original/candidate external
hosted, portable and bare-metal checks in
`evidence/compat/20261116T035133Z-1254026/`. The candidate log confirms all eleven
new recovery scenarios run through only the public umbrella dependency, using
exact archived candidate sources. Original fixtures and wire provenance remain
unchanged. `evidence/f1/durable-recovery/metrics.json` records +454 production
syntax lines, +2,537 lexical tokens, +50 branch nodes and +38 public syntax items
relative to preparation checkpoint `0fb6179`. The delta owns typed uncertainty,
authoritative retained-record validation and private receipt construction; it
is a justified safety addition, not a net source-reduction or performance claim.

## Hosted declared-MSRV repair

The original hosted let-chain syntax was not confined to rate limiting: removing
that first compiler failure exposed the same unsupported syntax in executor
batch/settlement handling and rebuild continuity checks. Four files now use
Rust 1.85-compatible conditions with the same left-to-right checks, sink and
rollback calls, early returns and refusal behavior. No MSRV, lockfile or frozen
reference change is needed. Scoped Clippy allowances explain why the nested
conditions must not be collapsed back into unsupported let chains.

`evidence/msrv/hosted-let-chains/` retains the original failure, intermediate
partial repairs and final source hashes. Rust 1.85 now checks all workspace
targets/features, passes all 822 tests in 63 binaries and all four doctests across
13 crates, including the new protected plan/receipt/claim constructor checks.
Strict Rust 1.100 workspace Clippy and formatting also pass. This resolves the
previously recorded hosted syntax incompatibility at this checkpoint; final
release qualification must still run on the exact final candidate. External
original/candidate durability evidence above remains pinned to `ea988b6`, before
this separate compiler-compatibility repair.

Exact MSRV-repaired candidate `59ca749` also passes all five external hosted,
portable and bare-metal checks using `RUSTUP_TOOLCHAIN=1.85.0` in
`evidence/compat/20261116T035612Z-1366672/`. The retained candidate log executes
all eleven new recovery scenarios through the public umbrella dependency in
addition to frozen legacy and proof/preparation fixtures. The minimum-toolchain
bare-metal target is `riscv32imafc-unknown-none-elf`. Source metrics record one
additional production syntax line, 29 lexical tokens, 12 AST branch nodes and
no public syntax changes relative to `ea988b6`. The branch count increase
reflects explicit nested `if` syntax replacing short-circuited let chains, not
additional sink calls, retry decisions or domain checks. This compiler repair
makes no speedup or final-candidate qualification claim.


## Protected preparation to signed mutation

The F1 single-intent composition now owns `PreparedMutation` through
`statechronicle::mutation::SignedMutation`. It rejects substituted events,
wrong tenant, unsupported schema/scope and event count/root mismatch before
calling the injected signer. Derived projections and protected principal/digest
cannot be independently replaced at commit. Event producer and commit signer
remain distinct authorized roles. Persistence and recovery reuse the existing
advanced verified ledger path rather than introducing another transaction owner.

Five added public-API scenarios cover commit/replay/recovery, binding rejection,
separate producer/signer roles, retained exact identity after lost ACK, and
policy/signer/outbox rejection before ledger operations. The API takes a borrowed
commit body and stores its signed owned copy. Private construction has a new
compile-fail doctest. See [signed mutation](signed-mutation.md).

This closes an arbitrary-event substitution gap between protected preparation
and the advanced caller-assembled persistence request. It does not complete
protected batch composition, the shared F2 pipeline, the F4 facade or F5 adapter
conformance; host context and backend atomicity remain explicit responsibilities.
No source-shrink or equivalent-workload speedup is claimed for this safety addition.

Rust 1.85 passes 827 workspace tests in 63 binaries and five doctests across
13 crates for this checkpoint. Rust 1.100 strict workspace Clippy and rustdoc,
formatting and whitespace checks pass. Development failures and source hashes
are retained in `evidence/f1/signed-mutation/`; final documentation links were
qualified separately after test execution. Exact committed external and source
metric evidence follows separately.

Exact production checkpoint `fc641fe` measures 1,400,945 tracked source bytes,
15,043 production syntax lines, 110,912 lexical tokens, 1,786 branch nodes and
1,302 public syntax items using the pinned F0 parser methodology. Relative to
`59ca749`, this adds 6,834 bytes, 90 syntax lines, 561 tokens, six branch nodes
and eight public syntax items. The delta binds the protected preparation token
to signing/storage and adds one shared structural-validation seam; it is a
justified authority boundary, not a measured simplification or speedup.

Exact `fc641fe` external qualification is retained in
`evidence/compat/20261116T041110Z-1773334/`. All five candidate hosted/portable/
bare-metal checks pass on Rust 1.85, including all 16 public-API recovery and
signed-plan scenarios. That manifest retains a preexisting generic limitation
saying bare-metal was unqualified despite its explicit passing compilation
record. The runner disclaimer was corrected to describe the recorded checks
and flag-controlled scope; the subsequent qualification manifest records the
corrected runner hash without changing frozen fixtures or production source.

The corrected-runner repetition
`evidence/compat/20261116T041204Z-1800235/` also passes all five checks against
the same exact production candidate. Bare-metal evidence is compilation only,
not target execution or power-loss qualification.


## Trade proof distinct coverage and unsigned metadata audit

Genuine signed trade fixtures reproduced four structural failures before the
fix: repeating an alpha leg could omit beta while preserving leg count;
duplicate tenant sides passed; duplicate declared assets passed; and repeating
one asset proof twice could omit the other asset while preserving proof count.
`evidence/f1/trade-coverage/trade-coverage-before.log.gz` retains both failing
end-to-end cases against unchanged production source at `208abd8`.

A shared private uniqueness check now rejects duplicate summary tenants,
summary assets, proof-leg tenants and proof resources. Existing orphan/count
checks retain precedence; uniqueness runs before cryptographic lookup. With
existing per-leg membership checks, count equality now proves structural
coverage against the supplied summary. Both compatibility and scoped verifiers
share this implementation, preserve valid distinct per-asset commits and accept
proof-order permutations. No new error variant, schema or signed-byte change:
malformed duplicate coverage intentionally changes from acceptance to the
existing `InvalidState` failure. Borrowed identifier sets add validation work;
this is not a speedup or source-reduction claim.

The same genuine signed fixtures explicitly retain the separate limitation:
changing both trade IDs, previous owner, representative commit references,
clearing per-asset summary references/event IDs and dropping value declarations
still passes signed-state verification. These fields are not authenticated
settlement witnesses. Earlier probes also prove operation metadata is unsigned.
The actual state proof's commit reference is still independently resolved and
verified; the summary's representative and per-asset references are not an
additional authority. Neither structural coverage nor trusted state signatures
prove a caller's expected trade, settlement history, payment or cross-tenant
atomicity. Public verifier docs now state these boundaries. Full expected
trade/history verification remains an explicit open contract, not a wire-format
change smuggled into a cleanup.

`ci/canonical-0.3/trade_coverage.rs` executes the archived candidate's public
umbrella trade integration tests externally without copying fixtures or changing
the frozen original consumers. These are real executor/commit/signature/Merkle
fixtures with local in-memory adapters; they do not qualify production storage.

Rust 1.85 passes all 827 workspace tests in 63 binaries; the final five-test
trade integration separately passes after expanded metadata characterizations.
Rust 1.100 strict workspace Clippy/rustdoc, formatting and whitespace checks
pass. Manifests retain failures, exact source hashes and execution limitations.
Exact committed external qualification and source metrics follow separately.

Pinned-parser source metrics at `45b61aa` record 1,403,060 tracked source
bytes, 15,079 production syntax lines, 111,154 lexical tokens, 1,795 branch
nodes and 1,302 public syntax items. Relative to `fc641fe`, this is +2,115
bytes, +36 lines, +242 tokens, +9 branch nodes and no public syntax growth.
This is a measured safety-check addition. Borrowed sets avoid identifier clones
but still allocate nodes; matched runtime/allocation qualification remains due
before final release and no performance improvement is claimed here.

Exact candidate `45b61aa` and frozen original release pass all ten hosted,
portable and bare-metal checks on Rust 1.100 in
`evidence/compat/20261116T041727Z-1926425/`. The new external trade overlay
executes all five public-API integration scenarios, including the before-fix
coverage reproducers and unsigned-metadata characterizations. Frozen producer
history, legacy exhaustive imports and dependency provenance remain unchanged.
Bare-metal checks qualify compilation, not execution on target hardware.


## F2 shared event preparation and final legacy claim

Primary and companion events now share private owned `PreparedEventStates` and
one emitter, preserving checked version increment, unborn-state behavior,
existing before hashes, canonical after hashes and callback order. Legacy claim
writes share one final branch after complete event construction. Three exact
duplicate sites in `pipeline.rs` each fall from two implementations to one:
`Event::new`, checked version increment and final `put_intent`. The public API,
wire format, ingress gates and legacy marker semantics are unchanged.

Before the extraction, the unmodified original release generated canonical
digests for creation and debit/credit event vectors. The same final test file
passes against original production and candidate, including primary/companion
overflow refusal, no poisoned intent marker and subsequent event-ID consumption.
The original pipeline, helper and lockfile hashes are independently checked.
`evidence/f2/event-preparation/` retains producer/frozen logs, development
failures and source hashes. See [ordered gates](preparation-pipeline.md).

Rust 1.85 passes all 829 workspace tests in 64 binaries. Rust 1.100 strict
workspace Clippy/rustdoc, formatting and whitespace checks pass. No new public
API or dependency was introduced. This is an initial shared typed stage with
actual duplicate implementations removed, not completion of F2 or a speedup
claim. Exact committed external qualification and parser metrics follow.

Exact candidate `e30fdda` passes all five hosted/portable/bare-metal external
checks on Rust 1.85 in `evidence/compat/20261116T042504Z-2228907/`, with frozen
legacy consumers/wire data and new canonical overlays. Pinned-parser metrics
report 1,402,524 tracked source bytes, 15,077 production syntax lines, 111,062
tokens, 1,791 branch nodes and 1,302 public syntax items: -536 bytes, -2 lines,
-92 tokens, -4 branches and no public syntax change relative to `45b61aa`.
The reduction is small; it substantiates three duplicate sites removed, not
the roadmap's final hotspot-reduction or whole-workspace footprint target.

Cumulative original/candidate measurement checks also pass in
`evidence/compat/20261116T042553Z-2255276/`: all 22 legacy workloads and portable
zero-added-allocation probes. Seven follow-up alternating pairs retain all
samples rather than suppressing the initial review triggers. They confirm an
open stale-head refusal timing finding (+33 ns / +13.78% median paired change;
no final performance acceptance). See [measurement disposition](performance-baseline.md).
This is not attributed to the event-stage extraction without a controlled
same-dependency experiment, and final performance qualification remains open.


## Proof lookup future and shared ingress ordering

Controlled same-dependency experiments support removing the extra async lookup
state machine rather than adding an ineffective inline hint. Production returns
the adapter future directly and retains one synchronous owner for store errors
and absent rows. All six proof consumers preserve their trust/crypto/head checks.
See [lookup investigation](proof-lookup-performance.md); exact final performance
qualification is recorded separately from prototype evidence.

Three new multi-fault ingress fixtures execute against unmodified original
production and candidate. They preserve the deliberate difference between
single registry lanes (key policy before binding/authorization) and batches
(binding/authorization before key policy), with signature presence first in all
player lanes. Protected preparation retains reconstruction of public structural
fields and skips legacy markers. Six signature-refusal implementations become
one, two binding/policy implementations become one, and four registry calls
become one private implementation. Each caller still chooses the original order.
This removes duplicate decisions rather than imposing one policy on every lane.

Rust 1.85 passes 832 workspace tests in 65 binaries and five doctests; the
final renamed fixture separately passes all three original-release tests.
Strict Rust 1.100 workspace Clippy/rustdoc, formatting and whitespace pass.
No public trait, default feature, dependency or wire schema changed. The earlier
emitter extraction's misplaced internal documentation/lint scope was corrected
without a runtime change. Full F2/F3–F8 qualification remains incomplete.

Exact combined candidate `abd7245` also passes all five external Rust 1.85
hosted/portable/bare-metal checks and all ten Rust 1.100 measurement checks.
Evidence is in `evidence/compat/20261116T044250Z-2789154/` and
`evidence/compat/20261116T044250Z-2789155/`. Production metrics are 1,401,366
tracked source bytes, 15,043 syntax lines, 110,940 tokens, 1,792 branches and
1,303 public syntax items: versus `5632d00`, -1,158 bytes, -34 lines, -122
tokens, +1 branch and +1 crate-restricted helper (no exported consumer API).
This is a checkpoint reduction, not the final hotspot target. The repeated
stale-head observation improves to +12 ns versus original, with substantial
noise and final performance qualification still open.

## Hosted typed transition evaluation

Original-release instrumentation confirms that adjacent `TypedProfile::check`
and `apply` decode both native inputs twice and validate the command twice.
The executor now uses a compatible `ProfileRules::evaluate` default method;
existing implementations still run their original check then apply. The typed
override owns decoded values locally and shares them through validation and
reduction. Current and resulting state invariants, signed operation binding,
canonical roundtrips and bounds remain checked. Check and reduction failures
retain their distinct executor mappings through a typed stage error.

Five profile fixtures cover counts, original behavior, 224 differential cases
and default-method refusal order; one executor fixture covers structured check
errors and transition-error rendering. The Rust 1.85 workspace run passes 837
tests in 66 binaries; a subsequent targeted run passes all six profile/executor
fixtures, including the newly added executor test. Five doctests, strict stable
Clippy and rustdoc pass. The initial strict lint failure was test indexing and
was repaired using checked access without changing expectations.

Pinned-parser metrics at `d1550a9` report 1,403,910 tracked source bytes, 15,083
syntax lines, 111,210 tokens, 1,796 branches and 1,305 public syntax items:
+2,544 bytes, +40 lines, +270 tokens, +4 branches and +2 public syntax items
versus `abd7245`. This adds a compatible default trait method and stage error
to remove actual runtime repetition; it is not a source-shrink claim. Native
`StateProfile`, dependencies, portable execution and wire schemas are unchanged.
Timing/allocation qualification and the rest of F2/F3–F8 remain open. See
[typed evaluation contract](typed-evaluation.md) and its F3 evidence directory.

Exact candidate `d1550a9` passes all five external hosted/portable/bare-metal
checks on Rust 1.100 and independently on Rust 1.85, including frozen legacy
consumers and canonical overlays. Manifests: `evidence/compat/20261116T045523Z-3083571/`
and `evidence/compat/20261116T045532Z-3089968/`.
