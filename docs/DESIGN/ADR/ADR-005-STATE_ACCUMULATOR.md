**Document Version:** 1.0
**Last Updated:** 2026-08-03
**Status:** Draft
**Related Documents:** [ADR README](README.md), [Architecture Summary](../../ARCHITECTURE.md),
[ADR-004](ADR-004-CANONICALIZATION_HASHING_SIGNATURE.md)

# ADR-005: Baseline State Accumulator, Sparse Merkle Tree (SMT)

**Date:** 2026-08-03

---

## Context

The protocol requires a state root that commits to all current resource state at a
commit boundary (§14), such that:

```text
previous_state_root + ordered events = next_state_root
```

is independently verifiable (§3, §14), with inclusion proofs (§16.2) and optional
non-membership evidence, across both tenant isolation modes (§8.1) and high-throughput
batch commits (§32). Protocol §36 open question 1 asks: sparse Merkle tree or ordered
Merkle map? §14 allows a family of accumulators (SMT, Merkle Patricia, Verkle, ordered
Merkle map, HAMT, authenticated B-tree, profile-defined).

The protocol's own proof example already assumes SMT: §16.2's `state_inclusion_proof`
is `"kind": "sparse_merkle_v0"` with a `path` of sibling hashes and a `leaf_hash`.

## Decision

**v0 baseline: a fixed 256-bit-depth sparse Merkle tree, one per tenant, hand-rolled in
`statechronicle-accumulator`, keyed by SHA-256 of a domain-separated composite key, with
SHA-256 node hashing.** (Answers §36.1: SMT.)

### 1. Why SMT (rationale, in weight order)

1. **Insertion-order-independent determinism (§3, §14, §13.2).** The SMT root is a pure
   function of the (key → leaf) set for a fixed depth, hash function, and empty-node
   constants. Two replays applying the same events in any order produce the same
    `next_state_root`: by construction, not by an added canonical-rebuild rule. This is
   exactly what `previous_state_root + ordered events = next_state_root` requires, and
   it makes §32 step 6 (merge partition roots) trivially correct: partition subtree
   diffs union without rebalancing or order-sensitive merge. Ordered maps (AVL/red-black)
   are insertion-order-dependent; Merkle Patricia has canonical shape but EVM-specific
   encoding machinery.
2. **Non-membership is free (§16, fail-closed).** Every SMT slot exists; the empty slot
   holds the empty-leaf constant. A non-membership proof is an inclusion proof of the
    empty leaf: same size, same verify path. This serves "resource does not exist as of
   commit X" (fail-closed per §18.2, §30). Ordered maps need ~2–3× neighbor proofs.
3. **Proof size for mobile verifiers (§16.2, §35).** Keys are SHA-256 images, so key
    distribution is uniform regardless of adversarial ID choice: tenants cannot force
    clustered keys. Expected inclusion-proof size is ~log₂N sibling hashes, not 256:
   - N ≈ 10⁶ leaves/tenant: **≈ 700 B**
   - N ≈ 10⁹: **≈ 1.1 KB**
   - Unreachable worst case: ≈ 8.3 KB. Per-tenant proofs never pay for the global
     checkpoint tree. Ordered Merkle maps are ~2.5× larger per node and
     distribution-sensitive (adversarial sorted-string keys bloat paths).
4. **Maturity + determinism interop (§17, §4).** The only production Rust SMT crate
   (`sparse-merkle-tree`, Nervos) is Blake2b-native with an undocumented SHA-256 escape
    hatch: adopting it would violate the SHA-256 baseline (ADR-004) or force us to
   reverse-engineer an encoding for non-Rust verifier interop. **The encoding is the
   commitment; it must be spec'd by us.** Hand-rolling is ~300–500 LOC of pure logic and
   matches the minimal-implementation ethos.

### 2. Key model and tenant composition

Key space: one SMT per tenant, 256-bit keys derived from domain IDs:

```text
key = SHA-256( 0x00 || len(tenant_id) || tenant_id
            || 0x01 || len(resource_id) || resource_id
            || [ 0x02 || len(subject_id) || subject_id ] )   // only for subject-held types
```

- Length-prefixing is unambiguous; IDs are canonical UTF-8 of the domain newtypes.
- `tenant_id` stays in the preimage even though trees are per-tenant (defense-in-depth
  against cross-tenant key collisions). No extra wire cost: verifiers already carry
  `tenant_id`.
- Subject-held types (fungible balance, consumable stack, meter, entitlement, §10)
  append the subject; unique assets (owner-based) do not. Key scheme is versioned by the
  leading `0x00` domain byte.
- Leaf value = the **state digest** (`state_hash`, SHA-256 of the canonical state
  projection, §9). So §16.3 step 6 ("claimed state hash matches the included leaf") is
  a direct comparison.
