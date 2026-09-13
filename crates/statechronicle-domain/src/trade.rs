//! Trade read-side wire types (Phase 2 of the trade completion).
//!
//! These are the canonical, BCS-deterministic domain objects for the trade
//! read-side vertical slice: the accumulated [`TradeRecord`](crate::trade::TradeRecord) produced by the
//! index builder, the assembled [`TradeHistory`](crate::trade::TradeHistory) and [`TradeSummary`](crate::trade::TradeSummary) views,
//! and the portable [`TradeProof`](crate::trade::TradeProof) that ties a settled trade's per-tenant state
//! proofs together.
//!
//! Every type here is serde + BCS-deterministic: collections that must be
//! stable across runs (sides, value legs, events) are `Vec`s that the builder
//! constructs in a sorted/canonical order, and there are no floats and no
//! `HashMap` in any output shape (determinism rule).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::event::Event;
use crate::ids::EventId;
use crate::intent::Operation;
use crate::proof::{CommitRef, ResourceStateProof};
use crate::resource::ResourceId;
use crate::subject::SubjectId;
use crate::tenant::TenantId;

/// Schema identifier for v0 trade proofs.
pub const TRADE_PROOF_SCHEMA: &str = "statechronicle.proof.trade.v0";

/// The lifecycle status of a trade, derived deterministically from its event
/// stream (first `trade.lock` -> `Open`; a settle batch -> `Settled`;
/// `trade.unlock` -> `Cancelled`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeStatus {
    /// The trade is pending settlement (at least one `trade.lock`, no settle
    /// or unlock yet).
    Open,
    /// The trade settled atomically (a `trade.settle` batch committed).
    Settled,
    /// The trade was cancelled (a `trade.unlock` returned the held asset).
    Cancelled,
}

/// The accumulated, deterministic projection of a single trade's lifecycle.
///
/// Keyed by `trade_id` in the read-side index (`BTreeMap<String, TradeRecord>`).
/// `sides` accumulates one entry per settling tenant; `value_legs` the declared
/// fungible value legs; `events` the ordered event references (locks, then
/// settles/unlocks, then value pairs) in canonical order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeRecord {
    /// The trade identifier that keys the record.
    pub trade_id: String,
    /// The deterministic lifecycle status.
    pub status: TradeStatus,
    /// One settle side per tenant, in sorted tenant order.
    pub sides: Vec<TradeSide>,
    /// The declared fungible value legs, in canonical order.
    pub value_legs: Vec<TradeValueLeg>,
    /// Ordered trade event references: locks, then settles/unlocks, then value
    /// pairs.
    pub events: Vec<TradeEventRef>,
}

impl TradeRecord {
    /// Constructs a fresh, empty trade record in the `Open` state.
    pub const fn new(trade_id: String) -> Self {
        Self {
            trade_id,
            status: TradeStatus::Open,
            sides: Vec::new(),
            value_legs: Vec::new(),
            events: Vec::new(),
        }
    }
}

/// One settled leg of a trade within a single tenant scope.
///
/// Groups the assets settled in a tenant and pins the committing commit, the
/// ownership transfer (from/to), and the ordered `trade.settle` event ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeSide {
    /// The tenant scope of the leg.
    pub tenant: TenantId,
    /// The assets settled by this leg (one or more).
    pub settle_assets: Vec<ResourceId>,
    /// The pre-settlement owner (from the settle event's before state).
    pub from_owner: String,
    /// The post-settlement owner (the settle event's `to_owner`).
    pub to_owner: String,
    /// The signed commit that pinned this leg's settle.
    pub settle_commit: CommitRef,
    /// The committing commit for each settled asset, keyed by asset.
    ///
    /// Additive per-asset commit tracking: a tenant may settle multiple assets
    /// of one trade in different commits (two separate `execute_settle` calls
    /// when no bundle is declared), so each asset's state proof must be fetched
    /// at the commit that settled it. `settle_commit` remains the first/leg
    /// representative commit for history and legacy compatibility.
    pub settle_commits_by_asset: BTreeMap<ResourceId, CommitRef>,
    /// The ordered `trade.settle` event ids for this leg.
    pub settle_event_ids: Vec<EventId>,
}

/// One declared fungible value leg of a settled trade.
///
/// The value declaration (resource, amount, recipient) survives only in the
/// settle intents, so the read-side derives this leg from those intents and
/// binds it to the net-zero `balance.transfer` pair that satisfied it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeValueLeg {
    /// The fungible resource moved by the value leg.
    pub resource: ResourceId,
    /// The canonical non-negative integer amount (declared string form).
    pub amount: String,
    /// The subject credited by the value leg.
    pub to_subject: SubjectId,
    /// The event ids of the net-zero `balance.transfer` pair (debit + credit).
    pub pair_event_ids: Vec<EventId>,
    /// The tenant scope of the value pair.
    pub tenant: TenantId,
    /// The commit that pinned the value pair.
    pub commit: CommitRef,
}

/// A lightweight reference to a trade event, used to order the read-side
/// history before the full `Event` is fetched from the event store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeEventRef {
    /// The referenced event id.
    pub event_id: EventId,
    /// The tenant scope of the event.
    pub tenant_id: TenantId,
    /// The event's operation.
    pub operation: Operation,
}

/// A full trade event as returned by the history read, carrying the tenant it
/// was executed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeEvent {
    /// The full validated event.
    pub event: Event,
    /// The tenant scope of the event.
    pub tenant_id: TenantId,
}

/// The ordered history view of a trade, reconstructed from the record's event
/// ordering and the event store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeHistory {
    /// The trade identifier.
    pub trade_id: String,
    /// The deterministic lifecycle status.
    pub status: TradeStatus,
    /// The ordered full trade events.
    pub events: Vec<TradeEvent>,
    /// The distinct committing commits, per tenant, in canonical order.
    pub commits: Vec<(TenantId, CommitRef)>,
}

/// The deterministic summary of a trade (sides + value legs), shared by the
/// history view and embedded in the trade proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeSummary {
    /// The trade identifier.
    pub trade_id: String,
    /// The deterministic lifecycle status.
    pub status: TradeStatus,
    /// The settled sides, in sorted tenant order.
    pub sides: Vec<TradeSide>,
    /// The declared value legs, in canonical order.
    pub value_legs: Vec<TradeValueLeg>,
}

/// One per-tenant leg of a [`TradeProof`]: a tenant, its committing commit, and
/// the state proofs of the assets settled in that tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeProofLeg {
    /// The tenant scope of the leg.
    pub tenant: TenantId,
    /// The commit that pinned the leg's settlement.
    pub commit: CommitRef,
    /// The state proofs of the assets settled in this tenant.
    pub state_proofs: Vec<ResourceStateProof>,
}

/// A portable proof of a settled trade (protocol §16, trade extension).
///
/// `summary` carries the deterministic sides and value legs; `legs` carry one
/// per-tenant state proof per settled asset, each independently verifiable
/// against its own commit under that tenant's verifying key. There is no
/// cross-tenant root: per-tenant verifiability, with `trade_id` as the
/// semantic binding (see `statechronicle_proof::trade::verify_trade_proof`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeProof {
    /// Schema identifier, always [`TRADE_PROOF_SCHEMA`] for v0.
    pub schema: String,
    /// The trade identifier bound to the proof.
    pub trade_id: String,
    /// The deterministic trade summary.
    pub summary: TradeSummary,
    /// The per-tenant settlement proof legs.
    pub legs: Vec<TradeProofLeg>,
}
