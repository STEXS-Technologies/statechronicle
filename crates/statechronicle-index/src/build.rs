//! Pure, deterministic trade index builder (Phase 2 of the trade completion).
//!
//! [`apply`](crate::build::apply) projects one committed batch (trade events + the settle intents
//! that declared them + the committing signed commit) into a
//! `BTreeMap<String, TradeRecord>` keyed by `trade_id`. The builder is pure
//! and deterministic by construction: it uses only `BTreeMap`/`BTreeSet`,
//! sorted iteration, no wall clock, no RNG, and no `HashMap` in any output, so
//! applying an event stream incrementally yields exactly the same index as
//! replaying the raw stream from scratch.
//!
//! `trade_id` is extracted from the **event bodies** (`trade.lock` carries it
//! in `after.state.trade_id`; `trade.settle`/`trade.unlock` in
//! `before.state.trade_id`, since the asset is `trade_held` at that point).
//! Value-leg declarations survive only in the **settle intents**
//! (`value_resource`/`value_amount`/`value_to_subject`); the net-zero
//! `balance.transfer` pairs in the batch are matched to those declarations by
//! the same `(amount, resource, to_subject)` multiset the executor validator
//! uses, so value-pair attribution stays batch-local and deterministic.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use statechronicle_core::amount::Amount;
use statechronicle_domain::commit::{Commit, ScopeKind};
use statechronicle_domain::event::Event;
use statechronicle_domain::ids::{EventId, IntentId};
use statechronicle_domain::intent::{Intent, Operation};
use statechronicle_domain::proof::CommitRef;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::resource_state::ResourceState;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_domain::trade::{
    TradeEventRef, TradeRecord, TradeSide, TradeStatus, TradeValueLeg,
};
use statechronicle_profiles::keys;
use statechronicle_profiles::unique_asset::op as asset_op;

use crate::error::IndexError;

/// One committed batch for the trade read-side: the events, the settle intents
/// that produced (and, for value-declaring settles, declared) them, and the
/// signing commit that pinned the batch.
///
/// The composition root ingests one trade execution result as one or more
/// batches (one per tenant commit). Value-leg attribution is batch-local: a
/// value pair is matched to a value-declaring settle intent present in the
/// same batch.
#[derive(Debug, Clone)]
pub struct IngestBatch {
    /// The committed events of this batch.
    pub events: Vec<Event>,
    /// The settle intents whose declarations drive value-leg attribution.
    pub settle_intents: Vec<Intent>,
    /// The signing commit that pinned this batch.
    pub commit: Signed<Commit>,
}

/// Returns the deterministic, deduplicated set of `trade_id`s a batch touches.
///
/// These are the trade ids whose records a batch may create or update: the ids
/// of the batch's trade events plus the `trade_id`s of its value-declaring
/// settle intents. Used by the service to seed the index state before applying
/// a batch, so incremental ingestion merges into (rather than clobbers)
/// existing records.
#[allow(clippy::collapsible_if)]
pub fn batch_trade_ids(batch: &IngestBatch) -> Vec<String> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for event in &batch.events {
        if let Ok(Some(id)) = trade_id_of(event) {
            ids.insert(id);
        }
    }
    for intent in &batch.settle_intents {
        if declares_value_leg(intent) {
            if let Ok(id) = intent_str(intent, keys::TRADE_ID) {
                ids.insert(id);
            }
        }
    }
    ids.into_iter().collect()
}

/// Applies a batch to the trade index, updating (or creating) the affected
/// trade records.
///
/// Deterministic: given the same batch and starting state, the result is
/// identical, and the builder never consults a clock, RNG, or `HashMap`.
///
/// # Errors
///
/// Returns [`IndexError::MissingTradeId`] when a trade event's body carries no
/// `trade_id`, [`IndexError::NonTenantCommit`] when the batch commit is not
/// tenant-scoped, and [`IndexError::MalformedValue`] when a value-leg
/// declaration or `balance.transfer` pair is malformed.
pub fn apply(
    state: &mut BTreeMap<String, TradeRecord>,
    batch: &IngestBatch,
) -> Result<(), IndexError> {
    let commit_ref = commit_ref_from_signed(&batch.commit)?;

    // Recover the net-zero `balance.transfer` value pairs in this batch.
    let value_pairs = recover_value_pairs(&batch.events)?;

    // Process trade events (lock / settle / unlock) per trade.
    let mut by_trade: BTreeMap<String, Vec<&Event>> = BTreeMap::new();
    for event in &batch.events {
        if let Some(trade_id) = trade_id_of(event)? {
            by_trade.entry(trade_id).or_default().push(event);
        }
    }
    for (trade_id, events) in by_trade {
        let record = state
            .entry(trade_id.clone())
            .or_insert_with(|| TradeRecord::new(trade_id));
        apply_trade_events(record, events, &commit_ref)?;
        record
            .sides
            .sort_by(|left, right| left.tenant.0.cmp(&right.tenant.0));
    }

    // Attach value legs: match the batch's value-declaring settle intents to the
    // recovered pairs by the same multiset the executor validator uses. Value
    // pairs are appended after locks/settles/unlocks in the record's canonical
    // event ordering.
    let value_legs = match_value_legs(batch, &value_pairs, &commit_ref)?;
    for (trade_id, leg) in value_legs {
        let record = state
            .entry(trade_id.clone())
            .or_insert_with(|| TradeRecord::new(trade_id));
        for event_id in &leg.pair_event_ids {
            record.events.push(TradeEventRef {
                event_id: event_id.clone(),
                tenant_id: leg.tenant.clone(),
                operation: balance_transfer(),
            });
        }
        record.value_legs.push(leg);
    }

    Ok(())
}

