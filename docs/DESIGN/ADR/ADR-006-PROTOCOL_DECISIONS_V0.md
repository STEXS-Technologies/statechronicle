**Document Version:** 1.0
**Last Updated:** 2026-08-05
**Status:** Accepted
**Related Documents:** [ADR README](README.md), [Architecture Summary](../../ARCHITECTURE.md),
[ADR-003](ADR-003-TRUSTGRANT_PORTS_ONLY.md), [ADR-004](ADR-004-CANONICALIZATION_HASHING_SIGNATURE.md),
[ADR-005](ADR-005-STATE_ACCUMULATOR.md)

# ADR-006: Protocol Decisions for v0 (§36 Resolution)

**Date:** 2026-08-05

---

## Context

Protocol §36 lists 13 open questions that the v0 implementation answered implicitly
during construction (484+ tests green across the pure-logic crates: core, domain,
intent, executor, commit, accumulator, proof, profiles, ports). Before v0 conformance
vectors and public documentation are frozen, each question must be formally decided so
the protocol text, the ADR set, and the code agree. This ADR records those decisions,
cross-references the ADRs they extend, and classifies each as protocol-binding, policy
(deployment-configurable), or deferred.

## Decision Summary

| Q# | Decision (abbreviated) | Status vs code | Binding class |
| --- | --- | --- | --- |
| Q1 | Fixed 256-bit SMT per tenant (ADR-005) | matches | protocol |
| Q2 | Commit authority bound to a trust anchor; TrustGrant standard, configured roots permitted | matches | protocol |
| Q3 | Freshness window policy-owned; check mandatory at acceptance; default 24h | matches | policy + protocol (check) |
| Q4 | Operations locally namespace-scoped per profile | matches | protocol |
| Q5 | Per-profile aggregation policy (require-all default, any-of where declared); evaluate every member; single aggregate digest over sorted sub-evaluations (Phase 2) | matches | protocol |
| Q6 | Proof bundles embed digests + resolvable references only (ADR-003) | matches | protocol |
| Q7 | Snapshots optional; authenticity mandatory if published | matches | policy + protocol (authenticity) |
| Q8 | Fork detection/evidence core; resolution policy deployment-defined | matches | protocol |
| Q9 | Timestamps advisory; canonical ordering ignores them | matches | protocol |
| Q10 | Six mandatory paid-unique exception states | matches | protocol (conformance floor) |
| Q11 | Hard deletion forbidden; removal is tombstone/terminal state | matches | protocol |
| Q12 | Per-commit tenant roots mandatory for all deployments | matches | protocol |
| Q13 | Canonical decimal-integer strings; u64 fixed precision baseline | matches | protocol |

## Decisions

### Q1. Baseline accumulator: SMT or ordered Merkle map?

**Decision:** Fixed 256-bit-depth sparse Merkle tree (SMT), one per tenant, SHA-256
keys and node hashing, as implemented in `statechronicle-accumulator` (confirming
ADR-005).

**Rationale:** The SMT root is a pure function of the (key→leaf) set, giving
insertion-order determinism by construction — exactly what
`previous_state_root + ordered events = next_state_root` requires. Non-membership is
free (empty-slot inclusion proof), serving the fail-closed "resource does not exist at
commit X" case. Ordered Merkle maps buy sorted/range proofs no v0 consumer requires and
pay ~2.5× larger proofs.

### Q2. Commit authority always through TrustGrant?

**Decision:** Commit authority MUST be bound to a verifiable trust anchor in every
conforming deployment. TrustGrant is the standard binding mechanism; the core protocol
represents the authority as a `SubjectId` + signing key and permits deployment-defined
trust roots ("TrustGrant or configured trust roots", §29).

**Rationale:** Forcing a hard "always TrustGrant" rule contradicts the ports-only,
infra-agnostic ADR-003. Making commit authority optional would break verification — an
unauthorized signer could fabricate the canonical chain. The executor/commit crates
treat the key as deployment-injected and the verifier takes the trust root as input,
which is the correct boundary.

### Q3. Minimum revocation freshness window?

**Decision:** The freshness window is policy-owned and deployment-configurable, enforced
at acceptance through `TrustGrantEvaluator::check_revocation_freshness` (Stale → fail
closed). The protocol mandates the check's existence and recommends a baseline default
of 24 hours, with a stricter ≤1-hour recommendation for paid-unique/ownership-critical
profiles.

**Rationale:** A numeric window is a risk-vs-availability tradeoff tied to each
deployment's revocation latency and caching, so it must not be a core constant. The
executor fails closed on `TrustGrantError::Stale` at §18.1 steps 3/8; the window lives
in the adapter/policy layer. `AuthorityProof.evaluated_at` (added 2026-08-05) lets an
offline verifier check age without resolving the evaluation digest.

