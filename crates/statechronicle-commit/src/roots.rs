//! Root computation for commit formation (protocol §13.1, §14).
//!
//! Two independent roots commit a batch:
//!
//! - [`event_root`]: a balanced Merkle root over the BCS canonical bytes of
//!   each event. The tree mirrors the accumulator's node convention
//!   (ADR-005): leaves are `H(0x11 || event_digest)` (the
//!   `LEAF_NODE_TAG` domain byte, with no key for events) and internal nodes
//!   are `H(0x10 || left || right)` (`INTERNAL_NODE_TAG`); a level with an
//!   odd node count duplicates its last node, exactly like the checkpoint
//!   tree. A single event therefore roots at its own leaf hash.
//!
//! - [`compute_state_root`]: the sparse Merkle state root produced by
//!   inserting the batch's [`StateUpdate`]s into a [`StateAccumulator`]
//!   (ADR-005). Keys are derived per event from its after-state: subject-held
//!   state types (consumable stack, fungible balance, entitlement, metered
//!   resource) key by `(tenant, resource, subject)` via
//!   [`StateKey::for_subject_held`]; owner-based types (unique asset, listing,
//!   escrow) key by `(tenant, resource)` via [`StateKey::for_resource`].
//!
//! The SMT root is a pure function of the `(key → digest)` set, so insertion
//! order never changes the result (ADR-005). Updates are nevertheless sorted
//! by key bytes before insertion so the pipeline order is stable and
//! documented.

use sha2::{Digest as _, Sha256};

use statechronicle_accumulator::key::StateKey;
use statechronicle_accumulator::sparse_merkle::{
    LEAF_NODE_TAG, StateAccumulator, StateRoot, StateUpdate, internal_hash,
};
use statechronicle_core::canonicalize::canonicalize_and_digest;
use statechronicle_core::digest::ContentDigest;

use statechronicle_domain::event::Event;
use statechronicle_domain::state_type::StateType;

use crate::error::CommitError;

/// Computes the deterministic Merkle root over a batch of events.
///
/// Each event is hashed through its BCS canonical bytes (ADR-004 §4), then the
/// digests are combined pairwise with the accumulator's internal-node domain
/// separation; an odd level duplicates its last node.
///
/// # Errors
///
/// Returns [`CommitError::EmptyBatch`] when `events` is empty, and
/// [`CommitError::Core`] when an event cannot be BCS canonicalized.
pub fn event_root(events: &[Event]) -> Result<ContentDigest, CommitError> {
    if events.is_empty() {
        return Err(CommitError::EmptyBatch);
    }
    let mut level: Vec<[u8; 32]> = Vec::with_capacity(events.len());
    for event in events {
        let digest = canonicalize_and_digest(event)?;
        level.push(event_leaf_hash(*digest.as_bytes()));
    }
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        for chunk in level.chunks(2) {
            let Some(left) = chunk.first().copied() else {
                break;
            };
            let right = chunk.get(1).copied().unwrap_or(left);
            next.push(internal_hash(left, right));
        }
        level = next;
    }
    let Some(root) = level.first().copied() else {
        return Err(CommitError::EmptyBatch);
    };
    Ok(ContentDigest::new(root))
}

/// Derives the sparse Merkle state key for an event's after-state.
///
/// Subject-held types ([`StateType::ConsumableStack`],
/// [`StateType::FungibleBalance`], [`StateType::Entitlement`],
/// [`StateType::MeteredResource`]) key by `(tenant, resource, subject)`
/// (ADR-005 §2); owner-based types ([`StateType::UniqueAsset`],
/// [`StateType::Listing`], [`StateType::Escrow`]) key by `(tenant, resource)`.
/// The subject is read from the after-state's `subject` field, which the
/// baseline profiles emit for subject-held projections.
///
/// # Errors
///
/// Returns [`CommitError::InvalidEvent`] when a subject-held type's after-state
/// is missing a non-empty string `subject`.
pub fn state_key_for(event: &Event, state_type: StateType) -> Result<StateKey, CommitError> {
    match state_type {
        StateType::ConsumableStack
        | StateType::FungibleBalance
        | StateType::Entitlement
        | StateType::MeteredResource => {
            let subject = event_subject(event)?;
            Ok(StateKey::for_subject_held(
                &event.tenant_id.0,
                &event.resource_id.0,
                subject,
            ))
        }
        StateType::UniqueAsset | StateType::Listing | StateType::Escrow => Ok(
            StateKey::for_resource(&event.tenant_id.0, &event.resource_id.0),
        ),
    }
}

