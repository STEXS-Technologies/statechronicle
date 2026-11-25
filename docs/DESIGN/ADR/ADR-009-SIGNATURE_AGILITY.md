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
original representation. Mixed-version binary containers introduced by this
ADR prefix each record with one byte, 0x00 for legacy BCS or 0x01 for v1 BCS,
followed by the entire record. A standalone legacy record keeps its original
bytes and is read only through a legacy entry point. Unknown discriminator
values, incomplete records, trailing bytes, and any decode or verification
failure terminate processing of the selected record. The container tag
selects a decoder, and independently trusted policy determines whether the
selected format is permitted.

The v1 BCS envelope has these fields in order:

| Field | Encoding |
| --- | --- |
| Envelope schema | BCS string, exactly `statechronicle.signed.v1` |
| Body | BCS byte vector containing the canonical BCS encoding of the body for the selected purpose |
| Suite ID | BCS string, 1–64 ASCII bytes |
| Key-set ID | 32 bytes, fixed array without a length prefix |
| Signature | BCS byte vector, bounded before allocation |

Suite IDs start with a lowercase ASCII letter and contain only lowercase
ASCII letters, digits, hyphens, and periods. A syntactically valid name
still requires an exact registration. Key-set IDs are opaque values assigned
by the trusted registry and permanently bound to one suite and ordered
public-key set.

Both BCS vectors use minimal ULEB128 length prefixes. The reader checks the
full record size before decoding, then checks each vector's declared
length against the applicable limit and the remaining input before
allocating or copying. The typed protocol entry point decodes the body
with its purpose-specific codec and requires its canonical re-encoding
to match the enclosed body bytes.

JSON uses a single object with `envelope_schema`, `body`, and
`signature` fields. The body is the existing typed JSON object for that
purpose. The signature object contains `suite_id`, `key_set_id`, and
`signature_bytes`. The key-set ID is exactly 64 lowercase hex digits.
The signature is unpadded base64url with the existing `b64u:` prefix.
The envelope and signature objects reject duplicate, unknown, and missing
fields. Duplicate body fields are rejected at every nesting depth. JSON member order and insignificant whitespace have no effect
on the signed bytes. The reader caps the full JSON record at six times `MAX_COMMIT_BYTES` plus
64 KiB before parsing. The base64url length and canonical signature encoding
are checked before provider dispatch. The typed codec applies the
purpose-specific binary body cap. The transcript
uses the canonical binary body from the typed JSON body.

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

### Actor-intent body codec

A v1 actor intent uses the transcript body schema `statechronicle.intent.v1`.
The binary body follows the existing intent field order and field encodings
except for the schema string and the values in the `inputs` map. The
JSON envelope uses the same logical intent fields, with the v1 schema
string and normal JSON values in `inputs`.

The existing intent stores `serde_json::Value` in `inputs`. Its legacy BCS
serializer emits values without type tags, and its JSON value decoder
requires self-describing input. Thus v1 needs an explicit value encoding
to verify and decode nonempty intent inputs without ambiguity. Keep the
legacy codec for legacy signatures and existing v0 identity calculations.

| BCS variant index | Value | Variant payload |
| --- | --- | --- |
| 0 | Null | None |
| 1 | Boolean | BCS bool |
| 2 | Negative integer | BCS i64, strictly less than zero |
| 3 | Nonnegative integer | BCS u64 |
| 4 | String | BCS string |
| 5 | Array | BCS vector of these values |
| 6 | Object | BCS map from string to these values |

Variant indexes use minimal ULEB128. Floats are excluded. Integers
outside the i64/u64 union are excluded. Map keys follow the existing BCS
canonical ordering and must be unique. Arrays preserve order. The body
codec enforces the intent byte and input-nesting limits. After decoding
and checking canonical bytes, conversion to the existing typed intent and
existing structural validation precede execution. The format-specific
evidence must be retained separately from the converted intent so legacy
signature verification cannot be used as a fallback.

### Suite definitions

| Suite ID | Mode | Signature bytes |
| --- | --- | --- |
| `ed25519` | Pure Ed25519 with existing strict verification | 64 |
| `ml-dsa-65` | FIPS 204 pure ML-DSA-65, empty algorithm context | 3309 |
| `slh-dsa-sha2-128s` | FIPS 205 pure SLH-DSA-SHA2-128s, empty algorithm context | 7856 |
| `hybrid-ed25519-ml-dsa-65.v1` | Ordered Ed25519 and ML-DSA-65 over the same suite-bound transcript | 3380 |

The suite-specific signature length is checked before provider invocation.
The hybrid payload occupies the envelope signature byte vector. It has a
version byte 1, a count byte 2, and two records in fixed order. Each record is a one-byte member tag, a minimal ULEB128 byte
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

### Durable retention

The ledger retains the exact actor-intent and commit envelopes in the same
fenced transaction as the claim, canonical head, events, projections, and
outbox. A v1 claim uses the digest of the canonical v1 actor-intent body.
Both formats share the tenant and intent identity namespace. Lookup reports
an existing row in another format explicitly, without treating it as an
absent row or creating a new reservation. A member of an atomic batch is
recovered only through the complete group record.

A legacy-only adapter refuses the v1 append capability. Before returning a
receipt, the durable owner verifies the independently retained signed bytes
and all original effects through authoritative readback. Unknown
acknowledgment requires original identity reconciliation.

## Alternatives and consequences

In-place changes to the legacy signature block would break positional BCS
encoding. Guessing the version from decoder behavior would leave the accepted
format dependent on untrusted bytes. Explicit version dispatch requires
read support before enabling new writes and preserves archived v0 evidence.

Conformance covers independent envelope vectors for each purpose and suite,
the exact length limit and one byte over, non-minimal and truncated lengths,
trailing bytes, unknown versions, wrong format for trusted policy, and
rejection of malformed v1 records without calling a legacy decoder. It
also covers substitutions of suite, key-set identity, purpose, or schema
and reordered, partial, and duplicated hybrid components. Review still
needs to confirm these vectors and the full-envelope and cumulative proof
budgets before these bytes become a compatibility contract.
