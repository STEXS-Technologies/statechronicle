**Document Version:** 1.0
**Last Updated:** 2026-08-03
**Status:** Draft
**Related Documents:** [ADR README](README.md), [Architecture Summary](../../ARCHITECTURE.md),
[ADR-001](ADR-001-VERTICAL_SLICE_ARCHITECTURE.md),
[ADR-003](ADR-003-TRUSTGRANT_PORTS_ONLY.md)

# ADR-002: Hexagonal Architecture (Ports & Adapters) for Domain Isolation

---

## Context

StateChronicle must enable:

1. **Testability**: deterministic transition rules must be testable without databases,
   frameworks, or external services (protocol §3: deterministic transitions).
2. **Infrastructure independence**: the protocol must not require a specific database,
   queue, object store, or runtime (protocol §2).
3. **Portable proofs**: verification must work without direct database access (protocol
   §16).
4. **Clean boundaries**: domain logic isolated from axum/sqlx/redis.
5. **Evolutionary architecture**: swap storage (Postgres vs FoundationDB vs SQLite) or
   transport without touching domain logic (protocol §27).

Without explicit boundaries, domain logic mixes with infrastructure:

```rust
// ❌ Coupled: domain logic directly uses SQL
pub async fn transfer_asset(&self, db: &PgPool, from: &SubjectId, to: &SubjectId) -> Result<()> {
    // Business logic mixed with database access
    sqlx::query("UPDATE assets SET owner = $1 WHERE id = $2").execute(db).await?;
}
```

This makes testing require real databases and makes backend substitution impossible.

## Decision

**Adopt Hexagonal Architecture (Ports & Adapters) within every slice and behind every
core crate, with ports centralized in `statechronicle-ports`.**

### Core Structure

```text
Domain Layer (Center)
├── Pure business logic
├── Domain models (entities, value objects, aggregates)
├── Domain events
├── No external dependencies
├── No port-trait *calls* (trustgrant convention: core works with assembled domain types)

Application Layer (Orchestration)
├── Use case handlers (commands, queries)
├── Coordinates domain + ports
├── Transaction boundaries
└── Consumes ports from statechronicle-ports

Infrastructure Layer (Outer)
├── Port implementations (adapters)
├── Database repositories (postgres/)
├── External service clients (trustgrant adapter, wired at the stexs root)
├── HTTP handlers (driving/rest/)
└── Framework-specific code
```

### Port Definition (Application Layer)

```rust
// crates/statechronicle-ports/src/event_store.rs
// Port = interface the application requires

#[trait_variant::make(EventStore: Send)]
pub trait EventStore {
    /// Append one or more validated events. Returns their sequence numbers.
    async fn append(&self, events: &[Event]) -> Result<Vec<u64>>;

    /// Read events for a resource in commit order.
    async fn history(&self, tenant: &TenantId, resource: &ResourceId) -> Result<Vec<Event>>;
}
```

### Adapter Implementation (Driven)

```rust
// consumer-owned adapter, e.g. stexs crates/slices/ledger/adapters/driven/postgres/event_store.rs

pub struct PostgresEventStore { pool: PgPool }

impl EventStore for PostgresEventStore {
    async fn append(&self, events: &[Event]) -> Result<Vec<u64>> {
        // sqlx transaction; insert rows; return sequence numbers
    }
}
```

### Test Adapter (In-Memory)

```rust
// crates/statechronicle/tests/common/mod.rs (or a consumer test crate)

pub struct InMemoryEventStore { events: Arc<Mutex<Vec<Event>>> }

impl EventStore for InMemoryEventStore {
    // pure in-memory implementation for unit/integration tests
}
```

### Dependency Injection at the Composition Root

