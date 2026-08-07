**Document Version:** 1.0
**Last Updated:** 2026-08-03
**Status:** Draft
**Related Documents:** [ADR README](README.md), [Architecture Summary](../../ARCHITECTURE.md),
[ADR-002](ADR-002-HEXAGONAL_ARCHITECTURE.md)

# ADR-003: TrustGrant Integration, Ports Only, Wired at the stexs Root

---

## Context

StateChronicle is a standalone component of the stexs platform rebuild. It replaces
stexs' existing `inventory_ledger` with a verifiable, append-only, replayable state
ledger. Two sibling protocols are relevant:

- **TrustGrant** provides authority truth: *who is allowed to act*. StateChronicle must
  consume TrustGrant evaluation results as authority proofs before applying state
  transitions (protocol §4.2, §18.1 step 8).
- **Shardline** provides content truth: *what the resource bytes are*. StateChronicle
  references content by digest only and never stores or resolves payloads.

### Why Shardline is out-of-band

The ledger records *that a transition happened, under whose authority, to what state*.
Its `content` references are self-describing digests (`sha256:...`, `media_type`, `size`)
carried as event fields. StateChronicle never calls out to a content store: resolution
and byte-verification happen in **another stexs system** (content/asset delivery), not in
the ledger and not client-side. Shardline is therefore **complementary, not a
dependency**: no port, no adapter, no crate dependency. The ledger's content field is
pure metadata; any content-addressed store satisfies it.

### Why TrustGrant is in-band

TrustGrant is different: authority evaluation is part of the **execution pipeline**. An
intent is not valid until the actor's authority for the operation on the resource in the
tenant scope is evaluated and bound into the event (`authority: { kind:
trustgrant.evaluation, evaluation_digest, result }`). This is a runtime *step* of the
pipeline: **not a compile-time dependency on the trustgrant crate**.

Two integration options were available for TrustGrant:

1. **Direct dependency**: StateChronicle crates import the `trustgrant` crate directly
   (path dep) and call its APIs inline.
2. **Ports-only**: StateChronicle defines its own `TrustGrantEvaluator` port; the real
   adapter is wired later at the composition root.

Considerations:

- TrustGrant's own convention (`trustgrant-ports`) is that the core never calls port
  traits itself and adapters are consumer-side.
- stexs treats `trustgrant` as a sibling workspace crate but wires it through slice ports and
  composition-root gateways, never through direct slice imports.
- The protocol must remain infrastructure-agnostic (§2) and its core deterministic and
  transport-free.

## Scope Correction

The TrustGrant evaluation port is **delegation of authority only**, not general platform
authorization. Two distinct concerns must not be conflated:

- **Platform basic authorization**: owner/actor identity and basic authorization are the
  platform's own auth system, applied before (or alongside) the execution pipeline.
  StateChronicle does not implement general authorization.
- **Delegated-authority evaluation**: the `TrustGrantEvaluator` port is how a platform
  hands authority *delegation* evidence (an opaque, content-addressed evaluation) to the
  ledger.

The port is **trait-only and dependency-free by construction**: it references only
`statechronicle-domain` types and is not coupled to any authority provider. Any evaluator
that returns an `allow` result and passes the freshness check can be plugged in. TrustGrant
is **one option, not a requirement**. The executor does not care what produced the
evaluation; it fails closed on non-`allow` or stale results.

## Decision

**Integrate TrustGrant through a dedicated port only. The real adapter is owned by the
stexs composition root. Shardline is not integrated at all: StateChronicle has no
content-store dependency.**

### Where Authority Evaluation Executes

Split the concern:

- **The gate lives in `statechronicle-executor`.** The execution pipeline (§18.1 step 8)
  calls the port, and §18.2 makes non-`allow` and stale evaluations fail closed. The
  event's `authority: { …, result: "allow" }` block is bound by the executor into the
  signed, append-only log, so a caller can never commit an event claiming `allow` with
  no evaluation behind it. The intent (§11.1) carries only `evaluation_digest` (no
  result), so the executor must call the port to obtain the outcome itself.
