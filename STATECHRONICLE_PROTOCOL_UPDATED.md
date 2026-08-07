# StateChronicle Protocol

**Verifiable Resource State Protocol**  
**Status:** Draft v0.1  
**Audience:** Protocol designers, infrastructure engineers, game/backend developers, marketplace operators, identity/auth systems, economy/inventory systems  
**Designed to compose with:** Shardline and TrustGrant

---

## 1. Summary

StateChronicle is an infrastructure-agnostic protocol for recording, verifying, and replaying resource state transitions across tenant-isolated resource scopes. It supports unique resources, consumable stacks, fungible balances, entitlements, metered resources, custody records, and other profile-defined state machines.

It provides a canonical answer to:

> What happened to this resource, in what order, under whose authority, and what is its current state?

StateChronicle is designed as the third pillar next to:

| Layer | Protocol | Truth Provided |
|---|---|---|
| Content | Shardline | What the resource bytes/object are |
| Authority | TrustGrant | Who is allowed to act |
| State | StateChronicle | What happened and what state is canonical |

StateChronicle is not a blockchain. It does not require mining, tokens, global consensus, proof-of-work, proof-of-stake, or public-chain settlement. It uses a Git-inspired, append-only, content-addressed history model with strict transaction semantics for ownership, fungible balances, consumables, entitlement, licensing, inventory, custody, and other resource-state systems.

---

## 2. Design Goals

StateChronicle aims to provide:

1. **Verifiable state**  
   Any verifier can determine the current state of a resource from committed events, snapshots, and proofs.

2. **Infrastructure independence**  
   The protocol does not require a specific database, queue, object store, cloud provider, or runtime.

3. **Deterministic transitions**  
   Given the same prior state and the same valid transition, independent implementations must derive the same result.

4. **High-throughput batching**  
   One commit may contain many validated transitions, allowing millions of events per second in optimized deployments.

5. **TrustGrant-native authority checks**  
   StateChronicle does not define who is allowed to act. It consumes TrustGrant evaluations as authority proofs.

6. **Shardline-native content references**  
   StateChronicle does not store large asset payloads. It references content-addressed resources, typically through Shardline.

7. **Replayability and auditability**  
   Current state is a projection of append-only committed events, not an unprovable mutable database row.

8. **Conflict safety**  
   Double-spends, stale transfers, duplicate intents, and conflicting mutations must fail closed.

9. **Portable proof bundles**  
   A resource state proof can be exported and verified by another service without granting it direct database access.

10. **Profile-based specialization**  
   The core protocol is generic. Domains such as games, software packages, licenses, datasets, marketplaces, and physical custody define profiles on top.

11. **Tenant isolation**  
   A deployment can isolate games, studios, marketplaces, organizations, worlds, or customers into independent tenant scopes with separate authority roots, state roots, commit heads, indexes, policies, and storage backends.

12. **Multiple resource state models**  
   The protocol supports unique resources, consumable stacks, fungible balances, entitlements, metered resources, listings, escrow records, and profile-defined custom state types.

13. **Durable paid ownership**  
   Paid unique resources should not be silently deleted, rewritten, confiscated, or invalidated by their creator after transfer. Restrictions must be explicit, append-only, and verifiable.

---

## 3. Non-Goals

StateChronicle does not define:

- A mandatory database engine
- A mandatory network protocol
- A mandatory cloud architecture
- A cryptocurrency
- A token economy
- A global public consensus mechanism
- Payment settlement
- Marketplace pricing
- Royalty distribution
- User-interface metadata
- Large binary storage
- Asset rendering behavior
- Identity proofing
- Permission/delegation semantics independent of TrustGrant
- Permanent game rendering or compatibility guarantees
- Creator-controlled hard deletion of already-committed paid ownership history

StateChronicle can be anchored to public witnesses or external chains later, but such anchoring is optional and outside the core protocol.

---

## 4. Relationship to Shardline and TrustGrant

### 4.1 Shardline

Shardline provides content truth.

It answers:

```text
Is this the exact content-addressed object we expect?
```

StateChronicle should reference large or immutable payloads by digest, manifest ID, or Shardline object reference. It should not embed large binaries directly in state events.

Example:

```json
{
  "content": {
    "kind": "shardline.object",
    "digest": "sha256:9e3b...",
    "media_type": "model/gltf+json",
    "size": 4821930
  }
}
```

### 4.2 TrustGrant

TrustGrant provides authority truth.

It answers:

```text
Is this actor allowed to perform this operation on this resource under this scope?
```

StateChronicle consumes TrustGrant verification/evaluation results as authority proofs before applying state transitions.

StateChronicle must not treat an authenticated actor as authorized merely because the actor is known. A valid state mutation requires both:

1. Identity/authentication of the actor.
2. Authorization for the requested operation, usually via TrustGrant.

### 4.3 StateChronicle

StateChronicle provides state truth.

It answers:

```text
What happened, what is the canonical order, and what state exists now?
```

Together:

```text
Shardline       proves what the resource is.
TrustGrant      proves who may act.
StateChronicle  proves what happened and what state is canonical.
```

---

## 5. Conceptual Model

StateChronicle is based on a small set of concepts:

| Concept | Description |
|---|---|
| Tenant | An isolated scope such as a game, studio, marketplace, organization, world, or customer |
| Resource | Anything whose state can change over time within a tenant scope |
| Subject | A user, account, service, organization, game, device, or authority |
| State type | The state model used by a resource, such as unique asset, stack, balance, entitlement, or meter |
| Intent | A request to mutate resource state |
| Event | A validated state transition derived from an intent |
| Transaction | One or more state transitions that must commit atomically |
| Commit | An ordered batch of one or more events or atomic transactions |
| State | The current projection for a resource or subject-resource pair |
| Snapshot | A compact representation of state at a commit |
| Proof | A verifiable package showing inclusion, state, authority, or history |
| Profile | A domain-specific set of operations and state rules |

---

## 6. Resource Model

A resource is any object, right, entitlement, asset, license, credential, namespace, package, dataset, balance, consumable stack, meter, custody record, or physical/digital item whose state must be tracked.

A resource identifier must be stable within its tenant scope. A globally meaningful resource reference should include both a tenant identifier and a resource identifier.

Recommended URI-like format:

```text
stc://<tenant_id>/<resource_type>/<resource_id>
```

Examples:

```text
stc://stexs.game.alpha/asset/sword_001
stc://stexs.game.alpha/currency/gold
stc://stexs.game.alpha/material/iron_ore
stc://stexs.game.alpha/entitlement/battle_pass_season_7
stc://stexs.marketplace/listing/listing_9281
stc://studio-a/cosmetic/dragon_skin
stc://registry/package/physics-core
stc://datahub/dataset/urban-v1
```

The protocol does not require this exact string form, but every interoperable profile must define:

- How tenant identity is represented.
- How resource identity is represented.
- Whether resource IDs are unique globally, unique per tenant, or unique per resource type.
- Whether a resource is unique, fungible, consumable, metered, entitlement-based, or custom.

For backwards compatibility with systems that already use string identifiers, a profile may represent a resource as:

