# Consumer-defined domains in 0.2.0

StateChronicle accepts native consumer state and command types while sharing its
version checks, authenticated execution, event commitments, signed commit chain,
atomic persistence ports, replay, state roots, and inclusion/non-membership proofs.
Configuration and equipment profiles in
[`examples/common/domains.rs`](../../crates/statechronicle/examples/common/domains.rs)
exercise independent non-economic lifecycles without owner, balance or quantity fields.

## Extension contract

Implement `statechronicle_profile::StateProfile`:

- `State`, `Command`, and `Error` are consumer-owned types.
- `DOMAIN_ID` is a stable namespaced identity; `SCHEMA_VERSION` starts at one.
- `MAX_PAYLOAD_BYTES` caps canonical state and command bytes. The standard adapter
  also applies the existing intent limit, whichever is smaller.
- `operation(command)` binds the command discriminator to its signed operation.
- `validate`, `apply`, and `validate_state` define legal transitions and invariants.

Reducers and serializers must be deterministic functions of their arguments.
They must not read clocks or randomness, perform I/O, or mutate global state.
The host supplies observed inputs as commands. State validation runs on both
loaded states and reduced outputs. BCS roundtrip checks reject noncanonical
encodings. BCS excludes floats; use integers or exact representations for data
that needs fractional precision. Consumer serializer implementations remain
part of the trusted profile implementation.

Use `TypedProfile::new(profile, operations)` to connect a native profile to the
standard pipeline. State and command types implement Serde `Serialize` and
`DeserializeOwned`; plain structs and discriminated enums are recommended.
Each public operation must have a matching command discriminator. Register
adapters with `ProfileRegistry::with_custom(&STATIC_PROFILES)` and pass the
registry to `Executor::builder().profiles(registry)`.

The registry borrows a fixed slice. Duplicate `(domain, schema)` registrations,
empty operation sets, zero schema versions, invalid payload limits and identities,
baseline operation prefixes, and operations shared by different domains fail
registration. Different schemas of the same domain may retain operation names.
Lookup performs no registry mutation.
Static `LazyLock` examples allocate operation strings once during composition;
registration is complete before requests are accepted.

## Requests, authority, and persistence

Use `state_type: "custom"` and supply these signed intent inputs:

```json
{
  "profile": "example.configuration",
  "schema_version": 1,
  "command": [0, 1]
}
```

The command is the byte array returned by `canonicalize(&command)`; `[0, 1]`
encodes `ConfigurationCommand::Create { enabled: true }` in the example.
Operations remain ordinary `Operation` values, such as `configuration.create`.
Unknown domain/schema pairs and command/operation substitutions fail closed.

`TypedProfile` requires authority bindings by default and uses the existing
require-all aggregation policy. Consumers can use `with_optional_authority()`
when their authenticated ingress authorizer supplies the required policy.
For another aggregation policy or operation-specific requirements, wrap the
adapter in a consumer `ProfileRules` implementation and delegate validation and
reduction. Authentication, authority evaluation, key resolution, and quotas use
existing ports. Profiles do not silently trust command fields as identities.

Production consumers use the existing authenticated durable ingress and
`persist_durable_verified` with one `LedgerTransaction`. The adapter must
atomically claim the intent, append events and the signed commit, write
projections and outbox rows, and finalize idempotency. A profile error produces
no event; a durable write failure rolls back the complete mutation. The legacy
`execute` entry point computes events; it alone is not durable execution.
No database or transport implementation is added to StateChronicle.

Custom states are resource-keyed by `(tenant, resource)`, using the existing
storage and sparse-Merkle key contracts. A resource remains bound to one domain
and schema for its lifetime: a different profile cannot reinterpret it. Give
independent resources different IDs, including domain-qualified IDs when domains
share a tenant. Baseline economic transfer/batch routing retains its existing
semantics; custom commands mutate one resource per intent.

## Commitments, replay, and proof verification

`ResourceState::Custom(CustomState)` carries a validated `ProfileId`, a `u32`
schema version, and canonical BCS state bytes. The full tagged envelope is hashed,
so identical native bytes in different domains or schemas have different
commitments. For creation, event version zero commits the same domain/schema
with an empty payload as the unborn state. It is never decoded as live state.

`TypedProfile::verify_transition(current, intent, event)` recomputes reduction
and checks scope, identity, versions, and both state commitments. Consumers must
also verify the accepted intent, historical authority, event/commit inclusion,
signatures and chain continuity. This helper checks domain semantics; it does
not reevaluate an external authority service during replay.