/// Infers the state key using profile payload conventions.
///
/// An after-state carrying a string `subject` is treated as subject-held;
/// otherwise the event is resource-keyed. This mirrors how the baseline
/// profiles emit projections (§10) and is the convention used by
/// [`state_root_updates`] for callers that do not track `StateType` per event.
/// Type-aware callers should prefer [`state_key_for`].
///
/// # Errors
///
/// Returns [`CommitError::InvalidEvent`] when the after-state carries a
/// `subject` that is not a non-empty string.
pub fn infer_state_key(event: &Event) -> Result<StateKey, CommitError> {
    if event.after.state.get("subject").is_some() {
        let subject = event_subject(event)?;
        Ok(StateKey::for_subject_held(
            &event.tenant_id.0,
            &event.resource_id.0,
            subject,
        ))
    } else {
        Ok(StateKey::for_resource(
            &event.tenant_id.0,
            &event.resource_id.0,
        ))
    }
}

/// Derives the [`StateUpdate`] set for a batch, keyed per the profile
/// conventions and sorted by key bytes.
///
/// Each update commits `event.after.state_hash` at the state key of
/// `event.after` (protocol §14, ADR-005 §2).
///
/// # Errors
///
/// Returns [`CommitError::EmptyBatch`] when `events` is empty and
/// [`CommitError::InvalidEvent`] when an event's after-state cannot be mapped
/// onto a state key.
pub fn state_root_updates(events: &[Event]) -> Result<Vec<StateUpdate>, CommitError> {
    if events.is_empty() {
        return Err(CommitError::EmptyBatch);
    }
    let mut updates: Vec<StateUpdate> = Vec::with_capacity(events.len());
    for event in events {
        let key = infer_state_key(event)?;
        updates.push(StateUpdate::new(key, *event.after.state_hash.as_bytes()));
    }
    // Sorted-by-key insertion is a documented pipeline invariant; the SMT root
    // is a pure function of the leaf set, so this never changes the result
    // (ADR-005).
    updates.sort_by_key(|a| a.key);
    Ok(updates)
}

/// Computes the sparse Merkle state root over an update set.
///
/// Equivalent to `StateAccumulator::empty()` → `insert_batch(updates)` →
/// `root()` (ADR-005). The root of the empty tree is `default[256]`.
///
/// # Errors
///
/// Returns [`CommitError::Accumulator`] when the accumulator rejects the batch.
pub fn compute_state_root(updates: &[StateUpdate]) -> Result<StateRoot, CommitError> {
    let mut accumulator = StateAccumulator::empty();
    accumulator.insert_batch(updates)?;
    Ok(accumulator.root())
}

/// Verifies the `previous_state_root + current_updates = next_state_root`
/// replay equation for a commit chain (protocol §14).
///
/// `prior_updates` must be the full accumulated leaf set committed before
/// `current_updates`, i.e. the union of every earlier commit's updates. Both
/// declared roots are re-derived and compared fail-closed.
///
/// # Errors
///
/// Returns [`CommitError::StateRootMismatch`] when either the prior leaf set
/// does not reproduce `previous_state_root` or the combined leaf set does not
/// reproduce `declared_next_state_root`, and
/// [`CommitError::Accumulator`] when the accumulator rejects an update batch.
pub fn verify_state_root_continuity(
    previous_state_root: &ContentDigest,
    prior_updates: &[StateUpdate],
    current_updates: &[StateUpdate],
    declared_next_state_root: &ContentDigest,
) -> Result<(), CommitError> {
    let prior_root = compute_state_root(prior_updates)?;
    if ContentDigest::new(*prior_root.as_bytes()) != *previous_state_root {
        return Err(CommitError::StateRootMismatch {
            expected: String::from(previous_state_root.as_str()),
            actual: String::from(ContentDigest::new(*prior_root.as_bytes()).as_str()),
        });
    }
    let mut all = prior_updates.to_vec();
    all.extend_from_slice(current_updates);
    all.sort_by_key(|a| a.key);
    let next_root = compute_state_root(&all)?;
    if ContentDigest::new(*next_root.as_bytes()) != *declared_next_state_root {
        return Err(CommitError::StateRootMismatch {
            expected: String::from(declared_next_state_root.as_str()),
            actual: String::from(ContentDigest::new(*next_root.as_bytes()).as_str()),
        });
    }
    Ok(())
}