```text
resource:<tenant>:<type>:<id>
```

Example:

```text
resource:stexs.game.alpha:asset:sword_001
```

---

## 7. Subject Model

A subject is any entity that can own, control, request, or receive state.

Examples:

```text
user:8
account:stexs:player_123
service:inventory.stexs.net
organization:studio-a
wallet:eip155:1:0xabc...
device:console:serial_123
```

StateChronicle does not define identity proofing. Identity is supplied by the deployment and authorization is evaluated through TrustGrant or a compatible authority profile.

---

## 8. Tenant Isolation Model

A tenant is an isolated StateChronicle scope. A tenant may represent a game, studio, marketplace, organization, shard, world, customer, environment, or application.

Tenant isolation is a core protocol concept because the same resource names may have different meanings in different economies or domains.

Example:

```text
tenant_id: stexs.game.alpha
resource_id: currency.gold
```

is distinct from:

```text
tenant_id: stexs.game.beta
resource_id: currency.gold
```

A tenant may define its own:

- Authority roots and TrustGrant policies
- Commit authority
- State accumulator
- Resource namespace
- Profile set
- Retention policy
- Revocation freshness policy
- Proof requirements
- Storage backend
- Throughput partitioning strategy
- Fork recovery policy

### 8.1 Isolation Modes

StateChronicle supports two baseline tenant isolation modes.

#### Hard isolation

Each tenant has a separate chronicle:

```text
Tenant A commit chain -> Tenant A state root
Tenant B commit chain -> Tenant B state root
Tenant C commit chain -> Tenant C state root
```

Hard isolation is recommended when tenants require separate keys, storage, compliance boundaries, disaster recovery, or administrative control.

#### Logical isolation

Multiple tenants may share one physical backend or batch executor, but every object includes `tenant_id` and every tenant has an independently verifiable state root. Per-commit tenant roots are mandatory for every deployment; the global checkpoint root is an optional overlay (ADR-006, §36 Q12).

```text
Global checkpoint root
  ├── tenant_a_state_root
  ├── tenant_b_state_root
  ├── tenant_c_state_root
  └── tenant_d_state_root
```

Logical isolation is useful for high-throughput platforms where millions of tenant-scoped transitions are processed together but verified separately.

### 8.2 Tenant Boundary Rule

A state transition must not read from or mutate another tenant's state unless the active profile explicitly defines a cross-tenant operation and the actor has authority for every affected tenant.

Cross-tenant transactions must include:

- Source tenant ID
- Destination tenant ID
- All affected resource IDs
- All affected expected versions
- Authority proof for each tenant boundary crossed
- Atomic commit or failure semantics

In v0.1, cross-tenant transactions execute atomically: the executor partitions
the affected intents by tenant, begins a multi-tenant transaction
(`begin_multi`), runs each tenant's leg through the single-tenant pipeline, and
commits (or rolls back) all legs together — producing **one tenant-scoped commit
per affected tenant**. Legs are linked by a shared `intent_id` (the cross-tenant
intent linkage); authority is evaluated **per tenant scope**, so the actor needs
authority for every affected tenant. Execution is all-or-nothing: every
affected tenant's leg commits or none do.

> **Execution-time-only durability limitation.** Atomicity is an *execution*
> guarantee, not a cross-tenant recovery guarantee: a verifier of one tenant's
> chain alone cannot detect a missing leg in another tenant. Cross-tenant
> atomicity holds at execution time; it is not reconstructable by replaying a
> single tenant's history.

---

## 9. State Model

Current state is a projection derived from committed events.

A current-state record is cacheable and indexable, but it is not the source of truth.

StateChronicle does not assume that every resource is an owned item. A state record may represent a unique asset, a fungible balance, a consumable stack, an entitlement, a metered quota, a listing, an escrow position, or a profile-defined custom state.

Example unique-asset state projection:

```json
{
  "tenant_id": "stexs.game.alpha",
  "resource_id": "asset:sword_001",
  "state_type": "unique_asset",
  "owner": "account:stexs:player_123",
  "status": "active",
  "version": 42,
  "last_event_id": "evt_01JZ...",
  "last_commit_id": "cmt_01JZ...",
  "state_hash": "sha256:7a92..."
}
```

Example fungible balance projection:

```json
{
  "tenant_id": "stexs.game.alpha",
  "resource_id": "currency:gold",
  "state_type": "fungible_balance",
  "subject": "account:stexs:player_123",
  "balance": "125000",
  "unit": "gold_minor",
  "version": 88,
  "last_event_id": "evt_01JZ...",
  "last_commit_id": "cmt_01JZ...",
  "state_hash": "sha256:64b9..."
}
```

Example consumable stack projection:

```json
{
  "tenant_id": "stexs.game.alpha",
  "resource_id": "material:iron_ore",
  "state_type": "consumable_stack",
  "subject": "account:stexs:player_123",
  "quantity": "42",
  "version": 9,
  "last_event_id": "evt_01JZ...",
  "last_commit_id": "cmt_01JZ...",
  "state_hash": "sha256:f1d4..."
}
```

The source of truth is:

```text
previous state + valid committed event = next state
```

For multi-resource transactions, the source of truth is:

```text
previous state set + valid atomic transaction = next state set
```

---

## 10. Resource State Types

StateChronicle Core defines the mechanics for state transitions. Profiles define the meaning of each state type.

Baseline state types should include the following.

### 10.1 Unique Asset

A unique asset has one current owner or controller.

Used for:

```text
skins
weapons
collectibles
one-of-one licenses
badges
tickets
land parcels
creator items
```

Typical state:

```json
{
  "state_type": "unique_asset",
  "owner": "account:stexs:player_123",
  "status": "active",
  "version": 17
}
```

### 10.2 Consumable Stack

A consumable stack represents a count of stackable units held by a subject.

Used for:

```text
potions
ammo
keys
crafting materials
boosters
loot boxes
energy packs
```

Typical state:

```json
{
  "state_type": "consumable_stack",
  "subject": "account:stexs:player_123",
  "quantity": "42",
  "version": 9
}
```

A consume operation reduces quantity and must fail if the requested quantity is greater than the current quantity.

### 10.3 Fungible Balance

A fungible balance represents a numerical amount of a resource held by a subject.

Used for:

```text
gold
coins
credits
gems
points
reputation
XP
marketplace credits
```

Typical state:

```json
{
  "state_type": "fungible_balance",
  "subject": "account:stexs:player_123",
  "balance": "125000",
  "unit": "gold_minor",
  "version": 88
}
```

Fungible balances must use canonical non-negative decimal integer strings on the wire (v0 baseline); internally the executor and profile rules compute with an exact fixed-point Amount (u128 mantissa × 10^scale, scale ≤ 18, checked/fail-closed on overflow and underflow), so the wire form is unchanged while internal computation is exact fixed-point arithmetic. Floating-point numbers must not be used for canonical economic state. Profiles may declare arbitrary precision via a profile precision extension without a wire-format change (ADR-006, §36 Q13).

### 10.4 Entitlement

An entitlement represents access, license, membership, subscription, feature availability, or claim status for a subject.

Used for:

