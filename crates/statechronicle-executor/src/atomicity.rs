//! Multi-resource atomic transactions (protocol §18.3).
//!
//! A transaction commits all affected state transitions or none; every
//! affected state record is validated against its expected version. The
//! executor returns batches of events that must be internally consistent:
//! distinct intent ids (except a transfer pair), distinct event ids, and a
//! single tenant scope.
//!
//! **Transfer pair invariant (§20.5).** An atomic transfer is a pair of events
//! that share one accepted intent id: a source debit and a destination credit.
//! The pair is the atomic unit: the source debit and destination credit are
//! both net-zero (debit == credit) and reference the same resource and the same
//! amount. Only `stack.transfer` / `balance.transfer` may share an intent id;
//! any other multi-event intent is internally inconsistent.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use statechronicle_core::amount::Amount;
use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::IntentId;
use statechronicle_domain::intent::{Intent, Operation};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_profiles::consumable_stack::op as stack_op;
use statechronicle_profiles::fungible_balance::op as balance_op;
use statechronicle_profiles::keys;
use statechronicle_profiles::unique_asset::op as asset_op;

use crate::error::ExecutorError;

/// Validates the internal consistency of an event batch (protocol §18.3).
///
/// A batch is internally valid only when:
///
/// * every event id is distinct: events are uniquely identifiable within the
///   tenant's history;
/// * every event shares the same tenant scope: a commit is scoped to exactly
///   one tenant (protocol §13.1);
/// * every intent id is distinct *except* that a `stack.transfer` /
///   `balance.transfer` may appear exactly twice as an atomic debit + credit
///   pair sharing one intent id, the same resource, and net-zero amounts
///   (§20.5, §18.3).
///
/// # Errors
///
/// Returns [`ExecutorError::AtomicityViolation`] when the batch contains a
/// duplicate event id, events from different tenants, or a multi-event intent
/// that is not a valid transfer pair, and
/// [`ExecutorError::TransferMismatch`] when a transfer pair's net-zero (debit
/// equals credit) or amount invariant is violated.
pub fn validate_batch_consistency(events: &[Event]) -> Result<(), ExecutorError> {
    let mut intent_to_events: BTreeMap<IntentId, Vec<&Event>> = BTreeMap::new();
    let mut event_ids = BTreeSet::new();
    let mut first_tenant: Option<&str> = None;

    for event in events {
        if !event_ids.insert(&event.event_id) {
            return Err(ExecutorError::AtomicityViolation(format!(
                "duplicate event id `{}` in batch",
                event.event_id.as_str()
            )));
        }
        let tenant = event.tenant_id.0.as_str();
        if let Some(first) = first_tenant {
            if first != tenant {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "mixed tenant scopes in batch (first tenant `{first}`)"
                )));
            }
        } else {
            first_tenant = Some(tenant);
        }
        intent_to_events
            .entry(event.intent_id.clone())
            .or_default()
            .push(event);
    }

    for (intent_id, group) in intent_to_events {
        if group.len() == 1 {
            continue;
        }
        validate_transfer_pair(&intent_id, &group)?;
    }
    Ok(())
}

/// Validates the batch-level shape of a value-leg settlement (Phase 2) and a
/// bundle settlement (Phase 4).
///
/// Runs AFTER [`validate_batch_consistency`] (which stays untouched) and is the
/// fail-closed shape check that makes a settlement an atomic unit. A value-leg
/// settlement batch may grow from `[trade.settle]` to
/// `[trade.settle, balance.transfer x2]`: one settle intent plus one atomic
/// net-zero `balance.transfer` pair (2 events sharing the value leg's distinct
/// intent id) when the settle declares a value leg.
///
/// The check enforces, for the given settle intents and the events they
/// produced:
///
/// * (a) exactly one `trade.settle` event per settle intent in the batch, or,
///   when the settle intents declare a bundle (Phase 4), a bundle shape via
///   [`check_bundle_shape`]: every settle intent declares the same positive
///   `bundle_size` and `trade_id`, the settle-event count equals the declared
///   bundle size, and the settled assets are distinct;
/// * (b) for every settle intent that declares a value leg, exactly one
///   net-zero `balance.transfer` pair is present, and the pair's debit equals
///   the declared `value_amount` (the pair's intent id is distinct from the
///   settle intent's id); a bundle may carry M value legs (M >= 0), each
///   matched 1:1;
/// * (c) no undeclared multi-event intent groups: every intent id that spans
///   more than one event must be a declared value-leg pair;
/// * (d) all events share the batch's tenant scope.
///
/// This validator is strictly additive: it does NOT weaken
/// [`validate_transfer_pair`] (the value pair is still validated by the
/// existing transfer-pair rule) or [`validate_batch_consistency`].
///
/// # Errors
///
/// Returns [`ExecutorError::AtomicityViolation`] for any shape violation
/// (wrong settle-event count, an invalid bundle declaration, a duplicate
/// asset, a missing/mismatched value pair, an undeclared multi-event group, or
/// mixed tenant scopes) and [`ExecutorError::TransferMismatch`] when a value
/// pair's amounts are malformed or a declared `value_amount` does not parse.
pub fn validate_settle_batch(
    events: &[Event],
    settle_intents: &[Intent],
) -> Result<(), ExecutorError> {
    // (d) All events must share one tenant scope.
    let mut first_tenant: Option<&str> = None;
    for event in events {
        let tenant = event.tenant_id.0.as_str();
        if let Some(first) = first_tenant {
            if first != tenant {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "mixed tenant scopes in settle batch (first tenant `{first}`)"
                )));
            }
        } else {
            first_tenant = Some(tenant);
        }
    }

    // (a) Exactly one trade.settle event per settle intent in the batch, OR a
    // declared bundle (Phase 4): every settle intent declares the same
    // positive `bundle_size` and `trade_id`, and the settle-event count equals
    // the declared bundle size with distinct assets.
    let settle_event_count = events
        .iter()
        .filter(|event| &event.operation == asset_op::trade_settle())
        .count();
    if settle_intents.iter().any(declares_bundle) {
        check_bundle_shape(settle_intents, settle_event_count, events)?;
    } else {
        if settle_event_count != settle_intents.len() {
            return Err(ExecutorError::AtomicityViolation(format!(
                "settle batch has {settle_event_count} trade.settle event(s) for {} settle intent(s)",
                settle_intents.len()
            )));
        }
    }

    // Group ALL events by intent id.
    let mut all_groups: BTreeMap<IntentId, Vec<&Event>> = BTreeMap::new();
    for event in events {
        all_groups
            .entry(event.intent_id.clone())
            .or_default()
            .push(event);
    }

    // Recover the value pairs: `balance.transfer` groups of exactly two events
    // (the atomic debit + credit pair sharing one value-leg intent id).
    let mut value_pairs: Vec<(IntentId, Amount)> = Vec::new();
    for (intent_id, group) in &all_groups {
        if group
            .first()
            .is_some_and(|event| &event.operation == balance_op::balance_transfer())
        {
            if group.len() != 2 {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "balance.transfer intent `{}` in settle batch is not an atomic debit + credit pair",
                    intent_id.as_str()
                )));
            }
            let debit = value_pair_debit(group)?;
            value_pairs.push((intent_id.clone(), debit));
        }
    }
    let value_pair_ids: BTreeSet<&IntentId> = value_pairs.iter().map(|(id, _)| id).collect();

    // (c) No undeclared multi-event intent groups: every intent id spanning more
    // than one event must be a declared value-leg pair. A stack.transfer pair,
    // a multi-event group of any other operation, or a lone balance.transfer
    // event are all undeclared and fail closed.
    for (intent_id, group) in &all_groups {
        if group.len() > 1 && !value_pair_ids.contains(intent_id) {
            return Err(ExecutorError::AtomicityViolation(format!(
                "undeclared multi-event intent group `{}` in settle batch",
                intent_id.as_str()
            )));
        }
    }

    // The value-leg pairs must not reuse a settle intent's id.
    let settle_ids: BTreeSet<&IntentId> = settle_intents
        .iter()
        .map(|intent| &intent.intent_id)
        .collect();
    for (intent_id, _) in &value_pairs {
        if settle_ids.contains(intent_id) {
            return Err(ExecutorError::AtomicityViolation(format!(
                "value leg intent id `{}` collides with a settle intent id",
                intent_id.as_str()
            )));
        }
    }

    // (b) Every value-declaring settle intent must be matched 1:1 by a
    // net-zero value pair whose debit equals the declared value_amount.
    let mut declared_amounts: Vec<Amount> = Vec::new();
    for intent in settle_intents {
        if declares_value_leg(intent) {
            declared_amounts.push(declared_value_amount(intent)?);
        }
    }
    if value_pairs.len() != declared_amounts.len() {
        return Err(ExecutorError::AtomicityViolation(format!(
            "settle batch declares {} value-leg settle(s) but has {} balance.transfer pair(s)",
            declared_amounts.len(),
            value_pairs.len()
        )));
    }
    let mut pair_debits: Vec<Amount> = value_pairs.into_iter().map(|(_, debit)| debit).collect();
    declared_amounts.sort();
    pair_debits.sort();
    if declared_amounts != pair_debits {
        return Err(ExecutorError::AtomicityViolation(String::from(
            "settle batch value amount mismatch: declared value_amount(s) do not match the balance.transfer pair debit(s)",
        )));
    }

    Ok(())
}

