# Authenticated mutation preparation

`statechronicle::mutation::Preparer::prepare_player` accepts a signed `ValidatedIntent`, an independently
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
The protected signed-plan and receipt/lookup path is described in
[signed mutation](signed-mutation.md); preparation alone does not complete the
durability requirements.

Legacy `execute*` signatures and marker/error ordering remain compatible. In
particular, the old sink lane can return empty events from a planning marker;
that behavior is retained and tested, and is not evidence of durable success.

No protocol wire representation, dependency or portable feature changes here.
The owned plan and extra structural/digest checks have a cost not measured by
the legacy execution benchmark. Benchmark the new path separately before final
release qualification; do not infer equal-work performance from old wrappers.

## Minimal read-capability composition

Use `statechronicle::mutation::{Preparer, PreparationPorts, PreparationConfig}`
for protected preparation. `PreparationPorts` requires only `StateIndex`,
`TenantStore` and the configured delegated-authority evaluators. It has no
`IntentStore`, `TransactionManager`, ledger writer, outbox writer or dispatch
sink. `PreparationConfig` names required profile registry, executor identity,
clock, event-id generator and trusted intent-signature verifier. All are
required fields; no permissive default verifier/authorizer is installed.
Each call still requires an independently authenticated principal, authorizer
and key registry. Read adapters and callbacks can have arbitrary host behavior;
the type surface limits supplied capabilities, not the internals of host code.

`Preparer::prepare_player_batch` returns one owned `PreparedBatch`, applying
all ingress checks before state reads, then all transition/group checks. Single
preparation retains key-before-principal/policy order; batch retains
principal/policy-before-key order. Both reconstruct public request fields.
Failed batches return no successful subset, but callbacks for earlier evaluated
legs may already have advanced. A successful plan is not a replay receipt.

The compatibility `Executor` builder and public method signatures are retained.
Its protected preparation methods and `Preparer` delegate to the same private
`pipeline::evaluation::Evaluation` implementation. That borrowed context holds
only read ports and configuration. Legacy marker handling receives an explicit
intent-store reference through a private enum; the preparation lane supplies no
marker capability. Transaction coordination remains outside evaluation.
The unchanged legacy builder validation order still reports missing ports,
executor, clock, event-id generator and verifier in that order.

A self-contained public consumer is exercised in
`crates/statechronicle/tests/readonly_preparation.rs` and
`tests/support/readonly.rs`, also compiled through the isolated candidate
consumer. It implements read, key-policy and authorization ports, with no
intent-store or transaction-manager implementation. It uses fixed fixture keys
and an empty-state read adapter for unique-asset mint; it is a test fixture,
not a durable storage tutorial or production key-management implementation.
`preparation_parity.rs` separately compares the new and compatibility surfaces.

This is the minimum read-only preparation composition. The [native](../../tutorials/minimal-native-domain/README.md) and
[durable](../../tutorials/durable-custom-ledger/README.md) standalone tutorials
now demonstrate the consumer journey. No performance equivalence or
whole-workspace source reduction is inferred from moving evaluation code.
