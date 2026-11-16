# Consumer workflow map

These are APIs on the refactoring branch. Package versions still read `0.2.0`;
release qualification and publishing are separate work. Add the umbrella crate
from this checkout as shown in the standalone tutorials; do not assume these
additive APIs exist in the released registry package.

| Goal | Recommended surface | Result and authority |
|---|---|---|
| Define legal native transitions | `StateProfile` | Application-owned validation and deterministic reduction; no authentication/storage |
| Register hosted domain/schema and encode commands | `TypedProfile`, static `ProfileRegistry`, `command_operations!` | Canonical hosted adaptation and command-operation binding |
| Authenticate and prepare one request | `mutation::Preparer::prepare_player` | Private-field `PreparedMutation`; uncommitted |
| Prepare a single-tenant group | `Preparer::prepare_player_batch` | Complete `PreparedBatch`; no successful subset |
| Sign and commit one plan | `mutation::SignedMutation::sign`, then `commit` | `DurableOutcome::Committed` or `Replay` with checked original receipt |
| Resolve uncertain single write | `mutation::recover` | Original record checked; `NotFound` and `InProgress` are not receipts |
| Sign/commit/recover a group | `mutation::SignedBatch` | Complete retained group; requires optional batch-capable ledger/recovery |
| Verify supplied present/absent evidence | `proof::claim::ClaimVerifier` | Independent `ExpectedClaim`, trusted key; private-field `VerifiedClaim` |
| Assert current state | `ClaimVerifier::verify_current` | Complete authoritative head tuple at final linearizable read |
| Assert state at a chosen commit | `ClaimVerifier::verify_historical` | Independent commit pin; no current-head freshness |
| Rebuild from signed provider history | `index::rebuild::verified::verify_history` | Independent `HistoryPin` and historical signer policy before resumable rebuild |
| Publish committed effects | `ports::outbox::dispatch_once` and delivery adapter | Leased post-commit delivery; consumers deduplicate, no exactly-once promise |

## Composition and ownership

```text
StateProfile ── native validation/reduction
      │
TypedProfile + ProfileRegistry ── codec, domain/schema/operation routing
      │
Preparer ── trusted principal/key/policy + read ports → uncommitted plan
      │
SignedMutation / SignedBatch ── signer + current policy + verified persistence
      │
LedgerTransaction ── atomic claim/events/commit/head/projections/outbox
      │
checked receipt / recovery       ClaimVerifier ← independent expectation/key/pin
```

`PreparationPorts` contains only `StateIndex`, `TenantStore` and configured
delegated evaluators. `PreparationConfig` supplies registry, executor, clock,
event IDs and intent verifier; each call receives independent principal, policy
and key registry. Callbacks/readers remain trusted host code.

The ledger owns storage atomicity, canonical head/state compare-and-swap, fenced
idempotency and immutable retained original effects. `LedgerRecovery` reads that
same authority. A second intent store or separate append-only event writer does
not supply this transaction. Batch support additionally requires
`LedgerTransaction::retain_batch` and `LedgerBatchRecovery`; its default refuses.
The file tutorial deliberately supports single mutations only.

Supplied proof verification needs only `CommitReader`. Existing `CommitStore`
adapters bridge automatically; direct readers need no dummy write methods.
Historical-only readers can omit head support, whose default refuses current
verification. Proof generation and query services still use broader existing
ports; they are separate from verification of supplied evidence.

The host owns authentication, current and historical key authorization, clock,
IDs, topology, backups, migration and delivery. Commit signing checks bytes;
trusted key selection and revocation/time policy remain independent decisions.
A `VerifiedClaim` is a statement at its pinned verification point, not a durable
receipt, current authorization capability or authenticated physical observation.

## Choose between similar concepts

| Concepts | Distinction |
|---|---|
| Profile / registry | Profile owns semantics; registry selects the correct domain/schema/routing policy |
| Plan / receipt | Plan may become stale and has no durable effects; receipt checks original canonical mutation |
| Current / historical | Current checks a live authoritative head; historical verifies an independently chosen commit |
| `TransactionManager` / `LedgerTransaction` | Legacy lifecycle cannot atomically enlist the independent stores; ledger transaction owns full durable mutation |
| Outbox / publisher | Outbox is atomically retained intent to deliver; publisher is a subsequent external effect |
| Index / history | Projections are derived; canonical signed history and head select authoritative records |
| Signature / trust | Valid bytes under a key do not establish that key's authority for tenant/time/purpose |

## Working applications

Start with [native preparation](../../tutorials/minimal-native-domain/README.md),
then [durable file ledger](../../tutorials/durable-custom-ledger/README.md).
The latter covers both models, reopen, unknown ACK, unchanged exact replay,
native history recomputation and temporal proof expectations. It uses a bounded
single-writer file contract, not production key management or crash qualification.

For specialized single-tenant batches see [protected batches](durable-batches.md).
Cross-tenant trade/settlement paths remain advanced explicit composition: retain
their existing domain-specific rules and demand one supported atomic backend.
Do not split an atomic group into successful independent single commits or infer
that `SignedBatch` supports cross-tenant groups.

See [migration](migration.md), [adapter qualification](adapter-contract.md),
[recovery decisions](../OPERATIONS/RECOVERY.md), [proof claims](proof-claims.md)
and [verified rebuild](verified-rebuild.md) for the detailed contracts.
