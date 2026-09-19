//! Deterministic token-bucket primitive for ingress throttling.
//!
//! This is a local guardrail. Distributed deployments must key an equivalent
//! policy in shared infrastructure (for example Redis or an API gateway) so
//! limits cannot be bypassed by moving between workers.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::limits::MAX_QUOTA_KEY_BYTES;

/// Result of attempting to consume rate-limit tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitDecision {
    /// Whether the request may proceed.
    pub allowed: bool,
    /// Tokens remaining after the decision.
    pub remaining: u64,
    /// Milliseconds until one token is expected, when denied.
    pub retry_after_ms: u64,
}

/// A thread-safe token bucket whose clock is supplied by the caller.
#[derive(Debug)]
pub struct TokenBucket {
    capacity: u64,
    refill_per_second: u64,
    state: Mutex<(u64, u64)>,
}

impl TokenBucket {
    /// Creates a bucket. Zero capacity or refill is rejected by clamping to
    /// one, preventing accidental permanent denial from configuration errors.
    pub fn new(capacity: u64, refill_per_second: u64) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            refill_per_second: refill_per_second.max(1),
            state: Mutex::new((capacity, 0)),
        }
    }

    /// Attempts to consume one token at a monotonic millisecond timestamp.
    pub fn try_acquire(&self, now_ms: u64) -> RateLimitDecision {
        self.try_acquire_n(now_ms, 1)
    }

    /// Attempts to consume `tokens` at a monotonic millisecond timestamp.
    /// Batch and high-cost operations can charge more than one token while
    /// retaining the same atomic bucket decision.
    pub fn try_acquire_n(&self, now_ms: u64, tokens: u64) -> RateLimitDecision {
        if tokens == 0 {
            return RateLimitDecision {
                allowed: true,
                remaining: self.available(now_ms),
                retry_after_ms: 0,
            };
        }
        if tokens > self.capacity {
            return RateLimitDecision {
                allowed: false,
                remaining: self.capacity,
                retry_after_ms: u64::MAX,
            };
        }
        let Ok(mut state) = self.state.lock() else {
            return RateLimitDecision {
                allowed: false,
                remaining: 0,
                retry_after_ms: 1_000,
            };
        };
        let elapsed = now_ms.saturating_sub(state.1);
        let refill = elapsed
            .saturating_mul(self.refill_per_second)
            .checked_div(1_000)
            .unwrap_or(u64::MAX);
        if refill > 0 {
            state.0 = state.0.saturating_add(refill).min(self.capacity);
            state.1 = now_ms;
        }
        if state.0 >= tokens {
            state.0 = state.0.saturating_sub(tokens);
            return RateLimitDecision {
                allowed: true,
                remaining: state.0,
                retry_after_ms: 0,
            };
        }
        let deficit = tokens.saturating_sub(state.0);
        let retry_after_ms = deficit
            .saturating_mul(1_000)
            .saturating_add(self.refill_per_second.saturating_sub(1))
            .checked_div(self.refill_per_second)
            .unwrap_or(1_000);
        RateLimitDecision {
            allowed: false,
            remaining: state.0,
            retry_after_ms,
        }
    }

    fn available(&self, now_ms: u64) -> u64 {
        let Ok(state) = self.state.lock() else {
            return 0;
        };
        let elapsed = now_ms.saturating_sub(state.1);
        let refill = elapsed
            .saturating_mul(self.refill_per_second)
            .checked_div(1_000)
            .unwrap_or(u64::MAX);
        state.0.saturating_add(refill).min(self.capacity)
    }
}

/// Bounded collection of token buckets keyed by an account, tenant,
/// operation class, or other caller-defined dimension.
///
/// The map has a hard `max_keys` limit so an attacker cannot exhaust memory by
/// sending unbounded invalid identities. Distributed deployments must still
/// enforce the same policy in shared infrastructure; this type protects one
/// process only.
#[derive(Debug)]
pub struct KeyedRateLimiter {
    capacity: u64,
    refill_per_second: u64,
    max_keys: usize,
    buckets: Mutex<BTreeMap<String, TokenBucket>>,
}

