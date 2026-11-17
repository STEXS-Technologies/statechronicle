//! Optional baseline settlement coordinators using the common executor.

use super::{
    BatchCompletion, DurableBatchSink, Event, Executor, ExecutorError, ValidatedIntent, atomicity,
};
use statechronicle_domain::tenant::TenantId;
use statechronicle_profiles::unique_asset::op as asset_op;
use std::collections::BTreeMap;

impl Executor {
    /// Runs a value-leg settlement batch atomically (protocol §18.3, Phase 2).
    ///
    /// A settlement may settle an asset in exchange for a fungible value leg
    /// (asset-for-gold): the batch grows from `[trade.settle]` to
    /// `[trade.settle, balance.transfer x2]` — one settle intent plus one
    /// `balance.transfer` intent (the value leg), all in one atomic
    /// transaction. The intents run through the same transaction wrapper as
    /// [`Self::execute_batch`]; the emitted batch is validated by
    /// [`atomicity::validate_batch_consistency`] first (untouched), then by the
    /// value-leg shape check [`atomicity::validate_settle_batch`]. All-or-
    /// nothing: any failure rolls back and surfaces as
    /// [`ExecutorError::AtomicityViolation`].
    ///
    /// The settle intents passed to the shape check are those whose operation
    /// is `trade.settle`; the value-leg `balance.transfer` intents are the rest.
    /// The pure/planning route does not claim intents; use
    /// [`Self::execute_settle_durable`] for durable idempotency.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::AtomicityViolation`] when the batch is empty,
    /// when any intent fails, or when the emitted batch is not a coherent
    /// value-leg settlement, and [`ExecutorError::Store`] when the transaction
    /// manager itself fails.
    pub async fn execute_settle(
        &self,
        intents: &[ValidatedIntent],
    ) -> Result<Vec<Event>, ExecutorError> {
        self.execute_settle_inner(intents, None).await
    }

    async fn execute_settle_inner(
        &self,
        intents: &[ValidatedIntent],
        sink: Option<&dyn DurableBatchSink>,
    ) -> Result<Vec<Event>, ExecutorError> {
        let Some(first) = intents.first() else {
            return Err(ExecutorError::AtomicityViolation(String::from(
                "empty settle batch",
            )));
        };
        let tenant = &first.intent.tenant_id;
        let completion =
            BatchCompletion::begin(sink, || self.ports.transaction_manager.begin(tenant)).await?;

        // Both a leg failure and a validation failure are rolled back atomically.
        // `allow_value_legs` is `true`: the settle batch's value legs are
        // validated by [`atomicity::validate_settle_batch`] below.
        let result = match self.run_batch(intents, true, sink.is_some()).await {
            Ok(events) => validate_settle(&events, intents.iter()).map(|()| events),
            Err(error) => Err(error),
        };
        match result {
            Ok(events) => {
                completion.succeed(intents, &events).await?;
                Ok(events)
            }
            Err(error) => Err(completion.refuse(error, "settle rollback failed").await),
        }
    }

    /// Executes a value-leg settlement and persists it through the required
    /// durable batch boundary before returning success.
    ///
    /// # Errors
    ///
    /// Returns the settlement validation/atomicity errors or
    /// [`ExecutorError::Store`] when the sink rejects the durable write.
    pub async fn execute_settle_durable(
        &self,
        intents: &[ValidatedIntent],
        sink: &dyn DurableBatchSink,
    ) -> Result<Vec<Event>, ExecutorError> {
        self.execute_settle_inner(intents, Some(sink)).await
    }

    /// Runs a cross-tenant batch atomically (protocol §8.2, §18.3).
    ///
    /// A cross-tenant transaction spans two or more distinct tenants. The
    /// affected-tenant set is derived by partitioning the intents by
    /// `tenant_id` (preserving input order within each tenant); the sorted
    /// tenant keys are passed to `transaction_manager.begin_multi`, then each
    /// tenant's leg runs through the single-tenant batch pipeline.
    /// The per-tenant groups are validated with
    /// [`atomicity::validate_cross_tenant_consistency`], then committed
    /// atomically: success commits and returns one
    /// [`atomicity::TenantEventGroup`] per affected tenant; any error rolls back
    /// and surfaces as [`ExecutorError::AtomicityViolation`].
    ///
    /// Idempotent-replay legs return no events (existing semantics). A leg
    /// whose intents all replay produces an empty group; because such a group
    /// carries no intent id that links at least two distinct tenant groups,
    /// `validate_cross_tenant_consistency` fails and the whole transaction
    /// aborts with [`ExecutorError::AtomicityViolation`] and rolls back. A
    /// cross-tenant retry is therefore fail-closed and deterministic.
    /// The pure/planning route does not claim intents because its symbolic
    /// transaction handle cannot roll back an independent intent store; use
    /// [`Self::execute_cross_tenant_durable`] for durable idempotency.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::AtomicityViolation`] when the batch has fewer
    /// than two distinct tenants, when any leg fails, or when the cross-tenant
    /// groups are not consistent, and [`ExecutorError::Store`] when the
    /// transaction manager itself fails.
    pub async fn execute_cross_tenant(
        &self,
        intents: &[ValidatedIntent],
    ) -> Result<Vec<atomicity::TenantEventGroup>, ExecutorError> {
        self.execute_cross_tenant_inner(intents, None).await
    }