### Q4. Profiles: global registries or local namespaces?

**Decision:** Operations are locally namespace-scoped per profile. Each profile owns its
`allowed_operations` list; the dotted prefix (`asset.*`, `stack.*`) is a naming
convention, not a registry requirement.

**Rationale:** `Operation` is a registry-open newtype, each profile declares its own op
list, and `ProfileRegistry` resolves rule sets by state type. A global registry would
force cross-profile collisions and a schema bump for every new operation, contradicting
§20's profile-specialization goal.

### Q5. Multi-authority resource scopes?

**Decision (Phase 2, 2026-08-05):** Each deployment configures an authority **set** (one or
more trust anchors). A transition is gated by every member's TrustGrant evaluation, aggregated
under the active profile's authority policy: **require-all** (default) requires every member to
`Allow`; **any-of** (declared by a profile) passes when at least one allows. Any deny, stale,
unavailable, or missing binding fails closed; an authority-required operation MUST carry a
binding (see deferral item 4). The bound `AuthorityProof.evaluation_digest` is a single
aggregate digest over the sorted, deduplicated sub-evaluation digests (BCS envelope tagged
`statechronicle.authority.aggregate.v0`); a single-member set preserves the sub-evaluation
digest itself (the v0 identity rule), keeping v0 single-evaluator bytes byte-identical.
`evaluated_at` on the aggregate proof is the oldest (stalest) member's, so freshness spans every
sub-evaluation.

**Rationale:** v0 shipped single-evaluation semantics for one evaluator port. Phase 2 generalizes
to an ordered authority set without changing the wire shape of `AuthorityProof`
(`Option<AuthorityProof> { kind, evaluation_digest, result, evaluated_at }`) or
`TRUSTGRANT_EVALUATION_KIND`. The aggregate digest is deterministic and order-independent
(sorting over raw bytes), so execution and verification agree regardless of evaluation order;
the identity rule keeps v0 single-evaluator bytes unchanged. Any ambiguity or conflict fails
closed to avoid confused-deputy ambiguity (§30).

### Q6. Proof bundles: full TrustGrant chains or digests?

**Decision:** Proof bundles embed only digests plus resolvable references
(`AuthorityProof` = `kind` + `evaluation_digest` + `result` + `evaluated_at`).
Verifiers resolve and re-verify through their own TrustGrant integration; full grant
chains are never embedded.

**Rationale:** ADR-003 decided this. Full chains bloat every portable bundle, embed
revocation-sensitive evidence, and force StateChronicle to parse TrustGrant internals —
violating the opaque, content-addressed authority boundary (§16.3).

### Q7. Mandatory snapshots after a fixed interval?

**Decision:** Snapshots are optional and operational in v0 — no fixed-interval mandate.
Any published snapshot MUST be authentic: payload digest bound to the enclosing
commit's state root via `SnapshotProof`. Snapshots never weaken tenant-level
verification.

**Rationale:** Correctness never depends on snapshots (replay from genesis or any
authenticated snapshot is always possible). Cadence is a storage/performance/recovery
decision that varies with throughput (§2.4) and belongs to deployment policy. The code
ships the `SnapshotStore` port, snapshot proofs, and the proof-service path with no
interval.

### Q8. Fork evidence: core protocol or federation profile?

**Decision:** Fork detection and append-only fork evidence are CORE protocol (all
deployments); fork-resolution policy (which head wins) is deployment-defined.

**Rationale:** A fork is a security-relevant violation of the canonical-chain invariant
that any verifier must detect and any operator must record (§30, §31). The code ships
`detect_fork`, `check_chain_continuity`, `validate_no_event_rewrite`, and `ForkEvidence`
in `statechronicle-commit`. Resolution policy depends on each deployment's quorum/
witness structure — a federation concern.

### Q9. Event timestamps: trusted, sequencer-derived, or advisory?

**Decision:** Timestamps are executor/sequencer-stamped advisory metadata. Canonical
ordering MUST ignore them (canonical key = `(resource_id, after.version, event_id)`).
Client-supplied `created_at` on intents is advisory; only `expires_at` is enforced
against the executor clock.

**Rationale:** The code orders purely by `(resource_id, version, event_id)` and stamps
event/commit `created_at` from the injected wall clock. Clients can lie and machines
skew, so timestamps cannot be ordering authority. Keeping them advisory preserves audit
and UX value without making determinism clock-dependent.

### Q10. Mandatory paid-unique exception states?

**Decision:** The baseline paid unique asset profile MUST support all six exception
states — `restricted`, `quarantined`, `unsupported`, `legal_hold`, `fraud_lock`,
`policy_restricted` — as explicit, append-only, owner-preserving transitions. Profiles
may add more.

