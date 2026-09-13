//! Distributed quota port for multi-worker deployments.
//!
//! [`statechronicle_core::rate_limit::KeyedRateLimiter`] protects one process
//! only. A game backend with multiple workers must install this contract over
//! shared infrastructure (Redis, a gateway, or a database-side procedure)
//! before accepting production traffic.

use async_trait::async_trait;
use statechronicle_core::limits::MAX_QUOTA_KEY_BYTES;
use statechronicle_core::rate_limit::RateLimitDecision;
use thiserror::Error;

/// Errors from a distributed quota provider.
#[derive(Debug, Error)]
pub enum QuotaError {
    /// The shared quota service could not be reached. Callers must fail closed
    /// or return a retryable response; they must not fall back to local-only
    /// enforcement for a protected operation.
    #[error("distributed quota unavailable: {0}")]
    Unavailable(String),
    /// The quota provider rejected malformed dimensions or cost.
    #[error("distributed quota request invalid: {0}")]
    Invalid(String),
    /// The provider does not implement an operation required by the caller.
    #[error("distributed quota operation unsupported: {0}")]
    Unsupported(String),
}

impl QuotaError {
    /// Returns whether retrying the same validated quota request may succeed.
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

/// Shared quota provider supplied by the deployment.
#[async_trait]
pub trait DistributedQuota: Send + Sync {
    /// Atomically charges `cost` against one fully-qualified key.
    ///
    /// `now_unix_ms` is supplied by the composition root so the provider can
    /// use a trusted clock. Implementations must make concurrent calls for the
    /// same key linearizable and return deterministic retry metadata.
    async fn try_acquire(
        &self,
        key: &str,
        cost: u64,
        now_unix_ms: i64,
    ) -> Result<RateLimitDecision, QuotaError>;