```text
DLC access
battle passes
early access
premium accounts
game licenses
server access
creator tools
```

Typical state:

```json
{
  "state_type": "entitlement",
  "subject": "account:stexs:player_123",
  "status": "active",
  "starts_at": "2026-07-14T00:00:00Z",
  "expires_at": null,
  "version": 4
}
```

### 10.5 Metered Resource

A metered resource represents a refillable, time-bound, or usage-limited counter.

Used for:

```text
energy
stamina
daily actions
crafting slots
API credits
usage quotas
```

Typical state:

```json
{
  "state_type": "meter",
  "subject": "account:stexs:player_123",
  "current": "17",
  "maximum": "100",
  "last_refill_at": "2026-07-14T10:00:00Z",
  "version": 12
}
```

### 10.6 Listing and Escrow State

Listings and escrow states represent temporary control constraints around sale, trade, auction, lending, or settlement workflows.

Typical state:

```json
{
  "state_type": "escrow",
  "resource_id": "asset:sword_001",
  "locked_owner": "account:stexs:player_123",
  "beneficiary": "account:stexs:player_456",
  "status": "escrowed",
  "version": 3
}
```

### 10.7 Profile Ownership Rule

Only state types that define an `owner` field should use ownership proofs. Balance, stack, entitlement, and meter profiles should use holder, subject, balance, quantity, access, or quota proofs as appropriate.


---

## 11. Intent Model

An intent is a client or service request to mutate state.

An intent is not automatically valid. It becomes an event only after validation.

### 11.1 Intent Fields

Recommended fields:

```json
{
  "schema": "statechronicle.intent.v0",
  "tenant_id": "stexs.game.alpha",
  "intent_id": "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2",
  "operation": "asset.transfer",
  "actor": "account:stexs:player_123",
  "resource_id": "asset:sword_001",
  "expected_version": 41,
  "inputs": {
    "from_owner": "account:stexs:player_123",
    "to_owner": "account:stexs:player_456"
  },
  "authority": {
    "kind": "trustgrant.evaluation",
    "evaluation_digest": "sha256:2b18...",
    "evaluated_at": "2026-07-14T00:00:00Z"
  },
  "created_at": "2026-07-14T00:00:00Z",
  "expires_at": "2026-07-14T00:05:00Z",
  "nonce": "b64u:Q3g...",
  "signature": {
    "alg": "ed25519",
    "key_id": "did:key:z6Mk...#key-1",
    "sig": "b64u:..."
  }
}
```

### 11.2 Intent Requirements

Every mutating intent must include:

- `tenant_id`
- `intent_id`
- `operation`
- `actor`
- `resource_id`
- `state_type` when required by the active profile
- `expected_version`
- `created_at`
- Expiry or replay policy
- Actor authentication proof
- Authority proof or reference — REQUIRED for operations the active profile declares
  authority-required (ownership transfer, terminal destruction, paid restriction);
  OPTIONAL otherwise, where the profile's transition and consent rules govern.

The tuple below must be idempotent:

```text
(tenant_id, intent_id, actor, resource_id, operation)
```

Replaying the same accepted intent must return the same committed result. Replaying a conflicting intent with the same `intent_id` must fail.

---

## 12. Event Model

An event is a validated state transition.

Events are generated by the ledger executor after validation. Clients submit intents; executors emit events.

### 12.1 Event Fields

```json
{
  "schema": "statechronicle.event.v0",
  "tenant_id": "stexs.game.alpha",
  "event_id": "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4",
  "intent_id": "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2",
  "operation": "asset.transfer",
  "resource_id": "asset:sword_001",
  "actor": "account:stexs:player_123",
  "before": {
    "owner": "account:stexs:player_123",
    "status": "active",
    "version": 41,
    "state_hash": "sha256:a1e0..."
  },
  "after": {
    "owner": "account:stexs:player_456",
    "status": "active",
    "version": 42,
    "state_hash": "sha256:7a92..."
  },
  "authority": {
    "kind": "trustgrant.evaluation",
    "evaluation_digest": "sha256:2b18...",
    "result": "allow",
    "evaluated_at": "2026-07-14T00:00:00Z"
  },
  "executor": "service:statechronicle.stexs.net",
  "created_at": "2026-07-14T00:00:01Z"
}
```

> **Note (Phase 2):** For multi-authority transitions `evaluation_digest` is the **aggregate**
> digest over the sorted sub-evaluation digests (or the sub-evaluation digest itself for a
> single-member set). The individual sub-evaluations are not embedded (ADR-006 §36 Q5, Q6).

### 12.2 Event Requirements

Every event must:

- Include the tenant scope in which it was executed.
- Reference exactly one accepted intent.
- Include before-state and after-state commitments.
- Be assigned to exactly one commit.
- Be ordered deterministically inside its commit.
- Be replayable by any conforming verifier.
- Be rejectable if its authority proof is missing for an authority-required operation,
  denied, expired, revoked, stale, or insufficient under the active profile's aggregation
  policy.

Event `created_at` timestamps are advisory metadata stamped by the executor; canonical ordering MUST ignore them (canonical key = `(resource_id, after.version, event_id)`). Client-supplied intent timestamps are likewise advisory — only `expires_at` is enforced against the executor clock (ADR-006, §36 Q9).

---

## 13. Commit Model

A commit is an ordered batch of validated events.

One commit may contain one event or millions of events.

### 13.1 Commit Fields

```json
{
  "schema": "statechronicle.commit.v0",
  "scope": {
    "kind": "tenant",
    "tenant_id": "stexs.game.alpha"
  },
  "commit_id": "cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W",
  "parent_commit_id": "cmt_01JZ8WZ0QH93JK8J19VVD3QXSC",
  "sequence": 918273,
  "event_count": 180000,
  "event_merkle_root": "sha256:4d88...",
  "previous_state_root": "sha256:8f12...",
  "next_state_root": "sha256:b91e...",
  "created_at": "2026-07-14T00:00:02Z",
  "executor": "service:statechronicle.stexs.net",
  "profile": "statechronicle.profile.resource.v0",
  "signature": {
    "alg": "ed25519",
    "key_id": "did:key:z6Mk...#statechronicle-commit",
    "sig": "b64u:..."
  }
}
```

### 13.2 Commit Requirements

Each commit must:

- Declare its tenant scope or global checkpoint scope.
- Link to the previous canonical commit, unless it is a genesis commit.
- Include an ordered event set.
- Include a Merkle root or equivalent cryptographic accumulator for events.
- Include previous and next state roots. Per-commit tenant state roots are MANDATORY for all deployments, including single-tenant ones; global checkpoint roots (cross-tenant composition) remain optional (ADR-006, §36 Q12).
- Be signed by an authorized commit authority.
- Be rejectable if event replay does not produce `next_state_root`.

A cross-tenant transaction produces **one tenant-scoped commit per affected
tenant**; a single commit carries one tenant root (ADR-006, §36 Q12). Global
checkpoints remain optional and are not required for cross-tenant transactions.

### 13.3 Batch Semantics

Within a commit, events are applied in deterministic order.

The profile must define ordering. Recommended default:

