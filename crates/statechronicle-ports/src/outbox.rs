//! Durable outbox delivery port.
//!
//! Outbox rows are inserted by [`crate::ledger_store::LedgerTransaction`] and
//! delivered only after the ledger commit succeeds.  Consumers must make
//! delivery idempotent using `delivery_key`; broker availability must never
//! decide whether a ledger mutation commits.

use async_trait::async_trait;
use statechronicle_core::digest::{ContentDigest, hash_bytes};
use statechronicle_domain::ids::CommitId;
use statechronicle_domain::tenant::TenantId;
use std::time::Duration;
use thiserror::Error;

/// A claimed outbox item ready for publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboxPayload {
    /// An opaque canonical notification body. The consumer chooses the wire
    /// envelope; the ledger stores and leases it without interpreting it.
    Opaque {
        tenant: TenantId,
        commit_id: CommitId,
        payload_digest: ContentDigest,
        payload: Vec<u8>,
    },
}

/// Errors produced by outbox adapters.
#[derive(Debug, Error)]
pub enum OutboxError {
    /// The item is already delivered or leased by another worker.
    #[error("outbox item is unavailable: {0}")]
    Unavailable(String),
    /// The item failed integrity or ownership checks.
    #[error("outbox item is corrupt: {0}")]
    Corrupt(String),
}

impl OutboxError {
    /// Returns whether a worker may retry the operation after backoff.
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

/// Durable, lease-based outbox worker port.
#[async_trait]
pub trait OutboxStore: Send + Sync {
    /// Claims up to `limit` pending records for `worker_id` until the lease
    /// timestamp. Claims are atomic and recoverable after worker death.
    async fn claim(
        &self,
        worker_id: &str,
        limit: usize,
        lease_until_unix: i64,
    ) -> Result<Vec<(String, OutboxPayload)>, OutboxError>;

    /// Marks a delivery key complete. Repeating this call is idempotent.
    ///
    /// This legacy method has no worker identity and therefore cannot prevent
    /// a stale worker from acknowledging a lease taken over by another
    /// worker. Production dispatchers must call [`Self::mark_delivered_by`].
    #[deprecated(note = "use mark_delivered_by to enforce lease ownership")]
    async fn mark_delivered(&self, delivery_key: &str) -> Result<(), OutboxError> {
        let _ = delivery_key;
        Err(OutboxError::Unavailable(String::from(
            "ownerless delivery completion is disabled; use mark_delivered_by",
        )))
    }

    /// Marks a delivery complete only when owned by `worker_id`.
    ///
    /// The default fails closed so a legacy adapter cannot acknowledge a
    /// stale worker's delivery accidentally.
    async fn mark_delivered_by(
        &self,
        delivery_key: &str,
        _worker_id: &str,
    ) -> Result<(), OutboxError> {
        let _ = delivery_key;
        Err(OutboxError::Unavailable(String::from(
            "ownership-aware delivery completion is not supported by this adapter",
        )))
    }

    /// Releases a failed claim for retry, optionally recording a bounded error.
    ///
    /// This legacy method has no worker identity. Production dispatchers must
    /// call [`Self::release_by`] so lease ownership is checked atomically.
    #[deprecated(note = "use release_by to enforce lease ownership")]
    async fn release(&self, delivery_key: &str, error: &str) -> Result<(), OutboxError> {
        let _ = (delivery_key, error);
        Err(OutboxError::Unavailable(String::from(
            "ownerless delivery release is disabled; use release_by",
        )))
    }

    /// Releases a claim only when owned by `worker_id`.
    async fn release_by(
        &self,
        delivery_key: &str,
        _worker_id: &str,
        _error: &str,
    ) -> Result<(), OutboxError> {
        let _ = delivery_key;
        Err(OutboxError::Unavailable(String::from(
            "ownership-aware delivery release is not supported by this adapter",
        )))
    }