- Node encoding (spec'd by us): internal `H(0x10 || left || right)`, leaf
  `H(0x11 || key || state_digest)`, precomputed empty-subtree constants per level. This
  becomes a conformance-vector artifact.

Tenant composition (§8.1):

- **Hard isolation:** each tenant's state root *is* its SMT root. No global structure.
- **Logical isolation:** global checkpoint root = a plain Merkle tree over **sorted**
  `(tenant_id, tenant_root)` pairs (matching §8.1's diagram and §13.4's
  `tenant_merkle_root`). Tenant counts are small, so this tree is tiny
  (`log₂T × 32 B` per tenant-root proof). Checkpoints are optional and never weaken
  per-tenant verification.

### 3. Proof shape

Inclusion (`sparse_merkle_v0`, §16.2):

```text
{ key: [u8; 32], leaf_hash: [u8; 32], siblings: Vec<[u8; 32]> }   // only non-empty siblings
```

Levels whose sibling subtree is empty are skipped. The verifier fills precomputed
empty-subtree constants. Verify: walk key bits, combine `leaf_hash` upward with siblings
and default constants, compare to the supplied root; then check
`claimed_state.hash == leaf_hash` (§16.3 step 6).

Non-membership: identical shape; the slot's leaf is the empty-leaf constant.

Sizes: both ≈ 700 B at 10⁶, ≈ 1.1 KB at 10⁹ leaves. Global checkpoint layer adds
`log₂T × 32 B` only when composing.

### 4. API sketch (`statechronicle-accumulator`)

```rust
pub struct StateRoot([u8; 32]);          // sha256 digest
pub struct StateKey([u8; 32]);           // opaque; built from domain ids

pub struct StateUpdate { pub key: StateKey, pub state_digest: [u8; 32] }

pub struct StateAccumulator { /* per-epoch in-memory node cache */ }
impl StateAccumulator {
    pub fn empty() -> Self;
    pub fn root(&self) -> StateRoot;
    pub fn insert_batch(&mut self, updates: &[StateUpdate]) -> Result<StateRoot, AccumulatorError>;
    pub fn prove_inclusion(&self, key: &StateKey) -> Option<InclusionProof>;
    pub fn prove_non_membership(&self, key: &StateKey) -> Option<NonMembershipProof>;
    pub fn verify_inclusion(root: &StateRoot, proof: &InclusionProof) -> bool;
    pub fn verify_non_membership(root: &StateRoot, proof: &NonMembershipProof) -> bool;
}

pub struct CheckpointRoot([u8; 32]);     // logical isolation composition
impl CheckpointRoot {
    pub fn from_tenant_roots(tenant_roots: &[(TenantId, StateRoot)]) -> Result<Self, AccumulatorError>;
    pub fn prove_tenant_root(&self, tenant_id: &TenantId) -> Option<TenantRootProof>;
    pub fn verify_tenant_root(root: &CheckpointRoot, proof: &TenantRootProof) -> bool;
}
```

The crate stays pure/in-memory; persistence is the ports crate's job (§27 `StateIndex`/
`SnapshotStore`). `commit`/`proof` crates consume `StateRoot` (32-byte digest into
§13.2's `next_state_root`) and the proof types. Nothing blocks on them.

Lint notes: path bit-math uses `checked_shr`/`wrapping_*` (`arithmetic_side_effects`
denied), sibling access uses `.get()` (`indexing_slicing` denied), no unwrap/expect/
panic, `#![deny(unsafe_code)]`, small `thiserror` error enums (`result_large_err`).

### 5. Rust dependency recommendation

**Hand-roll the minimal SMT.** No new dependencies beyond the workspace (`sha2`,
`thiserror`, `statechronicle-core`). Rejected: `sparse-merkle-tree` (Nervos): Blake2b
default, undocumented SHA-256 path, dormant (2022), its types would leak into the domain
surface, and even at best it saves ~400 LOC while forfeiting ownership of the encoding
that is the protocol's real commitment. The strict lint set is compatible with
hand-rolling; correctness is covered by conformance vectors + proptest roundtrip
properties (verify∘prove = identity; root is a function of the key→value set).

## Alternatives Considered

| Option | Pros | Cons |
| --- | --- | --- |
| **SMT (Chosen)** | Order-independent determinism by construction; free non-membership; ~log₂N proofs; trivial partition merging; matches §16.2 example; per-tenant + checkpoint composition | Must spec node encoding ourselves (we want to); no native range proofs |
| **Ordered Merkle map** | Sorted/range proofs possible | No v0 requirement for range proofs (§28/§35; `ProofIndex` side index covers enumeration); insertion-order determinism needs a canonical-rebuild rule; ~2.5× larger proofs; distribution-sensitive; harder partition merging |
| **Merkle Patricia (Parity `trie`)** | Canonical shape; Ethereum-proven | EVM-centric (RLP, keccak, hex-prefix); heavyweight against minimal-impl ethos; awkward non-membership |
| **Verkle** | Sub-KB proofs | IPA/KZG complexity; no mature general-purpose Rust impl; defer |
| **HAMT / authenticated B-tree** | Order-independent (HAMT); disk-oriented (B-tree) | Less-standard proof conventions; rebalancing determinism and proof encoding worse than SMT for v0 |

### Why SMT Wins

1. §3/§14 determinism is a property of the structure, not a rebuild rule.
2. Non-membership is free: directly serves fail-closed §18.2/§30 semantics.
3. Smallest proofs for the mobile-verifier use case (§16.2).
4. §32 partition merging is trivial (union of subtree diffs).
5. Only family the protocol's own proof example already encodes (§16.2).

## Consequences

**Positive:**

- Deterministic state roots independent of event insertion order.
- Free non-membership proofs for fail-closed verification.
- ~700 B – 1.1 KB proofs up to 10⁹ leaves; uniform key distribution defeats
  adversarial clustering.
- One SMT per tenant + tiny sorted-root checkpoint tree covers both isolation modes.
- No new dependencies; encoding is ours to spec and vector-test.

**Negative:**

- Must hand-roll and spec the node/leaf encoding (~300–500 LOC).
- No native range/sorted-enumeration proofs (covered by `ProofIndex` side index in v0).
- Fixed 256-bit depth (no parameterization in v0).

**Mitigations:**

- Conformance vectors for node/leaf encoding + proptest roundtrip properties.
- In-memory per-epoch node cache; persistence lives in `StateIndex` (Postgres, §27),
  re-hydrated from snapshots (§15).

## Explicit Deferrals (safe for v0)

- Verkle (revisit only if sub-KB proofs become a hard constraint at extreme scale).
- Non-membership proof-bundle wiring. **RESOLVED (v0.1)**: the §16.2
  `statechronicle.proof.non_membership.v0` bundle variant (§16.2) now authenticates
  "does not exist at commit X" fail-closed; see ADR-006 deferral item 5 (Phase 4).
- Range / sorted enumeration proofs (no §35/§28 requirement; `ProofIndex` covers it).
- Disk-backed node persistence (v0 keeps per-epoch cache in-memory).
- Batch/multi-key proofs, random-sampling proofs.
- Configurable tree depth (fixed 256-bit).

## Migration Path (if the baseline ever changes)

State roots are committed per-commit (§13.2), so a baseline change only affects future
commits, but it touches the signed chain from the switch point. State is always a
projection of events (§9), so you never re-commit history or re-sign old commits.

1. Treat the accumulator as profile-defined (§14/§20): the switch is an
   accumulator-version tag on the commit / tenant profile registry entry; verifiers
   accept a set of known (profile, accumulator-version) pairs.
2. At the boundary commit, re-accumulate the current state map under the new
    accumulator: one-time O(N) SHA-256: ~10–60 s single-core at 10⁹ leaves,
   parallelizable by partition (§32) down to seconds.
3. Publish new conformance vectors for the new node/leaf encoding; verifiers update.

Total swap cost is bounded: one re-accumulation pass + a version/profile bump + verifier
rollout. No history rewrite, no signature invalidation, no key rotation.

## Risks

- **Node/leaf encoding drift between implementations** (non-Rust verifiers): the real
  risk: mitigated by owning the spec and locking it with conformance vectors; a
  hand-rolled SHA-256-only tree is the simplest possible interop target.
- **Proof-size degradation** is not a real risk (keys are SHA-256 images; worst case
  unreachable).
- **No native range proofs**: bounded ordered-map swap (§Migration Path) or an
  authenticated side index on `ProofIndex` if a later profile demands it.
- **Strict-lint friction** (`arithmetic_side_effects`, `indexing_slicing`): pure
  coding-discipline cost, already the workspace norm.

---

## Review & Maintenance

- **Last Reviewed:** 2026-08-03
- **Next Review:** Before first production release, or when a profile requires a
  different accumulator
- **Change Log:**
  - v1.1 (2026-08-05): Non-membership proof-bundle wiring deferred item marked RESOLVED
    (v0.1); the §16.2 bundle variant adds fail-closed absence proofs. Accumulator
    primitive itself unchanged (empty-leaf assertion lives in the bundle verifier).
  - v1.0 (2026-08-03): Baseline accumulator = fixed-256-bit SMT, one per tenant,
    hand-rolled, SHA-256 keys + node hashing; checkpoint root for logical isolation;
    non-membership shipped as a primitive; deferrals and migration path recorded