**Rationale:** §20.3 lists five exception states and the base profile contributes
`restricted`; the code implements all six consistently in `paid_unique_asset.rs`
(`EXCEPTIONAL_STATUSES`) and `conflict.rs`, with `asset.restore` recovering from all six.
A conformance floor guarantees every paid-asset deployment can represent loss of utility
without erasing ownership.

### Q11. Deletion: tombstone or hard delete?

**Decision:** Yes — hard deletion is forbidden for any committed state in any conforming
profile. Removal is always an append-only tombstone or terminal state; even the
owner-consent "hard_delete" path is a tombstone transition, never an erasure.

**Rationale:** The code enforces this structurally: `unique_asset` omits `hard_delete`
entirely; `paid_unique_asset` gates `asset.hard_delete` behind `authorized_by_owner:
true`; `transition.rs` maps the operation to `TOMBSTONED`; and the SMT retains the leaf
forever. This pins the §2.13/§20.3 no-erasure guarantee and keeps replay, audit, and
proof generation total over all history.

### Q12. Tenant roots mandatory for single-tenant deployments?

**Decision:** Yes — per-commit tenant state roots are mandatory for ALL deployments
(they are structural fields of every tenant-scoped `Commit`). Global checkpoint roots
(cross-tenant composition) remain optional.

**Rationale:** Every `Commit` already carries `previous_state_root` + `next_state_root`;
there is no valid commit without them. §13.4's global checkpoint is an optional overlay
that "must not weaken tenant-level verification." Mandating the per-commit tenant root
keeps every deployment independently replayable and verifiable at zero cost.

### Q13. Balances: decimal strings, arbitrary integers, or fixed precision?

**Decision:** v0 baseline = canonical non-negative decimal integer strings on the wire,
with the internal representation an exact fixed-point `Amount` (u128 mantissa × 10^-scale,
scale ≤ 18, checked add/sub, fail closed on overflow, underflow, and floats). The string
form is the canonical wire representation, unchanged: amounts are never serialized in BCS
as anything but their canonical integer string.

**Rationale:** The executor stores amounts as canonical decimal strings and computes with
the exact fixed-point `Amount` (`statechronicle_core::amount`, used by `transition.rs` and
`atomicity.rs`), satisfying ADR-004's no-float-by-construction. A `u128` mantissa with up
to 18 fractional digits is exact and covers game economies well past the former `u64`
ceiling; the canonical string wire form keeps BCS serialization stable and remains the
single canonical representation of a balance.

## Alternatives Considered

| Option | Pros | Cons |
| --- | --- | --- |
| Ordered Merkle map as accumulator (Q1) | sorted/range proofs, enumeration | ~2.5× larger proofs, distribution-sensitive paths, no v0 consumer |
| Mandatory TrustGrant for all commit authority (Q2) | uniform authority model | contradicts ADR-003 ports-only, infra-agnostic goal |
| Fixed mandatory snapshot interval (Q7) | predictable recovery cost | adds protocol complexity no verifier needs; cadence is deployment policy |
| Mandatory multi-authority in v0 (Q5) | general conflict model | no consumer, wire-format churn, confused-deputy risk |
| Full TrustGrant chains in proof bundles (Q6) | self-contained bundles | bloats proofs, embeds revocation-sensitive evidence, breaks opaque boundary |
| Arbitrary-precision arithmetic in v0 (Q13) | no precision ceiling | changes executor arithmetic and conformance vectors; no consumer needs > 2⁶⁴−1 units |

## Consequences

**Positive:**
- All 13 protocol questions now have formal, code-verified decisions.
- v0 conformance requirements are unambiguous (SMT accumulator, tombstone-only
  deletion, mandatory tenant roots, advisory timestamps, six paid-exception states,
  trust-anchored commit authority, freshness check at acceptance, single-authority
  scopes, digest-only authority references).
- No migration needed — every decision matches the shipped implementation.
- `AuthorityProof.evaluated_at` closes the offline freshness-verification gap.

**Negative:**
- Protocol §36, §19, §15, §12/§13, §20.1/§20.3, §10.3, §8.1 need text updates to state
  the binding rules (scheduled with this ADR).
