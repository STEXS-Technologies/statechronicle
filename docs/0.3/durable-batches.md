# Protected atomic batches

The protected single-tenant batch path is:

1. `Executor::prepare_player_batch` authenticates and authorizes every leg,
   rechecks public request structure and prepares the complete owned event set
   without reading or writing an independent intent marker.
2. `mutation::SignedBatch::sign` consumes that whole plan. It checks tenant,
   event count and event root before signing; projections are derived inside
   the wrapper. A signature is not a durable receipt.
3. `SignedBatch::commit` independently checks commit signer trust, reauthorizes
   every member and validates the complete events/projections/outbox. It uses
   one `LedgerStore::begin`, claims every member in intent-ID order, stages one
   signed commit and all effects, retains original batch membership, finalizes
   every fenced claim and calls transaction commit once.
4. A confirmed result requires original-group readback through every member.
   `DurableBatchOutcome::Committed` and `Replay` contain a privately constructed
   `CommittedBatch` with the complete original record. Neither exposes success
   for a subset. Delivery belongs after confirmed durable authority.

The advanced commit-crate functions accept `BatchMember` and
`DurableBatchRequest`; they do not verify ingress signatures or recompute domain
reduction. Applications should use the owned protected wrapper.

## Membership and replay

Batch identity includes each member's original tenant, intent ID and canonical
body digest, plus a domain-separated digest of the complete ordered intent
bodies. Request order is part of identity; claim acquisition uses sorted IDs to
avoid opposing lock order for overlapping groups. The group is bounded to 256
intents/events, bounded individual/aggregate event bytes, and bounded aggregate
canonical intent bytes. Each member must contribute an event. All members are
in one tenant.

The ordinary tenant/intent idempotency namespace is shared with single writes.
All claims must be new, or all must already refer to the same original commit.
Mixed new/committed claims, changed payloads and different existing commit IDs
refuse with no new effects. Replay additionally checks exact original ordered
membership. A subset, superset or reorder cannot be promoted into a newly
accepted atomic group. Independently committed single mutations cannot be
combined into a batch receipt.

Replay checks the original signed record under the supplied signer policy,
not a newly proposed replacement commit. Recovery reauthorizes every original
request before any ledger lookup and never reruns reduction, signs or claims.
A historical batch receipt remains valid after later heads and outbox delivery;
it does not assert that those projections are current.

## Adapter capability

`LedgerTransaction::retain_batch` has a default implementation that refuses
unsupported retention, so existing adapters continue to compile. Supporting it
requires `ports::ledger_batch::LedgerBatchRecovery` on the same authoritative
store. The transaction must atomically retain `LedgerBatch` with the exact
ordered intent bodies, signed commit, complete events, original projections and
original outbox, including delivered rows. Commit must reject any omitted or
mismatched claim/effect/finalization. Every member must resolve to the same
immutable record in canonical history.

A batch lookup of a finalized standalone mutation must return an invariant
error, not absence or a fabricated singleton group. Likewise a single lookup
must not filter a batch down to a falsely independent single receipt.

Signature/root verification checks event integrity and signer trust. It cannot
prove adapter atomicity, physical durability, original membership metadata or
outbox completeness independently of the ledger contract. Those require
independent adapter tests. No wire representation of existing intents, events,
commits, signatures or proofs changes; the group is optional adapter metadata.

## Failure and recovery

Every final-commit error, rollback failure or uncertain finalized readback is an
explicit `DurableError::Unknown` within `DurableBatchError`. The outer error
retains the complete validated `BatchIdentity`; a contained single identity is
only member context. Preserve every original body and identity. Do not replace
a subset, invent new IDs, or dispatch on an unknown result.

Use the retained `SignedBatch::recover`, or independently reconstruct all
original members and call `recover_batch`. `NotFound` is a point-in-time anchor
observation, not proof that an outstanding transaction cannot later commit.
`InProgress` is reservation evidence, never a group receipt; lease expiry is not
proof of rollback. Failed lookup leaves prior uncertainty unresolved.

## Qualification and remaining scope

`crates/statechronicle/tests/durable_batch.rs` exercises the public path against
a bounded in-memory transactional adapter: complete write/readback/replay,
lost ACK, rollback and cancellation, failures on later member operations,
2/8/32-client same-group contention, original history after a later head and
outbox delivery, and negative cases for substituted/omitted/corrupt members,
events and projections. The same tests are compiled as an external candidate
consumer. Existing single-mutation recovery tests run unchanged against the
extended fixture.

These are logical atomicity tests, not database restart, process-kill or
power-loss qualification. The reusable `ci/conformance::batch` module now
checks staged visibility, rollback/cancellation, replay, lost ACK and distinct
group head contention against a separate reference adapter, with nine broken
batch controls. Its raw inspection includes original group membership, not just
lookup results. Generic checks additionally cover tenant isolation with coincident
IDs, cross-lane identity collisions and refusal of fabricated singleton/subset
receipts, plus 2/8/32-client exact-group contention. Lease expiry/races and
non-genesis retention/rebuild still need generic qualification.
This path does not support multi-tenant group commits or
sequential preparation overlays. Specialized companion/economic event binding
and cross-tenant settlement require separate authority review and tests; do
not infer that every legacy batch operation is supported by this protected
path. Full release fuzz/performance/API review gates remain open.