    async fn execute_cross_tenant_inner(
        &self,
        intents: &[ValidatedIntent],
        sink: Option<&dyn DurableBatchSink>,
    ) -> Result<Vec<atomicity::TenantEventGroup>, ExecutorError> {
        // Partition by tenant id, preserving input order within each tenant.
        // Keyed by the tenant id string: `TenantId` is not `Ord`, so sorting by
        // the id string yields the same deterministic sorted-tenant scope.
        let (by_name, sorted_tenants) = partition_tenants(intents);
        if sorted_tenants.len() < 2 {
            return Err(ExecutorError::AtomicityViolation(String::from(
                "cross-tenant batch requires at least two distinct tenants",
            )));
        }

        let completion = BatchCompletion::begin(sink, || {
            self.ports.transaction_manager.begin_multi(&sorted_tenants)
        })
        .await?;

        // Both a leg failure and an inconsistent-group validation failure are
        // rolled back atomically (a failed validation must not short-circuit
        // past the rollback via `?`). `allow_value_legs` is `false`: a value-leg
        // `trade.settle` has no declared manifest here, so it must fail the
        // value-leg routing gate (see [`Self::execute_inner`]).
        let result = match self
            .run_cross_tenant_legs(&by_name, false, sink.is_some())
            .await
        {
            Ok(groups) => atomicity::validate_cross_tenant_consistency(&groups).map(|()| groups),
            Err(error) => Err(error),
        };
        match result {
            Ok(groups) => {
                let events: Vec<Event> = groups
                    .iter()
                    .flat_map(|group| group.events.iter().cloned())
                    .collect();
                completion.succeed(intents, &events).await?;
                Ok(groups)
            }
            Err(error) => Err(completion
                .refuse(error, "cross-tenant rollback failed")
                .await),
        }
    }

    /// Executes a validated cross-tenant batch through the required durable
    /// sink. The sink must persist every tenant leg in one supported database
    /// transaction; independent databases cannot provide atomic settlement.
    ///
    /// # Errors
    ///
    /// Returns cross-tenant validation/atomicity errors or
    /// [`ExecutorError::Store`] when durable persistence fails.
    pub async fn execute_cross_tenant_durable(
        &self,
        intents: &[ValidatedIntent],
        sink: &dyn DurableBatchSink,
    ) -> Result<Vec<atomicity::TenantEventGroup>, ExecutorError> {
        self.execute_cross_tenant_inner(intents, Some(sink)).await
    }

    /// Runs a cross-tenant trade settlement atomically (protocol §8.2, §18.3,
    /// Phase 3).
    ///
    /// This is the declared-linkage entry point for cross-tenant trades. A
    /// trade spans two or more tenants with a distinct intent id per leg (the
    /// asset leg in one tenant, the value leg in another), so the legs are tied
    /// together by the caller-declared [`atomicity::TradeManifest`] rather than
    /// by a shared intent id. It mirrors [`Self::execute_cross_tenant`]'s
    /// transaction wrapper: the affected-tenant set is derived by partitioning
    /// the intents by `tenant_id`, the sorted tenant keys are passed to
    /// `transaction_manager.begin_multi`, each tenant's leg runs through the
    /// single-tenant batch pipeline, and the per-tenant groups are
    /// validated with [`atomicity::validate_cross_tenant_trade`] before an
    /// atomic commit. Any error rolls back and surfaces as
    /// [`ExecutorError::AtomicityViolation`].
    ///
    /// Durable idempotent-replay legs return no events (existing semantics). A retry
    /// that replays only some legs produces a partial, incoherent batch that
    /// [`atomicity::validate_cross_tenant_trade`] rejects (a missing settle or
    /// value leg fails closed), so the whole transaction aborts and rolls back:
    /// partial-replay semantics stay fail-closed and deterministic.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::AtomicityViolation`] when the batch has fewer
    /// than two distinct tenants, when any leg fails, or when the cross-tenant
    /// groups do not satisfy the declared manifest, and
    /// [`ExecutorError::Store`] when the transaction manager itself fails.
    /// The pure/planning route does not claim intents; use
    /// [`Self::execute_cross_tenant_trade_durable`] for durable idempotency.
    pub async fn execute_cross_tenant_trade(
        &self,
        intents: &[ValidatedIntent],
        manifest: &atomicity::TradeManifest,
    ) -> Result<Vec<atomicity::TenantEventGroup>, ExecutorError> {
        self.execute_cross_tenant_trade_inner(intents, manifest, None)
            .await
    }

