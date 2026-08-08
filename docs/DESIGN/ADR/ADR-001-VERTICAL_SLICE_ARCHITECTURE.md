**Document Version:** 1.0
**Last Updated:** 2026-08-03
**Status:** Draft
**Related Documents:** [ADR README](README.md), [Architecture Summary](../../ARCHITECTURE.md),
[ADR-002](ADR-002-HEXAGONAL_ARCHITECTURE.md)

# ADR-001: Vertical Slice Architecture as Primary Code Organization

---

## Context

StateChronicle is a verifiable resource-state protocol and platform that must:

1. **Compose with a consumer platform**: StateChronicle must follow the same structural
   conventions as the consumer platform so it can be absorbed or consumed cleanly.
2. **Mirror sibling workspaces**: the sibling workspaces (shardline, trustgrant) each
   organize code as bounded contexts (per-stage crates in trustgrant, shardline
   per-protocol-frontend crates, and platform slices under `crates/slices/*`).
3. **Support many resource state models**: unique assets, consumable stacks, fungible
   balances, entitlements, meters, listings, escrow; each is a natural bounded context.
4. **Enable independent testing**: each state model and each platform feature must be
   testable in isolation.
5. **Be monolith-friendly**: a modular monolith that can decompose later without
   re-architecting.

Traditional horizontal layering (Controllers → Services → Repositories) scatters feature
code, couples layers, and blurs ownership. For a protocol with this many distinct state
machines, scattered code would make the deterministic-transition guarantee (protocol §3)
hard to audit.

## Decision

**Adopt Vertical Slice Architecture as the primary code organization strategy.**

Each bounded context is a vertical slice. Slices live in two tiers:

1. **Protocol core slices**: one crate per protocol concern (intent, executor, commit,
   accumulator, proof, profiles). These are pure; they contain no transport or
   persistence. The `statechronicle` umbrella crate re-exports them as the consumer's
   single entry point.
2. **Platform slices**: in the consuming platform these are one crate per domain bounded
   context under `crates/slices/*` (ledger, inventory, economy, entitlement, marketplace,
   tenant), each containing the full hexagonal stack for that context. StateChronicle the
   standalone workspace does **not** ship platform slices; the protocol crates replace the
   `inventory_ledger` bounded context, and the platform slices live in the consuming
   platform, not here.

Platform slice layout (sibling convention):

```text
crates/slices/<name>/src/
├── lib.rs              # public contract export
├── domain/             # aggregates, entities, events, value_objects, services, specifications
├── application/        # commands/, queries/, view.rs
├── ports/              # input/, output/
├── adapters/           # driving/, driven/
├── error/              # domain.rs, infra.rs, rest.rs
└── tests/
```

### Slice Principles

1. **One slice = one bounded context**: e.g. `economy` owns fungible balances and
   currency transfers; `inventory` owns unique assets and consumable stacks.
2. **Complete feature ownership**: all code for a feature lives in one crate.
3. **Explicit dependencies**: cross-slice communication only via ports/gateways composed
   at the composition root, or via the shared kernel.
4. **Independent testing**: each slice is unit/integration tested in isolation with
   in-memory fakes.
5. **Clear boundaries**: slices never import each other's internals.

### Cross-Slice Communication

- **Domain events**: published via `EventPublisher`/outbox; other slices subscribe.
- **Public API contracts**: explicit gateway ports when synchronous calls are needed.
- **Shared kernel**: transport-agnostic primitives in `statechronicle-core`/
  `statechronicle-domain`; HTTP-only concerns live in the consuming platform's own
  shared kernel, never in this workspace.

---

## Alternatives Considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Traditional Layers** | Familiar; clear separation | Scattered feature code; high coupling; hard to audit transition determinism |
| **Microservices per Feature** | Complete isolation; independent deployment | Distributed complexity; premature for a protocol-first v0 |
| **Feature Folders with Shared Layers** | Some organization; reusable services | Still coupled; shared services become bottlenecks |
| **Vertical Slices (Chosen)** | Complete feature isolation; clear ownership; deterministic rules auditable per slice | Initial learning curve; potential duplication across slices |

### Why Vertical Slices Wins

1. **Protocol auditability**: each state machine's transition rules live in one place.
2. **Composition with a consumer**: identical `slices/*` convention makes integration natural.
3. **Profile-based specialization** (protocol §20) maps directly to slice boundaries.
4. **Monolith-first**: modular monolith now, service extraction later if needed.
5. **Reduced cognitive load**: understand an entire state model in one crate.

---

## Consequences

**Positive:**

- Deterministic transition rules are auditable per bounded context.
- Clear ownership of each resource state model.
- Independent testing with in-memory fakes.
- Natural service boundaries for later decomposition.
- Matches sibling workspace conventions (shardline, trustgrant).

**Negative:**

- Potential duplication of shared protocol mechanics across slices (mitigated by the
  pure core crates).
- Learning curve for the pattern.
- Cross-cutting concerns (authority binding, tenant scope) require discipline.
- Slice boundaries must be enforced via lint/review.

**Mitigations:**

- Pure protocol core crates hold shared mechanics; slices are thin application layers.
- `statechronicle-core`/`statechronicle-domain` kernel; the consuming platform owns any
  HTTP/shared boundary.
- Cross-cutting middleware at the composition root.
- Architectural guardrails + code review checklist.

---

## Implementation Notes

### Workspace Membership

This workspace's members are the pure protocol crates plus the umbrella crate and the
fuzz targets:

```toml
[workspace]
members = [
    "crates/statechronicle-core",
    "crates/statechronicle-domain",
    "crates/statechronicle-intent",
    "crates/statechronicle-executor",
    "crates/statechronicle-commit",
    "crates/statechronicle-accumulator",
    "crates/statechronicle-proof",
    "crates/statechronicle-profiles",
    "crates/statechronicle-ports",
    "crates/statechronicle",
    "fuzz",
]
```

There is no `statechronicle-shared`, `statechronicle-shared-http`,
`statechronicle-http`, or `crates/slices/*` here. Platform slices and the HTTP
composition root are owned by the consuming platform, which wires these protocol
crates through `statechronicle-ports` at its own composition root.

### Cross-Slice Boundary Enforcement

```rust
// ❌ FORBIDDEN: direct import from another slice's internals
use crate::slices::economy::domain::balance::Balance;

// ✅ ALLOWED: import from the protocol core kernel
use statechronicle_domain::subject::SubjectId;

// ✅ ALLOWED: communicate via ports composed at the root
economy_port.debit(subject, amount).await?;
```

### Testing Strategy

- Pure core crates: inline unit tests for transition determinism + integration tests
  against in-memory fake ports.
- Umbrella crate: the e2e lane (`crates/statechronicle/tests/e2e.rs`) runs the full
  lifecycle through the real crates with in-memory port fakes.
- Platform slices (consumer-owned): unit tests with fake adapters; the consuming platform's
  composition root runs its own e2e flows.

---

## Review & Maintenance

- **Last Reviewed:** 2026-08-03
- **Next Review:** Before first production release or when slice boundaries require adjustment
- **Change Log:**
  - v1.0 (2026-08-03): Initial ADR documenting vertical slice architecture decision
