# Baseline trust-boundary findings

Status: reproduced on original production source. The trade tenant/cache fix
is implemented, and independent current/historical claims are implemented.
The authoritative durable-success route and remaining trade audit remain open.
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
