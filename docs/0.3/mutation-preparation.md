# Authenticated mutation preparation

`Executor::prepare_player` accepts a signed `ValidatedIntent`, an independently
authenticated principal, an explicit authorizer and a trusted key registry. The
injected intent verifier must resolve the same signature key id to independently
trusted public-key material and verify the canonical intent body. Key metadata
checks do not replace cryptographic verification.

The method rechecks the public request's structure, binds actor and tenant to the
principal, applies the existing single-intent validation and transition rules,
and returns `PreparedMutation`. Its private fields own the accepted request,
principal, canonical body digest and complete event set. The cached idempotency
tuple is derived from that body. Borrowed getters cannot retarget the plan.

Preparation does not read or write the separate intent store, begin a ledger or
symbolic transaction, update projections, or invoke a durable sink. A planning
marker cannot become an empty-event success or suppress signature verification.
The clock, key/policy/state readers and event-id callback remain injected trust
boundaries: they must not perform durable writes or external effect dispatch.
A local event-id counter may advance. Repeated preparation can emit different
event ids and does not establish that any mutation committed.

This is an uncommitted plan, not an authoritative receipt. State and policy can
change after preparation. A durable composition must recheck authorization and
concurrency and atomically bind idempotency, events, signed commit, canonical
head, projections and outbox. Unknown acknowledgements must be resolved against
that ledger using the original intent identity and payload digest before retry.
The canonical receipt/lookup path is still pending; preparation alone does not
complete the roadmap's durability requirement.

Legacy `execute*` signatures and marker/error ordering remain compatible. In
particular, the old sink lane can return empty events from a planning marker;
that behavior is retained and tested, and is not evidence of durable success.

No protocol wire representation, dependency or portable feature changes here.
The owned plan and extra structural/digest checks have a cost not measured by
the legacy execution benchmark. Benchmark the new path separately before final
release qualification; do not infer equal-work performance from old wrappers.
