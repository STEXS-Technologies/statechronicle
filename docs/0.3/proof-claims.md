# Independent proof claims

The new `ProofService::verify_current` and `verify_historical` methods require
an `ExpectedClaim` created from the application's request context. Do not copy
those expected identities from the supplied proof. Constructors derive the
state key from the independently supplied tenant, resource and optional holder,
so a requested resource cannot accidentally be paired with another resource's
key. Signer trust is separate: obtain the verifying key from policy for that
expected tenant and the requested historical/current trust mode.

```rust
use statechronicle::{ResourceId, SubjectId, TenantId};
use statechronicle::proof::claim::{ClaimProof, ExpectedClaim};

let expected = ExpectedClaim::ownership(
    TenantId(String::from("acme.game.alpha")),
    ResourceId(String::from("asset:wire")),
    SubjectId(String::from("account:bob")),
);
// `service`, `proof`, and `trusted_key` come from the application's adapters,
// supplied evidence, and independent signer policy, respectively.
// let verified = service.verify_current(
//     ClaimProof::State(&proof), &expected, &trusted_key,
// ).await?;
```

Use `ExpectedClaim::state` for present resource-keyed state, `ownership` to
add an expected owner, and `subject_state` for a particular holder's state.
Use `absence` or `subject_absence` with `ClaimProof::Absence` for absence at
those addresses. Present and absent evidence cannot satisfy each other's
expectations. Absence does not assert an owner, and resource-keyed state does
not assert a subject-held address.

Current verification checks the expected identities and key, loads the signed
commit under the expected tenant, checks its reference/signature/root and any
owner assertion, and finally reads the authoritative canonical head once.
The complete head tuple must match. Missing head support, unavailable stores,
and a different head fail closed. The adapter must provide immutable commit
lookups and a linearizable tenant-head read, atomically advanced with the
canonical ledger transaction. A stale cache does not satisfy this contract.

A `VerifiedClaim` has private fields and no deserializing constructor. Its
`expectation()` identifies precisely what was established. Its `commit()`
comes from the verified stored signed commit, including the checked signature;
untrusted reference metadata is not promoted into the result. `point()` is
`VerificationPoint::Current` or `Historical`. A current result is pinned to the
head observed at that final read. It cannot promise freshness after the read,
and keeping or cloning the result does not keep it current.

Historical verification additionally requires the application's independently
chosen `CommitId` pin. It checks the same claim and cryptographic evidence but
does not read the current head or imply freshness. This permits an old absence
proof to remain valid historically after a resource has been created, while
rejecting that proof as evidence of current absence.

The executable external consumer is
[`proof_claims.rs`](../../ci/canonical-0.3/proof_claims.rs). The compatibility
runner overlays these candidate-only tests into its independently built public
consumer; it never adds them to the original-release lane. Frozen original
history/proof bytes supply the evidence, while expected identities and signer
keys are independently fixed in the consumer.

Existing `verify`, `verify_with_key`, `verify_canonical`,
`verify_with_key_canonical`, and `verify_non_membership` retain their compatibility
semantics. Their `Result<()>` does not carry this new verified-claim contract.
In particular, old absence verification derives its key from the bundle, and
old canonical helpers do not require a separate caller expectation.

These claims establish signed state or absence at a particular root. They do
not authenticate unsigned event-operation metadata, previous-owner fields,
trade summaries, physical observations, or storage durability. Custom-domain
payload validity and authorization remain separate policy responsibilities.