/// Returns whether a settle intent declares a bundle (Phase 4).
///
/// A bundle is declared when the settle intent carries a `bundle_size` input.
/// The `trade_id` is required by the profile for every settle; the bundle shape
/// check ([`check_bundle_shape`]) requires all settle intents in a bundle to
/// declare the same `bundle_size` and `trade_id`.
fn declares_bundle(intent: &Intent) -> bool {
    intent.inputs.contains_key(keys::BUNDLE_SIZE)
}

/// Checks the bundle shape of a bundle-declaring settle batch (Phase 4).
///
/// A bundle settle is one atomic batch settling `bundle_size` distinct assets.
/// Every settle intent in a bundle declares the bundle (any one declaring it
/// requires them all to): each carries a positive-integer `bundle_size` and a
/// `trade_id`, and all must agree on both. The batch must carry exactly
/// `bundle_size` `trade.settle` events, one per asset, with no resource id
/// appearing twice.
///
/// The per-asset gates (asset active and owned by the proposer, trade consent)
/// live in the unique-asset profile's `trade.lock` / `trade.settle` rules; this
/// batch-level check enforces only the cross-asset constraints that no single
/// [`ProfileRules::check`](crate::registry::ProfileRules::check) sees: a shared
/// declared bundle size and trade id, the expected settle-event count, and
/// asset distinctness.
///
/// # Errors
///
/// Returns [`ExecutorError::AtomicityViolation`] for any shape violation: a
/// partial bundle declaration, a non-positive or inconsistent `bundle_size`, a
/// mixed `trade_id`/`bundle_size` across the bundle, a settle-event count that
/// does not equal the declared bundle size, or a duplicated asset.
fn check_bundle_shape(
    settle_intents: &[Intent],
    settle_event_count: usize,
    events: &[Event],
) -> Result<(), ExecutorError> {
    // Every settle intent in a bundle must declare the bundle (a partial
    // bundle, mixing declared and undeclared settle intents, fails closed).
    if settle_intents.iter().any(|intent| !declares_bundle(intent)) {
        return Err(ExecutorError::AtomicityViolation(String::from(
            "partial bundle declaration: some settle intents declare `bundle_size` but not all",
        )));
    }

    let mut bundle_sizes: Vec<u64> = Vec::new();
    let mut trade_ids: Vec<&str> = Vec::new();
    for intent in settle_intents {
        let size_value = intent.inputs.get(keys::BUNDLE_SIZE).ok_or_else(|| {
            ExecutorError::AtomicityViolation(format!(
                "bundle settle intent `{}` is missing `bundle_size`",
                intent.intent_id.as_str()
            ))
        })?;
        let size = size_value.as_u64().ok_or_else(|| {
            ExecutorError::AtomicityViolation(format!(
                "bundle settle intent `{}` has a non-integer `bundle_size`",
                intent.intent_id.as_str()
            ))
        })?;
        if size == 0 {
            return Err(ExecutorError::AtomicityViolation(format!(
                "bundle settle intent `{}` declares a non-positive `bundle_size`",
                intent.intent_id.as_str()
            )));
        }
        let trade_id = intent.inputs.get(keys::TRADE_ID).ok_or_else(|| {
            ExecutorError::AtomicityViolation(format!(
                "bundle settle intent `{}` is missing `trade_id`",
                intent.intent_id.as_str()
            ))
        })?;
        let trade_id = trade_id.as_str().ok_or_else(|| {
            ExecutorError::AtomicityViolation(format!(
                "bundle settle intent `{}` has a non-string `trade_id`",
                intent.intent_id.as_str()
            ))
        })?;
        if trade_id.is_empty() {
            return Err(ExecutorError::AtomicityViolation(format!(
                "bundle settle intent `{}` has an empty `trade_id`",
                intent.intent_id.as_str()
            )));
        }
        bundle_sizes.push(size);
        trade_ids.push(trade_id);
    }

    // All settle intents in the bundle must agree on one `bundle_size` and one
    // `trade_id` (the batch is one trade, atomic end to end).
    let Some(first_size) = bundle_sizes.first() else {
        return Err(ExecutorError::AtomicityViolation(String::from(
            "empty bundle settle intent set",
        )));
    };
    if bundle_sizes.iter().any(|size| size != first_size) {
        return Err(ExecutorError::AtomicityViolation(String::from(
            "inconsistent `bundle_size` across the bundle settle intents",
        )));
    }
    let Some(first_trade) = trade_ids.first() else {
        return Err(ExecutorError::AtomicityViolation(String::from(
            "empty bundle settle intent set",
        )));
    };
    if trade_ids.iter().any(|trade| trade != first_trade) {
        return Err(ExecutorError::AtomicityViolation(String::from(
            "mixed `trade_id` across the bundle settle intents",
        )));
    }

    // The settle-event count must equal the declared bundle size (every asset
    // in the bundle is settled exactly once).
    let expected: usize = usize::try_from(*first_size).map_err(|_source| {
        ExecutorError::AtomicityViolation(String::from(
            "declared `bundle_size` does not fit a usize",
        ))
    })?;
    if settle_event_count != expected {
        return Err(ExecutorError::AtomicityViolation(format!(
            "bundle settle batch has {settle_event_count} trade.settle event(s) but declares bundle_size {first_size}",
        )));
    }

    // Assets are distinct: no resource id appears twice among the settle events.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for event in events {
        if &event.operation == asset_op::trade_settle()
            && !seen.insert(event.resource_id.0.as_str())
        {
            return Err(ExecutorError::AtomicityViolation(format!(
                "duplicate asset `{}` in bundle settle batch",
                event.resource_id.0
            )));
        }
    }

    Ok(())
}

/// Returns whether a settle intent declares a value leg.
///
/// A value leg is declared when any of `value_resource`, `value_amount`, or
/// `value_to_subject` is present (the profile requires all three together;
/// `validate_settle_batch` fails closed on a partial declaration via
/// [`declared_value_amount`]).
fn declares_value_leg(intent: &Intent) -> bool {
    intent.inputs.contains_key(keys::VALUE_RESOURCE)
        || intent.inputs.contains_key(keys::VALUE_AMOUNT)
        || intent.inputs.contains_key(keys::VALUE_TO_SUBJECT)
}

/// Parses the declared `value_amount` from a value-leg settle intent.
///
/// # Errors
///
/// Returns [`ExecutorError::AtomicityViolation`] when the declaration is
/// partial (missing `value_amount` / `value_resource` / `value_to_subject`),
/// and [`ExecutorError::TransferMismatch`] when `value_amount` is not a
/// canonical non-negative integer string.
fn declared_value_amount(intent: &Intent) -> Result<Amount, ExecutorError> {
    let amount = intent.inputs.get(keys::VALUE_AMOUNT).ok_or_else(|| {
        ExecutorError::AtomicityViolation(format!(
            "settle intent `{}` declares a value leg but is missing `value_amount`",
            intent.intent_id.as_str()
        ))
    })?;
    let resource = intent.inputs.get(keys::VALUE_RESOURCE).ok_or_else(|| {
        ExecutorError::AtomicityViolation(format!(
            "settle intent `{}` declares a value leg but is missing `value_resource`",
            intent.intent_id.as_str()
        ))
    })?;
    let to_subject = intent.inputs.get(keys::VALUE_TO_SUBJECT).ok_or_else(|| {
        ExecutorError::AtomicityViolation(format!(
            "settle intent `{}` declares a value leg but is missing `value_to_subject`",
            intent.intent_id.as_str()
        ))
    })?;
    let resource_text = resource.as_str().ok_or_else(|| {
        ExecutorError::AtomicityViolation(format!(
            "settle intent `{}` has a non-string `value_resource`",
            intent.intent_id.as_str()
        ))
    })?;
    let to_subject_text = to_subject.as_str().ok_or_else(|| {
        ExecutorError::AtomicityViolation(format!(
            "settle intent `{}` has a non-string `value_to_subject`",
            intent.intent_id.as_str()
        ))
    })?;
    if resource_text.is_empty() || to_subject_text.is_empty() {
        return Err(ExecutorError::AtomicityViolation(format!(
            "settle intent `{}` has an empty value-leg member",
            intent.intent_id.as_str()
        )));
    }
    Amount::try_from_str(amount.as_str().ok_or_else(|| {
        ExecutorError::AtomicityViolation(format!(
            "settle intent `{}` has a non-string `value_amount`",
            intent.intent_id.as_str()
        ))
    })?)
    .map_err(|_source| {
        ExecutorError::TransferMismatch(format!(
            "settle intent `{}` declares a malformed `value_amount`",
            intent.intent_id.as_str()
        ))
    })
}

