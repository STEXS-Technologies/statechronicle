//! Privacy-safe mutation telemetry port.
//!
//! Implementations should emit counters/histograms to the deployment's
//! monitoring system. The protocol deliberately passes stable classification
//! fields rather than raw payloads or credentials.

use async_trait::async_trait;
use statechronicle_core::limits::MAX_ID_LENGTH;
use statechronicle_domain::intent::Operation;
use statechronicle_domain::tenant::TenantId;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Outcome classification for one mutation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MutationOutcome {
    /// Durable commit accepted.
    Committed,
    /// Idempotent replay of a committed mutation.
    Replay,
    /// Rejected by validation, authorization, or conflict checks.
    Rejected,
    /// Failed due to a retryable infrastructure condition.
    RetryableFailure,
}

/// Structured mutation telemetry event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationMetric {
    /// Tenant classification (pseudonymize before external export if needed).
    pub tenant: TenantId,
    /// Operation name.
    pub operation: Operation,
    /// Result classification.
    pub outcome: MutationOutcome,
    /// Elapsed wall-clock milliseconds measured by the composition root.
    pub latency_ms: u64,
}

/// Aggregated metric series exposed by [`InMemoryMetrics`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricAggregate {
    /// Tenant label (pseudonymize before exporting externally).
    pub tenant: TenantId,
    /// Operation label.
    pub operation: Operation,
    /// Outcome label.
    pub outcome: MutationOutcome,
    /// Number of observations.
    pub count: u64,
    /// Sum of observed latency in milliseconds.
    pub total_latency_ms: u64,
    /// Maximum observed latency in milliseconds.
    pub max_latency_ms: u64,
}

#[derive(Debug, Clone, Default)]
struct MetricAggregateState {
    count: u64,
    total_latency_ms: u64,
    max_latency_ms: u64,
}

/// Bounded in-process metrics sink useful for tests and small deployments.
///
/// It aggregates by tenant/operation/outcome instead of retaining raw events,
/// and drops new series after `max_series` is reached to prevent unbounded
/// cardinality. Production exporters should apply equivalent label limits and
/// pseudonymization before shipping metrics.
#[derive(Debug)]
pub struct InMemoryMetrics {
    max_series: usize,
    series: Mutex<BTreeMap<(String, Operation, MutationOutcome), MetricAggregateState>>,
    dropped: Mutex<u64>,
}

impl InMemoryMetrics {
    /// Creates an empty bounded sink. A zero bound is clamped to one series.
    pub fn new(max_series: usize) -> Self {
        Self {
            max_series: max_series.max(1),
            series: Mutex::new(BTreeMap::new()),
            dropped: Mutex::new(0),
        }
    }

    /// Returns a stable snapshot of retained aggregates.
    pub fn snapshot(&self) -> Vec<MetricAggregate> {
        let Ok(series) = self.series.lock() else {
            return Vec::new();
        };
        series
            .iter()
            .map(|((tenant, operation, outcome), state)| MetricAggregate {
                tenant: TenantId(tenant.clone()),
                operation: operation.clone(),
                outcome: *outcome,
                count: state.count,
                total_latency_ms: state.total_latency_ms,
                max_latency_ms: state.max_latency_ms,
            })
            .collect()
    }

    /// Returns observations dropped due to the series cardinality bound or a
    /// poisoned internal lock.
    pub fn dropped(&self) -> u64 {
        self.dropped.lock().map_or(0, |dropped| *dropped)
    }
}

impl Default for InMemoryMetrics {
    fn default() -> Self {
        Self::new(1_024)
    }
}

#[async_trait]
impl MetricsSink for InMemoryMetrics {
    async fn record_mutation(&self, metric: MutationMetric) {
        if metric.tenant.0.chars().count() > MAX_ID_LENGTH
            || metric.tenant.0.chars().any(char::is_control)
            || metric.operation.0.chars().count() > MAX_ID_LENGTH
            || metric.operation.0.chars().any(char::is_control)
        {
            if let Ok(mut dropped) = self.dropped.lock() {
                *dropped = dropped.saturating_add(1);
            }
            return;
        }
        let Ok(mut series) = self.series.lock() else {
            if let Ok(mut dropped) = self.dropped.lock() {
                *dropped = dropped.saturating_add(1);
            }
            return;
        };
        let key = (
            metric.tenant.0.clone(),
            metric.operation.clone(),
            metric.outcome,
        );
        if !series.contains_key(&key) && series.len() >= self.max_series {
            if let Ok(mut dropped) = self.dropped.lock() {
                *dropped = dropped.saturating_add(1);
            }
            return;
        }
        let state = series.entry(key).or_default();
        state.count = state.count.saturating_add(1);
        state.total_latency_ms = state.total_latency_ms.saturating_add(metric.latency_ms);
        state.max_latency_ms = state.max_latency_ms.max(metric.latency_ms);
    }
}

/// Deployment-provided telemetry sink.
#[async_trait]
pub trait MetricsSink: Send + Sync {
    /// Records one mutation metric without altering mutation outcome.
    async fn record_mutation(&self, metric: MutationMetric);
}

/// No-op sink for tests and callers that do not install telemetry yet.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopMetrics;

#[async_trait]
impl MetricsSink for NoopMetrics {
    async fn record_mutation(&self, _metric: MutationMetric) {}
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_metrics_aggregate_and_bound_series() {
        let metrics = InMemoryMetrics::new(1);
        let operation = Operation::from_static("balance.transfer");
        metrics
            .record_mutation(MutationMetric {
                tenant: TenantId(String::from("game")),
                operation: operation.clone(),
                outcome: MutationOutcome::Committed,
                latency_ms: 7,
            })
            .await;
        metrics
            .record_mutation(MutationMetric {
                tenant: TenantId(String::from("game")),
                operation,
                outcome: MutationOutcome::Committed,
                latency_ms: 11,
            })
            .await;
        metrics
            .record_mutation(MutationMetric {
                tenant: TenantId(String::from("other")),
                operation: Operation::from_static("balance.transfer"),
                outcome: MutationOutcome::Rejected,
                latency_ms: 1,
            })
            .await;
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.len(), 1);
        let aggregate = snapshot.first().unwrap();
        assert_eq!(aggregate.count, 2);
        assert_eq!(aggregate.total_latency_ms, 18);
        assert_eq!(aggregate.max_latency_ms, 11);
        assert_eq!(metrics.dropped(), 1);
    }

    #[tokio::test]
    async fn in_memory_metrics_drops_unbounded_or_control_labels() {
        let metrics = InMemoryMetrics::new(4);
        metrics
            .record_mutation(MutationMetric {
                tenant: TenantId("x".repeat(MAX_ID_LENGTH + 1)),
                operation: Operation::from_static("asset.transfer"),
                outcome: MutationOutcome::Rejected,
                latency_ms: 0,
            })
            .await;
        metrics
            .record_mutation(MutationMetric {
                tenant: TenantId(String::from("game\nlog")),
                operation: Operation::from_static("asset.transfer"),
                outcome: MutationOutcome::Rejected,
                latency_ms: 0,
            })
            .await;
        metrics
            .record_mutation(MutationMetric {
                tenant: TenantId(String::from("game")),
                operation: Operation::from_static("asset.\ttransfer"),
                outcome: MutationOutcome::Rejected,
                latency_ms: 0,
            })
            .await;
        assert!(metrics.snapshot().is_empty());
        assert_eq!(metrics.dropped(), 3);
    }
}
