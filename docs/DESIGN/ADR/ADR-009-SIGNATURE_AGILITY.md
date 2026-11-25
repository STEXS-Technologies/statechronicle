# ADR-009: Versioned signatures and trusted suite selection

Status — Proposed

## Context

The v0.3 Ed25519 types are part of the stored and exported protocol. Their
BCS layout, transcript, and verification rules must remain stable. The v1
format adds a separate signature envelope and a policy-selected suite to
support external providers.

## Decision

### Wire and transcript

A typed API, storage schema, or explicitly versioned outer container selects
the decoder before reading the envelope. Existing v0 records keep their
original representation. A failed decode or verification is terminal for
the selected version.

The v1 BCS envelope has these fields in order:

| Field | Encoding |
| --- | --- |
| Envelope schema | BCS string, exactly `statechronicle.signed.v1` |
| Body | Existing canonical BCS body for the selected purpose |
| Suite ID | BCS string, 1–64 ASCII bytes |
| Key-set ID | 32 bytes, fixed array without a length prefix |
| Signature | BCS byte vector, bounded before allocation |

Suite IDs start with a lowercase ASCII letter and contain only lowercase
ASCII letters, digits, hyphens, and periods. A syntactically valid name
still requires an exact registration. Key-set IDs are opaque values assigned
by the trusted registry and permanently bound to one suite and ordered
public-key set. JSON uses the same field order when emitted, with key-set ID
as 64 lowercase hex digits and the signature as unpadded base64url with
the existing `b64u:` prefix.

Each signature covers the literal ASCII bytes
`statechronicle.signature.v1` followed by one NUL, then these fields:

1. One-byte purpose, with actor intent = 1, tenant commit = 2, and global
   checkpoint = 3.
2. The ASCII body schema string expected for that purpose, at most 128 bytes.
3. The exact suite ID expected by policy.
4. The 32 raw bytes of the key-set ID expected by policy.
5. The original canonical BCS body bytes.

Each field has a canonical minimal ULEB128 byte-length prefix, including
the one-byte purpose and the fixed-length key-set ID. The caller selects the
purpose and expected body schema through the typed protocol entry point.
The kernel checks those expectations and constructs the transcript in one
place. Providers receive the complete transcript.

A layout-only vector is below. It uses tenant commit purpose, schema
`statechronicle.commit.v0`, suite `ed25519`, key-set bytes 00 through 1f,
and a synthetic body consisting of byte 2a.

```text
73746174656368726f6e69636c652e7369676e61747572652e763100
0102
1873746174656368726f6e69636c652e636f6d6d69742e7630
0765643235353139
20000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
012a
```

### Suite definitions

| Suite ID | Mode | Signature bytes |
| --- | --- | --- |
| `ed25519` | Pure Ed25519 with existing strict verification | 64 |
| `ml-dsa-65` | FIPS 204 pure ML-DSA-65, empty algorithm context | 3309 |
| `slh-dsa-sha2-128s` | FIPS 205 pure SLH-DSA-SHA2-128s, empty algorithm context | 7856 |
| `hybrid-ed25519-ml-dsa-65.v1` | Ordered Ed25519 and ML-DSA-65 over the same suite-bound transcript | 3380 |

The hybrid payload has a version byte 1, a count byte 2, and two records in
fixed order. Each record is a one-byte member tag, a minimal ULEB128 byte
length, and the raw signature. Tag 1 selects Ed25519 with length 64 and
tag 2 selects ML-DSA-65 with length 3309. The complete composite is parsed
and checked before any provider invocation. Verification requires both
providers to accept under the corresponding pinned keys. Partial signer
output is discarded on error. External provider support remains optional.

### Trust and work limits

The verifier obtains the expected format, suite, and key-set identity from
trusted policy scoped to tenant or global authority, subject or service
role, operation, and verification purpose. The key record binds validity
and lifecycle, each member identity, and the allowed historical policy.
Recovery pins the retained record and its applicable trusted history.
Retirement prevents new signing. Historical acceptance of retired or revoked
keys requires an explicit policy decision. Verification resolves policy
before invocation of any crypto provider.

Initial hard caps are 16 KiB for a v1 signature, 4 KiB per public key, 8 KiB
for an encoded key set, two crypto operations per envelope, and 32 enabled
suite registrations per policy context. Narrower adapter and deployment
limits may apply. Existing protocol limits govern body and proof semantics.
A v1 reader also caps the full record before decoding and applies cumulative
proof and batch verification limits before traversing embedded records.
Signing adapters enforce configured timeouts and concurrency bounds for
remote providers. On timeout or partial failure the caller has no usable
new envelope.

Check outer format and bounds first, then typed structure and trusted context
and metadata, then signature encoding and crypto, then authoritative
admission or claim checks under the existing invariants. Cryptographic
success alone grants no authority. Provider errors propagate without a
different-suite attempt.

Commit references in new proof formats bind the digest of the entire
canonical signed commit envelope under its explicit format. Proof
verification also checks the expected claim or head and validates each
referenced commit against the policy for that individual commit. Existing
Merkle paths and state hashing are reused.

## Alternatives and consequences

In-place changes to the legacy signature block would break positional BCS
encoding. Guessing the version from decoder behavior would leave the accepted
format dependent on untrusted bytes. Explicit version dispatch requires
read support before enabling new writes and preserves archived v0 evidence.

Review still needs to confirm independent envelope and hybrid vectors, the
exact full-envelope and cumulative proof budgets, and the downgrade and
substitution analysis before these bytes become a compatibility contract.