/// Computes the net debit of a `balance.transfer` value pair.
///
/// Sums the balance deltas of the events in the group that are debits (the
/// source legs). `validate_transfer_pair` already guarantees the pair is
/// net-zero, so the debit equals the credited value amount.
///
/// # Errors
///
/// Returns [`ExecutorError::TransferMismatch`] when a balance amount is
/// malformed or the accumulated debit overflows.
fn value_pair_debit(group: &[&Event]) -> Result<Amount, ExecutorError> {
    let mut debit = Amount::ZERO;
    for event in group {
        let before = commitment_amount(&event.before, keys::BALANCE)?;
        let after = commitment_amount(&event.after, keys::BALANCE)?;
        if before > after {
            let delta = before
                .checked_sub(after)
                .ok_or_else(|| ExecutorError::TransferMismatch(String::from("debit overflow")))?;
            debit = debit
                .checked_add(delta)
                .ok_or_else(|| ExecutorError::TransferMismatch(String::from("debit overflow")))?;
        }
    }
    Ok(debit)
}

/// A tenant-scoped group of events produced by one leg of a cross-tenant
/// transaction (protocol §8.2, §18.3).
///
/// This is an internal return type of [`crate::pipeline::Executor::execute_cross_tenant`];
/// it is not a wire object and carries no serde derives.
#[derive(Debug)]
pub struct TenantEventGroup {
    /// The tenant scope of this leg's events.
    pub tenant: TenantId,
    /// The events produced for this tenant leg, in execution order.
    pub events: Vec<Event>,
}

/// The declared linkage manifest for a cross-tenant trade settlement
/// (Phase 3).
///
/// A cross-tenant trade spans two or more tenants with a distinct intent id per
/// leg, so no single intent id spans two tenant groups and the inferred-linkage
/// rule ([`validate_cross_tenant_consistency`]) cannot admit it. Instead of
/// inferring the linkage from a shared id, the caller DECLARES it here: the
/// settle intent id (the linkage anchor) and, when the settle settles for
/// fungible value, the value-leg resource, amount, and recipient. The manifest
/// is the API input to
/// [`Executor::execute_cross_tenant_trade`](crate::pipeline::Executor::execute_cross_tenant_trade)
/// and is checked by [`validate_cross_tenant_trade`].
///
/// This is a wire/API type for settlement (not a persisted domain object). It
/// carries no serde derives, matching the adjacent internal [`TenantEventGroup`]
/// return type.
#[derive(Debug, Clone)]
pub struct TradeManifest {
    /// The trade identifier being settled.
    pub trade_id: String,
    /// The declared `trade.settle` intent id (the linkage anchor).
    pub settle_intent_id: IntentId,
    /// The declared value leg, when the settle settles for fungible value.
    pub value_leg: Option<ValueLeg>,
    /// The assets this manifest's side settles (Phase 4 bundles).
    ///
    /// An empty vector means a single-asset settle (the Phase 3 behavior):
    /// exactly one `trade.settle` event for [`Self::settle_intent_id`]. A
    /// non-empty vector declares a bundle settle: the batch must carry exactly
    /// one `trade.settle` event per declared asset, each asset appearing
    /// exactly once.
    pub settle_assets: Vec<ResourceId>,
}

/// The declared fungible value leg of a cross-tenant trade settlement.
///
/// Mirrors the Phase 2 value-leg inputs on a `trade.settle` intent
/// (`value_resource`, `value_amount`, `value_to_subject`), declared explicitly
/// in the manifest so the linkage is declared rather than inferred.
#[derive(Debug, Clone)]
pub struct ValueLeg {
    /// The fungible resource moved by the value leg.
    pub resource: ResourceId,
    /// The canonical non-negative integer amount of the value leg.
    pub amount: String,
    /// The subject credited by the value leg.
    pub to_subject: SubjectId,
}

/// Validates cross-tenant consistency across per-tenant event groups
/// (protocol §8.2, §18.3).
///
/// A cross-tenant transaction is consistent when:
///
/// * (a) every event in a group is scoped to that group's tenant: a partition
///   mismatch fails closed;
/// * (b) each group is internally batch-consistent via
///   [`validate_batch_consistency`] (distinct event ids and distinct intent ids
///   except a valid atomic transfer pair);
/// * (c) the transaction is genuinely cross-tenant: at least one intent id
///   appears in two or more distinct tenant groups (the linkage that ties the
///   legs together).
///
/// The existing per-tenant transfer-pair rule ([`validate_transfer_pair`],
/// reached through [`validate_batch_consistency`]) is still enforced within each
/// group; this function introduces no new conservation rules.
///
/// # Errors
///
/// Returns [`ExecutorError::AtomicityViolation`] when a group contains events
/// scoped to a different tenant, when a group is internally inconsistent, or
/// when no intent id links at least two distinct tenant groups.
pub fn validate_cross_tenant_consistency(groups: &[TenantEventGroup]) -> Result<(), ExecutorError> {
    let mut intent_tenants: BTreeMap<&IntentId, BTreeSet<&str>> = BTreeMap::new();
    for group in groups {
        for event in &group.events {
            if event.tenant_id != group.tenant {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "tenant mismatch in cross-tenant batch (event tenant `{}`, group tenant `{}`)",
                    event.tenant_id.0, group.tenant.0
                )));
            }
            intent_tenants
                .entry(&event.intent_id)
                .or_default()
                .insert(group.tenant.0.as_str());
        }
        validate_batch_consistency(&group.events)?;
    }
    if !intent_tenants.values().any(|tenants| tenants.len() >= 2) {
        return Err(ExecutorError::AtomicityViolation(String::from(
            "cross-tenant batch has no cross-tenant intent linkage",
        )));
    }
    Ok(())
}