impl KeyedRateLimiter {
    /// Creates a bounded keyed limiter. Zero values are clamped to safe
    /// minimums, matching [`TokenBucket::new`].
    pub fn new(capacity: u64, refill_per_second: u64, max_keys: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            refill_per_second: refill_per_second.max(1),
            max_keys: max_keys.max(1),
            buckets: Mutex::new(BTreeMap::new()),
        }
    }

    /// Atomically charges one key for `tokens` at a caller-supplied timestamp.
    /// A new key is denied once the configured cardinality bound is reached.
    pub fn try_acquire_n(&self, key: &str, now_ms: u64, tokens: u64) -> RateLimitDecision {
        if key.is_empty() || key.len() > MAX_QUOTA_KEY_BYTES {
            return RateLimitDecision {
                allowed: false,
                remaining: 0,
                retry_after_ms: 1_000,
            };
        }
        if tokens == 0 {
            return RateLimitDecision {
                allowed: true,
                remaining: self.capacity,
                retry_after_ms: 0,
            };
        }
        if tokens > self.capacity {
            return RateLimitDecision {
                allowed: false,
                remaining: self.capacity,
                retry_after_ms: u64::MAX,
            };
        }
        let Ok(mut buckets) = self.buckets.lock() else {
            return RateLimitDecision {
                allowed: false,
                remaining: 0,
                retry_after_ms: 1_000,
            };
        };
        if !buckets.contains_key(key) {
            if buckets.len() >= self.max_keys {
                return RateLimitDecision {
                    allowed: false,
                    remaining: 0,
                    retry_after_ms: 1_000,
                };
            }
            buckets.insert(
                key.to_owned(),
                TokenBucket::new(self.capacity, self.refill_per_second),
            );
        }
        buckets.get(key).map_or(
            RateLimitDecision {
                allowed: false,
                remaining: 0,
                retry_after_ms: 1_000,
            },
            |bucket| bucket.try_acquire_n(now_ms, tokens),
        )
    }

    /// Attempts to charge one token for a key.
    pub fn try_acquire(&self, key: &str, now_ms: u64) -> RateLimitDecision {
        self.try_acquire_n(key, now_ms, 1)
    }

    /// Atomically charges the same cost against every supplied dimension.
    ///
    /// This is intended for account + tenant + operation policies. Every
    /// bucket is checked before any bucket is consumed, so a denial on one
    /// dimension cannot partially debit the others. Duplicate keys are
    /// de-duplicated and an empty key list is denied.
    pub fn try_acquire_many(&self, keys: &[&str], now_ms: u64, tokens: u64) -> RateLimitDecision {
        if keys.is_empty()
            || keys
                .iter()
                .any(|key| key.is_empty() || key.len() > MAX_QUOTA_KEY_BYTES)
        {
            return RateLimitDecision {
                allowed: false,
                remaining: 0,
                retry_after_ms: 1_000,
            };
        }
        if tokens == 0 {
            return RateLimitDecision {
                allowed: true,
                remaining: self.capacity,
                retry_after_ms: 0,
            };
        }
        if tokens > self.capacity {
            return RateLimitDecision {
                allowed: false,
                remaining: self.capacity,
                retry_after_ms: u64::MAX,
            };
        }
        let unique: std::collections::BTreeSet<&str> = keys.iter().copied().collect();
        let Ok(mut buckets) = self.buckets.lock() else {
            return RateLimitDecision {
                allowed: false,
                remaining: 0,
                retry_after_ms: 1_000,
            };
        };
        let new_keys = unique
            .iter()
            .filter(|key| !buckets.contains_key(**key))
            .count();
        if buckets.len().saturating_add(new_keys) > self.max_keys {
            return RateLimitDecision {
                allowed: false,
                remaining: 0,
                retry_after_ms: 1_000,
            };
        }
        for key in &unique {
            buckets
                .entry((*key).to_owned())
                .or_insert_with(|| TokenBucket::new(self.capacity, self.refill_per_second));
        }
        // Preflight every bucket without consuming. The parent map lock makes
        // this check-and-consume sequence atomic with respect to all other
        // operations on this limiter.
        let mut minimum_remaining = u64::MAX;
        let mut maximum_retry = 0u64;
        for key in &unique {
            let Some(bucket) = buckets.get(*key) else {
                return RateLimitDecision {
                    allowed: false,
                    remaining: 0,
                    retry_after_ms: 1_000,
                };
            };
            let Ok(mut state) = bucket.state.lock() else {
                return RateLimitDecision {
                    allowed: false,
                    remaining: 0,
                    retry_after_ms: 1_000,
                };
            };
            refill_state(&mut state, now_ms, self.refill_per_second, self.capacity);
            minimum_remaining = minimum_remaining.min(state.0);
            if state.0 < tokens {
                let deficit = tokens.saturating_sub(state.0);
                let retry = deficit
                    .saturating_mul(1_000)
                    .saturating_add(self.refill_per_second.saturating_sub(1))
                    .checked_div(self.refill_per_second)
                    .unwrap_or(1_000);
                maximum_retry = maximum_retry.max(retry);
            }
        }
        if maximum_retry != 0 {
            return RateLimitDecision {
                allowed: false,
                remaining: minimum_remaining,
                retry_after_ms: maximum_retry,
            };
        }
        for key in &unique {
            if let Some(bucket) = buckets.get(*key) {
                if let Ok(mut state) = bucket.state.lock() {
                    state.0 = state.0.saturating_sub(tokens);
                }
            }
        }
        RateLimitDecision {
            allowed: true,
            remaining: minimum_remaining.saturating_sub(tokens),
            retry_after_ms: 0,
        }
    }

    /// Returns the number of active key buckets.
    pub fn key_count(&self) -> usize {
        self.buckets.lock().map_or(0, |buckets| buckets.len())
    }
}

