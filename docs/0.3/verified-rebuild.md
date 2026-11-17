# Verified historical projection rebuild

`statechronicle::index::rebuild::verified` is the recommended opt-in path when
rebuilding from provider-supplied signed tenant history. The legacy event-only
functions retain their existing contracts: their caller supplies verified
canonical events and pins numeric checkpoint keys to the correct source.

Supply full ordered event batches, all signed tenant commits from genesis,
an independently obtained `HistoryPin { tenant, tip_digest }`, and a
`SignedCommitVerifier` that enforces historical tenant/key/time/revocation
policy. `Ed25519CommitVerifier` can resolve trusted keys through a host callback;
its signature check does not infer whether the returned key was authorized.
Never obtain the expected pin solely from the same untrusted history provider.

```rust,ignore
use statechronicle::index::rebuild::verified::{
    HistoryPin, HistoryWitness, verify_history,
};

// Pin and signer policy come from independent trusted application context.
let source = verify_history(&witnesses, &expected_pin, &historical_verifier)?;
source.rebuild_resumable("inventory-recovery", 128, &projection_sink, &checkpoints)
    .await?;
```

A complete external application using only the umbrella dependency is compiled
and executed by the candidate compatibility lane in
`ci/canonical-0.3/verified_rebuild.rs`. Its in-memory sink is deliberately bounded
to the demonstrated unique-asset profile and does not simulate disk durability.

Verification checks the final body digest against the independent pin; tenant
scope and commit identity; signature/trust policy; genesis and consecutive
parent/sequence links; event counts, scope and signed batch order/root; existing
committed-event identity/schema/type/version/digest/custom-domain rules; state
roots replayed from the empty root; per-holder continuity and duplicate IDs.
The first event for each resource/holder must start at version zero. Missing
prefixes, missing or rewritten events, reordered batches, root/chain failures
and mispinned or untrusted sources refuse before any sink/checkpoint call.
This authenticates signed historical integrity, not domain authorization or
reexecution of every original business rule.

`VerifiedHistory` owns its checked events, has private fields, offers no mutable
view, and cannot be deserialized. Mutating the provider's input after verification
cannot alter that source. It computes a domain-separated digest over all signed
commits and ordered event batches. `checkpoint_key(job)` hashes the job name and
that digest, so distinct histories/jobs never intentionally reuse a numeric
offset. The resumable method always derives this key itself. The checkpoint
adapter must honor key isolation; a faulty or malicious adapter can violate its
storage contract. Returned storage errors remain `RebuildError::Sink`, and
verification/refusal errors remain `RebuildError::Invariant`.

Projection writes precede checkpoint save. The sink must support idempotent,
monotonic upserts when a chunk is replayed after failed acknowledgement. Use a
dedicated projection namespace for the selected source: source-bound offsets do
not clear unrelated rows or roll back projections left by another branch.
Numeric offsets alone do not attest to physical database contents.

The API accepts an in-memory complete history and reuses existing replay-root
verification, which recomputes accumulated roots. Hosts must bound supplied
history size, provide backpressure and retain source bytes/pins. This is not a
streaming or constant-memory verification claim, and long-history throughput
still needs measurement. No protocol encoding, signature, root algorithm,
existing event-only behavior, default dependency or runtime requirement changes.

The verified result establishes no current-head freshness, authoritative ledger
acceptance, distributed atomicity, exactly-once delivery or physical-crash
safety. Historical equivocation/fork selection and the origin of the pin remain
host trust decisions. Schema migration still requires explicit versioned events;
no old payload is silently decoded under a new schema.