/// Validates a cross-tenant trade settlement against its declared linkage
/// manifest (protocol §18.3, Phase 3).
///
/// This is the declared-linkage variant for trades: unlike
/// [`validate_cross_tenant_consistency`], it does not require one intent id to
/// span two tenant groups (distinct leg ids are the norm for a trade). Instead
/// it checks the batch against the [`TradeManifest`], which names the settle
/// intent and, optionally, the value leg:
///
/// * (a) each group is internally consistent via [`validate_batch_consistency`]
///   (untouched), and every event is scoped to its group's tenant (a partition
///   mismatch fails closed);
/// * (b) the manifest's `settle_intent_id` produced the declared settle
///   events: exactly one `trade.settle` event for a single-asset settle, or
///   exactly one per declared bundle asset (Phase 4) with no duplicate, missing,
///   or undeclared asset, and that intent id is one of the supplied
///   `settle_intents`;
/// * (c) when the manifest declares a value leg, exactly one net-zero
///   `balance.transfer` pair (a distinct intent id) landed in a (possibly
///   different) tenant group, its debit equals the manifest amount, and its
///   resource and credited recipient match the manifest declaration;
/// * (d) every intent id in the batch resolves to a declared leg: no
///   undeclared multi-event intent groups, and the value pair does not reuse
///   the settle intent id;
/// * (e) the batch spans at least two distinct tenant groups.
///
/// This validator is strictly additive: it does NOT weaken
/// [`validate_cross_tenant_consistency`] or [`validate_transfer_pair`]. The
/// value pair is still validated by the existing per-tenant transfer-pair rule
/// (reached through [`validate_batch_consistency`]).
///
/// # Errors
///
/// Returns [`ExecutorError::AtomicityViolation`] for any shape violation
/// (fewer than two tenants, a partition mismatch, a wrong settle-event count,
/// an undeclared multi-event group, a missing/mismatched value pair, a value
/// pair reusing the settle intent id, or a settle intent id absent from
/// `settle_intents`), and [`ExecutorError::TransferMismatch`] when a manifest
/// value amount is not a canonical non-negative integer string.
pub fn validate_cross_tenant_trade(
    groups: &[TenantEventGroup],
    manifest: &TradeManifest,
    settle_intents: &[Intent],
) -> Result<(), ExecutorError> {
    // (e) The transaction must genuinely span at least two distinct tenants.
    let tenant_names: BTreeSet<&str> = groups.iter().map(|group| group.tenant.0.as_str()).collect();
    if tenant_names.len() < 2 {
        return Err(ExecutorError::AtomicityViolation(String::from(
            "cross-tenant trade requires at least two distinct tenants",
        )));
    }

    // (a) Partition invariant + per-group internal consistency.
    let mut all_events: Vec<&Event> = Vec::new();
    for group in groups {
        for event in &group.events {
            if event.tenant_id != group.tenant {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "tenant mismatch in cross-tenant trade (event tenant `{}`, group tenant `{}`)",
                    event.tenant_id.0, group.tenant.0
                )));
            }
        }
        validate_batch_consistency(&group.events)?;
        all_events.extend(group.events.iter());
    }

    // The declared settle intent must actually be one of the settle intents
    // executed in the batch (the declared linkage resolves to a real settle).
    if !settle_intents
        .iter()
        .any(|intent| intent.intent_id == manifest.settle_intent_id)
    {
        return Err(ExecutorError::AtomicityViolation(format!(
            "manifest settle intent `{}` is not among the batch's settle intents",
            manifest.settle_intent_id.as_str()
        )));
    }

    // (b) The manifest settle intent must have produced the expected settle
    // events: exactly one for a single-asset settle, or exactly one per
    // declared bundle asset (Phase 4) with no duplicate, missing, or
    // undeclared asset.
    let settle_events: Vec<&Event> = all_events
        .iter()
        .filter(|event| {
            event.operation == *asset_op::trade_settle()
                && event.intent_id == manifest.settle_intent_id
        })
        .copied()
        .collect();
    if manifest.settle_assets.is_empty() {
        if settle_events.len() != 1 {
            return Err(ExecutorError::AtomicityViolation(format!(
                "cross-tenant trade settle intent `{}` produced {} settle event(s), expected exactly one",
                manifest.settle_intent_id.as_str(),
                settle_events.len()
            )));
        }
    } else {
        let expected = manifest.settle_assets.len();
        if settle_events.len() != expected {
            return Err(ExecutorError::AtomicityViolation(format!(
                "cross-tenant bundle settle intent `{}` produced {} settle event(s), expected {expected} for the declared bundle",
                manifest.settle_intent_id.as_str(),
                settle_events.len()
            )));
        }
        let mut settled: BTreeSet<&str> = BTreeSet::new();
        for event in &settle_events {
            if !settled.insert(event.resource_id.0.as_str()) {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "duplicate asset `{}` in cross-tenant bundle settle",
                    event.resource_id.0
                )));
            }
        }
        let declared: BTreeSet<&str> = manifest
            .settle_assets
            .iter()
            .map(|asset| asset.0.as_str())
            .collect();
        if settled != declared {
            return Err(ExecutorError::AtomicityViolation(String::from(
                "cross-tenant bundle settle assets do not match the manifest exactly (a declared asset is missing or an undeclared asset is present)",
            )));
        }
    }

    // Group ALL events by intent id to reason about declared legs.
    let mut all_groups: BTreeMap<IntentId, Vec<&Event>> = BTreeMap::new();
    for event in &all_events {
        all_groups
            .entry(event.intent_id.clone())
            .or_default()
            .push(event);
    }

    // Recover the value pairs: balance.transfer groups of exactly two events
    // (the atomic debit + credit pair sharing one value-leg intent id).
    let mut value_pairs: Vec<(IntentId, Amount)> = Vec::new();
    for (intent_id, group) in &all_groups {
        if group
            .first()
            .is_some_and(|event| &event.operation == balance_op::balance_transfer())
        {
            if group.len() != 2 {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "balance.transfer intent `{}` in cross-tenant trade is not an atomic debit + credit pair",
                    intent_id.as_str()
                )));
            }
            let debit = value_pair_debit(group)?;
            value_pairs.push((intent_id.clone(), debit));
        }
    }

    // (d) No undeclared multi-event intent groups: every intent id spanning more
    // than one event must be a declared value-leg pair. The settle intent is a
    // single event (already enforced by (b)).
    // (d) Every intent id in the batch must resolve to a declared leg: the
    // manifest's settle intent id or, when the manifest declares a value leg,
    // that value pair's intent id. This fails closed both on undeclared
    // multi-event groups and on undeclared single-event intents, so a 3-tenant
    // trade carrying a second asset leg is rejected: the manifest path supports
    // exactly one settle leg plus one optional value leg.
    let mut declared_ids: BTreeSet<&IntentId> = BTreeSet::new();
    declared_ids.insert(&manifest.settle_intent_id);
    if manifest.value_leg.is_some() {
        for (intent_id, _) in &value_pairs {
            declared_ids.insert(intent_id);
        }
    }
    for intent_id in all_groups.keys() {
        if !declared_ids.contains(intent_id) {
            return Err(ExecutorError::AtomicityViolation(format!(
                "undeclared intent id `{}` in cross-tenant trade (only the manifest settle and value leg are declared)",
                intent_id.as_str()
            )));
        }
    }

    // The value pair(s) must not reuse the settle intent id.
    for (intent_id, _) in &value_pairs {
        if intent_id == &manifest.settle_intent_id {
            return Err(ExecutorError::AtomicityViolation(format!(
                "value leg intent id `{}` collides with the settle intent id",
                intent_id.as_str()
            )));
        }
    }

    // (c) Value-leg validation against the manifest declaration.
    match &manifest.value_leg {
        Some(leg) => {
            let expected = Amount::try_from_str(&leg.amount).map_err(|_source| {
                ExecutorError::TransferMismatch(format!(
                    "manifest value leg declares a malformed amount `{}`",
                    leg.amount
                ))
            })?;
            if value_pairs.len() != 1 {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "manifest declares a value leg but the batch has {} balance.transfer pair(s)",
                    value_pairs.len()
                )));
            }
            let Some((pair_id, debit)) = value_pairs.first() else {
                return Err(ExecutorError::AtomicityViolation(String::from(
                    "missing value pair in cross-tenant trade",
                )));
            };
            if debit != &expected {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "cross-tenant trade value amount mismatch: manifest declares {expected} but the value pair debits {debit}"
                )));
            }
            let pair_events = all_groups.get(pair_id).ok_or_else(|| {
                ExecutorError::AtomicityViolation(String::from(
                    "missing value-pair events in cross-tenant trade",
                ))
            })?;
            // The pair must move the declared value-leg resource.
            if pair_events
                .first()
                .map(|event| event.resource_id != leg.resource)
                .unwrap_or(true)
            {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "cross-tenant trade value leg resource mismatch: manifest declares `{}`",
                    leg.resource.0
                )));
            }
            // The credited destination must be the declared recipient. The
            // credit event is the pair leg whose balance increased.
            let mut credited_subject: Option<&str> = None;
            for event in pair_events {
                let before = commitment_amount(&event.before, keys::BALANCE)?;
                let after = commitment_amount(&event.after, keys::BALANCE)?;
                if after > before {
                    credited_subject = event.after.state.get(keys::SUBJECT).and_then(Value::as_str);
                }
            }
            if credited_subject != Some(leg.to_subject.0.as_str()) {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "cross-tenant trade value leg recipient mismatch: manifest declares `{}`",
                    leg.to_subject.0
                )));
            }
        }
        None => {
            // No value leg declared: the batch must carry no value pair.
            if !value_pairs.is_empty() {
                return Err(ExecutorError::AtomicityViolation(String::from(
                    "cross-tenant trade declares no value leg but the batch carries a balance.transfer pair",
                )));
            }
        }
    }

    Ok(())
}