- ADR-003 and ADR-004 wording needs reconciliation (Q2 phrasing; §6 "typed u128
  newtypes" vs. string+u64 implementation).
- Deferred items (below) remain open for v0.1.

## Review & Maintenance

- **Last Reviewed:** 2026-08-05
- **Next Review:** When §36-deferred items are taken up for v0.1
- **Change Log:**
  - v0.1 (2026-08-05): Initial decision record; all 13 §36 questions resolved.
  - v0.1 Phase 2 (2026-08-05): Q5 amended to record the multi-authority per-profile aggregation
    policy; deferral items 1 and 4 resolved (authority-set evaluation + event-level authority
    mandatory-ness).
  - v0.1 Phase 3 (2026-08-05): deferral item 3 resolved — cross-tenant atomicity
    (`execute_cross_tenant`, `begin_multi`, `validate_cross_tenant_consistency`).
  - v0.1 Phase 4 (2026-08-05): deferral item 5 resolved — non-membership proof-bundle
    wiring (`statechronicle.proof.non_membership.v0` bundle, §16.2).
  - v0.1 Phase 5 (2026-08-05): deferral item 6 resolved — snapshot cadence heuristics
    (protocol §15 Cadence guidance); all six v0.1 deferrals are now resolved.

## Deferred to v0.1 (explicit)

> **Amendment (Phase 2, 2026-08-05):** Deferral items **1** (multi-authority semantics) and
> **4** (event-level authority mandatory-ness) are **resolved** by Phase 2 and removed below.
> Q5 now records the per-profile aggregation policy decision, and §11.2/§12.2 of the protocol
> text state the mandatory-binding rule. Remaining items below stay deferred.

> **Amendment (Phase 3, 2026-08-05):** Deferral item **3** (cross-tenant atomicity) is
> **resolved** by Phase 3: `execute_cross_tenant`, `begin_multi`, and
> `validate_cross_tenant_consistency` define v0.1 cross-tenant behavior (§8.2).

> **Amendment (Phase 4, 2026-08-05):** Deferral item **5** (non-membership proof-bundle
> wiring) is **resolved** by Phase 4: the §16.2 `statechronicle.proof.non_membership.v0`
> bundle variant authenticates absence fail-closed (inclusion proof of the empty-leaf
> constant, with verifiers MUST asserting the empty leaf).

> **Amendment (Phase 5, 2026-08-05):** Deferral item **6** (snapshot cadence heuristics) is
> **resolved** by Phase 5: protocol §15 adds Cadence guidance (replay-cost vs snapshot-cost,
> operational factors, and a recovery-SLA baseline heuristic). With this, **all six** v0.1
> deferrals are now resolved: Q5 multi-authority (Phase 2), Q13 fixed-point Amount (Phase 1),
> §8.2 cross-tenant atomicity (Phase 3), event-level authority mandatory-ness (Phase 2),
> non-membership proofs (Phase 4), and snapshot cadence (Phase 5).

1. **Multi-authority semantics (Q5)** — **RESOLVED in Phase 2**: per-profile aggregation
   policy (require-all default, any-of where declared); evaluate every member of the
   deployment's authority set; single bound digest over the sorted sub-evaluation digests
   (identity for a single-member set). Quorum-based aggregation remains a possible future
   policy extension.
2. **Arbitrary-precision arithmetic (Q13)** — **RESOLVED in Phase 1**: the fixed-point
   `Amount` (u128 mantissa × 10^-scale, scale ≤ 18) baseline is implemented; any further
   widening (e.g. bigint or decimal via a profile precision declaration) stays additive via
   the string wire form.
3. **Cross-tenant atomicity (§8.2)** — **RESOLVED in Phase 3**:
   `execute_cross_tenant` partitions intents by tenant, begins a multi-tenant
   transaction via `begin_multi`, runs each tenant's leg through the single-tenant
   pipeline, validates cross-tenant consistency (shared `intent_id` linkage +
   per-tenant batch consistency), and commits atomically or rolls back — one
   tenant-scoped commit per affected tenant.
4. **Event-level authority mandatory-ness** — **RESOLVED in Phase 2**: profiles declare
   authority-required operations (`requires_authority`); the executor rejects an
   authority-required operation lacking a binding with `AuthorityMissing` (protocol
   §11.2/§12.2). When `authority` is `None` for a non-required operation, the profile's
   transition and consent rules own authorization, preserving the v0 fallback.
5. **Non-membership proof-bundle wiring** — **RESOLVED in Phase 4**: the §16.2
   `statechronicle.proof.non_membership.v0` bundle variant authenticates absence at a
   commit root fail-closed (inclusion proof of the empty-leaf constant; verifiers MUST
   assert the empty leaf). The accumulator primitive itself is unchanged.
6. **Snapshot cadence heuristics** — **RESOLVED in Phase 5**: protocol §15 adds a
    "Cadence guidance" note — publish a snapshot when estimated cumulative replay cost
    since the last snapshot exceeds the snapshot store cost (full state serialization +
    digest + storage); operational factors are per-tenant event volume, recovery SLA,
    verifier replay budget, and storage cost (no protocol-mandated interval); baseline
    heuristic is snapshotting when estimated replay-from-genesis (or from the last
    authenticated snapshot) time exceeds the deployment's recovery SLA. Snapshots never
    weaken tenant-level verification; authenticity is via the SnapshotProof binding to the
    enclosing commit's state root (ADR-006, §36 Q7).
