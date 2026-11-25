# Migrate from v0.3 signatures to v1 envelopes

StateChronicle 0.4 adds a versioned signature format. Existing Ed25519
callers can continue using the v0 APIs and their existing records.
The writer chooses a format through deployment policy. Readers select
their decoder from the endpoint or retained discriminator before
processing a record.

## Deploy readers first

1. Keep existing v0 records byte for byte, including historical signatures,
   proof references, and commit IDs. Preserve the legacy reader and its
   historical trust policy for every legacy record still in scope.
2. Implement exact v1 retention and lookup. Keep original signed envelopes
   and finalized identities in the same transaction as the head, events,
   projections, and outbox. Preserve one tenant/intent namespace across
   formats. A finalized record of another format or a batch member is a
   distinct lookup outcome and cannot be treated as absent.
3. Qualify the adapter with fresh handle recovery, cancellation, lost
   acknowledgments, concurrent claims, head conflicts, and readback after
   a later write. Atomicity and storage durability are adapter guarantees.
4. Register the exact suite, key-set identity, provider, and signature size
   used for each permitted tenant, authority, operation, and purpose.
   Enable current and historical readers before changing a writer.

Mixed binary containers use 0x00 for legacy and 0x01 for v1 before the
complete standalone record. A typed storage schema can supply the format
instead. Unknown versions and signature failures terminate verification.
Normalize v1 JSON transport to the canonical binary envelope before
verification and retention. Binary byte identity is the commit pin.

## Connect v1 signing to policy

Keep the existing `CommitSigner`, `sign_commit`, and `verify_commit`
paths for v0. V1 providers receive the complete library-built transcript
and caller-owned buffers or slices. For a remote key service, use
`AsyncSigner` with an adapter deadline and concurrency limit. Treat an
incomplete write or timeout as a signing refusal.

Resolve `SignaturePolicy` from a configured authority or authenticated
actor, operation, verification mode, and trusted time. Select policy
independently of record-supplied suite and key-set identifiers. The
resolved format and immutable key set are the only eligible configuration.
A rotated key set receives a new identity. The reserved hybrid suite uses
the concrete hybrid verifier and its two pinned member keys.

New signatures and active requests require current eligibility and a
valid signing time. Historical readers use independently pinned evidence
and explicit policy for the original epoch. Retirement stops new signing.
Historical eligibility for retired or revoked keys must be encoded in
deployment policy. Revocation defaults to historical refusal.

## Write, recover, and roll back

Prepare requests using the v1 preparation entry points. Keep the signed
plan with its exact actor and commit records until the ledger confirms
its original effects. A batch is one ordered group. Recover it through
the batch interface instead of retrying individual members.

A successful transaction still requires authoritative readback. If the
result is unknown, keep the original tenant, intent identity, exact signed
bytes, and ordered membership. Use read-only recovery. A temporary lookup
failure or not-found observation does not prove rollback. Never sign
replacement evidence to infer whether the original write succeeded.

Before the first v1 write, the deployment can switch the writer back to
v0 policy. After any v1 write, readers that may encounter it must continue
to support v1 for recovery, proofs, and downstream processing. Rollback
to a v0-only binary requires an independently verified cutover boundary
outside all v1 history. Preserve original records and any newer attestation
alongside them. Re-signing old history does not make the original signatures
post-quantum.

Current claims require the authoritative canonical head to remain the
same through verification. Historical claims use the independently
selected signed commit. For portable v1 verification, the caller supplies
trusted policy, a canonical transcript, and a suitable provider. The
legacy portable Ed25519 verifier continues to use fixed-size keys and signatures.

See [ADR-009](../DESIGN/ADR/ADR-009-SIGNATURE_AGILITY.md) for format and
transcript details and [the v0.3 baseline](../../ci/wire-0.3/README.md)
for compatibility fixtures and measurements.
