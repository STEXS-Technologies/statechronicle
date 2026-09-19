# StateChronicle recovery runbook

This runbook applies to applications implementing the durable ledger ports. It
is not a substitute for provider backup guarantees or a game-specific incident
plan.

## Authoritative data

Back up the application-owned durable store together with its schema version,
application commit, profile definitions, tenant and key registry records, and
configuration. The authoritative ledger is the immutable events, signed
commits, canonical heads, and idempotency records. Projections, trade indexes,
proof indexes, caches, and outbox delivery leases are derived.

Backups must be encrypted, access-controlled, retained according to service
policy, and restored periodically into an isolated account. Record the backup
timestamp, canonical head per tenant, and a checksum outside the backup.

## Startup and restore procedure

1. Freeze player mutations and stop outbox workers.
2. Restore a verified backup into an isolated environment. Never overwrite the
   live store during the first validation pass.
3. Use the application's integrity verifier to enumerate every tenant and fail
   closed on malformed commits, chain discontinuities, canonical-head mismatch,
   or orphaned events.
4. Compare each restored canonical head with the selected recovery point. A
   stale backup must not replace a newer head.
5. Rebuild derived projections and indexes from the canonical event stream.
   Persist restartable checkpoints and stop promotion on an invalid checkpoint.
6. Record the recovery point and verification report, then atomically promote
   the restored store. Resume workers only after durable writes are enabled.
7. Reconcile pending outbox rows. Consumers must deduplicate by delivery key.

## Integrity failure

Keep writes frozen. Preserve the original store and logs for forensics; do not
delete or rewrite an event or signed commit. Compare replicas and backups,
verify signatures with the historical key registry, and identify the first
divergent sequence. Restore the last verified canonical point, replay forward
from immutable events, and obtain security and operations approval before
unfreezing writes.

## Outbox and projection incidents

- Broker outage: keep accepting durable commits if storage health is good, then
  run bounded dispatch passes after broker recovery.
- Poison payload: quarantine digest-invalid rows while retaining their delivery
  key and error for forensics. Never mark an unverified payload delivered.
- Projection corruption: stop commands that require the affected projection,
  rebuild from events, verify the resulting root and version, then resume reads.

## Signer compromise

Immediately freeze commits and player mutations, revoke the compromised key in
the key registry, preserve audit logs, and generate an audited replacement.
Never rewrite prior signed history. Verify old commits with the historical key
and use the replacement only for new commits.
