# Protected single-mutation signing and persistence

`statechronicle::mutation::SignedMutation` binds the result of
`Executor::prepare_player` to commit signing and verified persistence. Its
private fields prevent replacing the authenticated principal, canonical payload
digest, prepared events, derived projections or owned outbox after binding.

Call `SignedMutation::sign(prepared, &body, key_id, signer, outbox)` using a
commit body built from the prepared events and independently obtained canonical
history/state context. Schema, structural identifiers, tenant and event root/count
are checked before invoking the signer. Projections are derived internally.
Event producer and commit signer are distinct roles and may have different
identities. The injected signer may use a remote KMS; signing produces no ledger
claim or receipt. Outbox integrity, scope and limits are checked by shared
persistence validation before ledger operations, not before signing.

Call `plan.commit(ledger, authorizer, verifier).await`. No replacement-event or
principal argument exists. The shared advanced persistence implementation
reauthorizes, independently verifies signer trust and validates complete effects
before entering the transaction. Successful results require verified retained
record readback. The ledger remains responsible for head CAS, expected state,
canonical history and atomic claim/event/commit/projection/outbox persistence.
Host-supplied roots and history context do not become authoritative by signing.

Retain the plan across uncertain results. `DurableError::Unknown` requires
original-identity lookup through `mutation::recover`; never manufacture a new
intent or plan to resolve uncertainty. Verified replay returns the original
committed outcome. A point-in-time absent lookup alone does not grant retry
permission. Dispatch belongs after authoritative receipt verification.

This is the protected single-intent composition. The separate
[batch path](durable-batches.md) supplies complete-group signing and recovery, and
[read-only preparation](mutation-preparation.md) shares internal evaluation.
See the [qualification summary](qualification.md) for tested scope and limits. Preparation-time state and authorization facts are not
forever-fresh snapshots. See [preparation](mutation-preparation.md) and
[durable outcomes](durable-outcomes.md) for authority and adapter limitations.

The public-API integration tests in `crates/statechronicle/tests/durable_recovery.rs`
exercise event substitution and scope rejection before signing, separate producer
and signer roles, successful commit/replay/recovery, lost ACK with retained exact
identity, and policy/signer/outbox refusal before ledger access. The external
candidate consumer includes these same scenarios through its public umbrella
dependency. The reference in-memory adapter does not qualify disk crash safety.