/// Validates that a multi-event intent group is an atomic transfer pair.
///
/// A transfer pair must contain exactly two events of the same
/// `stack.transfer` / `balance.transfer` operation over the same resource,
/// whose net debit equals net credit (net-zero) with a strictly positive
/// amount (§20.5).
///
/// # Errors
///
/// Returns [`ExecutorError::AtomicityViolation`] when the group is not exactly
/// two events of a transfer operation over the same resource, and
/// [`ExecutorError::TransferMismatch`] when the pair's amounts are malformed or
/// net-zero is violated.
fn validate_transfer_pair(intent_id: &IntentId, group: &[&Event]) -> Result<(), ExecutorError> {
    if group.len() != 2 {
        return Err(duplicate_intent_error(intent_id));
    }
    let Some(first) = group.first() else {
        return Err(duplicate_intent_error(intent_id));
    };
    let Some(second) = group.get(1) else {
        return Err(duplicate_intent_error(intent_id));
    };
    let Some(field) = transfer_field(&first.operation) else {
        return Err(duplicate_intent_error(intent_id));
    };
    if transfer_field(&second.operation) != Some(field) {
        return Err(duplicate_intent_error(intent_id));
    }
    if first.operation != second.operation {
        return Err(duplicate_intent_error(intent_id));
    }
    if first.resource_id != second.resource_id {
        return Err(duplicate_intent_error(intent_id));
    }

    let mut debit = Amount::ZERO;
    let mut credit = Amount::ZERO;
    for event in group {
        let before = commitment_amount(&event.before, field)?;
        let after = commitment_amount(&event.after, field)?;
        if before >= after {
            let delta = before
                .checked_sub(after)
                .ok_or_else(|| ExecutorError::TransferMismatch(String::from("debit overflow")))?;
            debit = debit
                .checked_add(delta)
                .ok_or_else(|| ExecutorError::TransferMismatch(String::from("debit overflow")))?;
        } else {
            let delta = after
                .checked_sub(before)
                .ok_or_else(|| ExecutorError::TransferMismatch(String::from("credit overflow")))?;
            credit = credit
                .checked_add(delta)
                .ok_or_else(|| ExecutorError::TransferMismatch(String::from("credit overflow")))?;
        }
    }
    if debit == Amount::ZERO || debit != credit {
        return Err(ExecutorError::TransferMismatch(format!(
            "net-zero violated for intent `{}`: debit {debit} != credit {credit}",
            intent_id.as_str()
        )));
    }
    Ok(())
}

/// Returns the amount field name for a transfer operation, or `None` when the
/// operation is not a subject-held transfer.
fn transfer_field(operation: &Operation) -> Option<&'static str> {
    if operation == stack_op::stack_transfer() {
        Some(keys::QUANTITY)
    } else if operation == balance_op::balance_transfer() {
        Some(keys::BALANCE)
    } else {
        None
    }
}

/// Reads the fixed-point [`Amount`] for `field` from a state commitment.
///
/// A missing field (e.g. the empty before-state of a create-on-credit
/// destination) reads as zero.
///
/// # Errors
///
/// Returns [`ExecutorError::TransferMismatch`] when the field is present but
/// not a canonical non-negative decimal integer string.
fn commitment_amount(commitment: &StateCommitment, field: &str) -> Result<Amount, ExecutorError> {
    let Some(text) = commitment.state.get(field).and_then(Value::as_str) else {
        return Ok(Amount::ZERO);
    };
    Amount::try_from_str(text)
        .map_err(|_source| ExecutorError::TransferMismatch(format!("malformed `{field}` amount")))
}