- **The evaluation runs behind the port.** Verifying grants against discovery/revocation/
  ownership sources and running trustgrant's pure `EvaluationEngine` is external,
  non-deterministic I/O; it must never touch the deterministic core. This mirrors
  trustgrant's own split (verification pipeline takes assembled sources; core is
  stateless).

### Port (in `statechronicle-ports`)

```rust
// Authority evaluation: binds a TrustGrant evaluation to a transition.
#[trait_variant::make(TrustGrantEvaluator: Send)]
pub trait TrustGrantEvaluator {
    /// Evaluate whether `actor` may perform `operation` on `resource` in `scope`.
    /// Returns an evaluation outcome whose digest is bound into events.
    async fn evaluate(
        &self,
        scope: &TenantId,
        actor: &SubjectId,
        operation: &str,
        resource: &ResourceId,
    ) -> Result<TrustGrantOutcome>;

    /// Check revocation freshness for an authority proof.
    async fn check_revocation_freshness(&self, proof: &AuthorityProof) -> Result<()>;
}
```

The port signature uses **only statechronicle-domain types** (`TrustGrantOutcome`,
`AuthorityProof`, `TenantId`, `SubjectId`, `ResourceId`), deliberately unlike
trustgrant-ports which imports its own sibling crates. StateChronicle's port is
dependency-free by construction.

### Canonical Authority Wire Format

The authority block is **opaque, content-addressed, and versioned**:

- `kind` is a registered namespaced string identifier (`"trustgrant.evaluation"`), no
  more a dependency than a MIME type; register it canonically in the protocol.
- `evaluation_digest` = RFC 8785 canonicalization of the evaluation → SHA-256, so any
  verifier can reproduce it without parsing trustgrant internals.
- The ledger never parses the internal structure of an evaluation; verification resolves
  the digest through the verifier's own trustgrant integration (§16.3, §29).
- The executor cannot detect an adapter that fabricates digests: that is an inherent
  trust boundary at the port, backstopped by the verifier path.

### What the Protocol Binds

- **Events** carry `authority: { kind: trustgrant.evaluation, evaluation_digest, result }`
  (protocol §12.1).
- **Execution** requires the TrustGrant evaluation result to be `allow` and fresh before
  emitting an event (§18.1 step 8, §18.2 conflict rules).
- **Proof bundles** reference `trustgrant.proof_bundle` digests (protocol §16.2).
- **Commit authority** subjects are authorized through TrustGrant or configured trust roots (protocol §19; ADR-006 §36 Q2).

### Content References (no dependency)

- **Events** carry `content: { kind, digest, media_type, size }` (protocol §4.1) as
  **pure metadata fields**: never payload bytes, never resolved by StateChronicle.
- Digest format follows the protocol baseline: `sha256:<lowercase-hex>` (§17).

### Wiring Owned by the Consumer

- A delegated-authority adapter (implementing `statechronicle_ports::TrustGrantEvaluator`
  against the consumer's authority provider, e.g. the real `trustgrant` crate) lives in the
  consumer's composition root (extending the stexs `crates/api-http/src/infra/trustgrant/`
  pattern). stexs is the single place the three protocols meet; StateChronicle ships no
  adapter and no trustgrant dep.
- This workspace's own test harness (`crates/statechronicle/tests/common/mod.rs`) uses an
  in-memory fake, so all protocol logic is fully testable without the external workspace.
- **Deferred (tracked decision, rename noted):** an optional companion crate (the earlier
  working name `statechronicle-trustgrant` is provisional) for standalone adopters who
  want real authority without a platform. Triggers: a second standalone consumer,
  trustgrant stabilized and published on crates.io, or a standalone binary needing real
  authority out of the box. Not built now: trustgrant's API is still settling and churn
  would be wasted.

---