```text
1. resource_id ascending byte order
2. expected_version ascending
3. intent_id ascending byte order
```

However, high-throughput implementations may partition by resource ID and then merge partition roots, provided the resulting commit is deterministic and independently verifiable.

### 13.4 Tenant Checkpoint Commits

A deployment that batches many tenants together may publish global checkpoint commits that contain tenant roots rather than directly containing all events.

Example:

```json
{
  "schema": "statechronicle.global_checkpoint.v0",
  "sequence": 55102,
  "tenant_roots": [
    {
      "tenant_id": "stexs.game.alpha",
      "commit_id": "cmt_alpha_01JZ...",
      "state_root": "sha256:aaa1..."
    },
    {
      "tenant_id": "stexs.marketplace",
      "commit_id": "cmt_market_01JZ...",
      "state_root": "sha256:bbb2..."
    }
  ],
  "tenant_merkle_root": "sha256:cc33...",
  "signature": {
    "alg": "ed25519",
    "key_id": "did:key:z6Mk...#global-checkpoint",
    "sig": "b64u:..."
  }
}
```

Global checkpoints are optional. They must not weaken tenant-level verification.

---

## 14. State Root Model

A state root commits to the entire current state at a commit boundary.

Non-membership proofs authenticate the absence of a state key at a commit root (§16.2).

The protocol does not mandate one accumulator implementation, but a profile must define it.

Supported accumulator families may include:

- Sparse Merkle tree
- Merkle Patricia tree
- Verkle tree
- Ordered Merkle map
- Hash array mapped trie
- Authenticated B-tree
- Profile-defined equivalent

A verifier must be able to check:

```text
previous_state_root + ordered events = next_state_root
```

For interoperability, the baseline profile should define one default state accumulator.

---

## 15. Snapshot Model

A snapshot is a compact state checkpoint at a commit.

Snapshots allow verifiers to avoid replaying from genesis.

```json
{
  "schema": "statechronicle.snapshot.v0",
  "snapshot_id": "snp_01JZ8X9P4DC6YC4K1YZEJX45E2",
  "commit_id": "cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W",
  "state_root": "sha256:b91e...",
  "resource_count": 982193812,
  "created_at": "2026-07-14T00:00:05Z",
  "content": {
    "kind": "shardline.object",
    "digest": "sha256:e3f1...",
    "media_type": "application/vnd.statechronicle.snapshot+json"
  },
  "signature": {
    "alg": "ed25519",
    "key_id": "did:key:z6Mk...#statechronicle-snapshot",
    "sig": "b64u:..."
  }
}
```

Snapshots are optional for correctness but recommended for scale. Any published snapshot MUST be authentic: its payload digest is bound to the enclosing commit's state root, so a verifier can confirm the snapshot without replay (ADR-006, §36 Q7). Snapshot cadence is deployment policy, not a protocol rule.

**Cadence guidance (ADR-006, §36 Q7):** publish a snapshot when the estimated cumulative replay cost since the last snapshot exceeds the snapshot store cost (full state serialization + digest + storage). Relevant operational factors are per-tenant event volume, recovery SLA, verifier replay budget, and storage cost; there is no protocol-mandated interval. A baseline heuristic is to snapshot when the estimated replay-from-genesis (or from the last authenticated snapshot) time exceeds the deployment's recovery SLA. Snapshots never weaken tenant-level verification: authenticity is via the SnapshotProof binding to the enclosing commit's state root, as above.

---

## 16. Proof Model

StateChronicle proofs allow external verifiers to check current or historical state without direct database access.

### 16.1 Proof Types

| Proof | Purpose |
|---|---|
| Inclusion proof | Proves an event is included in a commit |
| State proof | Proves a resource state at a commit |
| Transition proof | Proves a valid before → after transition |
| Ownership proof | Proves current owner/controller/holder |
| History proof | Proves ordered event history for a resource |
| Snapshot proof | Proves snapshot authenticity and state root |
| Authority proof | Binds a TrustGrant evaluation to a transition |
| Commit proof | Proves a commit belongs to the canonical chain |
| Non-membership proof | Proves a state key (resource or subject-held) holds no state at a commit |

### 16.2 Resource State Proof Bundle

```json
{
  "schema": "statechronicle.proof.resource_state.v0",
  "tenant_id": "stexs.game.alpha",
  "resource_id": "asset:sword_001",
  "claimed_state": {
    "owner": "account:stexs:player_456",
    "status": "active",
    "version": 42
  },
  "commit": {
    "commit_id": "cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W",
    "sequence": 918273,
    "state_root": "sha256:b91e...",
    "signature": {
      "alg": "ed25519",
      "key_id": "did:key:z6Mk...#statechronicle-commit",
      "sig": "b64u:..."
    }
  },
  "state_inclusion_proof": {
    "kind": "sparse_merkle_v0",
    "path": ["sha256:...", "sha256:..."],
    "leaf_hash": "sha256:7a92..."
  },
  "latest_event": {
    "event_id": "evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4",
    "operation": "asset.transfer"
  },
  "authority": {
    "kind": "trustgrant.proof_bundle",
    "digest": "sha256:2b18..."
  }
}
```

The `state_inclusion_proof` is the sparse Merkle inclusion proof of the resource's state leaf against the commit's `state_root`.

#### Non-membership proof bundle

A non-membership proof authenticates that a state key holds no state at a commit. It is an inclusion proof of the **empty-leaf constant** under the commit's state root, wrapped in a portable bundle:

```json
{
  "schema": "statechronicle.proof.non_membership.v0",
  "tenant_id": "stexs.game.alpha",
  "resource_id": "asset:sword_001",
  "claimed_key": "sha256:4f2b...",
  "commit": {
    "commit_id": "cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W",
    "sequence": 918273,
    "state_root": "sha256:b91e...",
    "signature": {
      "alg": "ed25519",
      "key_id": "did:key:z6Mk...#statechronicle-commit",
      "sig": "b64u:..."
    }
  },
  "state_non_membership_proof": {
    "kind": "sparse_merkle_v0",
    "path": ["sha256:...", "sha256:..."],
    "leaf_hash": "sha256:3abd..."
  }
}
```

`claimed_key` is the 32 raw state key bytes encoded as a `sha256:` digest, binding the bundle to the caller's `StateKey`. It is the raw 32-byte state key rendered in the canonical digest string form — semantically a key, not a hash of arbitrary content — and verifiers compare raw bytes (`claimed_key.as_bytes()` against the caller's key bytes), so the `sha256:` prefix is a transport encoding, not a claim that the key is a hash. The proof is an inclusion proof of the empty-leaf constant; verifiers MUST assert that `state_non_membership_proof.leaf_hash` equals the empty-leaf constant (`EMPTY_LEAF_HASH`) and fail closed if the slot is occupied.

### 16.3 Proof Verification

A verifier must check:

1. Object canonicalization and hash correctness.
2. Commit signature validity.
3. Commit authority authorization.
4. Commit membership in the accepted canonical history for the declared tenant scope.
5. State inclusion proof against the commit state root.
6. Claimed state hash matches the included leaf.
7. Relevant event or history proof, if required by the verifier policy.
8. TrustGrant authority proof for the operation, if verifying a transition.
9. Revocation freshness policy, if the operation depended on revocable authority.