/// Applies the trade events of one trade within a batch, updating status,
/// sides, and the canonical event ordering.
///
/// # Errors
///
/// Returns [`IndexError::MalformedValue`] when a settle event's state is
/// missing its `owner` field.
fn apply_trade_events(
    record: &mut TradeRecord,
    events: Vec<&Event>,
    commit_ref: &CommitRef,
) -> Result<(), IndexError> {
    // Canonical intra-batch order: locks first, then settles, then unlocks.
    let mut locks: Vec<&Event> = Vec::new();
    let mut settles: Vec<&Event> = Vec::new();
    let mut unlocks: Vec<&Event> = Vec::new();
    for event in events {
        if &event.operation == asset_op::trade_lock() {
            locks.push(event);
        } else if &event.operation == asset_op::trade_settle() {
            settles.push(event);
        } else if &event.operation == asset_op::trade_unlock() {
            unlocks.push(event);
        }
    }

    for event in locks {
        record.status = TradeStatus::Open;
        record.events.push(TradeEventRef {
            event_id: event.event_id.clone(),
            tenant_id: event.tenant_id.clone(),
            operation: event.operation.clone(),
        });
    }
    for event in settles {
        record.status = TradeStatus::Settled;
        upsert_side(record, event, commit_ref)?;
        record.events.push(TradeEventRef {
            event_id: event.event_id.clone(),
            tenant_id: event.tenant_id.clone(),
            operation: event.operation.clone(),
        });
    }
    for event in unlocks {
        record.status = TradeStatus::Cancelled;
        record.events.push(TradeEventRef {
            event_id: event.event_id.clone(),
            tenant_id: event.tenant_id.clone(),
            operation: event.operation.clone(),
        });
    }
    Ok(())
}

/// Inserts or updates the settle side for an event's tenant.
///
/// # Errors
///
/// Returns [`IndexError::MalformedValue`] when the event's before/after state
/// is missing its `owner` field.
fn upsert_side(
    record: &mut TradeRecord,
    event: &Event,
    commit_ref: &CommitRef,
) -> Result<(), IndexError> {
    let from_owner = owner_of_state(&event.before.state)?;
    let to_owner = owner_of_state(&event.after.state)?;
    let asset = event.resource_id.clone();
    let event_id = event.event_id.clone();
    let tenant = event.tenant_id.clone();

    if let Some(side) = record.sides.iter_mut().find(|side| side.tenant == tenant) {
        if !side.settle_assets.contains(&asset) {
            side.settle_assets.push(asset.clone());
        }
        // Track this asset's own committing commit: a tenant may settle two
        // assets of one trade in different commits, so each asset's state proof
        // must be pinned to the commit that settled it, not the side's first.
        side.settle_commits_by_asset
            .insert(asset, commit_ref.clone());
        side.settle_event_ids.push(event_id);
    } else {
        record.sides.push(TradeSide {
            tenant,
            settle_assets: vec![asset.clone()],
            from_owner,
            to_owner,
            settle_commit: commit_ref.clone(),
            settle_commits_by_asset: BTreeMap::from([(asset, commit_ref.clone())]),
            settle_event_ids: vec![event_id],
        });
    }
    Ok(())
}

/// Returns the `owner` string field of a projected state payload.
///
/// # Errors
///
/// Returns [`IndexError::MalformedValue`] when the state carries no string
/// `owner` field.
fn owner_of_state(state: &ResourceState) -> Result<String, IndexError> {
    state
        .owner()
        .map(|owner| owner.0.clone())
        .filter(|owner| !owner.is_empty())
        .ok_or_else(|| {
            IndexError::MalformedValue(String::from(
                "trade settle event state is missing a non-empty `owner`",
            ))
        })
}

