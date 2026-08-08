**Document Version:** 1.0
**Last Updated:** 2026-08-08
**Status:** Proposed
**Related Documents:** [ADR README](README.md), [Architecture Summary](../../ARCHITECTURE.md),
[ADR-003](ADR-003-TRUSTGRANT_PORTS_ONLY.md), [ADR-006](ADR-006-PROTOCOL_DECISIONS_V0.md)

# ADR-007: Peer-to-Peer Trades — `trade_held` Freeze, Atomic Settlement, and the `trade.v1` Process

**Date:** 2026-08-08

---

## Context

StateChronicle covers single-resource transitions (mint, transfer, burn, list,
escrow) and marketplace atomic purchase. It does not cover **peer-to-peer
trades**: player A offers items X for items Y from player B, B accepts or
declines, and if accepted both items swap atomically. Trades add three things
the current model lacks:

1. **Freeze while negotiating**: items on the table must be unusable (no
   transfer, list, burn, or use) for the duration of the negotiation, so
   neither party can spend or sell an item they committed to a pending trade.
2. **Negotiation**: proposals, accept/decline, one-sided offers that the other
   party counters, per-party deadlines. This is a multi-step process, not a
   single ledger transition.
3. **Atomic settlement**: both items change owner in one all-or-nothing commit
   once the trade is agreed.

Verified constraint that shapes the design: `asset.transfer` is hard-gated to a
source status of `active` (`unique_asset.rs`), so a frozen item cannot be
transferred today; and a batch validator admits exactly one multi-event unit
per intent id (a `stack.transfer`/`balance.transfer` net-zero pair), so a
settlement must be expressed within the existing atomic-batch contract.

A second requirement from the owner: **paid assets** (bought with real money,
e.g. a skin) must remain permanently owned — the studio cannot delete, burn, or
transfer them away; only **earned** items may be removed for economic reasons.
The `paid_unique_asset` profile already enforces this; trades must not bypass
it.

## Decision

### 1. Freeze representation: a dedicated `trade_held` status

Add a new unique-asset status `trade_held` with three operations, **not** a new
`StateType`, **not** reuse of `locked`/`escrowed`:

| Op | From → To | Payload | Authority | Consent |
|---|---|---|---|---|
| `trade.lock` | `active → trade_held` | `owner`, `status`, `trade_id` | not required | `from_owner == owner`; paid: `authorized_by_owner` when actor ≠ owner |
| `trade.unlock` | `trade_held → active` | `owner`, `status` (trade_id dropped) | not required | `trade_id` must match stored |
| `trade.settle` | `trade_held → active` (new owner) | `owner: to_owner`, `status` | **required** | `from_owner == owner`, `to_owner`, `trade_id` matches; paid: `authorized_by_owner` when actor ≠ owner |

Rationale: `locked` conflates player locks with trade freezes; `escrowed`
carries funds-holding semantics with terminal states; a new `StateType` variant
breaks the closed enum and ripples through `subject_for`/`state_key_for`/the
registry. The `trade_held` status is itself the optimistic-concurrency mutex:
a second pending trade's `trade.lock` fails fail-closed (source already
`trade_held`), and the stored `trade_id` prevents cross-trade unlock/settle.

`conflict.rs::status_escapes` gains `trade_held → ["trade.unlock",
"trade.settle", "asset.restrict"]` (a legal hold must remain able to restrict a
held asset).

### 2. Settlement: one atomic transaction, never two independent commits

Both asset transfers land in a **single** atomic batch:

- **Same tenant**: two `trade.settle` intents with distinct intent ids in one
  `execute_batch`; `validate_batch_consistency` passes (each intent group has
  exactly one event; the transfer-pair rule is not involved because these are
  `trade.*` ops, not `stack.*`/`balance.*`).
- **Cross tenant**: both settle legs carry the **same intent id = the trade
  instance id** (the proven `execute_cross_tenant` shared-id linkage pattern);
  per-tenant groups each hold one event; per-tenant intent stores make the
  shared id safe (idempotency key includes `tenant_id`).
- **Consistency, not net-zero**: there is no fungible field to conserve. The
  invariants are: exactly one settle event per asset, both-or-none via
  `begin`/`commit`/`rollback`, per-leg profile gates, and `expected_version`
  observed at settle-submit time.

### 3. Negotiation: the `trade.v1` process in Penelope

The negotiation is a versioned Penelope `SagaDef` (`def_id "trade.v1"`), not
ledger logic:

| Step | Trigger | Action | Failure → |
|---|---|---|---|
| `validate_proposal` | proposal inbox event | reject self-swap, duplicate assets, non-`active`, unknown party | abort |
| `freeze_a` | proposal valid | submit `trade.lock(asset1)` intent | retry w/ backoff; escalate |
| `freeze_b` | freeze_a ok | submit `trade.lock(asset2)` intent | compensate `freeze_a`, cancel |
| `await_acceptance` | both frozen | wait on accept event or per-party deadline | LIFO unlock both, cancel |
| `settle` | both accepted | **one batch**: `trade.settle(A→B)` + `trade.settle(B→A)` | batch atomic rollback → unlock both → cancel; escalate on terminal failure |

