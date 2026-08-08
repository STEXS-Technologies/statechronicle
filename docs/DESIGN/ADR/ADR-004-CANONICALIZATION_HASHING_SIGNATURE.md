**Document Version:** 1.0
**Last Updated:** 2026-08-03
**Status:** Draft
**Related Documents:** [ADR README](README.md), [Architecture Summary](../../ARCHITECTURE.md),
[ADR-003](ADR-003-TRUSTGRANT_PORTS_ONLY.md),
[ADR-005](ADR-005-STATE_ACCUMULATOR.md)

# ADR-004: Canonical Serialization, Hashing, and Signature Baseline, BCS

**Date:** 2026-08-03

---

## Context

StateChronicle objects (intents, events, commits, proofs, snapshots) must be
**deterministically serialized, hashed, and signed** so that independent
implementations and verifiers derive identical digests (protocol §3: deterministic
transitions; §16: portable proofs; §17: canonicalization and hashing; §30: invalid
canonicalization, signature substitution).

The protocol (§17) originally recommended JSON + RFC 8785 JCS. The protocol is built
for **high-throughput batch execution** (§2.4: millions of events/sec; §32: one commit
signs once for a huge batch). JSON is the wrong canonical spine for that goal:

- RFC 8785 JCS pays for key sorting + ES6 number formatting on **every object at every
  nesting level**; per-event encode+hash is the hot path (events feed the
  `event_merkle_root`, §13.2), so this cost is paid per event, not per commit.
- JCS is verbose (2–4× larger canonical bytes), increasing hash input, memory
  bandwidth, proof bundles, and stored history.
- Economic state forbids floats (§10.3), yet JCS drags the entire float64/ECMAScript
  number-formatting machinery into the signing path: pure wasted surface.
- Cross-language JCS conformance is a known interop trap (number-formatting edge
  cases); `serde_jcs` is a single-maintainer crate.

Because the decision is being locked before v0 conformance freezes (the codebase has
shipped BCS since its initial implementation), switching the canonical form now costs
nothing compared to after v0, when switching would cost a schema migration and re-signing
of real data.

## Decision

**Adopt BCS (Binary Canonical Serialization) as the v0 canonical form for all hashed
and signed objects. Keep JSON only as the HTTP API logical-model wire format for
clients. The JSON examples in the protocol are the logical-model documentation, not the
canonical encoding.**

### 1. Canonical serialization: BCS

- The `bcs` crate (formally specified; deployed in Aptos/Sui as the transaction
  hashing/signing path) is the canonical serializer: `bcs::to_bytes::<T: Serialize>()`.
- **Determinism is by construction, not convention**: one encoding per value, minimal
  (non-overlong ULEB128) length prefixes, fixed-width little-endian integers, no
  floats, no configuration. Decode rejects non-minimal encodings.
- Structs are positional (field names never encoded) → determinism is a **schema**
  property. Serde maps are supported and sorted by BCS-encoded key bytes (byte-identical
  to JCS key sorting for string keys).
- **Integer-keyed maps are banned in canonical types** (BCS sorts them by little-endian
  byte order, not numeric order); use string keys or sorted vectors.
- Enums serialize as canonical ULEB128 variant index + payload; variant order is frozen
  per schema version.

### 2. Signed-envelope rule (structural)

Because BCS cannot skip fields, the signed object is an explicit body type:

```rust
struct Commit { body: CommitBody, signature: Signature }
// signature covers bcs::to_bytes(&body)
```

There is no ambiguity about which bytes were signed.

### 3. Schema versioning (load-bearing)

- Field order / variant order are frozen per `schema` version.
- `schema` is **field #0** of every canonical object so a verifier dispatches on it
  first.
- Any layout change (add/remove/reorder field, reorder variant) is a schema bump
  (`statechronicle.commit.v0` → `.v1`); old versions remain parseable for
  audit/replay (§31).

### 4. Digest: SHA-256

- `sha256:<lowercase-hex>` (unchanged from §17). `ContentDigest` in
  `statechronicle-core` enforces the format.
- `state_hash` (leaf), `event_merkle_root`, `previous/next_state_root` are all
  `sha256:` digests over BCS canonical bytes.

### 5. Signature: Ed25519

- `ed25519-dalek`; commit/snapshot signatures with `key_id` reference.
- Signatures cover the BCS canonical representation of the signed body, excluding the
  `signature` field (structural envelope, §2).

### 6. Economic values (no floats, by construction)

- Integer amounts → canonical non-negative decimal integer strings (UTF-8) with an
  exact fixed-point `Amount` in core transition arithmetic (checked add/sub, fail closed
  on overflow/underflow), e.g. `"125000"` with profile-defined `unit`/scale (§10.3).
- Arbitrary precision / fixed-point → decimal strings (UTF-8, profile-defined scale),
  additive via a profile precision extension. See ADR-006 §36 Q13.
- BCS has no float encoding: floats are structurally impossible in canonical state.

### 7. Limits

- `statechronicle-core::limits` enforces bounded input sizes for
  intent/event/commit payloads (§30) before canonicalization.

