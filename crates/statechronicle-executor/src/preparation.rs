//! Authenticated, uncommitted mutation plans.

pub use crate::pipeline::{PreparationConfig, PreparationPorts, Preparer};

use statechronicle_core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle_core::digest::ContentDigest;
use statechronicle_core::error::StateChronicleError;
use statechronicle_core::limits::{
    MAX_EVENT_BATCH_BYTES, MAX_EVENT_BYTES, MAX_EVENTS_PER_COMMIT, check_size,
};
use statechronicle_domain::event::Event;
use statechronicle_intent::validated::ValidatedIntent;
use statechronicle_ports::authorization::AuthenticatedPrincipal;

use crate::intent_v1::AuthenticatedIntentV1;

/// Read-only mutation plan authenticated from a v1 actor-intent envelope.
///
/// The signed record remains borrowed and immutable for the life of the plan.
#[derive(Debug)]
pub struct PreparedMutationV1<'record> {
    authenticated: AuthenticatedIntentV1<'record>,
    principal: AuthenticatedPrincipal,
    events: Vec<Event>,
}

impl<'record> PreparedMutationV1<'record> {
    pub(crate) fn new(
        authenticated: AuthenticatedIntentV1<'record>,
        principal: &AuthenticatedPrincipal,
        events: Vec<Event>,
    ) -> Self {
        Self {
            authenticated,
            principal: principal.clone(),
            events,
        }
    }

    /// Structurally validated intent with its original v1 evidence.
    pub const fn authenticated(&self) -> &AuthenticatedIntentV1<'record> {
        &self.authenticated
    }

    /// Principal authorized at preparation time.
    pub const fn principal(&self) -> &AuthenticatedPrincipal {
        &self.principal
    }

    /// Complete event set produced by the existing transition evaluator.
    pub fn events(&self) -> &[Event] {
        &self.events
    }
}

/// An owned event plan produced by [`Preparer::prepare_player`] or
/// [`crate::pipeline::Executor::prepare_player`].
///
/// This is not a signed commit, durable receipt or replay result. Its private
/// fields bind the authenticated inputs, canonical body digest and constructed
/// events; borrowed access cannot retarget the accepted plan. It provides no
/// promise that the observed state or policy is still current at commit time.
/// There is no deserialization or public constructor.
///
/// ```compile_fail
/// use statechronicle_executor::preparation::PreparedMutation;
/// let forged = PreparedMutation { events: Vec::new() };
/// ```
#[derive(Debug)]
pub struct PreparedMutation {
    validated: ValidatedIntent,
    principal: AuthenticatedPrincipal,
    payload_digest: ContentDigest,
    events: Vec<Event>,
}

impl PreparedMutation {
    pub(crate) fn new(
        validated: ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        events: Vec<Event>,
    ) -> Result<Self, StateChronicleError> {
        let payload_digest = canonicalize_and_digest(&validated.intent)?;
        Ok(Self {
            validated,
            principal: principal.clone(),
            payload_digest,
            events,
        })
    }

    /// Authenticated request with its idempotency tuple derived from the body.
    pub const fn validated(&self) -> &ValidatedIntent {
        &self.validated
    }

    /// Principal bound to this request at preparation time.
    pub const fn principal(&self) -> &AuthenticatedPrincipal {
        &self.principal
    }

    /// Canonical intent-body digest to retain with the original retry identity.
    pub const fn payload_digest(&self) -> &ContentDigest {
        &self.payload_digest
    }

    /// Complete constructed event set; these events have not been persisted.
    pub fn events(&self) -> &[Event] {
        &self.events
    }
}

/// One owned, authenticated, uncommitted single-tenant batch.
///
/// Produced by [`Preparer::prepare_player_batch`] or
/// [`Preparer::prepare_player_settlement`], including their compatibility
/// [`crate::pipeline::Executor`] forwarding methods. A
/// failure returns no successful subset. Borrowed leg access cannot extract
/// owned single-mutation plans for separate commits. This is not a receipt,
/// and carries no promise of current state or durable batch atomicity.
/// There is no public constructor, cloning or deserialization.
///
/// ```compile_fail
/// use statechronicle_executor::preparation::PreparedBatch;
/// let forged = PreparedBatch { mutations: Vec::new() };
/// ```
#[derive(Debug)]
pub struct PreparedBatch {
    mutations: Vec<PreparedMutation>,
}

/// One protected v1 batch. Member plans remain grouped and cannot be moved
/// out for separate commits; original evidence stays borrowed from the input.
#[derive(Debug)]
pub struct PreparedBatchV1<'record> {
    mutations: Vec<PreparedMutationV1<'record>>,
}

impl<'record> PreparedBatchV1<'record> {
    pub(crate) fn new(
        mutations: Vec<PreparedMutationV1<'record>>,
    ) -> Result<Self, StateChronicleError> {
        validate_event_budget(mutations.iter().flat_map(PreparedMutationV1::events))?;
        Ok(Self { mutations })
    }

    /// Borrowed member plans in original request order.
    pub fn mutations(&self) -> &[PreparedMutationV1<'record>] {
        &self.mutations
    }

    /// All constructed events in member order.
    pub fn events(&self) -> impl Iterator<Item = &Event> + Clone {
        self.mutations.iter().flat_map(PreparedMutationV1::events)
    }
}

impl PreparedBatch {
    pub(crate) fn new(mutations: Vec<PreparedMutation>) -> Result<Self, StateChronicleError> {
        validate_event_budget(mutations.iter().flat_map(PreparedMutation::events))?;
        Ok(Self { mutations })
    }

    /// Authenticated requests, principals and immutable event groups in order.
    pub fn mutations(&self) -> &[PreparedMutation] {
        &self.mutations
    }

    /// All events in leg order without cloning payloads or building a second set.
    pub fn events(&self) -> impl Iterator<Item = &Event> + Clone {
        self.mutations.iter().flat_map(PreparedMutation::events)
    }
}

fn validate_event_budget<'event>(
    events: impl Iterator<Item = &'event Event>,
) -> Result<(), StateChronicleError> {
    let mut count = 0usize;
    let mut bytes = 0usize;
    for event in events {
        count = count.saturating_add(1);
        check_size("events", MAX_EVENTS_PER_COMMIT, count)?;
        let size = canonicalize(event)?.len();
        check_size("event", MAX_EVENT_BYTES, size)?;
        bytes = bytes.saturating_add(size);
        check_size("event_batch", MAX_EVENT_BATCH_BYTES, bytes)?;
    }
    Ok(())
}