## Alternatives Considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Direct path dep on trustgrant in core** | No indirection; immediate access to trustgrant APIs | Couples core to external workspace layout; violates §2 infra-agnosticism; unpublishable as standalone crate; forces trustgrant on all adopters despite §7 "compatible authority profile"; cross-publish version skew |
| **Ports-only, adapter at stexs root (Chosen)** | Core stays pure; protocol fully testable now; adapter swappable; matches trustgrant/stexs conventions; statechronicle stays independently publishable | Extra indirection; adapter must be built by stexs before production use |
| **Ports-only, adapter in a consumer binary** | Standalone binary works out of the box | Duplicates stexs' integration; makes the workspace unpublishable (path dep); two sources of truth for one consumer |
| **Optional companion crate** | Standalone adopters get real authority; core stays clean | Churn against a settling API; deferred until triggers hit |

### Why Ports-Only Wins

1. Protocol core remains deterministic and infrastructure-independent (§2).
2. Full protocol behavior testable immediately with in-memory fakes.
3. Matches trustgrant-ports and stexs composition-root conventions.
4. Authority boundary stays explicit and auditable.
5. Shardline excluded entirely: content is out-of-band metadata, resolving in another
   stexs system.
6. StateChronicle stays independently publishable (no path deps, no version skew).

---

## Consequences

**Positive:**

- Core crates have zero dependency on trustgrant/shardline internals.
- Protocol correctness is verifiable before the trustgrant integration exists.
- The trustgrant adapter is swappable (test double ↔ real service).
- Content references are portable: any digest-addressed store works; StateChronicle is
  not coupled to shardline semantics.
- The authority gate is an executor invariant: no event enters the append-only log with a
  claimed `allow` unless the port returned `allow` and fresh at emit time (TOCTOU-safe,
  §30).
- StateChronicle remains independently publishable as an open-source workspace.

**Negative:**

- Adapter code required (by stexs) before production use of real authority.
- Indirection between StateChronicle's authority intent and trustgrant's actual API.
- Must keep the port contract aligned with trustgrant's real API to avoid adapter
  friction later.

**Mitigations:**

- Port signature is modeled on trustgrant's real API surface (`EvaluationEngine`,
  `EvaluationOutcome`, discovery/revocation sources).
- ADR-014 in stexs (TrustGrant deployment profile) informs the adapter design.
- A compatibility note in the port's doc comment maps it to the upstream API.

---

## Implementation Notes

### Adapter Sketch (stexs Composition Root Only)

```rust
// stexs/crates/api-http/src/infra/trustgrant/evaluator.rs
pub struct TrustGrantEvaluatorAdapter { engine: trustgrant::evaluate::EvaluationEngine, /* ... */ }

impl statechronicle_ports::TrustGrantEvaluator for TrustGrantEvaluatorAdapter {
    // wrap trustgrant EvaluationRequest → StateChronicle TrustGrantOutcome
    // bind evaluation_digest into the event's authority block
}
```

### Migration Path

1. v0: in-memory fakes in the workspace's test harness; protocol fully exercised.
2. v0.5: stexs implements a delegated-authority adapter at its composition root.
3. v1: revocation freshness policies, proof-bundle enrichment, key rotation via the
   consumer's authority provider.
4. Triggered (not scheduled): a companion crate for standalone adopters.

---

## Review & Maintenance

- **Last Reviewed:** 2026-08-03
- **Next Review:** When the TrustGrant adapter is implemented
- **Change Log:**
  - v1.2 (2026-08-03): Oracle audit applied: no compile-time dependency on trustgrant
    anywhere; executor owns the authority gate; adapter owned by the stexs composition
    root; canonical authority wire format specified; optional companion crate recorded
    as deferred.
  - v1.1 (2026-08-03): Shardline removed: complementary, out-of-band; TrustGrant-only
    integration. StateChronicle is a standalone component replacing stexs'
    `inventory_ledger`; content resolution happens in another stexs system.
  - v1.0 (2026-08-03): Initial ADR documenting ports-only integration decision