/// `H(0x11 || event_digest)`: the event tree leaf, mirroring the
/// accumulator's `LEAF_NODE_TAG` domain separation (ADR-005 §2). Events have
/// no SMT key, so the leaf covers only the event's canonical digest.
fn event_leaf_hash(event_digest: [u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([LEAF_NODE_TAG]);
    hasher.update(event_digest);
    hasher.finalize().into()
}

/// Reads the subject of a subject-held after-state.
///
/// # Errors
///
/// Returns [`CommitError::InvalidEvent`] when the after-state is missing a
/// `subject` field, its `subject` is not a string, or the subject is empty.
fn event_subject(event: &Event) -> Result<&str, CommitError> {
    let subject = event
        .after
        .state
        .get("subject")
        .ok_or_else(|| CommitError::InvalidEvent(String::from("after-state is missing `subject`")))?
        .as_str()
        .ok_or_else(|| {
            CommitError::InvalidEvent(String::from("after-state `subject` is not a string"))
        })?;
    if subject.is_empty() {
        return Err(CommitError::InvalidEvent(String::from(
            "after-state `subject` is empty",
        )));
    }
    Ok(subject)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use statechronicle_accumulator::sparse_merkle::{StateRoot, default_hash};
    use statechronicle_domain::event::StateCommitment;
    use statechronicle_domain::ids::{EventId, IntentId};
    use statechronicle_domain::intent::Operation;
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::subject::SubjectId;
    use statechronicle_domain::tenant::TenantId;

    const TENANT: &str = "stexs.game.alpha";

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-14T00:00:01Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn sample_commitment(version: u64, state: serde_json::Value) -> StateCommitment {
        StateCommitment {
            version,
            state_hash: canonicalize_and_digest(&state).unwrap(),
            state,
        }
    }

    /// Owner-based event (unique asset style payload).
    fn owner_event(id: &str, owner: &str) -> Event {
        let state = serde_json::json!({ "owner": owner, "status": "active" });
        Event::new(
            TenantId(String::from(TENANT)),
            EventId::new(format!("evt_{id}")).unwrap(),
            IntentId::new(format!("int_{id}")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            ResourceId(format!("asset:{id}")),
            SubjectId(String::from("account:stexs:player_123")),
            sample_commitment(41, serde_json::json!({})),
            sample_commitment(42, state),
            None,
            SubjectId(String::from("service:statechronicle.stexs.net")),
            timestamp(),
        )
    }

    /// Subject-held event (fungible balance style payload).
    fn held_event(id: &str, subject: &str) -> Event {
        let state = serde_json::json!({
            "subject": subject,
            "balance": "100",
            "unit": "gold_minor"
        });
        Event::new(
            TenantId(String::from(TENANT)),
            EventId::new(format!("evt_{id}")).unwrap(),
            IntentId::new(format!("int_{id}")).unwrap(),
            Operation::new(String::from("currency.transfer")).unwrap(),
            ResourceId(format!("balance:{id}")),
            SubjectId(String::from(subject)),
            sample_commitment(7, serde_json::json!({})),
            sample_commitment(8, state),
            None,
            SubjectId(String::from("service:statechronicle.stexs.net")),
            timestamp(),
        )
    }

    #[test]
    fn event_root_empty_is_rejected() {
        assert!(matches!(event_root(&[]), Err(CommitError::EmptyBatch)));
    }

    #[test]
    fn event_root_is_deterministic() {
        let events = vec![
            owner_event("a", "alice"),
            held_event("b", "bob"),
            owner_event("c", "carol"),
        ];
        assert_eq!(event_root(&events).unwrap(), event_root(&events).unwrap());
    }

    #[test]
    fn event_root_single_event_is_its_leaf() {
        let events = [owner_event("a", "alice")];
        let root = event_root(&events).unwrap();
        let digest = canonicalize_and_digest(&events[0]).unwrap();
        assert_eq!(root.as_bytes(), &event_leaf_hash(*digest.as_bytes()));
    }

    #[test]
    fn event_root_two_events_matches_pairwise_hash() {
        let events = [owner_event("a", "alice"), owner_event("b", "bob")];
        let root = event_root(&events).unwrap();
        let leaf_a = event_leaf_hash(*canonicalize_and_digest(&events[0]).unwrap().as_bytes());
        let leaf_b = event_leaf_hash(*canonicalize_and_digest(&events[1]).unwrap().as_bytes());
        assert_eq!(root.as_bytes(), &internal_hash(leaf_a, leaf_b));
    }

    #[test]
    fn event_root_odd_count_duplicates_last_node() {
        let events = [
            owner_event("a", "alice"),
            owner_event("b", "bob"),
            owner_event("c", "carol"),
        ];
        let root = event_root(&events).unwrap();
        let leaf_a = event_leaf_hash(*canonicalize_and_digest(&events[0]).unwrap().as_bytes());
        let leaf_b = event_leaf_hash(*canonicalize_and_digest(&events[1]).unwrap().as_bytes());
        let leaf_c = event_leaf_hash(*canonicalize_and_digest(&events[2]).unwrap().as_bytes());
        let expected = internal_hash(internal_hash(leaf_a, leaf_b), internal_hash(leaf_c, leaf_c));
        assert_eq!(root.as_bytes(), &expected);
    }

    #[test]
    fn state_key_for_owner_based_uses_resource_key() {
        let event = owner_event("sword", "alice");
        let key = state_key_for(&event, StateType::UniqueAsset).unwrap();
        assert_eq!(key, StateKey::for_resource(TENANT, "asset:sword"));
    }

    #[test]
    fn state_key_for_subject_held_uses_subject_key() {
        let event = held_event("gold", "account:stexs:player_123");
        let key = state_key_for(&event, StateType::FungibleBalance).unwrap();
        assert_eq!(
            key,
            StateKey::for_subject_held(TENANT, "balance:gold", "account:stexs:player_123")
        );
    }

    #[test]
    fn state_key_for_missing_subject_fails_closed() {
        let event = owner_event("sword", "alice");
        assert!(matches!(
            state_key_for(&event, StateType::FungibleBalance),
            Err(CommitError::InvalidEvent(_))
        ));
    }

    #[test]
    fn state_root_updates_are_sorted_by_key() {
        let events = vec![
            owner_event("zeta", "zoe"),
            held_event("alpha", "alice"),
            owner_event("beta", "bob"),
        ];
        let updates = state_root_updates(&events).unwrap();
        for pair in updates.windows(2) {
            assert!(pair[0].key <= pair[1].key);
        }
    }

    #[test]
    fn compute_state_root_empty_is_default_256() {
        let root = compute_state_root(&[]).unwrap();
        assert_eq!(root, StateRoot::new(default_hash(256)));
        assert_eq!(root, StateRoot::empty());
    }

    #[test]
    fn compute_state_root_is_deterministic() {
        let events = vec![
            owner_event("a", "alice"),
            held_event("b", "bob"),
            owner_event("c", "carol"),
        ];
        let updates = state_root_updates(&events).unwrap();
        assert_eq!(
            compute_state_root(&updates).unwrap(),
            compute_state_root(&updates).unwrap()
        );
    }

    #[test]
    fn compute_state_root_is_order_independent() {
        let events = vec![
            owner_event("a", "alice"),
            held_event("b", "bob"),
            owner_event("c", "carol"),
        ];
        let updates = state_root_updates(&events).unwrap();
        let mut reversed = updates.clone();
        reversed.reverse();
        assert_eq!(
            compute_state_root(&updates).unwrap(),
            compute_state_root(&reversed).unwrap()
        );
    }

    #[test]
    fn verify_state_root_continuity_accepts_chain() {
        let first = vec![owner_event("a", "alice"), held_event("b", "bob")];
        let second = vec![owner_event("c", "carol")];
        let first_updates = state_root_updates(&first).unwrap();
        let second_updates = state_root_updates(&second).unwrap();
        let first_root = compute_state_root(&first_updates).unwrap();
        let mut combined = first_updates.clone();
        combined.extend_from_slice(&second_updates);
        combined.sort_by_key(|a| a.key);
        let second_root = compute_state_root(&combined).unwrap();
        let previous = ContentDigest::new(*first_root.as_bytes());
        let next = ContentDigest::new(*second_root.as_bytes());
        assert!(
            verify_state_root_continuity(&previous, &first_updates, &second_updates, &next).is_ok()
        );
    }

    #[test]
    fn verify_state_root_continuity_rejects_wrong_declared_next() {
        let updates = state_root_updates(&[owner_event("a", "alice")]).unwrap();
        let previous = ContentDigest::new(*StateRoot::empty().as_bytes());
        let wrong = ContentDigest::new([0xabu8; 32]);
        let error = verify_state_root_continuity(&previous, &[], &updates, &wrong).unwrap_err();
        assert!(matches!(error, CommitError::StateRootMismatch { .. }));
    }

    #[test]
    fn verify_state_root_continuity_rejects_wrong_declared_previous() {
        let updates = state_root_updates(&[owner_event("a", "alice")]).unwrap();
        let actual_root = compute_state_root(&updates).unwrap();
        let previous = ContentDigest::new(*actual_root.as_bytes());
        let declared = ContentDigest::new(*actual_root.as_bytes());
        // `prior_updates` is empty but `previous_state_root` claims a root from
        // non-empty updates: the prior leaf set cannot reproduce it.
        let error = verify_state_root_continuity(&previous, &[], &[], &declared).unwrap_err();
        assert!(matches!(error, CommitError::StateRootMismatch { .. }));
    }
}
