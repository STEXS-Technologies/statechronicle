# Architecture Decision Records

This directory records architecture decisions for StateChronicle, following the same
ADR convention used by its sibling workspaces.

## Active ADRs

| ADR | Title | Status |
| --- | --- | --- |
| [ADR-001](ADR-001-VERTICAL_SLICE_ARCHITECTURE.md) | Vertical Slice Architecture as Primary Code Organization | Draft |
| [ADR-002](ADR-002-HEXAGONAL_ARCHITECTURE.md) | Hexagonal Architecture (Ports & Adapters) for Domain Isolation | Draft |
| [ADR-003](ADR-003-TRUSTGRANT_PORTS_ONLY.md) | TrustGrant Integration: Ports Only, Wired at the Consumer Root | Draft |
| [ADR-004](ADR-004-CANONICALIZATION_HASHING_SIGNATURE.md) | Canonical Serialization, Hashing, and Signature Baseline: BCS | Draft |
| [ADR-005](ADR-005-STATE_ACCUMULATOR.md) | Baseline State Accumulator: Sparse Merkle Tree (SMT) | Accepted |
| [ADR-006](ADR-006-PROTOCOL_DECISIONS_V0.md) | Protocol Decisions for v0 (§36 Resolution) | Accepted |
| [ADR-007](ADR-007-TRADE_PROFILE.md) | Peer-to-Peer Trades: trade_held Freeze, Atomic Settlement, trade.v1 Process | Proposed |

## Proposed / Pending

- Storage contract / backend selection (PostgreSQL baseline): protocol §27/§35. (The
  logical stores are resolved as `statechronicle-ports` traits; concrete backend choice
  is the consumer's, so this row remains informational.)

## Process

1. Copy `TEMPLATE.md`.
2. Number sequentially (`ADR-00N-...`).
3. Fill Context, Decision, Alternatives, Consequences.
4. Record in the table above.
5. Update `docs/ARCHITECTURE.md` when the decision changes the architecture.