    /// Atomically charges `cost` against every supplied dimension.
    ///
    /// The default rejects the operation: silently looping over
    /// [`Self::try_acquire`] could partially charge a request when a later
    /// dimension denies it. Providers should override this with one
    /// server-side atomic script/transaction when multi-dimensional quotas
    /// are required.
    async fn try_acquire_many(
        &self,
        _keys: &[&str],
        _cost: u64,
        _now_unix_ms: i64,
    ) -> Result<RateLimitDecision, QuotaError> {
        Err(QuotaError::Unsupported(String::from(
            "atomic multi-dimension quota is not supported",
        )))
    }
}

/// Validates and then delegates one quota charge to a provider.
///
/// Composition roots should call this wrapper instead of invoking an adapter
/// directly when request dimensions originate from clients.
///
/// # Errors
///
/// Returns [`QuotaError::Invalid`] for malformed input, or propagates the
/// provider's [`QuotaError::Unavailable`] failure.
pub async fn try_acquire_validated(
    provider: &dyn DistributedQuota,
    key: &str,
    cost: u64,
    now_unix_ms: i64,
) -> Result<RateLimitDecision, QuotaError> {
    validate_request(key, cost, now_unix_ms)?;
    provider.try_acquire(key, cost, now_unix_ms).await
}

/// Validates and delegates an atomic multi-dimension quota charge.
///
/// # Errors
///
/// Returns [`QuotaError::Invalid`] when any key or timestamp is malformed, or
/// propagates the provider's unavailable/unsupported response.
pub async fn try_acquire_many_validated(
    provider: &dyn DistributedQuota,
    keys: &[&str],
    cost: u64,
    now_unix_ms: i64,
) -> Result<RateLimitDecision, QuotaError> {
    if keys.is_empty() {
        return Err(QuotaError::Invalid(String::from(
            "quota dimensions must not be empty",
        )));
    }
    for key in keys {
        validate_request(key, cost, now_unix_ms)?;
    }
    // Match the local limiter's all-or-nothing semantics: a repeated
    // dimension represents one quota, not multiple charges. Sort through a
    // set so providers receive deterministic input independent of caller
    // ordering.
    let unique: std::collections::BTreeSet<&str> = keys.iter().copied().collect();
    let unique_keys: Vec<&str> = unique.into_iter().collect();
    provider
        .try_acquire_many(&unique_keys, cost, now_unix_ms)
        .await
}

/// Validates a quota request before dispatching it to a shared provider.
///
/// This helper is intentionally independent of the provider so every adapter
/// rejects empty dimensions, non-positive timestamps, and zero cost
/// consistently. Zero-cost work should not allocate distributed quota keys.
///
/// # Errors
///
/// Returns [`QuotaError::Invalid`] when the key is empty, the cost is zero, or
/// the supplied timestamp is negative.
pub fn validate_request(key: &str, cost: u64, now_unix_ms: i64) -> Result<(), QuotaError> {
    if key.is_empty() {
        return Err(QuotaError::Invalid(String::from(
            "quota key must not be empty",
        )));
    }
    if key.len() > MAX_QUOTA_KEY_BYTES {
        return Err(QuotaError::Invalid(format!(
            "quota key must be at most {MAX_QUOTA_KEY_BYTES} bytes"
        )));
    }
    if cost == 0 {
        return Err(QuotaError::Invalid(String::from(
            "quota cost must be positive",
        )));
    }
    if now_unix_ms < 0 {
        return Err(QuotaError::Invalid(String::from(
            "quota timestamp must not be negative",
        )));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    struct AllowQuota;

    #[async_trait]
    impl DistributedQuota for AllowQuota {
        async fn try_acquire(
            &self,
            _key: &str,
            _cost: u64,
            _now_unix_ms: i64,
        ) -> Result<RateLimitDecision, QuotaError> {
            Ok(RateLimitDecision {
                allowed: true,
                remaining: 9,
                retry_after_ms: 0,
            })
        }

        async fn try_acquire_many(
            &self,
            _keys: &[&str],
            _cost: u64,
            _now_unix_ms: i64,
        ) -> Result<RateLimitDecision, QuotaError> {
            Ok(RateLimitDecision {
                allowed: true,
                remaining: 8,
                retry_after_ms: 0,
            })
        }
    }

    #[test]
    fn quota_request_validation_is_fail_closed() {
        assert!(validate_request("tenant:game", 1, 0).is_ok());
        assert!(validate_request("", 1, 0).is_err());
        assert!(validate_request("tenant:game", 0, 0).is_err());
        assert!(validate_request("tenant:game", 1, -1).is_err());
        assert!(!QuotaError::Invalid(String::new()).is_retryable());
        assert!(QuotaError::Unavailable(String::new()).is_retryable());
        assert!(validate_request(&"x".repeat(MAX_QUOTA_KEY_BYTES + 1), 1, 0).is_err());
    }

    #[tokio::test]
    async fn validated_wrapper_rejects_before_provider_call() {
        let provider = AllowQuota;
        assert!(try_acquire_validated(&provider, "", 1, 0).await.is_err());
        let decision = try_acquire_validated(&provider, "tenant:game", 1, 0)
            .await
            .unwrap();
        assert!(decision.allowed);
    }

    #[tokio::test]
    async fn multi_validated_wrapper_rejects_empty_dimensions() {
        let provider = AllowQuota;
        assert!(
            try_acquire_many_validated(&provider, &[], 1, 0)
                .await
                .is_err()
        );
        let decision =
            try_acquire_many_validated(&provider, &["account:alice", "tenant:game"], 1, 0)
                .await
                .unwrap();
        assert!(decision.allowed);
    }

    #[tokio::test]
    async fn multi_validated_wrapper_deduplicates_dimensions() {
        struct RecordingQuota(std::sync::Mutex<Vec<String>>);
        #[async_trait]
        impl DistributedQuota for RecordingQuota {
            async fn try_acquire(
                &self,
                _key: &str,
                _cost: u64,
                _now_unix_ms: i64,
            ) -> Result<RateLimitDecision, QuotaError> {
                Err(QuotaError::Unsupported(String::from(
                    "single-dimension path",
                )))
            }

            async fn try_acquire_many(
                &self,
                keys: &[&str],
                _cost: u64,
                _now_unix_ms: i64,
            ) -> Result<RateLimitDecision, QuotaError> {
                *self.0.lock().unwrap() = keys.iter().map(|key| (*key).to_owned()).collect();
                Ok(RateLimitDecision {
                    allowed: true,
                    remaining: 1,
                    retry_after_ms: 0,
                })
            }
        }

        let provider = RecordingQuota(std::sync::Mutex::new(Vec::new()));
        try_acquire_many_validated(
            &provider,
            &["tenant:game", "account:alice", "tenant:game"],
            1,
            0,
        )
        .await
        .unwrap();
        assert_eq!(
            *provider.0.lock().unwrap(),
            vec![String::from("account:alice"), String::from("tenant:game")]
        );
    }

    #[tokio::test]
    async fn multi_dimension_defaults_to_fail_closed() {
        struct SingleOnly;
        #[async_trait]
        impl DistributedQuota for SingleOnly {
            async fn try_acquire(
                &self,
                _key: &str,
                _cost: u64,
                _now_unix_ms: i64,
            ) -> Result<RateLimitDecision, QuotaError> {
                Ok(RateLimitDecision {
                    allowed: true,
                    remaining: 1,
                    retry_after_ms: 0,
                })
            }
        }

        let error = try_acquire_many_validated(&SingleOnly, &["a", "b"], 1, 0)
            .await
            .unwrap_err();
        assert!(!error.is_retryable());
    }
}
