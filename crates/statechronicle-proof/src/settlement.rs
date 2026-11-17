//! Independently requested historical settlement facts, authenticated by events.
//!
//! This verifier deliberately does not promote the unsigned `TradeSummary` or
//! `latest_event` metadata into authority. Supply full ordered commit event
//! batches and expectations obtained independently from the proof provider.
use std::collections::{BTreeMap, BTreeSet};

use crate::{error::ProofError, verify::verify_commit_signature_with_key};
use ed25519_dalek::VerifyingKey;
use statechronicle_accumulator::event_tree::event_merkle_root;
use statechronicle_core::{
    amount::Amount,
    canonicalize::{canonicalize, canonicalize_and_digest},
    digest::ContentDigest,
    limits::{
        MAX_COMMIT_BYTES, MAX_EVENT_BATCH_BYTES, MAX_EVENT_BYTES, MAX_EVENTS_PER_COMMIT,
        MAX_ID_LENGTH, check_size,
    },
};
use statechronicle_domain::{
    commit::{COMMIT_SCHEMA, Commit, ScopeKind},
    event::{EVENT_SCHEMA, Event},
    ids::{EventId, IntentId},
    resource::ResourceId,
    resource_state::ResourceState,
    signed::Signed,
    subject::SubjectId,
    tenant::TenantId,
};

/// Independently pinned signed event identity. `commit_digest` hashes the full
/// canonical commit body, not an untrusted commit ID or reference alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSettlementEvent {
    pub tenant: TenantId,
    pub resource: ResourceId,
    pub event_id: EventId,
    pub intent_id: IntentId,
    pub commit_digest: ContentDigest,
}

/// An asset's requested historical transfer out of the named trade hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedAssetSettlement {
    pub event: ExpectedSettlementEvent,
    pub from: SubjectId,
    pub to: SubjectId,
}

/// An independently requested payment, including both signed event identities.
/// Payment events do not contain a trade ID: the caller explicitly requests
/// this payment alongside the asset facts. This does not prove an unsigned
/// intent's value declaration or a cross-tenant atomic relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSettlementPayment {
    pub debit: ExpectedSettlementEvent,
    pub credit: ExpectedSettlementEvent,
    pub from: SubjectId,
    pub to: SubjectId,
    pub amount: Amount,
    pub unit: String,
}

/// The complete set of historical facts the caller wants authenticated.
/// An empty payment list requests no payment facts; it does not prove none occurred.
/// The asset list must be nonempty. Do not derive expectations from a summary
/// supplied by the same untrusted party supplying the witnesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSettlement {
    pub trade_id: String,
    pub assets: Vec<ExpectedAssetSettlement>,
    pub payments: Vec<ExpectedSettlementPayment>,
}

/// Untrusted full, ordered event batch and its enclosing signed commit.
/// This is a borrowed verifier input, not a new wire representation.
pub struct SettlementWitness<'witness> {
    pub commit: &'witness Signed<Commit>,
    pub events: &'witness [Event],
}

/// Authenticated historical facts only. No current ownership, canonical-head
/// freshness, full lifecycle completeness or distributed atomicity is implied.
/// Private construction and no deserialization prevent manufacturing a receipt.
///
/// ```compile_fail
/// use statechronicle_proof::settlement::VerifiedSettlement;
/// let receipt = VerifiedSettlement { expected: todo!() };
/// ```
#[derive(Debug)]
pub struct VerifiedSettlement {
    expected: ExpectedSettlement,
}
impl VerifiedSettlement {
    /// Returns only the independently requested and authenticated facts.
    pub const fn claim(&self) -> &ExpectedSettlement {
        &self.expected
    }
}

fn demand(condition: bool, detail: &str) -> Result<(), ProofError> {
    if condition {
        Ok(())
    } else {
        Err(ProofError::InvalidState(detail.into()))
    }
}