/// Builds the fail-closed duplicate-intent error for a non-transfer group.
fn duplicate_intent_error(intent_id: &IntentId) -> ExecutorError {
    ExecutorError::AtomicityViolation(format!(
        "duplicate intent id `{}` in batch (only transfer pairs may share an intent id)",
        intent_id.as_str()
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use statechronicle_core::digest::hash_bytes;
    use statechronicle_domain::ids::{EventId, IntentId};
    use statechronicle_domain::intent::{Intent, Nonce, Operation};
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::state_type::StateType;
    use statechronicle_domain::subject::SubjectId;
    use statechronicle_domain::tenant::TenantId;

    fn tenant(name: &str) -> TenantId {
        TenantId(String::from(name))
    }

    fn event(id: &str, tenant_name: &str, intent: &str) -> Event {
        Event::new(
            tenant(tenant_name),
            EventId::new(format!("evt_{id}")).unwrap(),
            IntentId::new(format!("int_{intent}")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            ResourceId(String::from("asset:sword_001")),
            SubjectId(String::from("account:example:player_123")),
            statechronicle_domain::event::StateCommitment {
                version: 1,
                state_hash: hash_bytes(b"before"),
                state: serde_json::json!({}),
            },
            statechronicle_domain::event::StateCommitment {
                version: 2,
                state_hash: hash_bytes(b"after"),
                state: serde_json::json!({ "owner": "bob", "status": "active" }),
            },
            None,
            SubjectId(String::from("service:statechronicle.example.net")),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }

    #[test]
    fn consistent_batch_passes() {
        let events = vec![
            event("a", "acme.game.alpha", "1"),
            event("b", "acme.game.alpha", "2"),
        ];
        assert!(validate_batch_consistency(&events).is_ok());
    }

    #[test]
    fn duplicate_intent_id_fails_closed() {
        let events = vec![
            event("a", "acme.game.alpha", "1"),
            event("b", "acme.game.alpha", "1"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("duplicate intent id")
        ));
    }

    #[test]
    fn duplicate_event_id_fails_closed() {
        let events = vec![
            event("a", "acme.game.alpha", "1"),
            event("a", "acme.game.alpha", "2"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("duplicate event id")
        ));
    }

    #[test]
    fn mixed_tenants_fail_closed() {
        let events = vec![
            event("a", "acme.game.alpha", "1"),
            event("b", "acme.game.other", "2"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("mixed tenant scopes")
        ));
    }

    #[test]
    fn empty_batch_passes() {
        assert!(validate_batch_consistency(&[]).is_ok());
    }

    /// Builds a `balance.transfer` event for a subject-held resource with the
    /// given before/after balances (canonical integer strings).
    fn balance_transfer(
        event_id: &str,
        intent: &str,
        subject: &str,
        before: &str,
        after: &str,
    ) -> Event {
        Event::new(
            tenant("acme.game.alpha"),
            EventId::new(format!("evt_{event_id}")).unwrap(),
            IntentId::new(format!("int_{intent}")).unwrap(),
            Operation::new(String::from("balance.transfer")).unwrap(),
            ResourceId(String::from("currency:gold")),
            SubjectId(String::from(subject)),
            StateCommitment {
                version: 1,
                state_hash: hash_bytes(b"before"),
                state: serde_json::json!({
                    "subject": "alice",
                    "balance": before,
                    "unit": "gold_minor",
                }),
            },
            StateCommitment {
                version: 2,
                state_hash: hash_bytes(b"after"),
                state: serde_json::json!({
                    "subject": subject,
                    "balance": after,
                    "unit": "gold_minor",
                }),
            },
            None,
            SubjectId(String::from("service:statechronicle.example.net")),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }

    #[test]
    fn transfer_pair_net_zero_passes() {
        // Source: 100 -> 60 (debit 40). Destination: 0 (create) -> 40 (credit 40).
        let events = vec![
            balance_transfer("src", "t1", "alice", "100", "60"),
            balance_transfer("dst", "t1", "bob", "0", "40"),
        ];
        assert!(validate_batch_consistency(&events).is_ok());
    }

    #[test]
    fn transfer_pair_to_existing_destination_net_zero_passes() {
        // Source: 100 -> 60 (debit 40). Destination: 50 -> 90 (credit 40).
        let events = vec![
            balance_transfer("src", "t1", "alice", "100", "60"),
            balance_transfer("dst", "t1", "bob", "50", "90"),
        ];
        assert!(validate_batch_consistency(&events).is_ok());
    }

    #[test]
    fn transfer_pair_mismatched_amounts_fail_closed() {
        // Source: 100 -> 60 (debit 40). Destination: 0 -> 30 (credit 30).
        let events = vec![
            balance_transfer("src", "t1", "alice", "100", "60"),
            balance_transfer("dst", "t1", "bob", "0", "30"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::TransferMismatch(message))
            if message.contains("net-zero")
        ));
    }

    #[test]
    fn transfer_pair_two_debits_fail_closed() {
        let events = vec![
            balance_transfer("a", "t1", "alice", "100", "60"),
            balance_transfer("b", "t1", "bob", "50", "10"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::TransferMismatch(message))
            if message.contains("net-zero")
        ));
    }

    #[test]
    fn non_transfer_multi_event_rejected() {
        let events = vec![
            event("a", "acme.game.alpha", "1"),
            event("b", "acme.game.alpha", "1"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("duplicate intent id")
        ));
    }

    #[test]
    fn transfer_pair_with_three_events_rejected() {
        let events = vec![
            balance_transfer("a", "t1", "alice", "100", "60"),
            balance_transfer("b", "t1", "bob", "0", "40"),
            balance_transfer("c", "t1", "carol", "0", "1"),
        ];
        assert!(matches!(
            validate_batch_consistency(&events),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("duplicate intent id")
        ));
    }

    #[test]
    fn transfer_pair_different_resources_rejected() {
        let source = balance_transfer("src", "t1", "alice", "100", "60");
        let mut destination = balance_transfer("dst", "t1", "bob", "0", "40");
        destination.resource_id = ResourceId(String::from("currency:silver"));
        assert!(matches!(
            validate_batch_consistency(&[source, destination]),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("duplicate intent id")
        ));
    }

    /// Builds a `balance.transfer` event scoped to an arbitrary tenant, with the
    /// given before/after balances (canonical integer strings).
    fn transfer_event(
        tenant_name: &str,
        event_id: &str,
        intent: &str,
        subject: &str,
        before: &str,
        after: &str,
    ) -> Event {
        Event::new(
            tenant(tenant_name),
            EventId::new(format!("evt_{event_id}")).unwrap(),
            IntentId::new(format!("int_{intent}")).unwrap(),
            Operation::new(String::from("balance.transfer")).unwrap(),
            ResourceId(String::from("currency:gold")),
            SubjectId(String::from(subject)),
            StateCommitment {
                version: 1,
                state_hash: hash_bytes(b"before"),
                state: serde_json::json!({
                    "subject": "alice",
                    "balance": before,
                    "unit": "gold_minor",
                }),
            },
            StateCommitment {
                version: 2,
                state_hash: hash_bytes(b"after"),
                state: serde_json::json!({
                    "subject": subject,
                    "balance": after,
                    "unit": "gold_minor",
                }),
            },
            None,
            SubjectId(String::from("service:statechronicle.example.net")),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }

    #[test]
    fn cross_tenant_valid_pair_passes() {
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.alpha", "a", "x", "alice", "100", "60"),
                    transfer_event("acme.game.alpha", "b", "x", "bob", "0", "40"),
                ],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![
                    transfer_event("acme.game.beta", "c", "x", "carol", "100", "60"),
                    transfer_event("acme.game.beta", "d", "x", "dave", "0", "40"),
                ],
            },
        ];
        assert!(validate_cross_tenant_consistency(&groups).is_ok());
    }

    #[test]
    fn single_tenant_batch_rejected() {
        // Both groups declare the same tenant, so no intent spans two distinct
        // tenants. The batch is not genuinely cross-tenant.
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.alpha", "a", "x", "alice", "100", "60"),
                    transfer_event("acme.game.alpha", "b", "x", "bob", "0", "40"),
                ],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.alpha", "c", "y", "carol", "100", "60"),
                    transfer_event("acme.game.alpha", "d", "y", "dave", "0", "40"),
                ],
            },
        ];
        assert!(matches!(
            validate_cross_tenant_consistency(&groups),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("no cross-tenant intent linkage")
        ));
    }

    #[test]
    fn partition_mismatch_rejected() {
        // The first group declares alpha but holds beta-scoped events.
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.beta", "a", "x", "alice", "100", "60"),
                    transfer_event("acme.game.beta", "b", "x", "bob", "0", "40"),
                ],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![
                    transfer_event("acme.game.beta", "c", "x", "carol", "100", "60"),
                    transfer_event("acme.game.beta", "d", "x", "dave", "0", "40"),
                ],
            },
        ];
        assert!(matches!(
            validate_cross_tenant_consistency(&groups),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("tenant mismatch")
        ));
    }

    #[test]
    fn no_linkage_rejected() {
        // Two tenants, but each intent id appears in exactly one tenant.
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.alpha", "a", "x", "alice", "100", "60"),
                    transfer_event("acme.game.alpha", "b", "x", "bob", "0", "40"),
                ],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![
                    transfer_event("acme.game.beta", "c", "y", "carol", "100", "60"),
                    transfer_event("acme.game.beta", "d", "y", "dave", "0", "40"),
                ],
            },
        ];
        assert!(matches!(
            validate_cross_tenant_consistency(&groups),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("no cross-tenant intent linkage")
        ));
    }

    #[test]
    fn per_tenant_transfer_pair_still_enforced() {
        // Alpha's transfer pair is not net-zero (debit 40 != credit 30); the
        // per-tenant transfer-pair rule still applies across the cross-tenant
        // groups.
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![
                    transfer_event("acme.game.alpha", "a", "x", "alice", "100", "60"),
                    transfer_event("acme.game.alpha", "b", "x", "bob", "0", "30"),
                ],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![
                    transfer_event("acme.game.beta", "c", "x", "carol", "100", "60"),
                    transfer_event("acme.game.beta", "d", "x", "dave", "0", "40"),
                ],
            },
        ];
        assert!(matches!(
            validate_cross_tenant_consistency(&groups),
            Err(ExecutorError::TransferMismatch(message))
            if message.contains("net-zero")
        ));
    }

    /// Builds a `trade.settle` intent, optionally declaring a value leg.
    fn settle_intent(id: &str, value: Option<(&str, &str, &str)>) -> Intent {
        let mut inputs = BTreeMap::new();
        inputs.insert(String::from("from_owner"), serde_json::json!("alice"));
        inputs.insert(String::from("to_owner"), serde_json::json!("bob"));
        inputs.insert(String::from("trade_id"), serde_json::json!("trade_001"));
        if let Some((resource, amount, to_subject)) = value {
            inputs.insert(String::from("value_resource"), serde_json::json!(resource));
            inputs.insert(String::from("value_amount"), serde_json::json!(amount));
            inputs.insert(
                String::from("value_to_subject"),
                serde_json::json!(to_subject),
            );
        }
        Intent::new(
            tenant("acme.game.alpha"),
            IntentId::new(format!("int_{id}")).unwrap(),
            Operation::new(String::from("trade.settle")).unwrap(),
            SubjectId(String::from("account:example:player_456")),
            ResourceId(String::from("asset:sword_001")),
            Some(StateType::UniqueAsset),
            3,
            inputs,
            None,
            DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            None,
            Nonce::from_bytes(vec![1]).unwrap(),
        )
    }

    /// Builds a `trade.settle` event for a settled asset.
    fn settle_event(event_id: &str, intent: &str) -> Event {
        Event::new(
            tenant("acme.game.alpha"),
            EventId::new(format!("evt_{event_id}")).unwrap(),
            IntentId::new(format!("int_{intent}")).unwrap(),
            Operation::new(String::from("trade.settle")).unwrap(),
            ResourceId(String::from("asset:sword_001")),
            SubjectId(String::from("account:example:player_123")),
            StateCommitment {
                version: 3,
                state_hash: hash_bytes(b"before"),
                state: serde_json::json!({
                    "owner": "alice",
                    "status": "trade_held",
                    "trade_id": "trade_001",
                }),
            },
            StateCommitment {
                version: 4,
                state_hash: hash_bytes(b"after"),
                state: serde_json::json!({ "owner": "bob", "status": "active" }),
            },
            None,
            SubjectId(String::from("service:statechronicle.example.net")),
            DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }

    #[test]
    fn settle_batch_valid_asset_for_gold_passes() {
        // One settle intent declaring a value leg + one net-zero balance.transfer
        // pair whose debit matches the declared value_amount.
        let intents = vec![settle_intent(
            "settle",
            Some(("wallet:gold", "100", "alice")),
        )];
        let events = vec![
            settle_event("s1", "settle"),
            balance_transfer("vsrc", "value", "alice", "200", "100"),
            balance_transfer("vdst", "value", "bob", "0", "100"),
        ];
        assert!(validate_settle_batch(&events, &intents).is_ok());
    }

    #[test]
    fn settle_batch_pure_asset_for_asset_passes() {
        // A settle intent with no value leg needs no balance.transfer pair.
        let intents = vec![settle_intent("settle", None)];
        let events = vec![settle_event("s1", "settle")];
        assert!(validate_settle_batch(&events, &intents).is_ok());
    }

    #[test]
    fn settle_batch_missing_value_leg_fails() {
        // A settle intent declares a value leg but the batch carries no
        // balance.transfer pair.
        let intents = vec![settle_intent(
            "settle",
            Some(("wallet:gold", "100", "alice")),
        )];
        let events = vec![settle_event("s1", "settle")];
        assert!(matches!(
            validate_settle_batch(&events, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("balance.transfer pair")
        ));
    }

    #[test]
    fn settle_batch_value_amount_mismatch_fails() {
        // The settle declares 100 but the pair debits 50.
        let intents = vec![settle_intent(
            "settle",
            Some(("wallet:gold", "100", "alice")),
        )];
        let events = vec![
            settle_event("s1", "settle"),
            balance_transfer("vsrc", "value", "alice", "100", "50"),
            balance_transfer("vdst", "value", "bob", "0", "50"),
        ];
        assert!(matches!(
            validate_settle_batch(&events, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("value amount mismatch")
        ));
    }

    #[test]
    fn settle_batch_two_settles_one_value_leg_fails() {
        // Two settle intents both declaring value legs but only one value pair.
        let intents = vec![
            settle_intent("s1", Some(("wallet:gold", "100", "alice"))),
            settle_intent("s2", Some(("wallet:gold", "100", "alice"))),
        ];
        let events = vec![
            settle_event("e1", "s1"),
            settle_event("e2", "s2"),
            balance_transfer("vsrc", "value", "alice", "200", "100"),
            balance_transfer("vdst", "value", "bob", "0", "100"),
        ];
        assert!(matches!(
            validate_settle_batch(&events, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("balance.transfer pair")
        ));
    }

    #[test]
    fn settle_batch_undeclared_pair_fails() {
        // A balance.transfer pair with no declaring settle intent is rejected.
        let intents = vec![settle_intent("settle", None)];
        let events = vec![
            settle_event("e1", "settle"),
            balance_transfer("vsrc", "value", "alice", "200", "100"),
            balance_transfer("vdst", "value", "bob", "0", "100"),
        ];
        assert!(matches!(
            validate_settle_batch(&events, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("balance.transfer pair")
        ));
    }

    #[test]
    fn settle_batch_cross_tenant_mixed_fails() {
        // The value pair is scoped to a different tenant than the settle.
        let intents = vec![settle_intent(
            "settle",
            Some(("wallet:gold", "100", "alice")),
        )];
        let mut src = balance_transfer("vsrc", "value", "alice", "200", "100");
        src.tenant_id = tenant("acme.game.other");
        let mut dst = balance_transfer("vdst", "value", "bob", "0", "100");
        dst.tenant_id = tenant("acme.game.other");
        let events = vec![settle_event("e1", "settle"), src, dst];
        assert!(matches!(
            validate_settle_batch(&events, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("mixed tenant scopes")
        ));
    }

    /// Builds a value-declaring cross-tenant trade manifest over the settle
    /// intent `int_settle`, optionally naming a value leg.
    fn trade_manifest(value_leg: Option<ValueLeg>) -> TradeManifest {
        TradeManifest {
            trade_id: String::from("trade_001"),
            settle_intent_id: IntentId::new(String::from("int_settle")).unwrap(),
            value_leg,
            settle_assets: Vec::new(),
        }
    }

    /// Builds a bundle-declaring manifest settling the given assets.
    fn bundle_manifest(assets: &[&str]) -> TradeManifest {
        TradeManifest {
            trade_id: String::from("trade_001"),
            settle_intent_id: IntentId::new(String::from("int_settle")).unwrap(),
            value_leg: None,
            settle_assets: assets
                .iter()
                .map(|name| ResourceId(String::from(*name)))
                .collect(),
        }
    }

    /// Builds the canonical two-tenant asset-for-gold groups: a settle in alpha
    /// plus a net-zero value pair in beta (debit 100, credited to bob).
    fn asset_for_gold_groups() -> Vec<TenantEventGroup> {
        vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![settle_event("s1", "settle")],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![
                    transfer_event("acme.game.beta", "vsrc", "value", "alice", "200", "100"),
                    transfer_event("acme.game.beta", "vdst", "value", "bob", "0", "100"),
                ],
            },
        ]
    }

    #[test]
    fn cross_tenant_trade_asset_for_gold_passes() {
        let intents = vec![settle_intent("settle", None)];
        let manifest = trade_manifest(Some(ValueLeg {
            resource: ResourceId(String::from("currency:gold")),
            amount: String::from("100"),
            to_subject: SubjectId(String::from("bob")),
        }));
        assert!(validate_cross_tenant_trade(&asset_for_gold_groups(), &manifest, &intents).is_ok());
    }

    #[test]
    fn cross_tenant_trade_single_tenant_rejected() {
        let groups = vec![TenantEventGroup {
            tenant: tenant("acme.game.alpha"),
            events: vec![settle_event("s1", "settle")],
        }];
        let intents = vec![settle_intent("settle", None)];
        let manifest = trade_manifest(None);
        assert!(matches!(
            validate_cross_tenant_trade(&groups, &manifest, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("at least two distinct tenants")
        ));
    }

    #[test]
    fn cross_tenant_trade_missing_value_leg_fails() {
        // Manifest declares a value leg but no value pair landed anywhere.
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![settle_event("s1", "settle")],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![],
            },
        ];
        let intents = vec![settle_intent("settle", None)];
        let manifest = trade_manifest(Some(ValueLeg {
            resource: ResourceId(String::from("currency:gold")),
            amount: String::from("100"),
            to_subject: SubjectId(String::from("bob")),
        }));
        assert!(matches!(
            validate_cross_tenant_trade(&groups, &manifest, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("balance.transfer pair")
        ));
    }

    #[test]
    fn cross_tenant_trade_value_amount_mismatch_fails() {
        // Manifest declares 100 but the pair debits 50.
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![settle_event("s1", "settle")],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![
                    transfer_event("acme.game.beta", "vsrc", "value", "alice", "100", "50"),
                    transfer_event("acme.game.beta", "vdst", "value", "bob", "0", "50"),
                ],
            },
        ];
        let intents = vec![settle_intent("settle", None)];
        let manifest = trade_manifest(Some(ValueLeg {
            resource: ResourceId(String::from("currency:gold")),
            amount: String::from("100"),
            to_subject: SubjectId(String::from("bob")),
        }));
        assert!(matches!(
            validate_cross_tenant_trade(&groups, &manifest, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("value amount mismatch")
        ));
    }

    #[test]
    fn cross_tenant_trade_undeclared_value_pair_fails() {
        // Manifest declares no value leg but the batch carries a value pair.
        let intents = vec![settle_intent("settle", None)];
        let manifest = trade_manifest(None);
        assert!(matches!(
            validate_cross_tenant_trade(&asset_for_gold_groups(), &manifest, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("undeclared intent id")
        ));
    }

    #[test]
    fn cross_tenant_trade_extra_settle_leg_rejected() {
        // A second settle leg (a 3-tenant-style trade) is not declared by the
        // manifest and fails closed: only one settle + one optional value leg
        // are admitted on this path.
        let mut other = settle_event("s2", "other");
        other.tenant_id = tenant("acme.game.beta");
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![settle_event("s1", "settle")],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![other],
            },
        ];
        let intents = vec![settle_intent("settle", None)];
        let manifest = trade_manifest(None);
        assert!(matches!(
            validate_cross_tenant_trade(&groups, &manifest, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("undeclared intent id")
        ));
    }

    /// Builds a bundle-declaring `trade.settle` intent for the given resource,
    /// bundle size, and trade id.
    fn bundle_settle_intent(id: &str, resource: &str, bundle_size: u64, trade_id: &str) -> Intent {
        let mut inputs = BTreeMap::new();
        inputs.insert(String::from("from_owner"), serde_json::json!("alice"));
        inputs.insert(String::from("to_owner"), serde_json::json!("bob"));
        inputs.insert(String::from(keys::TRADE_ID), serde_json::json!(trade_id));
        inputs.insert(
            String::from(keys::BUNDLE_SIZE),
            serde_json::json!(bundle_size),
        );
        Intent::new(
            tenant("acme.game.alpha"),
            IntentId::new(format!("int_{id}")).unwrap(),
            Operation::new(String::from("trade.settle")).unwrap(),
            SubjectId(String::from("account:example:player_456")),
            ResourceId(String::from(resource)),
            Some(StateType::UniqueAsset),
            3,
            inputs,
            None,
            DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            None,
            Nonce::from_bytes(vec![1]).unwrap(),
        )
    }

    /// Builds a `trade.settle` event for the given resource.
    fn settle_event_for(event_id: &str, intent: &str, resource: &str) -> Event {
        let mut event = settle_event(event_id, intent);
        event.resource_id = ResourceId(String::from(resource));
        event
    }

    #[test]
    fn bundle_settle_two_assets_passes() {
        // A 2-asset same-side bundle: two settle intents declaring the same
        // bundle_size (2) and trade_id, settling two distinct assets.
        let intents = vec![
            bundle_settle_intent("s1", "asset:sword_001", 2, "trade_001"),
            bundle_settle_intent("s2", "asset:shield_001", 2, "trade_001"),
        ];
        let events = vec![
            settle_event_for("e1", "s1", "asset:sword_001"),
            settle_event_for("e2", "s2", "asset:shield_001"),
        ];
        assert!(validate_settle_batch(&events, &intents).is_ok());
    }

    #[test]
    fn bundle_settle_duplicate_asset_rejected() {
        let intents = vec![
            bundle_settle_intent("s1", "asset:sword_001", 2, "trade_001"),
            bundle_settle_intent("s2", "asset:sword_001", 2, "trade_001"),
        ];
        let events = vec![
            settle_event_for("e1", "s1", "asset:sword_001"),
            settle_event_for("e2", "s2", "asset:sword_001"),
        ];
        assert!(matches!(
            validate_settle_batch(&events, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("duplicate asset")
        ));
    }

    #[test]
    fn bundle_settle_wrong_event_count_rejected() {
        // Bundle declares size 3 but only 2 settle events are present.
        let intents = vec![
            bundle_settle_intent("s1", "asset:sword_001", 3, "trade_001"),
            bundle_settle_intent("s2", "asset:shield_001", 3, "trade_001"),
        ];
        let events = vec![
            settle_event_for("e1", "s1", "asset:sword_001"),
            settle_event_for("e2", "s2", "asset:shield_001"),
        ];
        assert!(matches!(
            validate_settle_batch(&events, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("bundle_size")
        ));
    }

    #[test]
    fn bundle_settle_inconsistent_bundle_size_rejected() {
        let intents = vec![
            bundle_settle_intent("s1", "asset:sword_001", 2, "trade_001"),
            bundle_settle_intent("s2", "asset:shield_001", 3, "trade_001"),
        ];
        let events = vec![
            settle_event_for("e1", "s1", "asset:sword_001"),
            settle_event_for("e2", "s2", "asset:shield_001"),
        ];
        assert!(matches!(
            validate_settle_batch(&events, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("inconsistent `bundle_size`")
        ));
    }

    #[test]
    fn bundle_settle_mixed_trade_id_rejected() {
        let intents = vec![
            bundle_settle_intent("s1", "asset:sword_001", 2, "trade_001"),
            bundle_settle_intent("s2", "asset:shield_001", 2, "trade_002"),
        ];
        let events = vec![
            settle_event_for("e1", "s1", "asset:sword_001"),
            settle_event_for("e2", "s2", "asset:shield_001"),
        ];
        assert!(matches!(
            validate_settle_batch(&events, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("mixed `trade_id`")
        ));
    }

    #[test]
    fn bundle_settle_mismatched_value_leg_rejected() {
        // A one-asset bundle settle declares a value leg of 100, but the value
        // pair debits only 50.
        let mut settle = bundle_settle_intent("s1", "asset:sword_001", 1, "trade_001");
        settle.inputs.insert(
            String::from(keys::VALUE_RESOURCE),
            serde_json::json!("wallet:gold"),
        );
        settle
            .inputs
            .insert(String::from(keys::VALUE_AMOUNT), serde_json::json!("100"));
        settle.inputs.insert(
            String::from(keys::VALUE_TO_SUBJECT),
            serde_json::json!("alice"),
        );
        let intents = vec![settle];
        let events = vec![
            settle_event_for("e1", "s1", "asset:sword_001"),
            balance_transfer("vsrc", "value", "alice", "100", "50"),
            balance_transfer("vdst", "value", "bob", "0", "50"),
        ];
        assert!(matches!(
            validate_settle_batch(&events, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("value amount mismatch")
        ));
    }

    #[test]
    fn bundle_settle_value_leg_matches_passes() {
        // A bundle settle with one value leg matching the declared amount.
        let mut settle = bundle_settle_intent("s1", "asset:sword_001", 1, "trade_001");
        settle.inputs.insert(
            String::from(keys::VALUE_RESOURCE),
            serde_json::json!("wallet:gold"),
        );
        settle
            .inputs
            .insert(String::from(keys::VALUE_AMOUNT), serde_json::json!("100"));
        settle.inputs.insert(
            String::from(keys::VALUE_TO_SUBJECT),
            serde_json::json!("alice"),
        );
        let intents = vec![settle];
        let events = vec![
            settle_event_for("e1", "s1", "asset:sword_001"),
            balance_transfer("vsrc", "value", "alice", "200", "100"),
            balance_transfer("vdst", "value", "bob", "0", "100"),
        ];
        assert!(validate_settle_batch(&events, &intents).is_ok());
    }

    #[test]
    fn single_asset_settle_without_bundle_still_passes() {
        // No bundle declared: the single-asset settle path is unchanged.
        let intents = vec![settle_intent("settle", None)];
        let events = vec![settle_event("s1", "settle")];
        assert!(validate_settle_batch(&events, &intents).is_ok());
    }

    /// Builds two tenant groups, each carrying one `trade.settle` event for the
    /// manifest settle intent (both share the settle intent id `int_settle`).
    fn two_group_bundle_groups() -> Vec<TenantEventGroup> {
        let mut shield = settle_event_for("s2", "settle", "asset:shield_001");
        shield.tenant_id = tenant("acme.game.beta");
        vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![settle_event_for("s1", "settle", "asset:sword_001")],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![shield],
            },
        ]
    }

    #[test]
    fn cross_tenant_bundle_two_assets_passes() {
        let intents = vec![settle_intent("settle", None)];
        let manifest = bundle_manifest(&["asset:sword_001", "asset:shield_001"]);
        assert!(
            validate_cross_tenant_trade(&two_group_bundle_groups(), &manifest, &intents).is_ok()
        );
    }

    #[test]
    fn cross_tenant_bundle_missing_asset_fails() {
        // Manifest declares sword + shield, but the batch settles sword +
        // gauntlets: the declared shield is missing and gauntlets is undeclared.
        let mut gauntlets = settle_event_for("s2", "settle", "asset:gauntlets_001");
        gauntlets.tenant_id = tenant("acme.game.beta");
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![settle_event_for("s1", "settle", "asset:sword_001")],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![gauntlets],
            },
        ];
        let intents = vec![settle_intent("settle", None)];
        let manifest = bundle_manifest(&["asset:sword_001", "asset:shield_001"]);
        assert!(matches!(
            validate_cross_tenant_trade(&groups, &manifest, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("do not match the manifest")
        ));
    }

    #[test]
    fn cross_tenant_bundle_undeclared_asset_fails() {
        // The batch carries a third settle asset (gauntlets) not declared by
        // the two-asset manifest.
        let mut shield = settle_event_for("s2", "settle", "asset:shield_001");
        shield.tenant_id = tenant("acme.game.beta");
        let mut gauntlets = settle_event_for("s3", "settle", "asset:gauntlets_001");
        gauntlets.tenant_id = tenant("acme.game.gamma");
        let groups = vec![
            TenantEventGroup {
                tenant: tenant("acme.game.alpha"),
                events: vec![settle_event_for("s1", "settle", "asset:sword_001")],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.beta"),
                events: vec![shield],
            },
            TenantEventGroup {
                tenant: tenant("acme.game.gamma"),
                events: vec![gauntlets],
            },
        ];
        let intents = vec![settle_intent("settle", None)];
        let manifest = bundle_manifest(&["asset:sword_001", "asset:shield_001"]);
        assert!(matches!(
            validate_cross_tenant_trade(&groups, &manifest, &intents),
            Err(ExecutorError::AtomicityViolation(message))
            if message.contains("expected 2")
        ));
    }
}