    /// Quarantines a permanently corrupt/poisoned delivery so workers do not
    /// retry it forever. Adapters should retain the payload and error for
    /// operator inspection. The default falls back to release for backwards
    /// compatibility. Production dispatchers must call
    /// [`Self::quarantine_by`] so lease ownership is checked atomically.
    #[deprecated(note = "use quarantine_by to enforce lease ownership")]
    #[allow(deprecated)]
    async fn quarantine(&self, delivery_key: &str, error: &str) -> Result<(), OutboxError> {
        self.release(delivery_key, error).await
    }

    /// Quarantines a row only while its lease is owned by `worker_id`.
    async fn quarantine_by(
        &self,
        delivery_key: &str,
        _worker_id: &str,
        _error: &str,
    ) -> Result<(), OutboxError> {
        let _ = delivery_key;
        Err(OutboxError::Unavailable(String::from(
            "ownership-aware quarantine is not supported by this adapter",
        )))
    }

    /// Returns pending count for operational readiness checks.
    async fn pending_count(&self, tenant: Option<&TenantId>) -> Result<u64, OutboxError>;
}

/// Broker/publisher adapter used by [`dispatch_once`].
#[async_trait]
pub trait OutboxPublisher: Send + Sync {
    /// Publishes one canonical payload. The delivery key is the idempotency
    /// key consumers should use for deduplication.
    async fn publish(&self, delivery_key: &str, payload: &OutboxPayload) -> Result<(), String>;
}

/// Result of claiming a consumer delivery key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumerDeliveryClaim {
    /// The caller owns a new attempt identified by `attempt_id`.
    New { attempt_id: String },
    /// The delivery key was already applied successfully.
    AlreadyApplied,
    /// Another consumer currently owns the lease.
    InProgress {
        attempt_id: String,
        lease_expires_at_unix: i64,
    },
}

/// Durable deduplication record for a downstream consumer.
///
/// The implementation must store the claim and its final `Applied` state in
/// the same transaction as the consumer's business-side effect whenever the
/// effect is persisted locally. If the effect is external, the external API
/// must itself accept `delivery_key` as an idempotency key; this port alone
/// cannot make an uncoordinated side effect exactly once across a crash.
#[async_trait]
pub trait ConsumerDedupStore: Send + Sync {
    /// Atomically claims a delivery key or returns its prior state.
    async fn claim_delivery(
        &self,
        delivery_key: &str,
        lease_until_unix: i64,
    ) -> Result<ConsumerDeliveryClaim, OutboxError>;

    /// Marks a claimed delivery as applied. Repeating this call is safe.
    async fn mark_delivery_applied(
        &self,
        delivery_key: &str,
        attempt_id: &str,
    ) -> Result<(), OutboxError>;

    /// Releases a failed claim so it can be retried.
    async fn release_delivery(
        &self,
        delivery_key: &str,
        attempt_id: &str,
        error: &str,
    ) -> Result<(), OutboxError>;
}

/// Consumer callback used by [`consume_once`].
#[async_trait]
pub trait OutboxConsumer: Send + Sync {
    /// Applies one delivery. The callback should be idempotent when its state
    /// is not committed together with the deduplication record.
    async fn apply(&self, delivery_key: &str, payload: &OutboxPayload) -> Result<(), String>;
}

