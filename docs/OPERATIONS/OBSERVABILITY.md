# StateChronicle observability and alert contract

This document is the minimum dashboard and on-call contract for a deployment
that accepts player mutations. The library emits privacy-safe mutation
classifications through `statechronicle_ports::observability::MetricsSink`;
the composition root must export them to the monitoring system and attach the
database/outbox gauges below.

Projection rebuild workers should derive the `projection lag` panel from
`statechronicle_index::rebuild::rebuild_progress(total_events, next_event)`.
The helper rejects a checkpoint beyond the verified canonical stream so a
corrupt recovery marker cannot be reported as caught up.

## Required dashboard panels

| Signal | Required labels | What it answers |
|---|---|---|
| mutation outcome rate | tenant (pseudonymous), operation, outcome | Are commands being accepted, replayed, rejected, or failing transiently? |
| mutation latency | operation, outcome | Are hot inventory/market keys approaching the request SLO? |
| canonical head sequence | tenant | Is durable history advancing? |
| projection lag | tenant, projection | Are reads behind the canonical head? |
| outbox pending depth and oldest age | tenant | Are notifications or downstream consumers stuck? |
| database conflicts/timeouts | tenant, error class | Are retries bounded and safe? |
| key/authorization failures | operation, failure class | Is abuse or key compromise investigation needed? |
| rate-limit denials | dimension, operation | Is one account or tenant exhausting ingress capacity? |
| distributed quota provider health | provider, failure class | Is shared quota enforcement available, or are protected commands failing closed? |

Never export raw intent payloads, signatures, inventory contents, or stable
player identifiers. Hash or otherwise pseudonymize tenant/actor labels at the
composition root, and cap metric cardinality.

## Initial alert policy

Tune thresholds with production traffic, then record the approved values in
the service SLO document. The initial paging rules are:

1. Page when canonical-head advancement is zero for two consecutive command
   windows while mutation traffic is non-zero.
2. Page when retryable transaction failures or lock timeouts exceed 5% for
   five minutes, or when the outbox oldest age exceeds the delivery SLO.
3. Page when projection lag exceeds the read SLO or integrity verification
   fails. Stop writes on integrity failure and follow `RECOVERY.md`.
4. Page security on a sustained authorization/key-failure spike or a sudden
   increase in rate-limit denials; do not automatically disable the limits.

## Drill procedure

For every release, perform one controlled drill in a non-production tenant:

1. Force a broker outage and verify commits remain durable, pending depth
   rises, retries retain ownership, and delivery resumes after recovery.
2. Force a database lock/serialization conflict and verify the API returns a
   retryable result with the same idempotency key, without duplicate events.
3. Delete a derived projection, run the verified canonical rebuild, and
   confirm projection lag returns to zero.
4. Trigger a synthetic authorization failure and confirm the security page
   includes tenant/operation classification without raw payload data.

Record the timestamp, dashboard links, owner, observed recovery time, and any
threshold changes. A green unit-test run is not a substitute for this drill.
