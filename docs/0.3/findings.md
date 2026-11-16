# Baseline trust-boundary findings

Status: reproduced on original production source. The trade tenant/cache fix
is implemented; the independent-claim and durable-success routes remain open.
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