---

## 17. Canonicalization and Hashing

StateChronicle objects should use deterministic canonicalization before hashing or signing.

Recommended baseline:

- JSON data model
- RFC 8785 JSON Canonicalization Scheme, or a profile-defined equivalent
- SHA-256 as baseline digest
- Ed25519 as baseline signature algorithm

Digest format:

```text
sha256:<lowercase-hex>
```

Signatures must cover the canonical representation of the signed object, excluding the signature field unless the profile defines another signed envelope format.

---

## 18. Execution Semantics

StateChronicle execution turns intents into committed events.

### 18.1 Validation Pipeline

Recommended pipeline:

```text
1. Parse intent.
2. Enforce size and schema limits.
3. Check intent_id uniqueness/idempotency.
4. Authenticate actor.
5. Resolve tenant scope and enforce tenant boundary rules.
6. Load current resource state or affected state set.
7. Check expected_version for every affected state record.
8. Evaluate TrustGrant authority per the active profile's authority policy; authority-required
   operations MUST carry a binding; evaluate every member of the deployment's authority set and
   aggregate (require-all default, any-of where declared); the bound digest covers the sorted
   sub-evaluation digests of the evaluations that participated in the outcome (under require-all,
   all members; under any-of, the allowing members), or the sub-evaluation digest itself for a
   single-member set.
9. Evaluate profile-specific operation rules.
10. Compute after-state deterministically.
11. Emit event or atomic event set.
12. Add event or transaction to commit batch.
13. Compute event root and next state root.
14. Sign commit.
15. Persist commit and update projections atomically.
```

### 18.2 Conflict Rules

The following must fail closed:

- `expected_version` does not match current version.
- Tenant scope is missing, ambiguous, or not authorized.
- Current owner, holder, balance, quantity, or subject does not match operation input.
- Resource is locked, burned, revoked, or escrowed in a way that blocks the operation.
- Any required TrustGrant evaluation result is not `allow` under the active profile's aggregation policy.
- Authority proof missing for an authority-required operation.
- TrustGrant evaluation is stale under the verifier policy.
- Intent expired before acceptance.
- Duplicate `intent_id` with different payload.
- Commit replay does not match declared state root.
- Commit signer is not authorized.

### 18.3 Atomicity

A commit is accepted only if all included events are valid under deterministic replay.

Profiles may allow partial batch acceptance before commit formation, but a signed commit itself must be internally valid.

Multi-resource transactions must commit all affected state transitions or none of them. This is required for operations such as purchases, trades, escrow release, currency exchange, crafting, and bundled entitlement grants.

Example atomic transaction:

```text
1. debit buyer currency.gold by 500
2. credit seller currency.gold by 500
3. transfer asset:sword_001 from seller to buyer
4. commit all three transitions together or reject all three
```

Cross-tenant transactions follow the same all-or-nothing rule across tenant
boundaries: **one tenant-scoped commit per affected tenant**, legs linked by a
shared `intent_id`, and authority gated **per tenant scope**. The transaction
executes atomically — every affected tenant's leg commits or none do (protocol
§8.2).

---

## 19. Commit Authority

Because StateChronicle is not a blockchain, deployments must define who is allowed to finalize commits.

A commit authority is a subject authorized to sign canonical commits for a scope.

Every conforming deployment MUST bind commit authority to a verifiable trust anchor. TrustGrant is the standard binding mechanism; deployments may use configured trust roots where the authority model requires it (ADR-006, §36 Q2). External verifiers check both the commit signature and the authority chain.

Recommended approach:

```text
TrustGrant authorizes StateChronicle commit authority.
StateChronicle commit authority signs commits.
External verifiers verify both the commit signature and the TrustGrant authority chain.
```

Example authority scope:

```json
{
  "subject": "service:statechronicle.stexs.net",
  "operations": [
    "state.commit",
    "state.snapshot",
    "asset.mint",
    "asset.transfer",
    "asset.burn"
  ],
  "resources": [
    "stc://stexs.game.alpha/asset/*",
    "stc://stexs.game.alpha/currency/*",
    "stc://stexs.game.alpha/material/*"
  ]
}
```

Transitions are gated by the deployment's authority set (one or more trust anchors) under the active profile's aggregation policy (require-all by default, any-of where declared); any ambiguity or conflict fails closed, and the bound digest covers each sub-evaluation (ADR-006 §36 Q5).

### 19.1 Revocation Freshness

Every conforming deployment MUST check revocation freshness at acceptance for authority-bound transitions (fail closed on stale). Each sub-evaluation is subject to the freshness window; the aggregate proof carries the oldest `evaluated_at` (the stalest member), so a verifier can check age without resolving the evaluation digests. The freshness window is deployment-configurable policy, not a core constant. Baseline recommendation: 24 hours; paid-unique and ownership-critical profiles should use ≤1 hour. `AuthorityProof.evaluated_at` lets an offline verifier check age without resolving the evaluation digest (ADR-006, §36 Q3).

---

## 20. Profiles

The core protocol defines the general mechanics. Profiles define domain-specific operations and state rules.

A profile must define:

- Tenant ID conventions
- Resource ID conventions
- Subject ID conventions
- Allowed operations
- Required fields per operation
- State machine rules
- Versioning rules
- Conflict rules
- Proof requirements
- State accumulator choice
- Canonicalization rules, if different from baseline
- Whether hard deletion is forbidden, tombstoned, or profile-defined

### 20.1 Baseline Resource Profile

Recommended generic operations:

```text
resource.create
resource.update
resource.transfer
resource.lock
resource.unlock
resource.restrict
resource.restore
resource.tombstone
```

The baseline profile should not define destructive hard deletion as a normal operation. State removal must be represented by an append-only tombstone, restriction, burn, expiry, or profile-defined terminal state. Hard deletion is forbidden for ANY committed state in any conforming profile; even an owner-consent "hard_delete" path is a tombstone transition, never an erasure (ADR-006, §36 Q11).

### 20.2 Unique Asset Profile

Recommended operations:

```text
asset.mint
asset.transfer
asset.burn
asset.lock
asset.unlock
asset.redeem
asset.list
asset.delist
asset.escrow
asset.release
asset.attach_content
asset.detach_content
asset.update_metadata
asset.restrict
asset.restore
```

Recommended states:

```text
active
locked
listed
escrowed
redeemed
burned
restricted
quarantined
unsupported
tombstoned
```

A unique asset profile must define who can initiate each transition and which transitions require current-owner approval.

### 20.3 Paid Unique Asset Profile

Paid unique assets require stronger ownership invariants than ordinary resources.

A paid unique asset is a unique resource that has been sold, transferred for consideration, granted under a paid entitlement, or otherwise committed to a subject under a profile-defined ownership right.

Required invariants:

```text
1. Committed ownership history MUST NOT be erased.
2. Hard deletion MUST NOT be a valid transition for a paid unique asset.
3. The creator/origin authority MUST NOT be able to transfer, burn, or invalidate the current owner's ownership state unless the profile-defined exception path applies.
4. Revoking creator authority affects future creator operations, not already-committed paid ownership.
5. Unsupported, restricted, quarantined, or legally held states MUST be explicit append-only state transitions.
6. A verifier MUST be able to distinguish loss of utility from loss of ownership.
```

