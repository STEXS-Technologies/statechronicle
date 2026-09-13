//! History reconstruction over a trade record.
//!
//! [`reconstruct_history`](crate::history::reconstruct_history) assembles an ordered [`TradeHistory`](statechronicle_domain::trade::TradeHistory) from a trade
//! record by resolving each recorded event id through an event map and deriving
//! the distinct committing commits. It is pure: it orders strictly by the
//! record's canonical `events` ordering and never consults a clock or store.

use std::collections::BTreeMap;

use statechronicle_domain::event::Event;
use statechronicle_domain::proof::CommitRef;
use statechronicle_domain::tenant::TenantId;
use statechronicle_domain::trade::{TradeEvent, TradeHistory, TradeRecord};

use crate::error::IndexError;

/// Reconstructs the ordered history of a trade record.
///
/// `events_by_id` maps a `(tenant_id_string, event_id_string)` key to its full
/// [`Event`]. Every recorded event
/// reference in the record is resolved in canonical order; a missing reference
/// fails closed. The distinct committing commits are collected from the record's
/// sides and value legs in canonical order, deduplicated per `(tenant, commit)`.
///
/// # Errors
///
/// Returns [`IndexError::MissingEvent`] when a recorded event reference cannot
/// be resolved from `events_by_id`.
pub fn reconstruct_history(
    record: &TradeRecord,
    events_by_id: &BTreeMap<(String, String), Event>,
) -> Result<TradeHistory, IndexError> {
    let mut events: Vec<TradeEvent> = Vec::with_capacity(record.events.len());
    for event_ref in &record.events {
        let key = (event_ref.tenant_id.0.clone(), event_ref.event_id.0.clone());
        let event = events_by_id
            .get(&key)
            .ok_or_else(|| IndexError::MissingEvent(event_ref.event_id.0.clone()))?;
        events.push(TradeEvent {
            event: event.clone(),
            tenant_id: event_ref.tenant_id.clone(),
        });
    }

    let commits = collect_commits(record);

    Ok(TradeHistory {
        trade_id: record.trade_id.clone(),
        status: record.status,
        events,
        commits,
    })
}

/// Collects the distinct committing commits from a record's sides and value
/// legs in canonical order, deduplicated per `(tenant, commit_id)`.
fn collect_commits(record: &TradeRecord) -> Vec<(TenantId, CommitRef)> {
    let mut commits: Vec<(TenantId, CommitRef)> = Vec::new();
    let mut seen: Vec<(String, String)> = Vec::new();
    for side in &record.sides {
        let key = (
            side.tenant.0.clone(),
            side.settle_commit.commit_id.0.clone(),
        );
        if !seen.contains(&key) {
            seen.push(key);
            commits.push((side.tenant.clone(), side.settle_commit.clone()));
        }
    }
    for leg in &record.value_legs {
        let key = (leg.tenant.0.clone(), leg.commit.commit_id.0.clone());
        if !seen.contains(&key) {
            seen.push(key);
            commits.push((leg.tenant.clone(), leg.commit.clone()));
        }
    }
    commits
}