Per-party deadlines are `timer_scheduler` firings replayed as data.
Compensation is `trade.unlock` LIFO, each exactly-once by the step idempotency
key. Escalation to the manual-review queue is the only path that can leave an
asset `trade_held`.

### 4. Paid assets trade without losing invariants

`trade.settle` is an ownership transfer: it joins `AUTHORITY_REQUIRED` in both
profiles, and the paid overlay requires `authorized_by_owner: true` when the
acting actor is not the owner (the owner's approve action IS the consent).
A trade moves a paid asset; it never bypasses its consent gates. Earned items
under the base profile keep the studio's `asset.burn` authority for economic
removal; bought items under the paid profile cannot be removed by the studio
(ADR-006 Q10/Q11).

## v2 items (owner may add)

All six deferred items were designed in depth; recommendations below.

| # | Item | Recommendation | Why |
|---|---|---|---|
| v2-4 | Auto-settle on mutual-acceptance deadline | **Include first** (small; Penelope-only) | Cheapest operational win; needs v0 to define an acceptance *event* (replayable predicate), else the precondition is unreplayable |
| v2-5 | Manual-review resolution actions | **Include** (medium) | Completes the escalation contract v0 promises; resolution becomes a first-class input event, authorized at the adapter boundary |
| v2-1 | Value legs (asset for gold) | **Include** (medium) | The economically meaningful trade shape; same settle-manifest validator reused by v2-2/3; do not weaken `validate_transfer_pair` (new shape validator, not a relaxed rule) |
| v2-2 | Cross-tenant settle, three legs | **Include after v2-1** (medium-large) | The 3-leg cross-tenant case genuinely fails today's inferred-linkage rule; use a declared-linkage extension (manifest as the signed binding), not the marker-event workaround |
| v2-3 | Bundle trades (N assets per side) | **Defer** unless a customer demands it | N-lock atomicity is already free via `execute_batch`; the cost is manifest + bundle validator + Penelope batch-compensation; builds cheaply on v2-1/2 later |
| v2-6 | Trade proofs / history queries | **Defer** (large; read-side project) | Drags in the first generic read API; full value needs v2-1/2's manifest as the join key; a single-tenant `get_history` slice can ship on v0 alone |

**v2 sequencing** (if all are wanted): v2-4 + v2-5 first (Penelope-only, zero
executor risk, ship as `trade.v1` def v0.1), then v2-1 → v2-2 → v2-3 as one
"settle manifest" workstream (build the validator once, generalize linkage,
then extend to bundles), then v2-6 last.

## Alternatives Considered

| Option | Pros | Cons |
| --- | --- | --- |
| `trade_held` status (chosen) | Status is the mutex; auditable `trade_id`; no enum/registry ripple | Three crates must land in lockstep |
| Reuse `locked`/`escrowed` | No new ops | Conflates player locks/funds with trades; neither has a transfer path; escape-list pollution |
| Escrow object per trade | Centralized trade state | Does not freeze the asset itself; new object + store; double-bookkeeping |
| Marker event for cross-tenant linkage | Zero executor changes, ships fast | Ceremonial linkage (id spans two *different* intents); two permanent marker events per settle; does not generalize to bundles |
| Declared-linkage extension (chosen for v2-2) | Manifest is the signed binding; generalizes | Executor + validator + wire-type work |

## Consequences

**Positive:**
- Peer-to-peer trades (simple, one-sided + counter, accept/decline) become
  expressible with freeze-while-negotiating and atomic settlement.
- Paid-asset ownership invariants survive trades; earned items stay
  economically removable by the studio.
- Concurrency safety is structural: the status IS the mutex; no lock registry,
  no deadlock window.
- The v2 items are each designed with a clear include/defer recommendation and
  a dependency order, so the owner can pull them in incrementally.

**Negative:**
- `trade_held` without an escape path would make a held asset permanently
  unmovable, so `unique_asset.rs` + `transition.rs` + `conflict.rs` must land
  in a single atomic change.
- Negotiation lives in Penelope, which is scaffolded but not yet implemented;
  `trade.v1` becomes its reference implementation and depends on its `apply`
  contract being built first.
- Cross-tenant settlement's execution-time-only atomicity means a verifier of
  one tenant's chain cannot reconstruct the other leg (existing §8.2
  limitation, unchanged).

## Review & Maintenance

- **Last Reviewed:** 2026-08-08
- **Next Review:** When the trade profile or `trade.v1` begins implementation,
  or when a v2 item is pulled in
- **Change Log:**
  - v0.1 (2026-08-08): Initial — trade_held freeze, atomic settlement,
    trade.v1 process, paid-asset invariants, v2 recommendations