Creator authority is not the same as ownership authority.

```text
origin_authority      = who created or originally issued the resource
commit_authority      = who finalizes StateChronicle commits
current_owner         = who owns this specific paid instance now
policy_authority      = who may apply exceptional restrictions under declared rules
```

A creator may retain authority over:

```text
future minting
official metadata updates
compatibility profiles
visual patches
brand/IP policy
```

A creator must not silently perform:

```text
delete buyer inventory state
burn buyer-owned item
transfer buyer-owned item away
erase ownership history
rewrite the sale
silently invalidate ownership proof
```

Exceptional restrictions must be transparent and non-erasing.

The baseline paid unique asset profile MUST support all six exception states — `restricted`, `quarantined`, `unsupported`, `legal_hold`, `fraud_lock`, `policy_restricted` — as explicit, append-only, owner-preserving transitions. Profiles may add more (ADR-006, §36 Q10).

Allowed exceptional states:

```text
restricted
quarantined
legal_hold
fraud_lock
policy_restricted
unsupported
```

Example restriction state:

```json
{
  "status": "quarantined",
  "reason_code": "stolen_payment",
  "evidence_ref": "case_9382",
  "appeal_ref": "support:appeal_9382",
  "effective_at": "2026-07-14T00:00:00Z",
  "authorized_by": "service:policy.stexs.net"
}
```

The user may lose display, equip, transfer, or service utility under a declared policy, but the ownership claim remains provable unless the owner authorizes burn/transfer or the original profile explicitly defined the exception before purchase.

### 20.4 Consumable Stack Profile

Recommended operations:

```text
stack.create
stack.credit
stack.debit
stack.consume
stack.transfer
stack.reserve
stack.release
stack.expire
stack.adjust
```

Required rules:

```text
quantity >= 0
consume amount > 0
transfer amount > 0
debit amount <= current quantity
expected_version must match current version
all quantities must use integer strings or profile-defined fixed precision
```

A consumable stack generally proves a subject's quantity, not ownership of a unique instance.

### 20.5 Fungible Balance Profile

Recommended operations:

```text
balance.create
balance.mint
balance.credit
balance.debit
balance.transfer
balance.reserve
balance.release
balance.spend
balance.burn
balance.convert
```

Required rules:

```text
balance >= 0
amount > 0
no floating-point canonical values
debit amount <= current balance
transfers are atomic debit + credit transactions
mint and burn require explicit authority
expected_version must match every affected balance record
```

This profile is suitable for in-game money, points, credits, reputation, XP, marketplace credits, and other fungible resources.

### 20.6 Entitlement Profile

Recommended operations:

```text
entitlement.grant
entitlement.activate
entitlement.suspend
entitlement.restore
entitlement.expire
entitlement.revoke
entitlement.transfer
```

Entitlements prove access or license state for a subject. They may be transferable or non-transferable depending on the profile.

### 20.7 Meter Profile

Recommended operations:

```text
meter.create
meter.consume
meter.refill
meter.set_maximum
meter.reset
meter.expire
```

Meter profiles must define refill semantics deterministically so independent verifiers can derive the same state from the same inputs.

### 20.8 Game Inventory Profile

Recommended game operations:

```text
asset.recognize
asset.display
asset.equip
asset.unequip
asset.transfer
asset.trade
stack.consume
stack.credit
balance.spend
balance.transfer
asset.craft.consume
asset.craft.produce
asset.reward.grant
asset.reward.claim
entitlement.grant
entitlement.revoke
```

StateChronicle should record durable inventory state. Ephemeral runtime state, such as a temporary in-match buff, should usually not be committed unless the game profile explicitly requires it.

### 20.9 Economy and Marketplace Profile

Recommended operations:

```text
listing.create
listing.cancel
listing.buy
listing.expire
escrow.lock
escrow.release
escrow.refund
balance.reserve
balance.release
balance.settle
royalty.accrue
```

Marketplace operations usually require atomic multi-resource transactions involving listing state, asset ownership, escrow state, buyer balance, seller balance, and optional royalty balances.

---

## 21. Example: Asset Mint

### 21.1 Intent

```json
{
  "schema": "statechronicle.intent.v0",
  "tenant_id": "stexs.game.alpha",
  "intent_id": "int_mint_001",
  "operation": "asset.mint",
  "actor": "service:inventory.stexs.net",
  "resource_id": "asset:sword_001",
  "expected_version": 0,
  "inputs": {
    "owner": "account:stexs:player_123",
    "content": {
      "kind": "shardline.object",
      "digest": "sha256:9e3b..."
    },
    "metadata_hash": "sha256:44c1..."
  },
  "authority": {
    "kind": "trustgrant.evaluation",
    "evaluation_digest": "sha256:abcd..."
  },
  "created_at": "2026-07-14T00:00:00Z"
}
```

### 21.2 Event Result

```json
{
  "schema": "statechronicle.event.v0",
  "tenant_id": "stexs.game.alpha",
  "event_id": "evt_mint_001",
  "intent_id": "int_mint_001",
  "operation": "asset.mint",
  "resource_id": "asset:sword_001",
  "before": null,
  "after": {
    "owner": "account:stexs:player_123",
    "status": "active",
    "version": 1,
    "content_digest": "sha256:9e3b...",
    "metadata_hash": "sha256:44c1..."
  },
  "authority": {
    "kind": "trustgrant.evaluation",
    "evaluation_digest": "sha256:abcd...",
    "result": "allow"
  }
}
```

---

## 22. Example: Asset Transfer

A transfer must prove that the source owner is still current at the expected version.

```json
{
  "schema": "statechronicle.intent.v0",
  "tenant_id": "stexs.game.alpha",
  "intent_id": "int_transfer_001",
  "operation": "asset.transfer",
  "actor": "account:stexs:player_123",
  "resource_id": "asset:sword_001",
  "expected_version": 1,
  "inputs": {
    "from_owner": "account:stexs:player_123",
    "to_owner": "account:stexs:player_456"
  },
  "authority": {
    "kind": "trustgrant.evaluation",
    "evaluation_digest": "sha256:ef01..."
  },
  "created_at": "2026-07-14T00:01:00Z"
}
```

If current state is:

```json
{
  "owner": "account:stexs:player_123",
  "status": "active",
  "version": 1
}
```

Then after-state is:

```json
{
  "owner": "account:stexs:player_456",
  "status": "active",
  "version": 2
}
```

If the current version is already `2`, the transfer must fail as stale.

---

## 23. Example: Consumable Stack Consume

A consumable stack operation must prove that the subject has enough quantity at the expected version.

```json
{
  "schema": "statechronicle.intent.v0",
  "tenant_id": "stexs.game.alpha",
  "intent_id": "int_consume_001",
  "operation": "stack.consume",
  "actor": "account:stexs:player_123",
  "resource_id": "material:iron_ore",
  "expected_version": 9,
  "inputs": {
    "subject": "account:stexs:player_123",
    "amount": "3"
  },
  "authority": {
    "kind": "trustgrant.evaluation",
    "evaluation_digest": "sha256:cc01..."
  },
  "created_at": "2026-07-14T00:02:00Z"
}
```

