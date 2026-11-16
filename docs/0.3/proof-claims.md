# Independent proof claims

`ClaimVerifier::verify_current` and `verify_historical` require
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
Its `state()` accessor returns the verified, owned payload for present claims
and `None` for absence. Read application data from this result: later mutation
of the original proof cannot change the verified payload. Ownership assertions
use the protocol state's native owner field; opaque custom-domain owner policy
requires its domain verifier.

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


## Minimum read-only composition

Use `statechronicle::proof::claim::ClaimVerifier::new(Box::new(reader))` when
verifying supplied proof bundles. Implement only
`statechronicle::ports::commit_store::CommitReader`: immutable tenant-scoped
`read_commit` and, for current claims, linearizable `read_canonical_head`.
There is no required proof index, projection index, snapshot store, write method
or sequence lookup. A historical-only reader can omit the head method; its
default `None` always refuses current verification.

`CommitReadFuture` is the object-safe boxed-future alias used by these two
methods. An ordinary implementation returns `Box::pin(async move { ... })`.
An existing `CommitStore` adapter automatically implements `CommitReader`,
forwarding its original boxed futures directly. It need not change or implement
another trait. The bridge adds no extra future allocation, but this is a source
ownership fact, not a measured latency or whole-path allocation guarantee.
A custom read-only adapter should implement `CommitReader` directly and never
supply dummy write methods. Read adapters remain host code and must obey the
no-persistence/no-dispatch contract; the library cannot sandbox their internals.

`ProofService::verify_current` and `verify_historical` remain source-compatible
and delegate to the same private expected-claim verification implementation.
Canonical-head tuple comparison and commit-error/absence normalization also
have one owner, shared by compatibility paths. Legacy lookup and head helpers
forward futures directly rather than adding an async layer. Proof generation,
state queries and snapshots stay on the existing broader service; those
capabilities are distinct from verification of supplied evidence.

The new external consumer `ci/canonical-0.3/claims_readonly.rs` compiles a reader
with no write/index/snapshot implementations against the public umbrella crate.
It verifies frozen original-release state/absence proofs and checks read order,
stale/missing/failing heads, wrong pin/expectation/signer and unavailable commit
reads. A history-only adapter proves the default head behavior. The separate
compatibility bridge test compares results/errors and head-read counts across
sixteen expectation/head combinations, including a head advance during lookup,
and checks that historical verification performs no head read.

This closes the minimum claim-verifier composition obstacle. It does not
complete coherent proof serving, the durable custom-ledger tutorial, full
history/current authorization policy or final performance/release qualification.