/// Authenticates the exact requested historical asset settlements and payments.
///
/// `resolve_key` must independently authorize the signer for the expected
/// tenant and historical commit (including executor, key ID and time policy).
/// A supplied verifying key is never inferred from the witness. This function
/// itself always performs strict Ed25519 verification after policy resolution.
/// Every used commit is pinned by its full body digest; no live store is read.
/// This establishes historical signed facts under those pins, not canonical
/// chain membership or current state. Use the current-claim verifier separately.
///
/// Bounds: at most 256 requested events and witnesses, 256 events per witness,
/// 64 KiB per event, 4 MiB aggregate event bytes, 1 MiB per commit body.
///
/// # Errors
/// Returns structural/size, signature/key-policy or canonicalization errors
/// when any requested identity, root, scope, state hash or transition differs.
pub fn verify_settlement_history(
    expected: &ExpectedSettlement,
    witnesses: &[SettlementWitness<'_>],
    mut resolve_key: impl FnMut(&TenantId, &Signed<Commit>) -> Result<VerifyingKey, ProofError>,
) -> Result<VerifiedSettlement, ProofError> {
    demand(
        !expected.trade_id.is_empty() && !expected.assets.is_empty(),
        "settlement requires a trade ID and assets",
    )?;
    check_size("trade ID", MAX_ID_LENGTH, expected.trade_id.len())?;
    check_size(
        "settlement assets",
        MAX_EVENTS_PER_COMMIT,
        expected.assets.len(),
    )?;
    check_size(
        "settlement payments",
        MAX_EVENTS_PER_COMMIT / 2,
        expected.payments.len(),
    )?;
    check_size(
        "settlement events",
        MAX_EVENTS_PER_COMMIT,
        expected
            .assets
            .len()
            .saturating_add(expected.payments.len().saturating_mul(2)),
    )?;
    check_size(
        "settlement witnesses",
        MAX_EVENTS_PER_COMMIT,
        witnesses.len(),
    )?;
    let mut pins = BTreeSet::new();
    let mut requested_events = BTreeSet::new();
    let mut assets = BTreeSet::new();
    for asset in &expected.assets {
        demand(
            assets.insert((&asset.event.tenant.0, &asset.event.resource.0)),
            "duplicate requested asset",
        )?;
    }
    for event in expected
        .assets
        .iter()
        .map(|a| &a.event)
        .chain(expected.payments.iter().flat_map(|p| [&p.debit, &p.credit]))
    {
        demand(
            requested_events.insert((&event.tenant.0, &event.event_id.0)),
            "duplicate requested event",
        )?;
        pins.insert((event.tenant.0.as_str(), event.commit_digest.as_str()));
    }
    let mut events = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut bytes = 0usize;
    for witness in witnesses {
        let signed = witness.commit;
        let commit = &signed.body;
        check_size(
            "settlement commit",
            MAX_COMMIT_BYTES,
            canonicalize(commit)?.len(),
        )?;
        let tenant = commit
            .scope
            .tenant_id
            .as_ref()
            .ok_or_else(|| ProofError::CommitScope("missing tenant".into()))?;
        demand(
            commit.schema == COMMIT_SCHEMA && commit.scope.kind == ScopeKind::Tenant,
            "unsupported settlement commit scope/schema",
        )?;
        let digest = canonicalize_and_digest(commit)?;
        let pin = (tenant.0.as_str(), digest.as_str());
        demand(pins.contains(&pin), "unrequested settlement commit")?;
        demand(
            seen.insert((tenant.0.clone(), digest.clone())),
            "duplicate settlement witness",
        )?;
        let key = resolve_key(tenant, signed)?;
        verify_commit_signature_with_key(signed, &key)?;
        check_size(
            "settlement commit events",
            MAX_EVENTS_PER_COMMIT,
            witness.events.len(),
        )?;
        demand(
            commit.event_count == witness.events.len() as u64,
            "settlement event count mismatch",
        )?;
        for event in witness.events {
            let size = canonicalize(event)?.len();
            check_size("settlement event", MAX_EVENT_BYTES, size)?;
            bytes = bytes
                .checked_add(size)
                .ok_or_else(|| ProofError::InvalidState("settlement byte count overflow".into()))?;
            check_size("settlement event batch", MAX_EVENT_BATCH_BYTES, bytes)?;
            demand(
                event.schema == EVENT_SCHEMA && event.tenant_id == *tenant,
                "settlement event scope/schema mismatch",
            )?;
            demand(
                event.before.version.checked_add(1) == Some(event.after.version),
                "settlement event version mismatch",
            )?;
            demand(
                canonicalize_and_digest(&event.before.state)? == event.before.state_hash
                    && canonicalize_and_digest(&event.after.state)? == event.after.state_hash,
                "settlement state hash mismatch",
            )?;
            demand(
                events
                    .insert(
                        (tenant.0.clone(), event.event_id.0.clone()),
                        (event, digest.clone()),
                    )
                    .is_none(),
                "duplicate witnessed event identity",
            )?;
        }
        demand(
            event_merkle_root(witness.events)?.as_ref() == Some(&commit.event_merkle_root),
            "settlement event root mismatch",
        )?;
    }
    demand(seen.len() == pins.len(), "missing settlement witness")?;
    let find = |pin: &ExpectedSettlementEvent| -> Result<&Event, ProofError> {
        let (event, digest) = events
            .get(&(pin.tenant.0.clone(), pin.event_id.0.clone()))
            .ok_or_else(|| ProofError::InvalidState("missing requested settlement event".into()))?;
        demand(
            event.resource_id == pin.resource
                && event.intent_id == pin.intent_id
                && *digest == pin.commit_digest,
            "settlement event identity mismatch",
        )?;
        Ok(event)
    };
    for asset in &expected.assets {
        let event = find(&asset.event)?;
        demand(
            event.operation.as_str() == "trade.settle",
            "requested asset event is not trade.settle",
        )?;
        let (ResourceState::UniqueAsset(before), ResourceState::UniqueAsset(after)) =
            (&event.before.state, &event.after.state)
        else {
            return Err(ProofError::InvalidState(
                "settlement asset state kind mismatch".into(),
            ));
        };
        demand(
            event.before.version != 0
                && before.trade_id.as_deref() == Some(expected.trade_id.as_str())
                && before.status.as_str() == "trade_held"
                && after.status.as_str() == "active"
                && after.trade_id.is_none()
                && before.owner == asset.from
                && after.owner == asset.to
                && asset.from != asset.to,
            "requested trade/asset ownership transition mismatch",
        )?;
    }
    for payment in &expected.payments {
        let debit = find(&payment.debit)?;
        let credit = find(&payment.credit)?;
        demand(
            payment.debit.tenant == payment.credit.tenant
                && payment.debit.resource == payment.credit.resource
                && payment.debit.intent_id == payment.credit.intent_id
                && payment.debit.commit_digest == payment.credit.commit_digest
                && payment.from != payment.to
                && !payment.amount.is_zero()
                && !payment.unit.is_empty(),
            "invalid requested payment pair",
        )?;
        let group_count = events
            .values()
            .filter(|(e, d)| {
                e.tenant_id == debit.tenant_id
                    && e.intent_id == debit.intent_id
                    && *d == payment.debit.commit_digest
            })
            .count();
        demand(group_count == 2, "payment intent is not exactly two events")?;
        for (event, subject, is_debit) in
            [(debit, &payment.from, true), (credit, &payment.to, false)]
        {
            let (ResourceState::FungibleBalance(before), ResourceState::FungibleBalance(after)) =
                (&event.before.state, &event.after.state)
            else {
                return Err(ProofError::InvalidState(
                    "payment balance state kind mismatch".into(),
                ));
            };
            let delta = if is_debit {
                before.balance.checked_sub(after.balance)
            } else {
                after.balance.checked_sub(before.balance)
            };
            let existing_holder = before.subject == *subject && before.unit == payment.unit;
            let unborn_credit = !is_debit
                && event.before.version == 0
                && event.before.state
                    == statechronicle_profiles::resource::policy(
                        statechronicle_domain::state_type::StateType::FungibleBalance,
                    )
                    .unborn_state(&event.after.state)
                    .map_err(|error| ProofError::InvalidState(error.to_string()))?;
            demand(
                event.operation.as_str() == "balance.transfer"
                    && event.actor == payment.from
                    && (if event.before.version == 0 {
                        unborn_credit
                    } else {
                        existing_holder
                    })
                    && after.subject == *subject
                    && after.unit == payment.unit
                    && delta == Some(payment.amount),
                "requested payment transition mismatch",
            )?;
        }
    }
    Ok(VerifiedSettlement {
        expected: expected.clone(),
    })
}
