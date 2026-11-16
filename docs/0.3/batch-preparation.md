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

This is a preparation stage, not a durable batch coordinator. Legs observe the
existing state index; there is no sequential overlay of earlier leg results.
A later coordinator must reject inconsistent observations at the authoritative
ledger boundary. Mixed tenants are currently refused.

## Remaining durable group boundary

The existing `LedgerMutation` recovery record binds one intent. Its verifier
checks all events against that one intent; it cannot establish a multi-intent
group. Looping over single-mutation commits would allow partial persistence.
Protected durable batches still require an atomic group membership record,
authoritative lookup of the entire original group, signer and payload binding,
head conflict checks, and ambiguous-acknowledgement recovery. This preparation
API does not close that roadmap requirement.

## Verification

The original independent two-mint batch event hash was produced by the frozen
original implementation and retained in
`evidence/f1/batch-preparation/original-characterization.rs`. Candidate tests
compare that hash, exercise corrupted cached fields, mutated caller inputs,
late signature/transition failures, authorization ordering, duplicate and mixed
scope refusal, missing signatures, size bounds, duplicate event IDs, and a
store that panics on either access. A candidate-only external consumer uses the
public facade to prepare and inspect both legs. None of these tests qualifies
physical database crashes or durable group atomicity.