/// Extracts the `trade_id` from a trade event body, or `None` for non-trade
/// events (e.g. the `balance.transfer` value-pair events).
///
/// # Errors
///
/// Returns [`IndexError::MissingTradeId`] when a trade event's body carries no
/// `trade_id`.
fn trade_id_of(event: &Event) -> Result<Option<String>, IndexError> {
    if &event.operation == asset_op::trade_lock() {
        return state_str_opt(&event.after.state, keys::TRADE_ID)
            .map(Some)
            .ok_or_else(|| {
                IndexError::MissingTradeId(format!(
                    "trade.lock event `{}` is missing `trade_id` in its after state",
                    event.event_id.as_str()
                ))
            });
    }
    if &event.operation == asset_op::trade_settle() || &event.operation == asset_op::trade_unlock()
    {
        return state_str_opt(&event.before.state, keys::TRADE_ID)
            .map(Some)
            .ok_or_else(|| {
                IndexError::MissingTradeId(format!(
                    "trade event `{}` is missing `trade_id` in its before state",
                    event.event_id.as_str()
                ))
            });
    }
    Ok(None)
}

/// Reads a string field from a projected state payload.
fn state_str_opt(state: &ResourceState, key: &str) -> Option<String> {
    let value = match key {
        keys::SUBJECT => state.subject().map(|s| s.0.clone()),
        keys::TRADE_ID => match state {
            ResourceState::UniqueAsset(v) => v.trade_id.clone(),
            ResourceState::ConsumableStack(_)
            | ResourceState::FungibleBalance(_)
            | ResourceState::Entitlement(_)
            | ResourceState::MeteredResource(_)
            | ResourceState::Listing(_)
            | ResourceState::Escrow(_) => None,
        },
        _ => None,
    }?;
    (!value.is_empty()).then_some(value)
}

/// A net-zero `balance.transfer` value pair recovered from a batch (the atomic
/// debit + credit events sharing one value-leg intent id).
#[derive(Debug, Clone)]
struct RecoveredValuePair {
    /// The net debit (equals the credited amount; net-zero by construction).
    debit: Amount,
    /// The resource moved by the pair.
    resource: ResourceId,
    /// The credited recipient's subject, when the credit event names one.
    credited_subject: Option<SubjectId>,
    /// The pair's event ids, sorted for determinism.
    event_ids: Vec<EventId>,
    /// The tenant scope of the pair.
    tenant: TenantId,
}

/// Recovers the net-zero `balance.transfer` value pairs from a batch.
///
/// Mirrors the executor's `recover_value_pairs` shape so the read-side matches
/// pairs exactly as the validator does, and additionally records the pair's
/// event ids and tenant.
///
/// # Errors
///
/// Returns [`IndexError::MalformedValue`] when a `balance.transfer` group is
/// not exactly two events, or when a pair's balance amount is malformed.
fn recover_value_pairs(events: &[Event]) -> Result<Vec<RecoveredValuePair>, IndexError> {
    let mut all_groups: BTreeMap<IntentId, Vec<&Event>> = BTreeMap::new();
    for event in events {
        all_groups
            .entry(event.intent_id.clone())
            .or_default()
            .push(event);
    }

    let mut pairs = Vec::new();
    for (intent_id, group) in all_groups {
        let Some(first) = group.first() else {
            continue;
        };
        if first.operation != balance_transfer() {
            continue;
        }
        if group.len() != 2 {
            return Err(IndexError::MalformedValue(format!(
                "balance.transfer intent `{}` is not an atomic debit + credit pair",
                intent_id.as_str()
            )));
        }
        let mut debit = Amount::ZERO;
        let mut credited_subject: Option<SubjectId> = None;
        for event in &group {
            let before = commitment_amount(&event.before.state);
            let after = commitment_amount(&event.after.state);
            if before > after {
                let delta = before
                    .checked_sub(after)
                    .ok_or_else(|| IndexError::MalformedValue(String::from("debit overflow")))?;
                debit = debit
                    .checked_add(delta)
                    .ok_or_else(|| IndexError::MalformedValue(String::from("debit overflow")))?;
            } else if after > before {
                credited_subject = state_str_opt(&event.after.state, keys::SUBJECT).map(SubjectId);
            }
        }
        let mut event_ids: Vec<EventId> =
            group.iter().map(|event| event.event_id.clone()).collect();
        event_ids.sort();
        pairs.push(RecoveredValuePair {
            debit,
            resource: first.resource_id.clone(),
            credited_subject,
            event_ids,
            tenant: first.tenant_id.clone(),
        });
    }
    Ok(pairs)
}

/// The multiset key used to match value legs to pairs: `(amount, resource,
/// recipient)`.
type ValuePairKey = (Amount, String, Option<String>);

/// A value-declaring settle intent, pre-matched to a `balance.transfer` pair.
struct DeclaredValueLeg {
    /// Multiset key: amount, resource, recipient.
    key: ValuePairKey,
    /// The settle intent's `trade_id`.
    trade_id: String,
    /// The declared amount string.
    amount_str: String,
    /// The declared recipient.
    to_subject: SubjectId,
}

