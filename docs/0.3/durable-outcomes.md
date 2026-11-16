# Verified durable outcomes and recovery

The advanced `commit::durable` module provides `commit_verified` and `recover`.
It shares preflight checks and the atomic transaction implementation with legacy
`persist_durable_verified`. Old signatures, result variants, error order and
wire representations remain unchanged. Existing adapters need not implement the
new `ports::ledger_store::LedgerRecovery` extension until adopting this path.

This is a commit/recovery boundary, not a replacement for authenticated ingress.
First bind an independently authenticated principal, trusted intent key and
explicit policy through `mutation::Preparer::prepare_player`. Sign a commit covering that
protected event set and derive its projections. The advanced composition
assembles `DurableCommitRequest`; prefer the protected
[SignedMutation](signed-mutation.md) wrapper for ordinary single writes and
[SignedBatch](durable-batches.md) for supported single-tenant groups. A signed arbitrary event set is not proof that an application
command was correctly reduced.

`commit_verified` validates signer trust, intent/digest, principal/policy, event
integrity and bindings, event root, projections and outbox before starting the
ledger transaction. The one transaction owns the idempotency claim, events,
signed commit and canonical head, projections, outbox and finalization. After a
successful commit or finalized replay claim, it reads the original immutable
ledger record and checks it before issuing a receipt. A commit-id-only replay
claim cannot construct a receipt. Newly committed records must also exactly
match the acknowledged signed commit, events and outbox.

| Result | Meaning and required action |
|---|---|
| `DurableOutcome::Committed` | New atomic transaction acknowledged; original ledger record checked. |
| `DurableOutcome::Replay` | Original finalized mutation read and checked; no second write. |
| `DurableError::Rejected` | Input or signer was refused before claiming. |
| `Authorization` | Preserves `Denied`, `Unauthenticated`, `ContextMismatch` or policy `Unavailable` as a typed cause. |
| `PayloadConflict` | Original tenant/id is bound to another canonical payload. |
| `InProgress` | Another matching reservation exists; retain original identity. |
| `Conflict` / `Unavailable` | Pre-commit/read failure; preserve identity and distinguish a prior unknown attempt. |
| `Corrupt` | Inconsistent adapter evidence or invariant; no receipt. |
| `Unknown` | Commit/rollback/readback uncertain; retain identity AND original intent, and reconcile with lookup. |

Every final-commit error is unknown, including an optimistic-conflict error,
because the general adapter error contract does not prove a failed ACK means no
write. A failed rollback is also unknown. Missing, corrupt or unavailable
readback after acknowledged commit or finalized replay cannot become safe
failure. The original typed adapter cause and any secondary rollback failure
are retained where available. Authorization errors retain their typed cause;
the existing commit-verifier port still supplies a string detail. No refusal on
a subsequent attempt resolves an earlier unknown write: reconcile that original
identity separately before deciding what happened.

Call `recover` with the original intent and authenticated principal before
constructing another transition after an uncertain result. It rechecks structural
scope and access policy, then performs one authoritative lookup. A committed
record is independently checked against that intent/digest, tenant, trusted
commit signer, complete event root and deterministic projections, including
outbox row integrity. It needs no reducer, new event identity, signature on a new
commit, or idempotency reservation. `RecoveryOutcome::NotFound` describes only
the read's linearization point: an outstanding transaction might still commit.
An expired pending lease does not prove rollback. Neither observation permits
inventing a new identity or claiming a previously unknown write safely failed.

`CommittedMutation` has private fields and no public constructor or deserializer.
It owns the original record, so later caller/adapter buffer mutations cannot
retarget it. Its identity digest covers the full intent body, including actor,
resource, operation, nonce and inputs. A receipt describes the original canonical
mutation; later legitimate commits do not invalidate historical replay.

## Adapter trust and retained evidence

`LedgerRecovery` must read the SAME authoritative atomic datastore as its
`LedgerStore` write transaction. It must observe finalized idempotency, immutable
original events/commit/projections/outbox consistently and only return commits
from canonical append-only history. An independent intent store is insufficient.
Missing or inconsistent finalized data is an error, never absence or success.
Lookup cannot dispatch effects. Stored/decoded record bounds must match write
limits.

Retain original effects after projections advance and outbox rows are delivered
or compacted. A current projection or today's delivery queue is not the original
record. Event and projection completeness can be checked against the signed
commit and deterministic derivation. Original outbox completeness, canonical
history membership and physical persistence still depend on the qualified
adapter: a state/event signature cannot independently prove all those facts.
A newly acknowledged write additionally compares outbox readback with the
caller's exact expected rows. Recovery does not claim cryptographic proof of
outbox absence, database atomicity, power-loss durability or physical truth.

The retained tests use an in-memory transactional adapter and logical fault
injection/reopening over shared committed state. They do not qualify process
crashes, disk recovery or a production database. The separate
[conformance harness](adapter-contract.md), protected batch path
and [file tutorial](../../tutorials/durable-custom-ledger/README.md) add scoped
evidence; full adapter/crash, cross-tenant composition and final exact-candidate
qualification gates remain open. This path adds readback and verification
work, so legacy persistence timings do not establish equal-work performance.