Use the existing state-proof builders, then
`proof::custom::verify_custom_bundle` to authenticate the signed commit and
inclusion before checking the expected domain/schema and decoding typed state.
Generic inclusion verifies integrity; domain-aware decoding verifies the expected
interpretation and state invariants. Signer trust and canonical-head freshness
remain consumer policies. A state proof establishes a committed value under
those assumptions; externally observed data remains the consumer's responsibility.

## Compatibility and migration

The seven baseline `StateType` and `ResourceState` variants keep their BCS
indices, fields, and encodings. `Custom` is appended at index seven. Existing
intent, event, commit and proof envelope schemas, SHA-256 digests, Ed25519
signatures, and sparse-Merkle node/key encodings remain unchanged. Existing
resource commits, roots and proofs need no re-encoding or re-signing.

This is a Rust source compatibility change for exhaustive enum matches: add a
`Custom` arm. Older readers reject new custom variants and must upgrade before
reading custom histories. Registry `get(Custom)` returns `None`; custom requests
resolve through `resolve(state_type, inputs)` using their explicit identity and
schema. Baseline reducers now live in the profiles crate and are invoked through
`ProfileRules::apply`; executor transition functions remain compatibility wrappers.

Changing native BCS field order, command discriminants, validation semantics,
or serialization behavior requires a new schema version. Register old versions
for replay and verification. In-place reinterpretation or automatic migration
of an existing resource is rejected; consumers should create a new versioned
resource and record the migration explicitly. The release does not invent a
universal migration policy for consumer schemas.

## Bounded-memory and no_std path

`statechronicle-profile` is dependency-free, allocation-free, and `#![no_std]`.
Native profiles can use fixed-size structs, enums, arrays and consumer-provided
fixed-capacity containers. The contract compiles for
`riscv32imafc-unknown-none-elf` without an allocator.

`statechronicle-portable` supplies actual allocation-free transition execution,
BCS primitive codecs over caller-owned buffers, custom-envelope hashes, resource
keys, strict Ed25519 verification and borrowed sparse-Merkle proof verification.
It compiles for the same bare-metal target. Its transition API checks domain,
schema, operation binding, versions, canonical roundtrips and state invariants.
Standard/portable interoperability tests preserve the existing protocol bytes,
hashes, signatures and roots. Consumers implement bounded native codecs rather
than depending on a general allocating serializer.

The standard adapter, BCS crate, executor, storage ports and proof services
still require `std`. The adapter bounds input/output byte buffers and BCS
nesting; consumer reducers and Deserialize implementations must also enforce
native collection bounds. A byte cap alone does not bound reducer allocations.
The complete engine is not advertised as allocation-free or no_std.

For a bounded deployment, reuse the portable profile contract and fixed native
types, statically compose profiles, enforce command/state and execution budgets,
and use the portable transition/codec/integrity APIs with caller-owned buffers.
The consumer supplies authentication, trust and durable transaction facilities
as appropriate to its environment. The complete standard runtime is not
advertised as allocation-free or no_std.

## Validation

Profile-owned execution policies supply access gates, state addressing, unborn
commitments, companion transitions and event-group validation. Native profiles
use resource-keyed defaults without ownership or economic lifecycle assumptions.
Baseline resource policies and settlement coordinators retain their existing
semantics through compatibility entry points.

`tests/custom_domains.rs` exercises both independent profiles through signed
intent parsing, authority gates, deterministic reduction/replay, state roots,
signed proofs, domain/schema rejection, version conflicts, idempotency and the
unchanged transactional ledger port, including injected rollback.
Golden BCS/digest/root vectors live in `docs/CONFORMANCE/custom_domains.json`.
`fuzz_custom_state` exercises custom payload decoding and domain/schema isolation.
`fuzz_profile_id` checks validated domain identity constructors and JSON/BCS
deserialization, including invalid Unicode bytes, control characters and bounds.
`fuzz_portable` checks bounded codec boundaries and standard/portable proof
equivalence. `fuzz_custom_transition` compares standard and portable native
configuration/equipment transitions, verifies replay commitments and rejects
tampered versions, domains and authorities. Durable tests inject each transaction-boundary failure, rollback
failure, cancellation and commit response loss; seeded schedules mix outages
and duplicate delivery while checking persisted invariants after each action.
Reordered concurrent histories additionally check commit-head compare-and-swap,
projection versions, exact durable prefixes and recovery to the expected root.
Run `cargo run -p statechronicle --example custom_domains` for a small consumer
composition example.
