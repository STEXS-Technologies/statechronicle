# StateChronicle recovery runbook

This runbook applies to deployments using the durable ledger contract. It is
not a substitute for database-provider backup guarantees or a game-specific
incident plan.

## Authoritative data

Back up the SQLite/database file (or the equivalent server schema) together
with the schema version, application commit, profile definitions, tenant/key
registry records, and configuration. The authoritative ledger is the immutable
events, signed commits, canonical heads, and idempotency records. Projections,
trade indexes, proof indexes, caches, and outbox delivery leases are derived.

Backups must be encrypted, access-controlled, retained according to the
service policy, and restored periodically into an isolated account. Record the
backup timestamp, database WAL/checkpoint state, canonical head per tenant,
and a checksum outside the backup itself.

For SQLite, create the source artifact with
`SqliteLedgerStore::backup_to(destination)`. It verifies every tenant before
and after the transactional snapshot; the destination must be a new path.
For PostgreSQL, use provider-native encrypted backups (`pg_basebackup`, WAL
archiving, or the managed equivalent), then run
`cargo run -p statechronicle-postgres --example verify_integrity` with the
restored connection string before promotion. The command enumerates every
tenant and fails closed on chain, event, projection, or outbox corruption.

## Startup and restore procedure

1. Freeze player mutations and stop outbox workers.
2. Restore a verified backup into an isolated database; never overwrite the
   live database during the first validation pass.
3. Run `cargo run -p statechronicle-sqlite --example verify_integrity --
   <db-path>`. This enumerates scopes and calls
   `SqliteLedgerStore::verify_all_integrity` for every tenant. A malformed
   commit, chain discontinuity, canonical-head mismatch, or unbound/orphan
   event is a hard stop requiring investigation.
4. Compare each restored canonical head (commit ID, sequence, and root) with
   the selected recovery point. A stale backup must not replace a newer head.
5. Rebuild derived projections/indexes with
   `SqliteLedgerStore::rebuild_projections_from_canonical`; it verifies the
   canonical stream before replay and persists restartable checkpoints. During
   long rebuilds, expose `projection_rebuild_progress` as the lag gauge and
   stop promotion if it reports an invalid checkpoint. After verification and
   promotion, call `clear_rebuild_checkpoint_for_tenant` to bound recovery
   metadata without affecting another tenant using the same operator key.
   Compare resulting roots and versions with the signed commit metadata.
   For a bounded operator command, use the `rebuild_projections` example;
   it refuses promotion unless the final progress is caught up and integrity
   verification succeeds, then clears the checkpoint.
6. Record the recovery point and verification report, then atomically promote
   the restored database. Resume workers only after durable writes are enabled.
7. Reconcile pending outbox rows; consumers must deduplicate by delivery key.

For PostgreSQL, run
`STATECHRONICLE_POSTGRES_URL='...' cargo run -p statechronicle-postgres --example rebuild_projections -- <tenant>`
after the integrity gate. The command verifies the tenant before invoking the
head-locked rebuild and exits nonzero on any integrity or projection conflict.

## Integrity failure

Keep writes frozen. Preserve the original database and logs for forensics;
do not delete or rewrite an event or signed commit. Compare replicas/backups,
verify signatures with the historical key registry, and identify the first
divergent sequence. Restore the last verified canonical point, replay forward
from immutable events, and obtain security/operations approval before
unfreezing writes.

## Outbox and projection incidents

- Broker outage: keep accepting durable ledger commits if database health is
  good; run bounded `outbox::dispatch_until_idle` passes after broker recovery.
- Poison payload: use `PoisonPolicy::Quarantine` (and the adapter's durable
  quarantine state) for a digest-invalid row, retaining its delivery key and
  error for forensics, then page an owner. `PoisonPolicy::Retry` is appropriate
  only for transient publisher failures. Never mark an unverified payload
  delivered.
- Projection corruption: stop commands that require the affected projection,
  rebuild from events, verify the resulting root/version, then resume reads.

## Signer compromise

Immediately freeze commits and player mutations, revoke the compromised key in
the key registry, preserve audit logs, and generate an audited replacement.
Never rewrite prior signed history. Verify old commits with the historical key,
use the replacement only for new commits, and communicate the affected
recovery point to clients/verifiers.

## Game-day evidence

Record restore duration (RTO), the oldest accepted event at recovery (RPO),
all verification output, projection rebuild duration, outbox backlog/age, and
the names of approving backend, security, and operations reviewers. Repeat at
least quarterly and after schema/profile migrations.