    async fn execute_cross_tenant_trade_inner(
        &self,
        intents: &[ValidatedIntent],
        manifest: &atomicity::TradeManifest,
        sink: Option<&dyn DurableBatchSink>,
    ) -> Result<Vec<atomicity::TenantEventGroup>, ExecutorError> {
        // Partition by tenant id, preserving input order within each tenant.
        let (by_name, sorted_tenants) = partition_tenants(intents);
        if sorted_tenants.len() < 2 {
            return Err(ExecutorError::AtomicityViolation(String::from(
                "cross-tenant trade requires at least two distinct tenants",
            )));
        }

        let completion = BatchCompletion::begin(sink, || {
            self.ports.transaction_manager.begin_multi(&sorted_tenants)
        })
        .await?;

        // Both a leg failure and a manifest-validation failure are rolled back
        // atomically (a failed validation must not short-circuit past the
        // rollback via `?`).
        let result = match self
            .run_cross_tenant_legs(&by_name, true, sink.is_some())
            .await
        {
            Ok(groups) => {
                let settle_intents = settle_requests(intents.iter());
                atomicity::validate_cross_tenant_trade(&groups, manifest, &settle_intents)
                    .map(|()| groups)
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(groups) => {
                let events: Vec<Event> = groups
                    .iter()
                    .flat_map(|group| group.events.iter().cloned())
                    .collect();
                completion.succeed(intents, &events).await?;
                Ok(groups)
            }
            Err(error) => Err(completion
                .refuse(error, "cross-tenant trade rollback failed")
                .await),
        }
    }

    /// Executes a manifest-validated cross-tenant trade and persists every
    /// tenant leg through the required durable batch sink before success.
    ///
    /// The sink must use one supported atomic database transaction and retain
    /// the manifest linkage in its own audit metadata; independent databases
    /// are not made atomic by this helper.
    ///
    /// # Errors
    ///
    /// Returns cross-tenant validation/atomicity errors or
    /// [`ExecutorError::Store`] when durable persistence fails.
    pub async fn execute_cross_tenant_trade_durable(
        &self,
        intents: &[ValidatedIntent],
        manifest: &atomicity::TradeManifest,
        sink: &dyn DurableBatchSink,
    ) -> Result<Vec<atomicity::TenantEventGroup>, ExecutorError> {
        self.execute_cross_tenant_trade_inner(intents, manifest, Some(sink))
            .await
    }
}

// Keep partition/clone order identical for both cross-tenant policies. The
// caller still owns its minimum-scope error and declared/inferred linkage gate.
fn partition_tenants(
    intents: &[ValidatedIntent],
) -> (BTreeMap<String, Vec<ValidatedIntent>>, Vec<TenantId>) {
    let mut by_name: BTreeMap<String, Vec<ValidatedIntent>> = BTreeMap::new();
    for validated in intents {
        by_name
            .entry(validated.intent.tenant_id.0.clone())
            .or_default()
            .push(validated.clone());
    }
    let sorted_tenants = by_name.keys().map(|name| TenantId(name.clone())).collect();
    (by_name, sorted_tenants)
}

pub(super) fn validate_settle<'intent>(
    events: &[Event],
    intents: impl IntoIterator<Item = &'intent ValidatedIntent>,
) -> Result<(), ExecutorError> {
    let settle_intents = settle_requests(intents);
    atomicity::validate_batch_consistency(events)
        .and_then(|()| atomicity::validate_settle_batch(events, &settle_intents))
}

fn settle_requests<'intent>(
    intents: impl IntoIterator<Item = &'intent ValidatedIntent>,
) -> Vec<statechronicle_domain::intent::Intent> {
    intents
        .into_iter()
        .filter(|validated| &validated.intent.operation == asset_op::trade_settle())
        .map(|validated| validated.intent.clone())
        .collect()
}