If current quantity is `42`, the after-state quantity is `39`. If current quantity is less than `3`, the intent must fail.

---

## 24. Example: Fungible Currency Transfer

A fungible balance transfer is an atomic debit and credit transaction.

```json
{
  "schema": "statechronicle.transaction_intent.v0",
  "tenant_id": "stexs.game.alpha",
  "intent_id": "int_gold_transfer_001",
  "operation": "balance.transfer",
  "actor": "account:stexs:player_123",
  "inputs": {
    "resource_id": "currency:gold",
    "from_subject": "account:stexs:player_123",
    "to_subject": "account:stexs:player_456",
    "amount": "500",
    "expected_versions": {
      "account:stexs:player_123": 88,
      "account:stexs:player_456": 31
    }
  },
  "authority": {
    "kind": "trustgrant.evaluation",
    "evaluation_digest": "sha256:dd02..."
  },
  "created_at": "2026-07-14T00:03:00Z"
}
```

The transaction must either debit the sender and credit the receiver together, or reject both changes.

---

## 25. Example: Atomic Purchase of a Unique Asset

A purchase of a unique asset for in-game currency requires a multi-resource atomic transaction.

```text
Transaction: purchase sword_001 for 500 gold

Inputs:
- buyer currency:gold expected_version = 88
- seller currency:gold expected_version = 31
- asset:sword_001 expected_version = 12
- listing:list_991 expected_version = 4

Effects:
- debit buyer currency:gold by 500
- credit seller currency:gold by 500
- transfer asset:sword_001 from seller to buyer
- close listing:list_991
```

The transaction must fail if any affected state record is stale, locked, insufficient, unauthorized, or incompatible with the active profile.

---

## 26. Example: Paid Unique Ownership Protection

A paid unique asset cannot be silently deleted by its creator after it has been sold.

Invalid operation:

```json
{
  "operation": "asset.delete",
  "actor": "organization:studio-a",
  "resource_id": "asset:sword_001",
  "reason": "creator_removed_item"
}
```

A conforming Paid Unique Asset Profile must reject this operation if `asset:sword_001` has a committed paid ownership state for a current owner and the owner did not authorize the transition.

Allowed transparent restriction example:

```json
{
  "operation": "asset.restrict",
  "actor": "service:policy.stexs.net",
  "resource_id": "asset:sword_001",
  "expected_version": 42,
  "inputs": {
    "status": "policy_restricted",
    "reason_code": "legal_hold",
    "visible_to_owner": true
  }
}
```

This records a visible restriction without erasing the ownership history.

---

## 27. Infra-Agnostic Storage Contract

StateChronicle implementations may use any backend that satisfies the logical contract.

Required logical stores:

| Store | Purpose |
|---|---|
| Intent store | Deduplication and idempotency |
| Event store | Append-only validated transitions |
| Commit store | Signed batch commits |
| State index | Current state projection |
| Proof index | Efficient proof generation |
| Snapshot store | Optional compact checkpoints |

Possible implementations:

- PostgreSQL
- FoundationDB
- RocksDB
- SQLite for local/dev deployments
- Kafka-compatible log plus state store
- Object storage plus database indexes
- Shardline-backed immutable commit objects
- Custom high-throughput sequencer plus durable checkpointing

The backend may vary. The canonical objects and verification results must not.

---

## 28. API Surface

The protocol does not mandate HTTP, gRPC, WebSocket, GraphQL, or message queues.

A reference service should expose these logical operations:

```text
submit_intent(intent) -> intent_result
submit_transaction_intent(intent) -> transaction_result
get_resource_state(tenant_id, resource_id) -> state
get_subject_resource_state(tenant_id, subject, resource_id) -> state
get_resource_history(tenant_id, resource_id) -> events
get_commit(tenant_id, commit_id | sequence) -> commit
get_state_proof(tenant_id, resource_id, commit_id?) -> proof
get_ownership_proof(tenant_id, resource_id, subject, commit_id?) -> proof
get_balance_proof(tenant_id, resource_id, subject, commit_id?) -> proof
get_snapshot(tenant_id, commit_id) -> snapshot
verify_proof(proof) -> verification_result
```

---

## 29. Verification Algorithm

A verifier checking current ownership should:

```text
1. Parse the proof bundle.
2. Canonicalize all signed objects.
3. Verify tenant scope.
4. Verify commit signature.
5. Verify commit authority using TrustGrant or configured trust roots.
6. Verify state inclusion proof against commit state root.
7. Verify claimed state hash.
8. Verify owner/subject/balance/quantity/entitlement fields match the verifier policy.
9. Verify resource status is acceptable.
10. Verify commit is recent enough under local policy.
11. Optionally verify latest transition and authority proof.
```

A verifier checking absence (a non-membership proof, §16.2) should:

```text
1. Parse the non-membership proof bundle.
2. Canonicalize all signed objects.
3. Verify tenant scope.
4. Verify commit signature.
5. Verify commit authority using TrustGrant or configured trust roots.
6. Verify key match: the bundle's claimed_key equals the resource's state key.
7. Assert the sparse Merkle proof's leaf is the empty-leaf constant (fail closed if the slot is occupied).
8. Verify the path against the commit state root.
9. Verify commit is recent enough under local policy.
```

A verifier checking a transition should additionally:

```text
1. Verify before-state proof.
2. Verify after-state proof.
3. Verify event inclusion in commit.
4. Replay operation-specific transition rule.
5. Verify TrustGrant allow result for operation, actor, resource, and scope.
6. Verify revocation freshness for the authority proof.
```

---

## 30. Security Considerations

StateChronicle implementations must defend against:

- Double-spend attempts
- Replay attacks
- Duplicate intent IDs
- Stale expected versions
- Confused-deputy authorization mistakes
- Mismatched aggregation policy between execution and verification
- Unauthorized commit signers
- Forked histories
- Malicious snapshots
- Hash collision assumptions
- Oversized intent/event payloads
- Invalid canonicalization
- Signature substitution
- Time-of-check/time-of-use bugs between TrustGrant evaluation and state commit, spanning every
  authority-set member evaluated for a transition
- Revocation freshness failures
- Backdated commits
- Partial commit persistence
- Projection corruption
- Cross-tenant state leakage
- Creator overreach after paid transfer
- Silent deletion or invisible restriction of paid resources
- Floating-point economic state divergence
- Non-atomic purchase/trade settlement

Recommended safeguards:

- Fail closed on ambiguous authority.
- Use bounded input sizes.
- Persist accepted intents before side effects.
- Use serializable transactions or equivalent compare-and-swap boundaries.
- Sign commits only after deterministic replay succeeds.
- Continuously audit projections against event replay.
- Store immutable commit objects separately from mutable indexes.
- Rotate commit keys using TrustGrant-authorized key transition procedures.
- Forbid hard deletion for any committed state (tombstone or terminal state only).
- Represent restrictions, legal holds, fraud locks, and unsupported states as visible append-only transitions.
- Check revocation freshness at acceptance with a deployment-configured window (default 24h; ≤1h for paid-unique profiles) — ADR-006, §36 Q3.
- Use integer or fixed-precision amount encodings for balances and consumables.
- Enforce tenant boundaries before any state read or mutation.