/// Claims and applies one delivery with durable consumer deduplication.
///
/// A duplicate already marked applied is acknowledged without invoking the
/// callback. Failures release the lease and return a retryable error. A crash
/// after the callback and before `mark_delivery_applied` may invoke the
/// callback again after lease expiry, so the callback must share a database
/// transaction with the dedup store or use the delivery key at its own side
/// effect boundary.
///
/// # Errors
///
/// Returns [`OutboxError::Unavailable`] when the key is invalid, another
/// attempt owns the lease, the callback fails, or the deduplication store
/// cannot record the outcome.
pub async fn consume_once(
    dedup: &dyn ConsumerDedupStore,
    consumer: &dyn OutboxConsumer,
    delivery_key: &str,
    payload: &OutboxPayload,
    lease_until_unix: i64,
) -> Result<bool, OutboxError> {
    if delivery_key.is_empty() {
        return Err(OutboxError::Unavailable(String::from(
            "consumer delivery key must not be empty",
        )));
    }
    if lease_until_unix <= chrono::Utc::now().timestamp() {
        return Err(OutboxError::Unavailable(String::from(
            "consumer lease expiration must be in the future",
        )));
    }
    let claim = dedup.claim_delivery(delivery_key, lease_until_unix).await?;
    let attempt_id = match claim {
        ConsumerDeliveryClaim::AlreadyApplied => return Ok(false),
        ConsumerDeliveryClaim::InProgress { .. } => {
            return Err(OutboxError::Unavailable(String::from(
                "consumer delivery is already in progress",
            )));
        }
        ConsumerDeliveryClaim::New { attempt_id } => attempt_id,
    };
    if let Err(error) = consumer.apply(delivery_key, payload).await {
        dedup
            .release_delivery(delivery_key, &attempt_id, &error)
            .await?;
        return Err(OutboxError::Unavailable(error));
    }
    dedup
        .mark_delivery_applied(delivery_key, &attempt_id)
        .await?;
    Ok(true)
}

/// Outcome of one bounded outbox worker pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DispatchReport {
    /// Number of records successfully published and marked delivered.
    pub delivered: u64,
    /// Number released for retry after publisher failure.
    pub retried: u64,
    /// Number of permanently poisoned records quarantined.
    pub quarantined: u64,
}

/// Policy for handling a payload that fails integrity verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoisonPolicy {
    /// Release the claim and retry, preserving the legacy behavior.
    Retry,
    /// Remove the row from the pending queue while retaining it for review.
    Quarantine,
}

/// Claims, verifies, publishes, and completes one bounded batch.
///
/// A publisher failure never marks a row delivered: the row is released for a
/// later retry. A successful publish followed by a completion race is safe as
/// long as consumers deduplicate by delivery key.
///
/// # Errors
///
/// Returns [`OutboxError::Unavailable`] when claiming, completing, or
/// releasing a row fails, and [`OutboxError::Corrupt`] when a claimed payload
/// no longer matches its stored digest.
pub async fn dispatch_once(
    store: &dyn OutboxStore,
    publisher: &dyn OutboxPublisher,
    worker_id: &str,
    limit: usize,
    lease_until_unix: i64,
) -> Result<DispatchReport, OutboxError> {
    dispatch_once_with_policy(
        store,
        publisher,
        worker_id,
        limit,
        lease_until_unix,
        PoisonPolicy::Retry,
    )
    .await
}

/// Variant of [`dispatch_once`] with explicit poison-message handling.
///
/// # Errors
///
/// Returns [`OutboxError::Unavailable`] for storage/lease failures and
/// [`OutboxError::Corrupt`] after a corrupt row has been released or
/// quarantined.
pub async fn dispatch_once_with_policy(
    store: &dyn OutboxStore,
    publisher: &dyn OutboxPublisher,
    worker_id: &str,
    limit: usize,
    lease_until_unix: i64,
    poison_policy: PoisonPolicy,
) -> Result<DispatchReport, OutboxError> {
    let claimed = store.claim(worker_id, limit, lease_until_unix).await?;
    let mut report = DispatchReport::default();
    for (delivery_key, payload) in claimed {
        let (expected, bytes) = match &payload {
            OutboxPayload::Opaque {
                payload_digest,
                payload,
                ..
            } => (payload_digest, payload),
        };
        if hash_bytes(bytes) != *expected {
            match poison_policy {
                PoisonPolicy::Retry => {
                    store
                        .release_by(&delivery_key, worker_id, "payload digest mismatch")
                        .await?;
                }
                PoisonPolicy::Quarantine => {
                    store
                        .quarantine_by(&delivery_key, worker_id, "payload digest mismatch")
                        .await?;
                    report.quarantined = report.quarantined.saturating_add(1);
                    continue;
                }
            }
            return Err(OutboxError::Corrupt(format!(
                "payload digest mismatch for `{delivery_key}`"
            )));
        }
        match publisher.publish(&delivery_key, &payload).await {
            Ok(()) => {
                store.mark_delivered_by(&delivery_key, worker_id).await?;
                report.delivered = report.delivered.saturating_add(1);
            }
            Err(error) => {
                store.release_by(&delivery_key, worker_id, &error).await?;
                report.retried = report.retried.saturating_add(1);
            }
        }
    }
    Ok(report)
}