/// Matches the batch's value-declaring settle intents to the recovered pairs by
/// the executor's `(amount, resource, recipient)` multiset, producing value
/// legs attributed to each settle intent's `trade_id`.
///
/// Matching is lenient and batch-local: a leg is produced only when a declared
/// leg and a pair share the same multiset key in this batch. Unmatched
/// declarations or pairs are left for other batches (the executor already
/// validated the full trade).
///
/// # Errors
///
/// Returns [`IndexError::MalformedValue`] when a value-leg declaration is
/// partial or malformed.
fn match_value_legs(
    batch: &IngestBatch,
    pairs: &[RecoveredValuePair],
    commit_ref: &CommitRef,
) -> Result<Vec<(String, TradeValueLeg)>, IndexError> {
    let mut declared: Vec<DeclaredValueLeg> = Vec::new();
    for intent in &batch.settle_intents {
        if !declares_value_leg(intent) {
            continue;
        }
        let resource = intent_str(intent, keys::VALUE_RESOURCE)?;
        let amount = intent_str(intent, keys::VALUE_AMOUNT)?;
        let to_subject = intent_str(intent, keys::VALUE_TO_SUBJECT)?;
        let trade_id = intent_str(intent, keys::TRADE_ID)?;
        let amount_amt = Amount::try_from_str(&amount).map_err(|_source| {
            IndexError::MalformedValue(format!("malformed value_amount `{amount}`"))
        })?;
        declared.push(DeclaredValueLeg {
            key: (amount_amt, resource.clone(), Some(to_subject.clone())),
            trade_id,
            amount_str: amount,
            to_subject: SubjectId(to_subject),
        });
    }
    declared.sort_by(|left, right| cmp_key(&left.key, &right.key));

    let mut pair_keys: Vec<(ValuePairKey, &RecoveredValuePair)> = pairs
        .iter()
        .map(|pair| {
            (
                (
                    pair.debit,
                    pair.resource.0.clone(),
                    pair.credited_subject
                        .as_ref()
                        .map(|subject| subject.0.clone()),
                ),
                pair,
            )
        })
        .collect();
    pair_keys.sort_by(|left, right| cmp_key(&left.0, &right.0));

    let mut result = Vec::new();
    for (decl, (pair_key, pair)) in declared.iter().zip(pair_keys.iter()) {
        if decl.key == *pair_key {
            result.push((
                decl.trade_id.clone(),
                TradeValueLeg {
                    resource: pair.resource.clone(),
                    amount: decl.amount_str.clone(),
                    to_subject: decl.to_subject.clone(),
                    pair_event_ids: pair.event_ids.clone(),
                    tenant: pair.tenant.clone(),
                    commit: commit_ref.clone(),
                },
            ));
        }
    }
    Ok(result)
}

/// Compares two multiset keys with partial ordering over the optional recipient.
fn cmp_key(left: &ValuePairKey, right: &ValuePairKey) -> std::cmp::Ordering {
    left.0
        .cmp(&right.0)
        .then_with(|| left.1.cmp(&right.1))
        .then_with(|| left.2.cmp(&right.2))
}

/// Returns whether a settle intent declares a value leg (any of the three
/// value-leg inputs present).
fn declares_value_leg(intent: &Intent) -> bool {
    intent.inputs.contains_key(keys::VALUE_RESOURCE)
        || intent.inputs.contains_key(keys::VALUE_AMOUNT)
        || intent.inputs.contains_key(keys::VALUE_TO_SUBJECT)
}

/// Reads a required string input from an intent.
///
/// # Errors
///
/// Returns [`IndexError::MalformedValue`] when the input is missing or not a
/// non-empty string.
fn intent_str(intent: &Intent, key: &str) -> Result<String, IndexError> {
    intent
        .inputs
        .get(key)
        .and_then(Value::as_str)
        .map(String::from)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            IndexError::MalformedValue(format!(
                "settle intent `{}` is missing a non-empty `{key}` input",
                intent.intent_id.as_str()
            ))
        })
}

/// Reads the fixed-point `balance` amount from a value-pair event's state.
///
/// A missing field reads as zero (e.g. a create-on-credit destination).
///
/// # Errors
///
/// Returns [`IndexError::MalformedValue`] when the field is present but not a
/// canonical non-negative integer string.
fn commitment_amount(state: &ResourceState) -> Amount {
    let Some(amount) = state.amount("balance") else {
        return Amount::ZERO;
    };
    amount
}

/// The `balance.transfer` operation literal.
fn balance_transfer() -> Operation {
    Operation::from_static("balance.transfer")
}