```rust
// consumer composition root only (e.g. stexs)

pub fn build_inventory_slice(pool: PgPool, evaluator: TrustGrantEvaluator) -> InventoryApi {
    InventoryApi {
        transfer: TransferAssetCommandHandlerV1 {
            assets: Arc::new(PostgresAssetRepository::new(pool)),
            events: Arc::new(PostgresEventPublisher::new(pool)),
            authority: Arc::new(AuthorityEvaluatorAdapter::new(evaluator)),
            tx: Arc::new(PostgresTransactionManager::new(pool)),
        },
        // ...
    }
}

// Tests use in-memory adapters
#[cfg(test)]
fn build_test_inventory_slice() -> InventoryApi { /* in-memory fakes */ }
```

---

## Alternatives Considered

| Option | Pros | Cons |
| --- | --- | --- |
| **No explicit boundaries** | Simple; less boilerplate | Untestable; coupled; rigid; violates protocol §2/§27 |
| **Traditional 3-tier** | Familiar | Services still coupled to infrastructure |
| **Clean Architecture** | Very explicit boundaries | Over-engineered; excessive layers |
| **Hexagonal (Ports & Adapters)** | Clear contracts; testable; flexible | Requires discipline; more interfaces |

### Why Hexagonal Wins

1. **Rust traits are ideal**: traits express ports; impls are adapters.
2. **Protocol mandate**: §2 infrastructure independence and §27 storage contract are
   direct consequences.
3. **Testability without mocking frameworks**: in-memory adapters.
4. **Proven pattern**: same as stexs (ADR-004), trustgrant (`trustgrant-ports`), shardline
   (trait crates + impl crates).

---

## Consequences

**Positive:**

- Domain logic is pure and deterministic: directly testable.
- Infrastructure swappable (Postgres ↔ SQLite ↔ FoundationDB) without touching domain.
- Portable proof verification without database access.
- Aligns with trustgrant-ports convention (ports crate with no impls).
- Framework-independent core.

**Negative:**

- More code (traits + impls + adapters).
- Indirection: trace through ports to understand full flow.
- Learning curve.
- Potential over-engineering for trivial reads.

**Mitigations:**

- Centralize ports in one crate (`statechronicle-ports`); reuse across slices.
- Simple queries may return read-model DTOs directly where justified.
- Macros (`shared::define_application_service!` style, as in stexs).
- Documentation + examples per slice.

---

## Implementation Guidelines

### Port Naming

```rust
trait IntentStore {}      // dedup + idempotency
trait EventStore {}       // append-only transitions
trait CommitStore {}      // signed batch commits
trait StateIndex {}       // current state projection
trait ProofIndex {}       // proof generation
trait SnapshotStore {}    // compact checkpoints
trait TenantStore {}      // tenant scope + isolation
trait TrustGrantEvaluator {} // authority evaluation (see ADR-003)
trait TransactionManager {}
trait EventPublisher {}
```

### Adapter Naming

```rust
struct PostgresEventStore {}
struct PostgresStateIndex {}
struct InMemoryEventStore {}
struct RedisStateCache {}
struct AuthorityEvaluatorAdapter {}
```

### Layer Dependencies

```text
Domain Layer         → (nothing)
Application Layer    → Domain Layer, statechronicle-ports
Infrastructure Layer → Application Layer, Domain Layer
```

**Rule:** inner layers never depend on outer layers. Domain never knows axum/sqlx/redis.

### Ports Crate Rule

`statechronicle-ports` contains **traits only**: no adapters, no persistence, no
transport. Adapters are the consumer's job (trustgrant-ports convention).

### Shared Boundary Clarification

- Protocol core (`statechronicle-core`, `statechronicle-domain`): transport-agnostic
  primitives (newtypes, error envelope, amounts, digests).
- `statechronicle-ports`: the ten port traits, no implementations.
- The consuming platform owns any HTTP/shared boundary (`shared`, `shared-http`) and its
  composition root; domain/application must never import from `shared-http`.

---

## Review & Maintenance

- **Last Reviewed:** 2026-08-03
- **Next Review:** Before first production release or when port patterns need refinement
- **Change Log:**
  - v1.0 (2026-08-03): Initial ADR documenting hexagonal architecture decision
