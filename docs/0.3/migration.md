# Migration from the original release

This guide maps the original `v0.2.0` at
`c418e98b91720188158c531cd5c2c4b8ab71cbcf` to the refactoring branch. It does not
announce a published `0.3.0`. Library qualification is recorded in the
[qualification summary](qualification.md); workspace versions remain unchanged
pending versioning and publication. Original protocol fixtures under
`ci/wire-0.2` and original consumer sources under `ci/compat-0.2` are frozen.

## Adopt by workflow

| Existing use | Adoption on this branch | Compatibility boundary |
|---|---|---|
| `Executor`, `Ports`, builders and `execute*` | Retain specialized compatibility use; use `mutation::Preparer` for new protected preparation | Old marker/sink behavior remains; empty events cannot establish a durable receipt |
| `Executor::prepare_player` | Prefer read-only `Preparer::prepare_player` with `PreparationPorts`/`PreparationConfig` | Both delegate to shared evaluation; old methods stay available |
| Separate `IntentStore` dedup and event persistence | One qualified `LedgerStore` transaction plus `LedgerRecovery` | Planning markers cannot migrate into accepted durable records without independently verified canonical evidence |
| `persist_durable_verified` / raw `DurableCommitRequest` | Protected preparation → `SignedMutation::sign` → `commit` | Advanced functions remain; they do not themselves authenticate ingress or recompute reduction |
| Player batch sink | `Preparer::prepare_player_batch` → `SignedBatch` | Single tenant only; optional complete-group retention and recovery required |
| `ProofService::verify*` returning `Result<()>` | `ClaimVerifier` + independent `ExpectedClaim`; explicitly current or historical | Old helper semantics stay; old absence verification derives its key from evidence |
| Existing `CommitStore` | Keep implementation; automatic `CommitReader` bridge | New direct read-only implementations use `CommitReader` instead of dummy writes |
| Event-only projection rebuild | Verified signed-history source with independent `HistoryPin` | Old functions still require caller-verified canonical events and correctly scoped checkpoints |
| Native `StateProfile` / custom `ProfileRules` | Keep implementations; opt into hosted decode-once/command mapping | Default evaluation forwarding preserves older implementations; no mandatory `Clone` on native state/command |

Before/after import choices (these resolve to public umbrella exports):

```rust
// Existing composition remains available.
use statechronicle::{Executor, Ports, ProofService};
// New protected and minimal read-only composition.
use statechronicle::mutation::{
    Preparer, PreparationConfig, PreparationPorts, SignedMutation,
    DurableOutcome, DurableError, recover,
};
use statechronicle::proof::claim::{ClaimVerifier, ClaimProof, ExpectedClaim};
use statechronicle::ports::commit_store::CommitReader;
```

