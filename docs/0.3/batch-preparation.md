# Protected batch preparation

`Executor::prepare_player_batch` returns an owned `PreparedBatch`. It checks
signature presence, principal binding and authorization, key-registry policy,
and structural validity for every leg before executing any leg. It then checks
single-tenant scope and unique intent identities, verifies signatures and
executes the shared preparation pipeline, bounds the combined events, and runs
the same profile group validation as legacy batches.

Preparation never reads or writes the independent intent store, starts a
symbolic transaction, or dispatches a durable sink. Any refusal returns no
successful subset. Injected read services and callbacks must obey the effect
restrictions documented on `prepare_player`; clocks and ID generators may
advance even on refusal. Preparation is not durable replay.

The batch owns its requests, independently authenticated principals, canonical
body digests, and events. Accessors borrow legs and events. Private construction,
no cloning/deserialization, and no owned leg extraction prevent this API from
turning an accepted group into separately committable single plans.

Preparation observes the existing state index; there is no sequential overlay
of earlier leg results. The ledger must reject inconsistent observations at the
authoritative boundary. Mixed tenants are currently refused.

## Durable composition

`mutation::SignedBatch` now binds the complete prepared batch to one signed
commit and derives projections internally. Its `commit` and `recover` methods
require the optional atomic batch retention/recovery contract. See
[durable-batches.md](durable-batches.md) for its transaction, original-group
replay, failure semantics and qualification limits. This does not turn the
prepared value itself into a receipt.

## Verification

The original independent two-mint batch event hash was produced by the frozen
original implementation and retained in
[the archived characterization source](https://github.com/STEXS-Technologies/statechronicle/blob/2827f0fe9c3f3945aae2f1a46ffb79670bbd06f7/docs/0.3/evidence/f1/batch-preparation/original-characterization.rs). Candidate tests
compare that hash, exercise corrupted cached fields, mutated caller inputs,
late signature/transition failures, authorization ordering, duplicate and mixed
scope refusal, missing signatures, size bounds, duplicate event IDs, and a
store that panics on either access. A candidate-only external consumer uses the
public facade to prepare and inspect both legs. None of these tests qualifies
physical database crashes or durable group atomicity.

## Protected settlement preparation

`Preparer::prepare_player_settlement` (also forwarded by `Executor`) returns the
same `PreparedBatch`, with the same per-member authentication and read-only
restrictions. It enables coordinated resource routes and applies the existing
settlement validator to the complete group, including declared payment pairs
and bundle cardinality. Bind its result through `SignedBatch` using a qualified
atomic ledger adapter. Preparation alone does not reserve ownership or money.

Ordinary single and batch preparation refuse declared value legs and bundles;
use the settlement coordinator for those declarations. The protected coordinator
shares settlement validation with legacy `execute_settle`. It does not provide
cross-tenant settlement or a sequential overlay of earlier group effects.