/// Derives the commit reference embedded in proof bundles from a signed commit.
///
/// # Errors
///
/// Returns [`IndexError::NonTenantCommit`] when the commit is not tenant-scoped
/// or is missing its tenant id.
fn commit_ref_from_signed(signed: &Signed<Commit>) -> Result<CommitRef, IndexError> {
    let body = &signed.body;
    if body.scope.kind != ScopeKind::Tenant {
        return Err(IndexError::NonTenantCommit(String::from(
            signed.body.commit_id.as_str(),
        )));
    }
    let _tenant = body.scope.tenant_id.as_ref().ok_or_else(|| {
        IndexError::NonTenantCommit(String::from(
            "tenant-scoped commit is missing its tenant id",
        ))
    })?;
    Ok(CommitRef {
        commit_id: body.commit_id.clone(),
        sequence: body.sequence,
        state_root: body.next_state_root.clone(),
        signature: signed.signature.clone(),
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::shadow_unrelated
)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use statechronicle_core::digest::hash_bytes;
    use statechronicle_core::signature::Signature;
    use statechronicle_domain::commit::{CommitScope, ProfileId};
    use statechronicle_domain::event::StateCommitment;
    use statechronicle_domain::ids::CommitId;
    use statechronicle_domain::intent::{KeyId, Nonce, SignatureAlg, SignatureBlock};
    use statechronicle_domain::state_type::StateType;

    fn tenant(name: &str) -> TenantId {
        TenantId(String::from(name))
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn signed_commit(tenant_name: &str, id: &str) -> Signed<Commit> {
        let commit = Commit::new(
            CommitScope::tenant(tenant(tenant_name)),
            CommitId::new(String::from(id)).unwrap(),
            None,
            1,
            1,
            hash_bytes(b"event-root"),
            hash_bytes(b"previous-root"),
            hash_bytes(b"next-root"),
            now(),
            SubjectId(String::from("service:statechronicle.example.net")),
            ProfileId::new(String::from("statechronicle.profile.resource.v0")).unwrap(),
        );
        Signed::new(
            commit,
            SignatureBlock {
                alg: SignatureAlg::Ed25519,
                key_id: KeyId::new(String::from("did:key:z6Mk...#test")).unwrap(),
                sig: Signature::from_bytes([0u8; 64]),
            },
        )
    }

    fn state(entries: &[(&str, &str)]) -> serde_json::Value {
        serde_json::json!(
            entries
                .iter()
                .map(|(k, v)| (*k, serde_json::json!(v)))
                .collect::<BTreeMap<&str, serde_json::Value>>()
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn evt(
        id: &str,
        tenant_name: &str,
        op: &str,
        resource: &str,
        before: serde_json::Value,
        after: serde_json::Value,
        intent: &str,
    ) -> Event {
        let state_type = if op == "balance.transfer" {
            statechronicle_domain::state_type::StateType::FungibleBalance
        } else {
            statechronicle_domain::state_type::StateType::UniqueAsset
        };
        let before = if before.as_object().is_some_and(|object| object.is_empty()) {
            if state_type == statechronicle_domain::state_type::StateType::FungibleBalance {
                serde_json::json!({"subject":"alice","balance":"0","unit":"gold"})
            } else {
                serde_json::json!({"owner":"alice","status":"active"})
            }
        } else {
            before
        };
        Event::new(
            tenant(tenant_name),
            EventId::new(format!("evt_{id}")).unwrap(),
            IntentId::new(format!("int_{intent}")).unwrap(),
            Operation::new(String::from(op)).unwrap(),
            ResourceId(String::from(resource)),
            SubjectId(String::from("account:example:player")),
            StateCommitment {
                version: 1,
                state_hash: hash_bytes(b"before"),
                state: statechronicle_domain::resource_state::ResourceState::from_legacy_json(
                    state_type, before,
                )
                .unwrap(),
            },
            StateCommitment {
                version: 2,
                state_hash: hash_bytes(b"after"),
                state: statechronicle_domain::resource_state::ResourceState::from_legacy_json(
                    state_type, after,
                )
                .unwrap(),
            },
            None,
            SubjectId(String::from("service:statechronicle.example.net")),
            now(),
        )
    }

    fn lock_event(
        id: &str,
        tenant_name: &str,
        resource: &str,
        trade_id: &str,
        owner: &str,
    ) -> Event {
        evt(
            id,
            tenant_name,
            "trade.lock",
            resource,
            state(&[("owner", owner), ("status", "active")]),
            state(&[
                ("owner", owner),
                ("status", "trade_held"),
                ("trade_id", trade_id),
            ]),
            &format!("{id}_lock"),
        )
    }

    fn settle_event(
        id: &str,
        tenant_name: &str,
        resource: &str,
        trade_id: &str,
        from: &str,
        to: &str,
    ) -> Event {
        evt(
            id,
            tenant_name,
            "trade.settle",
            resource,
            state(&[
                ("owner", from),
                ("status", "trade_held"),
                ("trade_id", trade_id),
            ]),
            state(&[("owner", to), ("status", "active")]),
            &format!("{id}_settle"),
        )
    }

    fn unlock_event(
        id: &str,
        tenant_name: &str,
        resource: &str,
        trade_id: &str,
        owner: &str,
    ) -> Event {
        evt(
            id,
            tenant_name,
            "trade.unlock",
            resource,
            state(&[
                ("owner", owner),
                ("status", "trade_held"),
                ("trade_id", trade_id),
            ]),
            state(&[("owner", owner), ("status", "active")]),
            &format!("{id}_unlock"),
        )
    }

    fn value_pair_event(
        id: &str,
        tenant_name: &str,
        subject: &str,
        before: &str,
        after: &str,
        intent: &str,
    ) -> Event {
        evt(
            id,
            tenant_name,
            "balance.transfer",
            "wallet:gold",
            state(&[
                ("subject", subject),
                ("balance", before),
                ("unit", "gold_minor"),
            ]),
            state(&[
                ("subject", subject),
                ("balance", after),
                ("unit", "gold_minor"),
            ]),
            intent,
        )
    }

    fn batch(
        events: Vec<Event>,
        settle_intents: Vec<Intent>,
        tenant_name: &str,
        commit_id: &str,
    ) -> IngestBatch {
        IngestBatch {
            events,
            settle_intents,
            commit: signed_commit(tenant_name, commit_id),
        }
    }

    fn value_settle_intent(
        trade_id: &str,
        resource: &str,
        amount: &str,
        to_subject: &str,
    ) -> Intent {
        let mut inputs: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        inputs.insert(String::from(keys::TRADE_ID), serde_json::json!(trade_id));
        inputs.insert(
            String::from(keys::VALUE_RESOURCE),
            serde_json::json!(resource),
        );
        inputs.insert(String::from(keys::VALUE_AMOUNT), serde_json::json!(amount));
        inputs.insert(
            String::from(keys::VALUE_TO_SUBJECT),
            serde_json::json!(to_subject),
        );
        Intent::new(
            tenant("acme.game.alpha"),
            IntentId::new(String::from("int_settle_value")).unwrap(),
            Operation::from_static("trade.settle"),
            SubjectId(String::from("account:example:player")),
            ResourceId(String::from("asset:sword")),
            Some(StateType::UniqueAsset),
            1,
            inputs,
            None,
            now(),
            None,
            Nonce::from_bytes(vec![0]).unwrap(),
        )
    }

    #[test]
    fn lock_opens_a_record() {
        let mut state = BTreeMap::new();
        let batch = batch(
            vec![lock_event(
                "a",
                "acme.game.alpha",
                "asset:sword",
                "trade_001",
                "alice",
            )],
            Vec::new(),
            "acme.game.alpha",
            "cmt_00000000000000000001",
        );
        apply(&mut state, &batch).unwrap();
        let record = state.get("trade_001").unwrap();
        assert_eq!(record.status, TradeStatus::Open);
        assert_eq!(record.events.len(), 1);
        assert_eq!(record.events[0].operation.as_str(), "trade.lock");
        assert!(record.sides.is_empty());
    }

    #[test]
    fn settle_sets_status_and_side() {
        let mut state = BTreeMap::new();
        apply(
            &mut state,
            &batch(
                vec![lock_event(
                    "a",
                    "acme.game.alpha",
                    "asset:sword",
                    "trade_001",
                    "alice",
                )],
                Vec::new(),
                "acme.game.alpha",
                "cmt_00000000000000000001",
            ),
        )
        .unwrap();
        apply(
            &mut state,
            &batch(
                vec![settle_event(
                    "b",
                    "acme.game.alpha",
                    "asset:sword",
                    "trade_001",
                    "alice",
                    "bob",
                )],
                Vec::new(),
                "acme.game.alpha",
                "cmt_00000000000000000002",
            ),
        )
        .unwrap();
        let record = state.get("trade_001").unwrap();
        assert_eq!(record.status, TradeStatus::Settled);
        assert_eq!(record.sides.len(), 1);
        assert_eq!(record.sides[0].tenant, tenant("acme.game.alpha"));
        assert_eq!(record.sides[0].settle_assets[0].0, "asset:sword");
        assert_eq!(record.sides[0].from_owner, "alice");
        assert_eq!(record.sides[0].to_owner, "bob");
        assert_eq!(record.events.len(), 2);
        assert_eq!(record.events[0].operation.as_str(), "trade.lock");
        assert_eq!(record.events[1].operation.as_str(), "trade.settle");
    }

    #[test]
    fn unlock_cancels() {
        let mut state = BTreeMap::new();
        apply(
            &mut state,
            &batch(
                vec![lock_event(
                    "a",
                    "acme.game.alpha",
                    "asset:sword",
                    "trade_001",
                    "alice",
                )],
                Vec::new(),
                "acme.game.alpha",
                "cmt_00000000000000000001",
            ),
        )
        .unwrap();
        apply(
            &mut state,
            &batch(
                vec![unlock_event(
                    "c",
                    "acme.game.alpha",
                    "asset:sword",
                    "trade_001",
                    "alice",
                )],
                Vec::new(),
                "acme.game.alpha",
                "cmt_00000000000000000003",
            ),
        )
        .unwrap();
        let record = state.get("trade_001").unwrap();
        assert_eq!(record.status, TradeStatus::Cancelled);
        assert!(record.sides.is_empty());
    }

    #[test]
    fn value_leg_attributed_from_settle_intent() {
        let mut state = BTreeMap::new();
        let intent = value_settle_intent("trade_001", "wallet:gold", "100", "alice");
        // One batch: the settle event + the value pair + the declaring intent.
        let settle = settle_event(
            "s",
            "acme.game.alpha",
            "asset:sword",
            "trade_001",
            "alice",
            "bob",
        );
        let pair = vec![
            value_pair_event("p1", "acme.game.beta", "bob", "1000", "900", "vleg"),
            value_pair_event("p2", "acme.game.beta", "alice", "0", "100", "vleg"),
        ];
        let mut events = Vec::new();
        events.push(settle);
        events.extend(pair);
        apply(
            &mut state,
            &batch(
                events,
                vec![intent],
                "acme.game.beta",
                "cmt_00000000000000000009",
            ),
        )
        .unwrap();
        let record = state.get("trade_001").unwrap();
        assert_eq!(record.value_legs.len(), 1);
        let leg = &record.value_legs[0];
        assert_eq!(leg.amount, "100");
        assert_eq!(leg.resource.0, "wallet:gold");
        assert_eq!(leg.to_subject.0, "alice");
        assert_eq!(leg.tenant, tenant("acme.game.beta"));
        assert_eq!(leg.pair_event_ids.len(), 2);
        // Value pair events come after the settle in canonical order.
        assert_eq!(record.events[0].operation.as_str(), "trade.settle");
        assert_eq!(record.events[1].operation.as_str(), "balance.transfer");
    }

    #[test]
    fn value_leg_matches_across_tenant_batches() {
        let mut state = BTreeMap::new();
        let intent = value_settle_intent("trade_001", "wallet:gold", "100", "alice");
        // Alpha batch: the settle event + declaring intent (no pair present).
        apply(
            &mut state,
            &batch(
                vec![settle_event(
                    "s",
                    "acme.game.alpha",
                    "asset:sword",
                    "trade_001",
                    "alice",
                    "bob",
                )],
                vec![intent.clone()],
                "acme.game.alpha",
                "cmt_0000000000000000000a",
            ),
        )
        .unwrap();
        // Beta batch: the value pair + the declaring intent.
        let pair = vec![
            value_pair_event("p1", "acme.game.beta", "bob", "1000", "900", "vleg"),
            value_pair_event("p2", "acme.game.beta", "alice", "0", "100", "vleg"),
        ];
        apply(
            &mut state,
            &batch(
                pair,
                vec![intent],
                "acme.game.beta",
                "cmt_0000000000000000000b",
            ),
        )
        .unwrap();
        let record = state.get("trade_001").unwrap();
        assert_eq!(record.status, TradeStatus::Settled);
        assert_eq!(record.sides.len(), 1);
        assert_eq!(record.value_legs.len(), 1);
        assert_eq!(record.value_legs[0].tenant, tenant("acme.game.beta"));
        // Canonical order: settle then value pair.
        assert_eq!(record.events[0].operation.as_str(), "trade.settle");
        assert_eq!(record.events[1].operation.as_str(), "balance.transfer");
    }

    /// F3 regression: a tenant settling two assets of one trade in two different
    /// commits must pin each asset to the commit that settled it, so a later
    /// `get_proof` fetches each asset's state proof at its own commit rather
    /// than the side's first commit.
    #[test]
    fn per_asset_commit_tracks_two_commits_for_one_tenant() {
        let mut state = BTreeMap::new();
        // Asset one settled at commit ...01.
        apply(
            &mut state,
            &batch(
                vec![settle_event(
                    "s1",
                    "acme.game.alpha",
                    "asset:sword",
                    "trade_001",
                    "alice",
                    "bob",
                )],
                Vec::new(),
                "acme.game.alpha",
                "cmt_00000000000000000001",
            ),
        )
        .unwrap();
        // Asset two of the SAME trade, same tenant, at a DIFFERENT commit.
        apply(
            &mut state,
            &batch(
                vec![settle_event(
                    "s2",
                    "acme.game.alpha",
                    "asset:shield",
                    "trade_001",
                    "alice",
                    "bob",
                )],
                Vec::new(),
                "acme.game.alpha",
                "cmt_00000000000000000002",
            ),
        )
        .unwrap();

        let record = state.get("trade_001").unwrap();
        assert_eq!(record.status, TradeStatus::Settled);
        assert_eq!(record.sides.len(), 1);
        let side = &record.sides[0];
        assert_eq!(side.settle_assets.len(), 2);
        assert_eq!(side.settle_assets[0].0, "asset:sword");
        assert_eq!(side.settle_assets[1].0, "asset:shield");
        // The representative settle_commit stays the first commit, but each
        // asset is pinned to the commit that actually settled it.
        assert_eq!(
            side.settle_commit.commit_id.as_str(),
            "cmt_00000000000000000001"
        );
        assert_eq!(
            side.settle_commits_by_asset
                .get(&ResourceId(String::from("asset:sword")))
                .unwrap()
                .commit_id
                .as_str(),
            "cmt_00000000000000000001"
        );
        assert_eq!(
            side.settle_commits_by_asset
                .get(&ResourceId(String::from("asset:shield")))
                .unwrap()
                .commit_id
                .as_str(),
            "cmt_00000000000000000002"
        );
    }

    /// Derives a deterministic batch from a byte slice using fixed event
    /// templates, so the builder is exercised over arbitrary shapes.
    #[allow(clippy::manual_is_multiple_of)]
    fn batch_from_bytes(data: &[u8]) -> IngestBatch {
        let tenant_name = if data.first().copied().unwrap_or(0) % 2 == 0 {
            "acme.game.alpha"
        } else {
            "acme.game.beta"
        };
        let trade = format!("trade_{:03}", data.get(1).copied().unwrap_or(0) % 7);
        let mut events = Vec::new();
        if data.len() > 2 && data[2] % 3 != 0 {
            events.push(lock_event(
                "f1",
                tenant_name,
                "asset:sword",
                &trade,
                "alice",
            ));
        }
        if data.len() > 3 && data[3] % 2 == 0 {
            events.push(settle_event(
                "f2",
                tenant_name,
                "asset:sword",
                &trade,
                "alice",
                "bob",
            ));
        } else if data.len() > 3 {
            events.push(unlock_event(
                "f2",
                tenant_name,
                "asset:sword",
                &trade,
                "alice",
            ));
        }
        let mut settle_intents = Vec::new();
        if data.len() > 4 && data[4] % 2 == 0 {
            let amount = (data.get(5).copied().unwrap_or(0) % 100).to_string();
            settle_intents.push(value_settle_intent(&trade, "wallet:gold", &amount, "alice"));
        }
        IngestBatch {
            events,
            settle_intents,
            commit: signed_commit(tenant_name, "cmt_0000000000000000000c"),
        }
    }

    proptest::proptest! {
        /// The determinism/replayability contract: incremental ingestion (each
        /// batch seeding from the records already present for the trade ids it
        /// touches, then applying and merging back) yields exactly the same
        /// index as rebuilding from the raw stream from scratch, and the
        /// builder never panics on arbitrary input.
        ///
        /// This mirrors [`crate::service::TradeService::ingest_batch`]'s seeding
        /// path (load existing records for `batch_trade_ids`, then apply), which
        /// is where incremental and rebuild could actually diverge, rather than
        /// applying the same batches to two empty maps.
        #[test]
        fn incremental_equals_rebuild_and_is_deterministic(data in proptest::prelude::any::<Vec<u8>>()) {
            let batches = partition_into_batches(&data);
            // Incremental ingest: for each batch, seed from the records already
            // present for the trade ids it touches, apply, and merge back into
            // the running index (the seeding/merge path of ingest_batch).
            let mut incremental: BTreeMap<String, TradeRecord> = BTreeMap::new();
            for b in &batches {
                let mut seeded: BTreeMap<String, TradeRecord> = BTreeMap::new();
                for trade_id in batch_trade_ids(b) {
                    if let Some(record) = incremental.get(&trade_id) {
                        seeded.insert(trade_id, record.clone());
                    }
                }
                apply(&mut seeded, b).unwrap();
                for (trade_id, record) in seeded {
                    incremental.insert(trade_id, record);
                }
            }
            // Rebuild from the raw stream (replaying the same batches).
            let mut rebuilt: BTreeMap<String, TradeRecord> = BTreeMap::new();
            for b in &batches {
                apply(&mut rebuilt, b).unwrap();
            }
            assert_eq!(incremental, rebuilt);
            // BCS canonical form is stable across runs (no hidden state).
            let first = bcs::to_bytes(&incremental).unwrap();
            let second = bcs::to_bytes(&incremental).unwrap();
            assert_eq!(first, second);
        }
    }

    fn partition_into_batches(data: &[u8]) -> Vec<IngestBatch> {
        if data.len() < 3 {
            return vec![batch_from_bytes(data)];
        }
        let mut batches = Vec::new();
        for window in data.chunks(6) {
            batches.push(batch_from_bytes(window));
        }
        batches
    }
}