---

## 31. Forks and Recovery

A fork occurs when two different commits claim the same parent and sequence under the same canonical scope.

Baseline behavior:

- Verifiers must reject ambiguous forks unless a configured fork-resolution policy exists.
- Commit authorities should publish signed head checkpoints.
- Implementations should maintain append-only evidence of rejected or superseded commits.
- Recovery should never rewrite accepted event objects without preserving audit history.

Optional future mechanisms:

- Witness signatures
- External timestamping
- Multi-authority quorum commits
- Public checkpoint anchoring
- Cross-region reconciliation profiles

---

## 32. Performance Model

StateChronicle is designed for high-throughput batch execution.

Recommended approach:

```text
1. Partition incoming intents by resource_id hash.
2. Validate authority and current version in parallel.
3. Reject conflicts before commit formation.
4. Apply deterministic transition rules.
5. Build partition roots.
6. Merge partition roots into one commit root.
7. Sign one commit for a large batch.
8. Update current-state projections asynchronously but verifiably.
```

The protocol does not require one process per event.

One commit may represent many state transitions.

---

## 33. Example Full Stack Flow

```text
1. Asset bytes are uploaded to Shardline.
2. Shardline returns a content digest.
3. A mint intent references the Shardline digest.
4. TrustGrant proves the actor may mint this asset.
5. StateChronicle validates the intent.
6. StateChronicle emits an asset.mint event.
7. The event is included in a signed commit.
8. The current state projection now shows the owner.
9. Another game requests an ownership proof.
10. The game verifies Shardline content, TrustGrant authority, and StateChronicle state.
11. If the asset was purchased, StateChronicle proves the paid ownership state remains in history even if the creator later stops supporting the asset.
```

---

## 34. Naming and Positioning

Recommended one-line description:

> StateChronicle is a verifiable resource state protocol for append-only, replayable, authority-checked state transitions.

Recommended ecosystem positioning:

```text
Shardline       Content-addressed storage and integrity
TrustGrant      Delegated authority and permission verification
StateChronicle  Verifiable resource state, ownership, balances, and history
```

Recommended tagline:

> Trust the bytes. Trust the authority. Verify the state.

---

## 35. Minimal v0 Implementation

A minimal implementation should support:

- JSON canonicalization
- SHA-256 digests
- Ed25519 commit signatures
- Tenant-scoped intent deduplication
- Append-only event storage
- Signed commits
- Current-state projection
- Asset mint
- Asset transfer
- Asset burn
- Asset lock/unlock
- Paid unique asset no-hard-delete rule
- Consumable stack credit/debit/consume
- Fungible balance credit/debit/transfer
- State proof for current owner
- Balance or quantity proof for subject-held resources
- TrustGrant evaluation binding, aggregated per profile authority policy
- Shardline content digest references

Recommended backend for first implementation:

```text
PostgreSQL for transactional execution and projections
Shardline or object storage for immutable commit/snapshot objects
TrustGrant for authority evaluation
```

The protocol should not require this backend.

---

## 36. Open Questions for v0.1

All v0 questions below are **resolved by [ADR-006](docs/DESIGN/ADR/ADR-006-PROTOCOL_DECISIONS_V0.md)**. The list is retained as a record of what was decided; the ADR holds the authoritative decisions, rationales, and v0.1 deferrals.

| # | Question | Decision (ADR-006) | Binding |
| --- | --- | --- | --- |
| 1 | Baseline accumulator: sparse Merkle tree or ordered Merkle map? | Fixed 256-bit SMT per tenant (confirms ADR-005) | protocol |
| 2 | Commit authority always through TrustGrant? | Bound to a trust anchor; TrustGrant standard, configured roots permitted | protocol |
| 3 | Minimum revocation freshness window? | Policy-owned, deployment-configurable; check mandatory at acceptance; default 24h (≤1h for paid profiles) | policy + protocol (check) |
| 4 | Profiles: global registries or local namespaces? | Locally namespace-scoped per profile; dotted prefix is a convention | protocol |
| 5 | Multi-authority conflict resolution? | Per-profile aggregation policy: require-all default, any-of where declared; evaluate every member of the deployment's authority set; single bound digest over the sorted sub-evaluations (identity for a single member); ambiguity/conflict fails closed | protocol |
| 6 | Proof bundles: full TrustGrant chains or digests? | Digests + resolvable references only (confirms ADR-003) | protocol |
| 7 | Snapshots mandatory after a fixed interval? | Optional; authenticity mandatory if published; cadence is deployment policy | policy + protocol (authenticity) |
| 8 | Fork evidence core or federation profile? | Detection/evidence core; resolution policy deployment-defined | protocol |
| 9 | Event timestamps trusted, sequencer-derived, or advisory? | Advisory; canonical ordering ignores them | protocol |
| 10 | Mandatory paid-unique exception states? | Six baseline states: restricted, quarantined, unsupported, legal_hold, fraud_lock, policy_restricted | protocol |
| 11 | Deletion always tombstone? | Yes — hard deletion forbidden for any committed state | protocol |
| 12 | Tenant roots mandatory even single-tenant? | Yes — per-commit tenant roots mandatory; global checkpoints optional | protocol |
| 13 | Balances: decimal strings, arbitrary integers, or fixed precision? | Canonical decimal-integer strings; u64 fixed precision baseline | protocol |

---

## 37. Glossary

**Actor**  
The subject requesting a state transition.

**Authority proof**  
Evidence that an actor was allowed to perform an operation, usually derived from TrustGrant.

**Commit**  
A signed, ordered batch of validated events.

**Commit authority**  
A subject authorized to finalize StateChronicle commits for a scope.

**Consumable stack**  
A subject-held quantity of stackable units that can be credited, debited, transferred, or consumed.

**Event**  
A validated state transition.

**Intent**  
A request to perform a state transition.

**Projection**  
A derived current-state view built from committed events.

**Paid unique asset**  
A unique resource whose ownership has been committed to a subject through a sale, paid entitlement, or profile-defined ownership right.

**Resource**  
Anything whose state can be tracked.

**State root**  
A cryptographic commitment to all current resource states at a commit.

**Snapshot**  
A compact checkpoint of state at a specific commit.

**State type**  
The profile-defined model for a resource state, such as unique asset, fungible balance, consumable stack, entitlement, meter, listing, or escrow.

**Subject**  
A user, service, organization, device, authority, account, or other principal.

**Tenant**  
An isolated protocol scope such as a game, studio, marketplace, organization, world, customer, or environment.

**Tombstone**  
An append-only terminal or archival state used to represent removal without erasing committed history.

---

## 38. Draft License Note

This document is a draft protocol design and should be reviewed before publication. A final repository version should include the project license, contribution rules, compatibility policy, security disclosure process, and conformance test requirements.