### 8. Conformance

- Golden vectors generated by the Rust codec, cross-checked against BCS
  implementations in at least TypeScript and Go before v0 conformance freezes.
- A future bench (e.g. criterion, added when a bench harness is introduced) on the
  encode+hash path to validate throughput claims.

## Alternatives Considered

| Option | Pros | Cons |
| --- | --- | --- |
| **BCS (Chosen)** | Deterministic by construction; no floats structurally; ~4–8× encode+hash headroom vs JCS; 2–4× smaller canonical bytes; formal spec; multi-language verifier ecosystem (Aptos/Sui SDKs: TS/Python/Go/Java/Swift); serde-derive native; `bcs` crate actively maintained (8M downloads) | Schema-coupled (not self-describing): mitigated by `schema` field-#0 rule; integer-keyed map ordering trap: banned; non-Rust impls are Aptos-maintained: mitigated by own golden vectors |
| **RFC 8785 JCS (previous ADR)** | JSON-native; matches trustgrant | Slow (per-object key sort + number formatting on the per-event hot path); verbose; float machinery for forbidden floats; cross-language conformance trap; `serde_jcs` single-maintainer |
| **CBOR canonical (RFC 8949 §4.2.1)** | IETF standard; broad adoption | Determinism is a *profile on top of a general format*: more configuration = more interop drift; `serde_cbor` deprecated; ciborium canonical usage thin |
| **SSZ (Ethereum)** | Canonical; designed for merkleization | Coupled to Ethereum container/merkleization model; no general serde support; small ecosystem outside ETH |
| **postcard / bincode** | Fast Rust formats | Neither is a cross-language canonical standard; fail §16 portable proof bundles |

### Why BCS Wins

1. The protocol's §2.4/§32 throughput goal makes the per-event canonicalization cost the
   deciding factor; BCS makes serialization a non-bottleneck instead of a tax.
2. §10.3's no-float rule becomes a *property of the format*, not a lint.
3. Multi-language verifier support already exists because other ledgers made the same
   bet (Diem/Aptos/Sui).
4. Determinism is guaranteed by construction, eliminating the §30 "invalid
   canonicalization" failure mode.
5. Now is the cheapest moment: zero committed bytes exist.

## Consequences

**Positive:**

- ~4–8× headroom on the per-event encode+hash hot path; 2–4× smaller canonical bytes
  (smaller hash input, proofs, history).
- No float path exists; economic values are structurally integer/string.
- Cross-verifiable by any implementation with the schema + BCS codec (§16).
- Signed-envelope body type removes ambiguity about which bytes were signed.
- Matches how other high-throughput ledgers do it.

**Negative:**

- BCS is not self-describing: a verifier needs the exact schema version (the protocol's
  `schema` field model already requires this).
- Field/variant order is load-bearing; additive changes require schema bumps.
- Clients see binary, not JSON: mitigated by keeping JSON at the HTTP API.
- TrustGrant (per ADR-003) computes its own `evaluation_digest` via its own codec; those
  digests are opaque references StateChronicle never re-derives: a verifier of a
  StateChronicle proof bundle needs the StateChronicle BCS codec, and trustgrant digests
  are resolved through trustgrant's own tooling (§16.3/§29 model, unchanged).

**Mitigations:**

- `schema` = field #0 dispatch; old schema versions stay parseable.
- JSON HTTP API retained for client ergonomics; canonical BCS bytes served to verifiers
  via a dedicated media type (e.g. `application/vnd.statechronicle.bcs`).
- Golden vectors + TS/Go cross-checks before v0 freezes (a bench harness may be added
  later).

---

## Implementation Notes

```rust
// statechronicle-core
pub fn canonicalize<T: Serialize>(value: &T) -> Result<Vec<u8>, Error>; // bcs::to_bytes
pub fn digest(bytes: &[u8]) -> ContentDigest;                            // sha256:<hex>
pub fn sign(body: &CommitBody, key: &SigningKey) -> Signature;           // ed25519-dalek
pub fn verify(body: &CommitBody, key: &VerifyingKey, sig: &Signature) -> Result<(), Error>;
// verify: verify(bcs::to_bytes(&body), key, sig): signature never covers the signature field
```

Workspace changes: add `bcs = "0.2.1"`; drop `serde_jcs` from `statechronicle-core`
(keep only if the consumer's composition root needs it for trustgrant-side work).

---

## Review & Maintenance

- **Last Reviewed:** 2026-08-03
- **Next Review:** Before first production release, or when a profile requires a
  different serialization/digest/signature scheme
- **Change Log:**
  - v1.1 (2026-08-03): Baseline flipped from JSON + RFC 8785 JCS to BCS. JSON retained
    as HTTP API logical-model view. Signed-envelope body type; schema-versioning rule;
    u128 exact fixed-point economic newtypes; golden-vector conformance. Rationale: performance (§2.4/
    §32), no-float-by-construction (§10.3), cross-language verifier ecosystem, cheapest
    moment (zero committed bytes).
  - v1.0 (2026-08-03): Initial ADR locking canonicalization/hashing/signature baseline