fn refill_state(state: &mut (u64, u64), now_ms: u64, refill_per_second: u64, capacity: u64) {
    let elapsed = now_ms.saturating_sub(state.1);
    let refill = elapsed
        .saturating_mul(refill_per_second)
        .checked_div(1_000)
        .unwrap_or(u64::MAX);
    if refill > 0 {
        state.0 = state.0.saturating_add(refill).min(capacity);
        state.1 = now_ms;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn bucket_refills_and_denies_when_empty() {
        let bucket = TokenBucket::new(1, 10);
        assert!(bucket.try_acquire(0).allowed);
        assert!(!bucket.try_acquire(0).allowed);
        assert!(bucket.try_acquire(100).allowed);
    }

    #[test]
    fn weighted_acquire_is_atomic() {
        let bucket = TokenBucket::new(3, 10);
        assert!(bucket.try_acquire_n(0, 2).allowed);
        let denied = bucket.try_acquire_n(0, 2);
        assert!(!denied.allowed);
        assert_eq!(denied.remaining, 1);
        assert_eq!(denied.retry_after_ms, 100);
    }

    #[test]
    fn keyed_limiter_isolates_dimensions_and_bounds_cardinality() {
        let limiter = KeyedRateLimiter::new(1, 1, 1);
        assert!(limiter.try_acquire("alice", 0).allowed);
        assert!(!limiter.try_acquire("alice", 0).allowed);
        assert!(!limiter.try_acquire("bob", 0).allowed);
        assert_eq!(limiter.key_count(), 1);
    }

    #[test]
    fn bounded_abuse_does_not_block_an_existing_unrelated_key() {
        let limiter = KeyedRateLimiter::new(2, 1, 2);
        assert!(limiter.try_acquire("tenant:legitimate", 0).allowed);
        assert!(limiter.try_acquire("tenant:legitimate", 0).allowed);
        let before_attack = limiter.try_acquire("tenant:legitimate", 0);
        assert!(!before_attack.allowed);
        // An invalid-request flood may consume the remaining cardinality
        // slot, but must not mutate the already-established tenant bucket.
        // This is the local guardrail expected before a distributed quota
        // service is installed.
        assert!(limiter.try_acquire("attacker:0", 0).allowed);
        for index in 1..1_000 {
            assert!(!limiter.try_acquire(&format!("attacker:{index}"), 0).allowed);
        }
        let after_attack = limiter.try_acquire("tenant:legitimate", 0);
        assert_eq!(after_attack, before_attack);
        assert!(limiter.try_acquire("tenant:legitimate", 1_000).allowed);
        assert_eq!(limiter.key_count(), 2);
    }

    #[test]
    fn multi_dimension_acquire_is_all_or_nothing() {
        let limiter = KeyedRateLimiter::new(2, 1, 4);
        assert!(limiter.try_acquire("account:alice", 0).allowed);
        assert!(limiter.try_acquire("operation:trade", 0).allowed);

        // The operation bucket has no capacity left. The account bucket must
        // retain both tokens because the combined request is denied.
        let denied = limiter.try_acquire_many(&["account:alice", "operation:trade"], 0, 2);
        assert!(!denied.allowed);
        let account_check = limiter.try_acquire_n("account:alice", 0, 2);
        assert!(!account_check.allowed);
        assert_eq!(account_check.remaining, 1);
    }

    #[test]
    fn multi_dimension_deduplicates_keys_and_bounds_new_cardinality() {
        let limiter = KeyedRateLimiter::new(3, 1, 1);
        assert!(
            limiter
                .try_acquire_many(&["tenant:game", "tenant:game"], 0, 2)
                .allowed
        );
        assert_eq!(limiter.key_count(), 1);
        assert!(
            !limiter
                .try_acquire_many(&["tenant:game", "account:alice"], 0, 1)
                .allowed
        );
        assert_eq!(limiter.key_count(), 1);
    }

    #[test]
    fn zero_cost_requests_do_not_allocate_unbounded_keys() {
        let limiter = KeyedRateLimiter::new(1, 1, 1);
        assert!(limiter.try_acquire_n("probe:one", 0, 0).allowed);
        assert!(
            limiter
                .try_acquire_many(&["probe:two", "probe:three"], 0, 0)
                .allowed
        );
        assert_eq!(limiter.key_count(), 0);
    }

    #[test]
    fn impossible_cost_requests_do_not_allocate_keys() {
        let limiter = KeyedRateLimiter::new(2, 1, 2);
        assert!(!limiter.try_acquire_n("oversized", 0, 3).allowed);
        assert!(
            !limiter
                .try_acquire_many(&["oversized-a", "oversized-b"], 0, 3)
                .allowed
        );
        assert_eq!(limiter.key_count(), 0);
    }

    #[test]
    fn oversized_keys_fail_closed_without_allocation() {
        let limiter = KeyedRateLimiter::new(2, 1, 2);
        let oversized = "x".repeat(MAX_QUOTA_KEY_BYTES + 1);
        assert!(!limiter.try_acquire(&oversized, 0).allowed);
        assert!(!limiter.try_acquire_many(&[&oversized], 0, 1).allowed);
        assert_eq!(limiter.key_count(), 0);
    }

    #[test]
    fn multi_dimension_acquire_allows_only_one_concurrent_cost() {
        let limiter = std::sync::Arc::new(KeyedRateLimiter::new(1, 1, 4));
        let mut workers = Vec::new();
        for _ in 0..32 {
            let limiter = std::sync::Arc::clone(&limiter);
            workers.push(std::thread::spawn(move || {
                limiter
                    .try_acquire_many(&["account:alice", "operation:trade"], 0, 1)
                    .allowed
            }));
        }
        let allowed = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|allowed| *allowed)
            .count();
        assert_eq!(allowed, 1);
    }
}
