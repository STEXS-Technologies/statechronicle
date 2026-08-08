# statechronicle-domain

## What it is

The canonical protocol objects: tenant and resource identity, subjects, state
types, intents, events, commits, proofs, and state projections. It is
infrastructure-agnostic: no transport, no persistence, no framework. These
types are what every other crate serializes, signs, executes, and verifies.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §5 | Conceptual Model | The append-only, content-addressed history model |
| §6 | Resource Model | What a resource is and how identity is scoped |
| §7 | Subject Model | Actors: users, accounts, services, authorities |
| §8 | Tenant Isolation Model | Tenant scopes and hard/logical isolation modes |
| §9 | State Model | Versioned state projections over event history |
| §10 | Resource State Types | Unique asset, consumable stack, fungible balance, entitlement, meter, listing, escrow |
| §11 | Intent Model | Requested transitions and their required fields |
| §12 | Event Model | Validated, append-only transitions and their fields |
| §13 | Commit Model | Signed batches of events and their fields |
| §16 | Proof Model | Proof types referenced by bundles (types live here) |
| §31 | Forks and Recovery | Fork evidence carried by commit chains |

## Key types

- `tenant::TenantId`, `resource::ResourceId`, `subject::SubjectId`,
  `state_type::StateType`.
- `intent::{Intent, Operation, SignatureBlock, SignatureAlg, KeyId, Nonce}`.
- `event::{Event, StateCommitment}`.
- `commit::{Commit, CommitBody, CommitScope, ProfileId, ScopeKind}`.
- `proof::{ResourceStateProof, SparseMerkleProof, NonMembershipProofBundle}`.
- `authority::{AuthorityProof, TrustGrantOutcome, EvaluationResult, AggregationPolicy}`.
- `signed::Signed`: the ADR-004 body + detached signature envelope.
- `state::StateProjection`, `ids::*` (prefixed newtype IDs).

## How it's used

Domain types are assembled by parsing (`statechronicle-intent`), executed
(`statechronicle-executor`), committed (`statechronicle-commit`), and served in
proof bundles (`statechronicle-proof`). They are the language of the protocol
wire format.

```rust
use statechronicle_domain::tenant::TenantId;
use statechronicle_domain::resource::ResourceId;

let tenant = TenantId(String::from("acme.game.alpha"));
let resource = ResourceId(String::from("asset:sword_001"));
```

## Dependencies

`statechronicle-core`, `chrono`, `serde`, `serde_json`, `bcs`. Dev-only:
`proptest`.

## Tests

`tests/`: `envelope.rs` (signed-envelope round trips), `property.rs`, plus
inline unit tests for object invariants, ID validation, and authority digest
aggregation.

## Where it fits

A pure middle layer above `statechronicle-core`. The executor, commit,
accumulator, proof, intent, and profiles crates all build on it. The umbrella
crate re-exports its most-used types both namespaced and at the top level.