/// Drains currently available outbox rows in bounded passes.
///
/// This helper is suitable for a worker tick or shutdown drain. It never
/// loops forever: `max_passes` bounds work even when an adapter repeatedly
/// returns the same leased rows. A long-running worker should call this from
/// its own supervisor with a delay/backoff between ticks.
///
/// # Errors
///
/// Returns [`OutboxError::Unavailable`] for invalid bounds or any claim,
/// publish, release, or completion error; returns [`OutboxError::Corrupt`]
/// when a claimed payload fails digest verification.
pub async fn dispatch_until_idle(
    store: &dyn OutboxStore,
    publisher: &dyn OutboxPublisher,
    worker_id: &str,
    limit: usize,
    lease_until_unix: i64,
    max_passes: usize,
) -> Result<DispatchReport, OutboxError> {
    if limit == 0 || max_passes == 0 {
        return Err(OutboxError::Unavailable(String::from(
            "outbox drain limit and max_passes must be non-zero",
        )));
    }
    let mut total = DispatchReport::default();
    for _ in 0..max_passes {
        let report = dispatch_once(store, publisher, worker_id, limit, lease_until_unix).await?;
        total.delivered = total.delivered.saturating_add(report.delivered);
        total.retried = total.retried.saturating_add(report.retried);
        total.quarantined = total.quarantined.saturating_add(report.quarantined);
        if report.delivered == 0 && report.retried == 0 {
            break;
        }
    }
    Ok(total)
}

/// Runtime-independent sleep hook used by [`run_outbox_worker`].
#[async_trait]
pub trait OutboxSleeper: Send + Sync {
    /// Suspends the worker for `duration`.
    async fn sleep(&self, duration: Duration);
}

/// Configuration for the bounded outbox supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboxWorkerConfig {
    /// Maximum rows claimed by one worker tick.
    pub batch_size: usize,
    /// Lease duration assigned to each claim.
    pub lease_duration_secs: i64,
    /// Maximum number of ticks before returning. A deployment can use a very
    /// large value and a shutdown predicate for a long-running worker.
    pub max_ticks: usize,
    /// Delay after an idle tick.
    pub idle_backoff: Duration,
    /// Initial delay after a transient storage/publisher error.
    pub error_backoff: Duration,
    /// Maximum transient-error delay.
    pub max_error_backoff: Duration,
    /// Handling policy for digest-invalid payloads.
    pub poison_policy: PoisonPolicy,
}

/// Aggregate result from a bounded supervisor run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkerReport {
    /// Number of ticks attempted.
    pub ticks: u64,
    /// Number of successfully delivered rows.
    pub delivered: u64,
    /// Number of publisher failures released for retry.
    pub retried: u64,
    /// Number of poison rows quarantined.
    pub quarantined: u64,
    /// Number of transient errors that caused backoff.
    pub transient_errors: u64,
}

