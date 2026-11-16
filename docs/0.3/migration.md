# Migration from the original release

This guide maps the original `v0.2.0` at
`c418e98b91720188158c531cd5c2c4b8ab71cbcf` to the refactoring branch. It does not
announce a published `0.3.0` or a completed release campaign. Workspace versions
remain unchanged pending final qualification. Original protocol fixtures under
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
was compiled. See [API inventory](api-inventory.md) and exact
[compatibility evidence](evidence/compat/) for scope.

A concrete dependency source boundary requires attention: the original checkout
used Ed25519 major version 3; this branch uses major version 2. Types from those
versions are distinct Rust types. Downstream code passing version-3 key objects
must align its dependency with this branch or convert through validated key bytes
at its own trust boundary. The frozen consumer permits `>=2, <4` so it can build
both revisions; that does not make version-3-only callers source compatible.
See the inventory and lockfiles for provenance and original-build limitations.

Original intent/event/commit/proof encodings, enum indices, signing bytes,
cryptographic roots and old schema meaning are intended to remain unchanged.
Frozen wire/proof records and external portable comparisons provide retained
checkpoint evidence; the final exact-candidate qualification remains open.
New receipts, plans, group retention and expectations do not replace historical
wire records with a new implicit format. Adapter-retained batch metadata is a
separate optional contract.

Register schema versions explicitly. Keep old decoders/reducers for historical
payloads; introduce an independently authorized migration event/resource policy
for changed semantics. Never preserve a schema number while interpreting old
bytes with a new model. Rebuild from independently pinned verified history,
retain scoped checkpoints, compare final roots and promote only validated
projections. Storage schema migration/backup/restore is the adapter owner's job.

MSRV remains Rust 1.85 and the supported portable path remains allocation-free
and `no_std`. The library manifests currently declare no package-defined feature
matrix. Do not infer that hosted types or async storage are available on bare
metal; the external portable consumer qualifies that separate boundary.
