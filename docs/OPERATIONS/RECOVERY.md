# StateChronicle recovery runbook

This runbook applies to applications implementing the durable ledger ports. It
is not a substitute for provider backup guarantees or a game-specific incident
plan.

## Authoritative data

Back up the application-owned durable store together with its schema version,
application commit, profile definitions, tenant and key registry records, and
configuration. The authoritative ledger is the immutable events, signed
commits, canonical heads, idempotency records, and immutable original mutation
effects including outbox rows needed for verified recovery. Current projections, trade/proof indexes and
caches can be rebuilt; delivery leases are operational state. Delivering or
compacting a queue must not erase the original outbox evidence required by
`LedgerRecovery` or complete batch recovery.

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

## Ambiguous mutation decisions

Retain the original tenant/intent ID, canonical payload digest, exact request,
signed plan where available, and independently trusted history context before
submitting a write. A request timeout or cancellation is not proof of rollback.
Do not dispatch notifications, create a replacement identity or re-reduce a new
plan to decide what happened to the original.

| Observation | Action |
|---|---|
| Checked `Committed` or `Replay` receipt | Use the original record; permit post-commit delivery under the outbox contract |
| `Unknown`, lost ACK, failed rollback or uncertain finalized readback | Reconcile the original identity through authoritative recovery; preserve uncertainty |
| `InProgress` | Preserve identity, establish owner/fence status; lease expiry alone proves no rollback |
| `NotFound` | Treat as one read point; establish that no outstanding writer can finalize before deciding an exact-identity retry |
| Different payload under the same identity | Refuse; investigate the identity collision, do not overwrite |
| Authorization/signer-policy refusal during lookup | Restore authorized recovery access or escalate under host policy; previous outcome remains unresolved |
| Storage/read unavailable | Retain uncertainty and reconcile when authority is available |
| Missing/mismatched original effects, bad root/signature or unexpected tip | Stop affected writes, preserve evidence, repair or restore from independently verified history/backup |

Use `mutation::recover` for single mutations or `SignedBatch::recover` /
`recover_batch` for the entire original group. Recovery must read the same
atomic authority as the write and apply current access policy plus independently
selected signer trust. It does not rerun reduction or create a reservation.
Never recover a batch as a set of independent successful singles. Unsupported
atomic scope must refuse before partial persistence.

An exact retry can replay a confirmed original mutation. If an attempt is known
to have rolled back before commit, retry policy may reuse its exact identity and
plan subject to current authorization and concurrency. An earlier unknown result
cannot be downgraded by a later pre-commit refusal. Where definitive resolution
is unavailable, keep the operation unresolved and reconcile; elapsed time alone
is not evidence.

For the bounded [file tutorial](../../tutorials/durable-custom-ledger/README.md),
release every handle before exclusive reopen and permit only independently
retained old/pending tips. A process death leaves a stale lock intentionally.
Establish that no writer remains, preserve the directory, validate the selected
history and reconcile scratch files before operator removal of a stale lock.
This is example-specific procedure, not a generic database crash guarantee.

## Integrity failure

Keep writes frozen. Preserve the original store and logs for forensics; do not
delete or rewrite an event or signed commit. Compare replicas and backups,
verify signatures with independently trusted historical tenant/key/time/revocation
policy, and identify the first
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
Never rewrite prior signed history. Select an explicit historical authorization
policy for the compromised key and incident interval; a valid signature under
the old key alone does not establish authorized history. Compare independently
retained trusted tips and backups. Use the replacement for newly authorized
commits only after policy and recovery checks.