/// Runs bounded outbox worker ticks with idle/error backoff.
///
/// The worker is deliberately runtime-neutral: the composition root supplies
/// an async sleeper and a shutdown predicate. `max_ticks` is always enforced,
/// preventing an accidentally immortal test or shutdown drain. Transient
/// errors use capped exponential backoff; permanent corruption is returned to
/// the caller after the selected retry/quarantine action.
///
/// # Errors
///
/// Returns [`OutboxError::Unavailable`] when bounds are invalid or the final
/// storage operation fails. Returns [`OutboxError::Corrupt`] for a poison row
/// when [`PoisonPolicy::Retry`] is selected.
pub async fn run_outbox_worker(
    store: &dyn OutboxStore,
    publisher: &dyn OutboxPublisher,
    worker_id: &str,
    config: OutboxWorkerConfig,
    sleeper: &dyn OutboxSleeper,
    should_stop: &dyn Fn() -> bool,
) -> Result<WorkerReport, OutboxError> {
    if worker_id.is_empty()
        || config.batch_size == 0
        || config.max_ticks == 0
        || config.lease_duration_secs <= 0
        || config.max_error_backoff < config.error_backoff
    {
        return Err(OutboxError::Unavailable(String::from(
            "worker id and bounds must be non-empty",
        )));
    }
    let mut report = WorkerReport::default();
    let mut error_backoff = config.error_backoff;
    for _ in 0..config.max_ticks {
        if should_stop() {
            break;
        }
        report.ticks = report.ticks.saturating_add(1);
        let lease_until = chrono::Utc::now()
            .timestamp()
            .saturating_add(config.lease_duration_secs);
        match dispatch_once_with_policy(
            store,
            publisher,
            worker_id,
            config.batch_size,
            lease_until,
            config.poison_policy,
        )
        .await
        {
            Ok(tick) => {
                report.delivered = report.delivered.saturating_add(tick.delivered);
                report.retried = report.retried.saturating_add(tick.retried);
                report.quarantined = report.quarantined.saturating_add(tick.quarantined);
                error_backoff = config.error_backoff;
                if tick.delivered == 0 && tick.retried == 0 && tick.quarantined == 0 {
                    sleeper.sleep(config.idle_backoff).await;
                }
            }
            Err(error) if error.is_retryable() => {
                report.transient_errors = report.transient_errors.saturating_add(1);
                sleeper.sleep(error_backoff).await;
                let doubled = error_backoff.saturating_mul(2);
                error_backoff = doubled.min(config.max_error_backoff);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(report)
}

/// Stable helper for commit delivery keys.
pub fn commit_delivery_key(tenant: &TenantId, commit_id: &CommitId) -> String {
    format!("commit:{}:{}", tenant.0, commit_id.0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EmptyStore;

    #[async_trait]
    impl OutboxStore for EmptyStore {
        async fn claim(
            &self,
            _worker_id: &str,
            _limit: usize,
            _lease_until_unix: i64,
        ) -> Result<Vec<(String, OutboxPayload)>, OutboxError> {
            Ok(Vec::new())
        }

        async fn mark_delivered(&self, _delivery_key: &str) -> Result<(), OutboxError> {
            Ok(())
        }

        async fn release(&self, _delivery_key: &str, _error: &str) -> Result<(), OutboxError> {
            Ok(())
        }

        async fn pending_count(&self, _tenant: Option<&TenantId>) -> Result<u64, OutboxError> {
            Ok(0)
        }
    }

    struct EmptyPublisher;

    #[async_trait]
    impl OutboxPublisher for EmptyPublisher {
        async fn publish(
            &self,
            _delivery_key: &str,
            _payload: &OutboxPayload,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    struct CountingSleeper(Arc<AtomicUsize>);

    #[async_trait]
    impl OutboxSleeper for CountingSleeper {
        async fn sleep(&self, _duration: Duration) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn retry_classification_rejects_corruption() {
        assert!(OutboxError::Unavailable(String::new()).is_retryable());
        assert!(!OutboxError::Corrupt(String::new()).is_retryable());
    }

    #[tokio::test]
    async fn worker_honors_tick_bound_and_idle_backoff() {
        let sleeps = Arc::new(AtomicUsize::new(0));
        let report = run_outbox_worker(
            &EmptyStore,
            &EmptyPublisher,
            "worker",
            OutboxWorkerConfig {
                batch_size: 4,
                lease_duration_secs: 30,
                max_ticks: 3,
                idle_backoff: Duration::from_millis(1),
                error_backoff: Duration::from_millis(1),
                max_error_backoff: Duration::from_millis(8),
                poison_policy: PoisonPolicy::Quarantine,
            },
            &CountingSleeper(Arc::clone(&sleeps)),
            &|| false,
        )
        .await
        .unwrap();
        assert_eq!(report.ticks, 3);
        assert_eq!(report.delivered, 0);
        assert_eq!(sleeps.load(Ordering::SeqCst), 3);
    }
}