Do not mechanically rename `execute` to `commit`. Authentication, signing,
authoritative transaction support and retained readback are real added contracts.
The [standalone tutorials](consumer-guide.md#working-applications) contain full
compiling compositions; their development policy/keys are explicit fixtures.
No deprecation removal date is declared by this guide.

## Results and retry behavior

A prepared result owns authenticated request facts and events but grants no
persistence authority. Signing binds those events; it still does not commit.
`DurableOutcome::Committed` and `Replay` contain the checked original record.
`DurableError::Unknown` preserves the original identity and typed storage cause.
Every final-commit error is unknown, including a backend-reported conflict.
A later refusal or unavailable policy does not settle an earlier unknown write.

Persist the original intent/body digest and recovery context before submitting a
write. Resolve uncertainty with the original principal under current access
policy and independent signer verification. Neither `NotFound` at one instant
nor lease expiry proves an outstanding transaction cannot commit. Follow the
[operator decisions](../OPERATIONS/RECOVERY.md#ambiguous-mutation-decisions).

## Source and protocol compatibility

Existing umbrella paths, executor builders, legacy verification helpers and
persistence signatures are retained and exercised by the frozen external
consumer matrix. Candidate-only overlays exercise additive APIs separately.
This is selected downstream coverage, not a claim that every possible consumer
was compiled. See [API inventory](release-compatibility.md) and exact
[compatibility evidence](https://github.com/STEXS-Technologies/statechronicle/tree/2827f0fe9c3f3945aae2f1a46ffb79670bbd06f7/docs/0.3/evidence/compat) for scope.

The original checkout and the current branch use Ed25519 Dalek major version 3.
Earlier refactor checkpoints used major version 2; those remain exact historical
evidence. Downstream consumers of those checkpoints must align their Dalek
major version with the current public key types. Types from different major
versions are distinct Rust types; byte conversion belongs at a validated trust
boundary. The current dependency refresh also adopts Base64 0.23 and SHA-2 0.11.
Independent frozen signing/root/proof vectors remain the compatibility oracle.

Original intent/event/commit/proof encodings, enum indices, signing bytes,
cryptographic roots and old schema meaning are intended to remain unchanged.
Frozen wire/proof records and external portable comparisons provide retained
checkpoint evidence; the completed source qualification is summarized in [qualification](qualification.md).
New receipts, plans, group retention and expectations do not replace historical
wire records with a new implicit format. Adapter-retained batch metadata is a
separate optional contract.

Register schema versions explicitly. Keep old decoders/reducers for historical
payloads; introduce an independently authorized migration event/resource policy
for changed semantics. Never preserve a schema number while interpreting old
bytes with a new model. Rebuild from independently pinned verified history,
retain scoped checkpoints, compare final roots and promote only validated
projections. Storage schema migration/backup/restore is the adapter owner's job.

The minimum supported Rust version is 1.88, raised from the original 1.85
baseline by current dependency requirements. The development toolchain remains
pinned to 1.100; it does not require applications to use that compiler. See
[Rust support](rust-support.md) for the minimum-version checks.
The supported portable path remains allocation-free
and `no_std`. The library manifests currently declare no package-defined feature
matrix. Do not infer that hosted types or async storage are available on bare
metal; the external portable consumer qualifies that separate boundary.

## Settlement route tightening

Resource-profile `trade.settle` intents declaring bundle metadata now require a
settlement coordinator. Previously, ordinary single/batch routes could accept
an incomplete bundle without checking its declared cardinality. Use legacy
`execute_settle` for its existing specialized workflow, or
`Preparer::prepare_player_settlement` followed by `SignedBatch` for protected
canonical composition. Ordinary value-leg routing retains its existing refusal
precedence. This is an intentional validation tightening; wire encodings and
native custom-domain execution policies are unchanged.

## Authorizing a schema migration

A verified historical source establishes signed integrity under an independently
selected historical signer policy and tip pin. It does not grant permission to
migrate. A host migration policy must separately bind the authenticated service,
operation, target and native command to the expected source resource/state and
verified-history identity. Include those references in the signed intent;
construct the authorization expectation from trusted source evidence before
considering client input. Retain the old decoder and migrate into a new versioned
resource when an in-place schema change would reinterpret old bytes.

The [authorized migration checkpoint](https://github.com/STEXS-Technologies/statechronicle/blob/2827f0fe9c3f3945aae2f1a46ffb79670bbd06f7/docs/0.3/evidence/f3/authorized-migration-7489a5f/README.md)
provides an executable public-API example using original-release BCS history,
read-only `Preparer`, `SignedMutation`, current authorization on commit/recovery,
and source-bound interrupted rebuild. Its grant authorizes a pinned historical
snapshot; live source freshness would require an additional atomically enforced
read dependency. Fixed fixture keys, session strings and in-memory stores are
not deployment authentication or durability policies. Applications must inject
unique event identities after restoring old history.


## Projection rebuild addresses and event identities

Rebuild continuity now uses tenant/resource and, for subject-held state, its
holder. Owner-based resource addresses exclude their mutable owner. Event-ID
deduplication is tenant-scoped, matching canonical identity: equal event-ID text
in different tenants must not collide. These changes correct rejected/merged
independent histories; they do not change old BCS bytes or signed roots.
Same-tenant duplicate events and broken continuity for the same full address
still fail before projection writes.

A projection adapter must retain the holder distinction carried by subject-held
state; a map keyed only by tenant/resource is insufficient for balances or other
independent holder records. The included resource-only benchmark sinks use
owner-based/custom resource fixtures and do not qualify such an adapter for
subject-held state. Rebuild tests separately exercise independent holders,
wrong-holder continuity and tenant-scoped duplicate identity. Existing old
history vectors continue to verify; schema reinterpretation remains forbidden.
