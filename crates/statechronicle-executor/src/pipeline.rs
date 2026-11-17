//! The validation pipeline (protocol §18.1).
//!
//! Ordered checks: idempotency, actor authentication, tenant scope, current
//! state loading, expected version, TrustGrant evaluation (via port), profile
//! rules, and deterministic after-state. Events are emitted only when every
//! check passes. The pipeline is the protocol's "brain", a deterministic
//! validator that drives pure [`crate::transition`] and [`crate::conflict`]
//! logic through injected [`Ports`](crate::pipeline::Ports).
//!
//! Events are **returned**, not persisted: commit formation, root computation,
//! and signing belong to the `statechronicle-commit` crate (§18.1 steps 13–15).
//! The executor never touches the event store.

#![allow(
    clippy::double_must_use,
    reason = "async_trait emits redundant must_use attributes"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use async_trait::async_trait;
use statechronicle_core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle_core::digest::ContentDigest;
use statechronicle_core::limits::{MAX_EVENTS_PER_COMMIT, MAX_INTENT_BYTES, check_size};
use statechronicle_domain::authority::{
    AggregationPolicy, AuthorityProof, EvaluationResult, TRUSTGRANT_EVALUATION_KIND,
    TrustGrantOutcome, aggregate_evaluation_digest,
};
use statechronicle_domain::commit::ProfileId;
use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::{EventId, IntentId};
#[cfg(test)]
use statechronicle_domain::intent::Operation;
use statechronicle_domain::intent::{Intent, SignatureBlock};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::resource_state::ResourceState;
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_intent::validated::ValidatedIntent;
use statechronicle_ports::authorization::{
    AuthenticatedPrincipal, AuthorizationContext, Authorizer,
};
use statechronicle_ports::intent_store::{IntentStore, IntentStoreError};
use statechronicle_ports::key_registry::KeyRegistry;
use statechronicle_ports::state_index::StateIndex;
use statechronicle_ports::tenant_store::TenantStore;
use statechronicle_ports::transaction_manager::{TransactionManager, TransactionManagerError};
use statechronicle_ports::trustgrant_evaluator::TrustGrantError;
use statechronicle_profiles::error::ProfileEvaluationError;
use statechronicle_profiles::execution::ExecutionPolicy;
use statechronicle_profiles::registry::ProfileRegistry;

use crate::atomicity;
use crate::conflict;
use crate::error::{ExecutorBuildError, ExecutorError, PortsBuildError};
use crate::preparation::{PreparedBatch, PreparedMutation};
#[cfg(test)]
use statechronicle_profiles::resource::{
    destination_holder, holder_for, is_transfer_operation, subject_for,
};
#[path = "batch_completion.rs"]
mod batch_completion;
use batch_completion::BatchCompletion;
#[path = "settlement.rs"]
mod settlement;

/// The injected intent-signature verifier.
///
/// Resolves a [`SignatureBlock`]'s `key_id` to a public key (wired by the
/// composition root) and verifies its signature over the BCS canonical bytes of
/// the intent body, returning [`ExecutorError::ActorAuthenticationFailed`] on
/// failure (protocol §18.1 step 4, ADR-004 §5).
type IntentVerifier =
    Arc<dyn Fn(&SignatureBlock, &[u8]) -> Result<(), ExecutorError> + Send + Sync>;

/// Dyn-compatible delegated-authority evaluator adapter (ADR-003).
///
/// The ports crate's `TrustGrantEvaluator` uses `trait_variant::make(Send)`,
/// which desugars `async fn` to `-> impl Future + Send` (RPITIT): a signature
/// that is **not object-safe**, so it cannot be held behind `dyn`. The executor
/// defines this boxed-future adapter so the pipeline can call delegated-authority
/// evaluation through `&dyn`. Concrete adapters (production consumer adapters and
/// test fakes) implement this trait directly.
#[async_trait]
pub trait TrustGrantPort: Send + Sync {
    /// Evaluates whether `actor` may perform `operation` on `resource` in
    /// `scope`, returning the outcome whose digest is bound into the event.
    ///
    /// # Errors
    ///
    /// Returns [`TrustGrantError::Denied`] when the evaluation result is not
    /// `allow`, [`TrustGrantError::Unavailable`] when the authority source
    /// cannot be resolved, and [`TrustGrantError::Stale`] when the evaluation
    /// is stale.
    async fn evaluate(
        &self,
        scope: &TenantId,
        actor: &SubjectId,
        operation: &str,
        resource: &ResourceId,
    ) -> Result<TrustGrantOutcome, TrustGrantError>;

    /// Checks revocation freshness for an authority proof.
    ///
    /// # Errors
    ///
    /// Returns [`TrustGrantError::Stale`] when the proof is no longer fresh.
    async fn check_revocation_freshness(
        &self,
        proof: &AuthorityProof,
    ) -> Result<(), TrustGrantError>;
}

/// The executor's injected port bundle.
///
/// Backends are injected (never implemented here) so the pipeline stays pure
/// and testable: the intent store, state index, tenant store, TrustGrant
/// adapter set, and transaction manager are all `Send + Sync` trait objects.
pub struct Ports {
    /// Stores intents for deduplication and idempotency (protocol §11.2).
    pub intent_store: Box<dyn IntentStore + Send + Sync>,
    /// Serves the current derived state projection of resources (protocol §9).
    pub state_index: Box<dyn StateIndex + Send + Sync>,
    /// Resolves tenant scope existence (protocol §8).
    pub tenant_store: Box<dyn TenantStore + Send + Sync>,
    /// The deployment's delegated-authority evaluator set, evaluated and aggregated per
    /// the active profile's authority policy (protocol §18.1 step 8, ADR-006
    /// §36 Q5). An empty set means no authority is configured.
    pub trustgrant: Vec<Box<dyn TrustGrantPort + Send + Sync>>,
    /// Coordinates atomic multi-store transactions (protocol §18.3).
    pub transaction_manager: Box<dyn TransactionManager + Send + Sync>,
}

impl Ports {
    /// Starts a fluent, struct-based port-bundle builder.
    ///
    /// Prefer [`PortsBuilder`] over positional construction so the injected
    /// backend adapters are named at the composition root.
    pub fn builder() -> PortsBuilder {
        PortsBuilder::default()
    }
}

/// Fluent builder for [`Ports`].
///
/// Collects the injected backend adapters with named setters and assembles the
/// bundle in [`PortsBuilder::build`]. `intent_store`, `state_index`,
/// `tenant_store`, and `transaction_manager` are required; `trustgrant`
/// defaults to an empty set (meaning no authority is configured, documented on
/// [`Ports`]).
#[derive(Default)]
pub struct PortsBuilder {
    intent_store: Option<Box<dyn IntentStore + Send + Sync>>,
    state_index: Option<Box<dyn StateIndex + Send + Sync>>,
    tenant_store: Option<Box<dyn TenantStore + Send + Sync>>,
    trustgrant: Vec<Box<dyn TrustGrantPort + Send + Sync>>,
    transaction_manager: Option<Box<dyn TransactionManager + Send + Sync>>,
}

impl PortsBuilder {
    /// Injects the intent store port (required).
    pub fn intent_store(mut self, intent_store: Box<dyn IntentStore + Send + Sync>) -> Self {
        self.intent_store = Some(intent_store);
        self
    }

    /// Injects the state index port (required).
    pub fn state_index(mut self, state_index: Box<dyn StateIndex + Send + Sync>) -> Self {
        self.state_index = Some(state_index);
        self
    }

    /// Injects the tenant store port (required).
    pub fn tenant_store(mut self, tenant_store: Box<dyn TenantStore + Send + Sync>) -> Self {
        self.tenant_store = Some(tenant_store);
        self
    }

    /// Injects the delegated-authority evaluator set.
    ///
    /// Defaults to an empty set, meaning no authority is configured.
    pub fn trustgrant(mut self, trustgrant: Vec<Box<dyn TrustGrantPort + Send + Sync>>) -> Self {
        self.trustgrant = trustgrant;
        self
    }

    /// Injects the transaction manager port (required).
    pub fn transaction_manager(
        mut self,
        transaction_manager: Box<dyn TransactionManager + Send + Sync>,
    ) -> Self {
        self.transaction_manager = Some(transaction_manager);
        self
    }

    /// Assembles the [`Ports`] bundle.
    ///
    /// # Errors
    ///
    /// Returns [`PortsBuildError`] naming the first missing required port.
    pub fn build(self) -> Result<Ports, PortsBuildError> {
        let intent_store = self
            .intent_store
            .ok_or(PortsBuildError::MissingIntentStore)?;
        let state_index = self.state_index.ok_or(PortsBuildError::MissingStateIndex)?;
        let tenant_store = self
            .tenant_store
            .ok_or(PortsBuildError::MissingTenantStore)?;
        let transaction_manager = self
            .transaction_manager
            .ok_or(PortsBuildError::MissingTransactionManager)?;
        Ok(Ports {
            intent_store,
            state_index,
            tenant_store,
            trustgrant: self.trustgrant,
            transaction_manager,
        })
    }
}

/// The execution engine (protocol §18).
///
/// Runs validated intents through the §18.1 pipeline and returns the events
/// that survived every gate. Deterministic by construction: all decision logic
/// lives in pure [`crate::transition`] / [`crate::conflict`] functions; the
/// only nondeterminism is the injected wall clock, event-id generator, and the
/// injected intent verifier's key-resolution.
///
/// **Actor authentication (Gap 2).** Signature presence is the platform's
/// submission policy: the executor verifies a present intent signature via the
/// injected intent verifier (which the composition
/// root wires to key resolution) and, when a signature is absent, applies the
/// v0 policy of allowing unsigned intents only for operations the active
/// profile does not require authority for. Authority-required paths are gated
/// downstream by the TrustGrant step and the profiles' `authorized_by` inputs.
///
/// **Multi-authority aggregation (Phase 2).** The deployment's authority set
/// ([`Ports::trustgrant`]) is a collection of delegated-authority evaluators. For every
/// authority-bound or authority-required transition the executor evaluates
/// every member and combines the results under the active profile's
/// [`AggregationPolicy`] (require-all by default, any-of where declared). The
/// bound proof carries a single aggregate digest over the sorted sub-evaluation
/// digests (or the sub-evaluation digest itself for a single-member set), with
/// `evaluated_at` set to the oldest sub-evaluation (protocol §18.1 step 8,
/// ADR-006 §36 Q5).
pub struct Executor {
    ports: Ports,
    preparation: PreparationConfig,
}

// Shared ingress decisions deliberately do not choose their evaluation order.
// Single registry lanes check keys before binding/policy; batch checks binding
// and policy first. Protected preparation also reconstructs structural input.
fn player_signature(validated: &ValidatedIntent) -> Result<&SignatureBlock, ExecutorError> {
    validated.signature.as_ref().ok_or_else(|| {
        ExecutorError::ActorAuthenticationFailed(String::from(
            "player mutation requires a detached intent signature",
        ))
    })
}

async fn authorize_principal(
    validated: &ValidatedIntent,
    principal: &AuthenticatedPrincipal,
    authorizer: &dyn Authorizer,
) -> Result<(), ExecutorError> {
    if principal.tenant != validated.intent.tenant_id || principal.subject != validated.intent.actor
    {
        return Err(ExecutorError::ActorAuthenticationFailed(String::from(
            "authenticated principal is not bound to intent actor and tenant",
        )));
    }
    authorizer
        .authorize(AuthorizationContext {
            principal,
            tenant: &validated.intent.tenant_id,
            intent: &validated.intent,
            resource: &validated.intent.resource_id,
        })
        .await
        .map_err(|error| ExecutorError::ActorAuthenticationFailed(error.to_string()))?;
    Ok(())
}

async fn check_player_key(
    validated: &ValidatedIntent,
    principal: &AuthenticatedPrincipal,
    signature: &SignatureBlock,
    key_registry: &dyn KeyRegistry,
) -> Result<(), ExecutorError> {
    key_registry
        .resolve_intent_key(
            &validated.intent.tenant_id,
            &principal.subject,
            &signature.key_id,
            &validated.intent.operation,
            validated.intent.created_at,
        )
        .await
        .map_err(|error| ExecutorError::ActorAuthenticationFailed(error.to_string()))?;
    Ok(())
}

// Owned checked commitments, produced once by the common preparation stage.
// Neither event construction nor callbacks can run before this succeeds.
struct PreparedEventStates {
    before: StateCommitment,
    after: StateCommitment,
}

impl PreparedEventStates {
    fn new(
        current: Option<&StateProjection>,
        after_state: ResourceState,
        policy: &dyn ExecutionPolicy,
    ) -> Result<Self, ExecutorError> {
        let version = current.map(|c| c.version).unwrap_or(0);
        let next_version = version
            .checked_add(1)
            .ok_or_else(|| ExecutorError::TransitionInvalid(String::from("version overflow")))?;
        let before_state = current
            .map(|c| Ok(c.state.clone()))
            .unwrap_or_else(|| policy.unborn_state(&after_state))?;
        let before_hash = match current {
            Some(projection) => projection.state_hash.clone(),
            None => canonicalize_and_digest(&before_state)?,
        };
        let after_hash = canonicalize_and_digest(&after_state)?;
        Ok(Self {
            before: StateCommitment {
                version,
                state_hash: before_hash,
                state: before_state,
            },
            after: StateCommitment {
                version: next_version,
                state_hash: after_hash,
                state: after_state,
            },
        })
    }
}

// Legacy lanes retain their historical marker lookup and optional write.
// Preparation neither observes nor creates non-authoritative replay markers.
#[derive(Clone, Copy)]
enum IntentHandling<'ports> {
    LegacyClaim(&'ports dyn IntentStore),
    LegacyReadOnly(&'ports dyn IntentStore),
    Prepare,
}

impl<'ports> IntentHandling<'ports> {
    const fn marker_store(self) -> Option<&'ports dyn IntentStore> {
        match self {
            Self::LegacyClaim(store) | Self::LegacyReadOnly(store) => Some(store),
            Self::Prepare => None,
        }
    }
    const fn legacy(persist_marker: bool, store: &'ports dyn IntentStore) -> Self {
        if persist_marker {
            Self::LegacyClaim(store)
        } else {
            Self::LegacyReadOnly(store)
        }
    }
}

/// Persistence boundary for player-facing execution.
///
/// The executor owns validation and deterministic event construction; the
/// composition root owns commit formation, signing, and durable storage. A
/// deployment that exposes player mutations should provide this sink and call
/// [`Executor::execute_player_durable`] so an accepted event cannot be
/// accidentally returned to the API without a persistence attempt.
#[async_trait]
pub trait DurableMutationSink: Send + Sync {
    /// Persists the complete event set atomically with the intent's
    /// idempotency reservation, signed commit, projections, and outbox.
    ///
    /// The sink must return an error unless all durable effects are committed;
    /// callers must preserve the intent id when retrying.
    async fn persist_player_mutation(
        &self,
        validated: &ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        events: &[Event],
    ) -> Result<(), String>;
}

/// Persistence boundary for multi-intent inventory, market, and settlement
/// batches. Implementations must claim every intent and write all events,
/// projections, commits, and outbox effects in one durable transaction (or
/// reject unsupported cross-tenant atomicity).
#[async_trait]
pub trait DurableBatchSink: Send + Sync {
    /// Persists a complete, already-validated batch atomically.
    async fn persist_batch(
        &self,
        intents: &[ValidatedIntent],
        events: &[Event],
    ) -> Result<(), String>;
}

/// One player-originated intent and the principal authenticated for it.
///
/// Batch and settlement operations frequently contain intents for different
/// actors. Keeping the principal next to the exact validated intent prevents
/// a composition root from accidentally authorizing an entire market or trade
/// batch under one unrelated identity.
#[derive(Debug, Clone, Copy)]
pub struct PlayerBatchItem<'item> {
    /// Signed, validated mutation request.
    pub validated: &'item ValidatedIntent,
    /// Principal authenticated for this exact request.
    pub principal: &'item AuthenticatedPrincipal,
}

impl Executor {
    /// Starts a fluent, struct-based executor builder.
    ///
    /// Prefer [`ExecutorBuilder`] over positional construction so the injected
    /// identity, clock, event-id generator, and intent verifier are named at
    /// the composition root.
    pub fn builder() -> ExecutorBuilder {
        ExecutorBuilder::default()
    }

    /// Runs one validated intent through the §18.1 pipeline.
    ///
    /// Returns the emitted event, or an empty vector when the intent is an
    /// idempotent replay of an already-accepted intent.
    ///
    /// # Pipeline (protocol §18.1)
    ///
    /// 1. **Parse / schema / size**: enforced upstream by the
    ///    `statechronicle-intent` crate (`validate::validate`). The executor
    ///    re-checks the canonical intent size against [`MAX_INTENT_BYTES`] as a
    ///    defense-in-depth gate (fail-closed on `SizeLimitExceeded`).
    /// 2. **Idempotency lookup**: `intent_store.get_intent`; a stored intent
    ///    with an equal payload is a replay (`Ok(vec![])`); a stored intent
    ///    with a different payload is [`ExecutorError::DuplicateIntent`]. A
    ///    legacy claim is made only after authentication, validation, and
    ///    event construction. Production callers must use the durable ledger
    ///    claim/finalize path for an atomic reservation.
    /// 3. **Actor authentication**: when `validated.signature` is present, the
    ///    injected intent verifier is invoked over
    ///    the BCS canonical bytes of the intent body; failure yields
    ///    [`ExecutorError::ActorAuthenticationFailed`]. When a signature is
    ///    absent, the v0 policy applies: unsigned intents are permitted only
    ///    for operations the active profile does not require authority for
    ///    (authority-required paths are gated downstream by the delegated-authority
    ///    evaluation step and the profiles' `authorized_by` inputs). When `authority` is
    ///    present, the executor still checks revocation freshness
    ///    (`check_revocation_freshness` → [`ExecutorError::AuthorityStale`]).
    /// 4. **Tenant scope**: `check_tenant_scope` then
    ///    `tenant_store.tenant_exists` → [`ExecutorError::TenantNotFound`].
    /// 5. **Load current state**: the profile's execution policy selects the
    ///    resource or subject index address. The intent's
    ///    `state_type` must be present ([`ExecutorError::StateTypeRequired`]).
    /// 6. **Expected version**: [`conflict::check_expected_version`].
    /// 7. **Profile access gates**: the execution policy checks whether the
    ///    loaded state may accept this command. Expiry
    ///    ([`conflict::check_expiry`]) is enforced before the intent is even
    ///    claimed in step 2.
    /// 8. **TrustGrant authority**: the active profile's rule set is resolved
    ///    here (before the gate) so `requires_authority` / `authority_policy`
    ///    drive the evaluation. Authority-required operations MUST carry a
    ///    binding ([`ExecutorError::AuthorityMissing`] otherwise). When
    ///    authority is present or required, every member of
    ///    [`Ports::trustgrant`] is evaluated and the outcomes are aggregated
    ///    under the profile's policy (require-all default, any-of where
    ///    declared; under require-all, any member failing (deny/stale/unavailable)
    ///    fails closed; under any-of, a failing member is tolerated so long as
    ///    at least one member allows; an empty set fails closed). The bound
    ///    proof carries the aggregate digest over the
    ///    sorted sub-evaluation digests (or the sub-evaluation digest itself
    ///    for a single-member set) and `evaluated_at` set to the oldest
    ///    sub-evaluation. When authority is absent and not required, the event
    ///    proceeds without a binding (profile-owned fallback).
    /// 9. **Profile rules**: `rules.check` must pass (→
    ///    [`ExecutorError::Profile`]).
    /// 10. **After-state**: the profile's reducer computes the deterministic
    ///     new projection payload.
    /// 11. **Emit event**: before/after [`StateCommitment`]s carry
    ///     `current.version` / `current.version + 1` and canonical state
    ///     digests (`canonicalize_and_digest`), the evaluated authority is
    ///     bound when present, and `executor`/`created_at` come from the
    ///     injected identity and clock.
    /// 12. **Additional mutation**: the profile's execution policy may declare
    ///     a companion address and reducer. Both events share one intent id.
    ///     Baseline resource profiles use this for atomic debit/credit pairs.
    /// 13. **Atomicity**: batch identity and tenant checks are generic; each
    ///     event group is validated by its owning profile's execution policy.
    ///
    /// # Errors
    ///
    /// Returns every [`ExecutorError`] variant in fail-closed order above. No
    /// event is emitted when any check fails.
    pub async fn execute(&self, validated: &ValidatedIntent) -> Result<Vec<Event>, ExecutorError> {
        self.execute_inner(
            validated,
            false,
            IntentHandling::LegacyClaim(self.ports.intent_store.as_ref()),
        )
        .await
    }

    /// Executes only after an authenticated principal is explicitly bound to
    /// the intent actor and an external policy authorizer allows the mutation.
    ///
    /// This is the safe ingress helper for player-facing APIs. The legacy
    /// [`Self::execute`] method remains available for pure planning and trusted
    /// internal callers, but it must not be exposed directly to clients.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::ActorAuthenticationFailed`] when the principal
    /// is not bound to the intent or policy denies/unavailable, then propagates
    /// the normal execution pipeline errors.
    pub async fn execute_authenticated(
        &self,
        validated: &ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        authorizer: &dyn Authorizer,
    ) -> Result<Vec<Event>, ExecutorError> {
        self.execute_authenticated_inner(
            validated,
            principal,
            authorizer,
            IntentHandling::LegacyClaim(self.ports.intent_store.as_ref()),
        )
        .await
    }

    async fn execute_authenticated_inner(
        &self,
        validated: &ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        authorizer: &dyn Authorizer,
        intent_handling: IntentHandling<'_>,
    ) -> Result<Vec<Event>, ExecutorError> {
        authorize_principal(validated, principal, authorizer).await?;
        self.execute_inner(validated, false, intent_handling).await
    }

    /// Authenticates and prepares a player mutation without persistence.
    ///
    /// Requires a detached signature, trusted key metadata, a matching
    /// principal and explicit authorization. Uses the ordinary single-intent
    /// profile gates, but never reads or writes the independent intent store,
    /// begins a transaction, or dispatches a persistence sink. A legacy marker
    /// cannot suppress authentication or become an empty-event replay.
    ///
    /// Reads and injected callbacks must not perform durable writes or external
    /// effect dispatch. The clock and event-id generator are still invoked;
    /// a local id counter may advance. Repeated preparation may produce new
    /// event ids, so this is neither a durable retry nor a replay receipt.
    /// Resolve durable retries by the original intent identity and digest.
    /// State can change after preparation: commit must enforce concurrency,
    /// current authorization and the complete atomic ledger boundary.
    ///
    /// # Errors
    ///
    /// Returns authentication/key-policy failures or normal profile/transition
    /// errors. Returns no committed outcome, including on success.
    pub async fn prepare_player(
        &self,
        validated: &ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
    ) -> Result<PreparedMutation, ExecutorError> {
        self.evaluation()
            .prepare_player(validated, principal, authorizer, key_registry)
            .await
    }

    /// Authenticate and prepare every leg without claims or persistence.
    ///
    /// Like the legacy player batch lane, each item's signature presence,
    /// principal/authorization and key policy are checked before execution.
    /// Public validated fields are reconstructed, then duplicate identities
    /// and mixed tenants refuse before state reads. Every leg uses the shared
    /// preparation core, including real signature verification and all profile
    /// gates; all resulting groups pass the shared batch validator.
    ///
    /// No intent-store call, transaction-manager operation or sink dispatch is
    /// made. No successful subset escapes on failure. Event ids and clock
    /// callbacks may advance; repeated preparation is not a durable replay.
    /// This prepares one tenant scope and does not supply durable group commit
    /// or recovery. Host callbacks/reads must obey the same effect restrictions
    /// as [`Self::prepare_player`].
    /// # Errors
    /// Refuses empty, oversized, duplicate or mixed-tenant batches, ingress
    /// failures and any leg or group invariant failure, without durable writes.
    pub async fn prepare_player_batch(
        &self,
        items: &[PlayerBatchItem<'_>],
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
    ) -> Result<PreparedBatch, ExecutorError> {
        self.evaluation()
            .prepare_player_batch(items, authorizer, key_registry)
            .await
    }

    /// Prepare one authenticated single-tenant settlement as an owned batch.
    ///
    /// Every member passes the same ingress and preparation gates as
    /// [`Self::prepare_player_batch`], with coordinated baseline resource routes
    /// enabled. The complete event set must additionally pass the existing
    /// settlement validator: declared value legs and bundle membership cannot
    /// escape as a successful subset. No claim, transaction or sink is invoked.
    ///
    /// Bind the result with `SignedBatch` in the public facade and commit it
    /// through one supported ledger transaction. Preparation alone supplies no
    /// durable authority, freshness guarantee or cross-tenant atomicity.
    /// Reads/callbacks obey [`Self::prepare_player`]'s effect restrictions.
    /// # Errors
    /// Refuses ingress, scope, profile, transfer, value-leg or bundle failures.
    pub async fn prepare_player_settlement(
        &self,
        items: &[PlayerBatchItem<'_>],
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
    ) -> Result<PreparedBatch, ExecutorError> {
        self.evaluation()
            .prepare_player_settlement(items, authorizer, key_registry)
            .await
    }

    /// Executes a player-originated mutation with the strict public-ingress
    /// policy: a detached signature is mandatory, the authenticated principal
    /// must match the intent actor/tenant, and the injected authorizer must
    /// allow the operation.  Use [`Self::execute_authenticated`] only for
    /// explicitly trusted service jobs that have a separate authentication
    /// mechanism.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::ActorAuthenticationFailed`] when the intent
    /// has no signature, the principal is not bound to the intent, or the
    /// authorizer/signature verifier rejects it. Other validation failures
    /// are propagated from the normal execution pipeline.
    pub async fn execute_player(
        &self,
        validated: &ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        authorizer: &dyn Authorizer,
    ) -> Result<Vec<Event>, ExecutorError> {
        player_signature(validated)?;
        self.execute_authenticated(validated, principal, authorizer)
            .await
    }

    /// Executes a signed player mutation and hands its events to the required
    /// durable persistence boundary before returning success.
    ///
    /// This is a compatibility sink route. It preserves the strict
    /// authentication and authorization checks of [`Self::execute_player`],
    /// then invokes the deployment's sink exactly once for the constructed
    /// event set. The sink is responsible for calling the commit crate's
    /// `persist_durable_verified` (or metrics variant) with an atomic ledger
    /// transaction. No events are returned when persistence fails.
    /// Idempotent replays (an empty event set) are returned without invoking
    /// the sink. An empty result is not evidence of a committed mutation;
    /// the independent intent store may contain a planning marker only.
    /// Use [`Self::prepare_player`] to prepare without those markers.
    ///
    /// # Errors
    ///
    /// Returns the normal player authentication, authorization, validation,
    /// and transition errors, or [`ExecutorError::Store`] when the sink does
    /// not report a fully committed durable mutation.
    pub async fn execute_player_durable(
        &self,
        validated: &ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        authorizer: &dyn Authorizer,
        sink: &dyn DurableMutationSink,
    ) -> Result<Vec<Event>, ExecutorError> {
        player_signature(validated)?;
        let events = self
            .execute_authenticated_inner(
                validated,
                principal,
                authorizer,
                IntentHandling::LegacyReadOnly(self.ports.intent_store.as_ref()),
            )
            .await?;
        // An empty event set is the executor's idempotent replay signal. Do
        // not rebuild or hand a replay to the persistence sink.
        if events.is_empty() {
            return Ok(events);
        }
        sink.persist_player_mutation(validated, principal, &events)
            .await
            .map_err(ExecutorError::Store)?;
        Ok(events)
    }

    /// Player ingress variant that also checks key ownership, tenant scope,
    /// operation scope, validity interval, and revocation through a
    /// deployment-provided [`KeyRegistry`] before cryptographic verification.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::ActorAuthenticationFailed`] when the intent
    /// lacks a signature or the registry rejects/unable to resolve its key.
    /// Signature, authorization, and transition failures are propagated from
    /// [`Self::execute_player`].
    pub async fn execute_player_with_key_registry(
        &self,
        validated: &ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
    ) -> Result<Vec<Event>, ExecutorError> {
        let signature = player_signature(validated)?;
        check_player_key(validated, principal, signature, key_registry).await?;
        self.execute_player(validated, principal, authorizer).await
    }

    /// Durable player ingress with mandatory key-lifecycle enforcement.
    ///
    /// This combines the two production ingress requirements: the detached
    /// intent key is resolved against the deployment's tenant/actor/operation
    /// registry, and accepted events are handed to the durable sink before a
    /// success result is returned. The sink must use the commit crate's
    /// verified durable-persistence API for the commit-signature boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::ActorAuthenticationFailed`] before state
    /// execution when the signature is absent or its registered key is not
    /// trusted for this intent. Other errors match [`Self::execute_player_durable`].
    pub async fn execute_player_durable_with_key_registry(
        &self,
        validated: &ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
        sink: &dyn DurableMutationSink,
    ) -> Result<Vec<Event>, ExecutorError> {
        let signature = player_signature(validated)?;
        check_player_key(validated, principal, signature, key_registry).await?;
        self.execute_player_durable(validated, principal, authorizer, sink)
            .await
    }

    /// Durably executes a batch of player requests after checking every
    /// request's signature presence, principal binding, authorization, and
    /// key lifecycle scope.
    ///
    /// This is the public durable route for player-driven inventory batches,
    /// market actions, and settlement legs. The existing durable batch APIs
    /// remain appropriate for trusted service jobs; they intentionally do not
    /// invent an identity for every item in a batch.
    ///
    /// # Errors
    ///
    /// Rejects the entire batch before execution if any item is unsigned,
    /// principal-mismatched, unauthorized, or uses an untrusted key. Other
    /// errors match [`Self::execute_batch_durable`].
    pub async fn execute_player_batch_durable_with_key_registry(
        &self,
        items: &[PlayerBatchItem<'_>],
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
        sink: &dyn DurableBatchSink,
    ) -> Result<Vec<Event>, ExecutorError> {
        let mut intents = Vec::with_capacity(items.len());
        for item in items {
            let signature = player_signature(item.validated)?;
            authorize_principal(item.validated, item.principal, authorizer).await?;
            check_player_key(item.validated, item.principal, signature, key_registry).await?;
            intents.push(item.validated.clone());
        }
        self.execute_batch_durable(&intents, sink).await
    }

    /// Shared execution core. A profile may require a coordinated batch route;
    /// ordinary single-intent and batch calls reject those requests before
    /// claiming them. A coordinator validates the required group semantics.
    async fn execute_inner(
        &self,
        validated: &ValidatedIntent,
        allow_coordinated: bool,
        intent_handling: IntentHandling<'_>,
    ) -> Result<Vec<Event>, ExecutorError> {
        self.evaluation()
            .execute_inner(validated, allow_coordinated, intent_handling)
            .await
    }

    /// Runs a batch of intents atomically (protocol §18.3).
    ///
    /// Each intent runs through [`Self::execute`] inside a transaction begun
    /// via `transaction_manager.begin`. All-or-nothing: if any intent fails,
    /// the transaction is rolled back and
    /// [`ExecutorError::AtomicityViolation`] is returned, so no partial results
    /// escape. The resulting batch is additionally validated with
    /// [`atomicity::validate_batch_consistency`] (distinct event ids, one
    /// tenant, and distinct intent ids except an atomic transfer pair sharing
    /// one intent id).
    ///
    /// v0 note: the transaction wrapper is symbolic. The executor does not
    /// persist anything itself (events are returned for the commit crate), and
    /// therefore this pure/planning route does not claim intents. The handle
    /// records commit/rollback intent only. Use [`Self::execute_batch_durable`]
    /// for claims and event persistence through one adapter-owned atomic
    /// boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::AtomicityViolation`] when any intent fails or
    /// the resulting batch is internally inconsistent, and
    /// [`ExecutorError::Store`] when the transaction manager itself fails.
    pub async fn execute_batch(
        &self,
        intents: &[ValidatedIntent],
    ) -> Result<Vec<Event>, ExecutorError> {
        self.execute_batch_inner(intents, None).await
    }

    async fn execute_batch_inner(
        &self,
        intents: &[ValidatedIntent],
        sink: Option<&dyn DurableBatchSink>,
    ) -> Result<Vec<Event>, ExecutorError> {
        let Some(first) = intents.first() else {
            return Err(ExecutorError::AtomicityViolation(String::from(
                "empty batch",
            )));
        };
        let tenant = &first.intent.tenant_id;
        let completion =
            BatchCompletion::begin(sink, || self.ports.transaction_manager.begin(tenant)).await?;

        // Both a leg failure and an inconsistent-batch validation failure are
        // rolled back atomically (a failed validation must not short-circuit
        // past the rollback via `?`).
        // The symbolic transaction manager cannot roll back the independent
        // intent store. Only a durable sink owns an atomic claim boundary;
        // pure/planning batches must never leave partial claims behind.
        let result = match self.run_batch_fail_closed(intents, sink.is_some()).await {
            Ok(events) => self
                .validate_profile_batch(intents, &events)
                .map(|()| events),
            Err(error) => Err(error),
        };
        match result {
            Ok(events) => {
                completion.succeed(intents, &events).await?;
                Ok(events)
            }
            Err(error) => Err(completion.refuse(error, "batch rollback failed").await),
        }
    }

    /// Executes a batch and returns success only after the deployment's
    /// durable batch sink commits every intent and event.
    ///
    /// This is the safe routing boundary for shared inventory, marketplace,
    /// and same-tenant settlement commands. The sink owns commit formation,
    /// signing, idempotency claims, projections, and outbox writes.
    ///
    /// # Errors
    ///
    /// Returns the normal batch execution/atomicity errors or
    /// [`ExecutorError::Store`] when durable persistence fails.
    pub async fn execute_batch_durable(
        &self,
        intents: &[ValidatedIntent],
        sink: &dyn DurableBatchSink,
    ) -> Result<Vec<Event>, ExecutorError> {
        self.execute_batch_inner(intents, Some(sink)).await
    }

    fn validate_profile_batch(
        &self,
        intents: &[ValidatedIntent],
        events: &[Event],
    ) -> Result<(), ExecutorError> {
        self.validate_profile_groups(
            |identity| {
                intents
                    .iter()
                    .find(|request| &request.intent.intent_id == identity)
            },
            events,
        )
    }

    fn validate_profile_groups<'event>(
        &self,
        intent_for: impl Fn(&IntentId) -> Option<&'event ValidatedIntent>,
        events: impl IntoIterator<Item = &'event Event>,
    ) -> Result<(), ExecutorError> {
        self.evaluation()
            .validate_profile_groups(intent_for, events)
    }

    /// Executes intents in order under the caller's coordination policy.
    async fn run_batch(
        &self,
        intents: &[ValidatedIntent],
        allow_coordinated: bool,
        persist_intent: bool,
    ) -> Result<Vec<Event>, ExecutorError> {
        let mut events = Vec::new();
        for validated in intents {
            events.extend(
                self.execute_inner(
                    validated,
                    allow_coordinated,
                    IntentHandling::legacy(persist_intent, self.ports.intent_store.as_ref()),
                )
                .await?,
            );
        }
        Ok(events)
    }

    /// Executes a single-tenant batch and fails closed on partial replay.
    ///
    /// Mirrors the cross-tenant fail-closed behavior for the batch entry point.
    /// Because the transaction wrapper is symbolic, a mid-batch failure leaves
    /// earlier intents claimed in the intent store; a retry would otherwise
    /// replay those as idempotent (empty output) alongside fresh intents and
    /// return a partial batch with the earlier transition silently lost. Here an
    /// intent whose execution emits no events is treated as an idempotent replay
    /// and rejects the whole batch, so no partial batch can silently escape.
    /// Requests requiring a coordinator are rejected by this ordinary batch path.
    ///
    /// # Errors
    ///
    /// Returns the first intent failure, or
    /// [`ExecutorError::AtomicityViolation`] when any intent in the batch
    /// replayed idempotently (emitted no events) rather than executing fresh.
    async fn run_batch_fail_closed(
        &self,
        intents: &[ValidatedIntent],
        persist_intent: bool,
    ) -> Result<Vec<Event>, ExecutorError> {
        let mut events = Vec::new();
        for validated in intents {
            let produced = self
                .execute_inner(
                    validated,
                    false,
                    IntentHandling::legacy(persist_intent, self.ports.intent_store.as_ref()),
                )
                .await?;
            if produced.is_empty() {
                return Err(ExecutorError::AtomicityViolation(format!(
                    "partial replay detected in batch: intent `{}` was already claimed and replayed idempotently; the batch fails closed so no partial results escape",
                    validated.intent.intent_id.as_str()
                )));
            }
            events.extend(produced);
        }
        Ok(events)
    }

    /// Runs each tenant's leg in sorted tenant order, collecting the emitted
    /// events into tenant-scoped groups.
    ///
    /// Propagates the coordinator's routing policy into each scoped batch.
    async fn run_cross_tenant_legs(
        &self,
        by_name: &BTreeMap<String, Vec<ValidatedIntent>>,
        allow_coordinated: bool,
        persist_intent: bool,
    ) -> Result<Vec<atomicity::TenantEventGroup>, ExecutorError> {
        let mut groups = Vec::new();
        for (name, sub_intents) in by_name {
            let events = self
                .run_batch(sub_intents, allow_coordinated, persist_intent)
                .await?;
            groups.push(atomicity::TenantEventGroup {
                tenant: TenantId(name.clone()),
                events,
            });
        }
        Ok(groups)
    }
}

/// Fluent builder for [`Executor`].
///
/// Collects the executor's injected components with named setters and assembles
/// the engine in [`ExecutorBuilder::build`]. `ports`, `executor`, `clock`,
/// `event_id_gen`, and `intent_verifier` are required; `profiles` defaults to
/// [`ProfileRegistry::baseline`].
#[derive(Default)]
pub struct ExecutorBuilder {
    ports: Option<Ports>,
    profiles: Option<ProfileRegistry>,
    executor: Option<SubjectId>,
    now: Option<Box<dyn Fn() -> DateTime<Utc> + Send + Sync>>,
    event_id_fn: Option<Box<dyn Fn() -> EventId + Send + Sync>>,
    intent_verifier: Option<IntentVerifier>,
}

impl ExecutorBuilder {
    /// Injects the port bundle (required).
    pub fn ports(mut self, ports: Ports) -> Self {
        self.ports = Some(ports);
        self
    }

    /// Injects the profile registry.
    ///
    /// Defaults to [`ProfileRegistry::baseline`] when unset.
    pub const fn profiles(mut self, profiles: ProfileRegistry) -> Self {
        self.profiles = Some(profiles);
        self
    }

    /// Sets the executor identity recorded on every emitted event (required).
    pub fn executor(mut self, executor: SubjectId) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Injects the wall clock used for expiry checks and `created_at`.
    ///
    /// Accepts any `Fn() -> DateTime<Utc> + Send + Sync + 'static` (a plain
    /// function pointer or closure), which the builder boxes internally.
    pub fn clock(mut self, clock: impl Fn() -> DateTime<Utc> + Send + Sync + 'static) -> Self {
        self.now = Some(Box::new(clock));
        self
    }

    /// Injects the event-id generator (required).
    ///
    /// The generator must return a valid `evt_`-prefixed id; the [`EventId`]
    /// newtype enforces that at construction. Accepts any
    /// `Fn() -> EventId + Send + Sync + 'static`, boxed internally.
    pub fn event_id_gen(
        mut self,
        event_id_gen: impl Fn() -> EventId + Send + Sync + 'static,
    ) -> Self {
        self.event_id_fn = Some(Box::new(event_id_gen));
        self
    }

    /// Injects the intent-signature verifier (required).
    ///
    /// The executor never assumes authenticity (protocol §18.1 step 4): it
    /// resolves the block's `key_id` to a public key and verifies the signature
    /// over the canonical body bytes, returning
    /// [`ExecutorError::ActorAuthenticationFailed`] on failure.
    pub fn intent_verifier(mut self, intent_verifier: IntentVerifier) -> Self {
        self.intent_verifier = Some(intent_verifier);
        self
    }

    /// Assembles the [`Executor`].
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorBuildError`] naming the first missing required
    /// component.
    pub fn build(self) -> Result<Executor, ExecutorBuildError> {
        let ports = self.ports.ok_or(ExecutorBuildError::MissingPorts)?;
        let profiles = self.profiles.unwrap_or_else(ProfileRegistry::baseline);
        let executor = self.executor.ok_or(ExecutorBuildError::MissingExecutor)?;
        let now = self.now.ok_or(ExecutorBuildError::MissingClock)?;
        let event_id_fn = self
            .event_id_fn
            .ok_or(ExecutorBuildError::MissingEventIdGen)?;
        let intent_verifier = self
            .intent_verifier
            .ok_or(ExecutorBuildError::MissingIntentVerifier)?;
        Ok(Executor {
            ports,
            preparation: PreparationConfig {
                profiles,
                executor,
                now,
                event_id_fn,
                intent_verifier,
            },
        })
    }
}

/// Maps an intent-store port failure onto an executor error, distinguishing a
/// duplicate claim from an unavailable store.
fn map_intent_store_error(error: IntentStoreError, intent_id: &str) -> ExecutorError {
    match error {
        IntentStoreError::Duplicate => ExecutorError::DuplicateIntent {
            intent_id: String::from(intent_id),
        },
        IntentStoreError::Unavailable(message) => ExecutorError::Store(message),
    }
}

/// Maps a TrustGrant port failure onto an executor error.
fn map_trustgrant_error(error: TrustGrantError) -> ExecutorError {
    match error {
        TrustGrantError::Denied => ExecutorError::AuthorityDenied,
        TrustGrantError::Stale => ExecutorError::AuthorityStale,
        TrustGrantError::Unavailable(message) => ExecutorError::AuthorityUnavailable(message),
    }
}

/// Maps a transaction-manager port failure onto an executor error.
fn map_transaction_manager_error(error: &TransactionManagerError) -> ExecutorError {
    ExecutorError::Store(error.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn subject_for_covers_subject_held_types() {
        let intent = sample_intent();
        assert!(subject_for(&intent, StateType::ConsumableStack).is_some());
        assert!(subject_for(&intent, StateType::FungibleBalance).is_some());
        assert!(subject_for(&intent, StateType::Entitlement).is_some());
        assert!(subject_for(&intent, StateType::MeteredResource).is_some());
        assert!(subject_for(&intent, StateType::UniqueAsset).is_none());
        assert!(subject_for(&intent, StateType::Listing).is_none());
        assert!(subject_for(&intent, StateType::Escrow).is_none());
    }

    #[test]
    fn intent_store_duplicate_maps_to_duplicate_intent() {
        let error = map_intent_store_error(
            IntentStoreError::Duplicate,
            "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2",
        );
        assert!(matches!(
            error,
            ExecutorError::DuplicateIntent { intent_id }
            if intent_id == "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2"
        ));
    }

    #[test]
    fn intent_store_unavailable_maps_to_store() {
        let error = map_intent_store_error(
            IntentStoreError::Unavailable(String::from("db down")),
            "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2",
        );
        assert!(matches!(error, ExecutorError::Store(message) if message == "db down"));
    }

    #[test]
    fn trustgrant_errors_map_fail_closed() {
        assert!(matches!(
            map_trustgrant_error(TrustGrantError::Denied),
            ExecutorError::AuthorityDenied
        ));
        assert!(matches!(
            map_trustgrant_error(TrustGrantError::Stale),
            ExecutorError::AuthorityStale
        ));
        assert!(matches!(
            map_trustgrant_error(TrustGrantError::Unavailable(String::from("down"))),
            ExecutorError::AuthorityUnavailable(message) if message == "down"
        ));
    }

    #[test]
    fn authority_proof_carries_outcome_evaluated_at() {
        use statechronicle_core::digest::hash_bytes;

        let evaluated_at = DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let outcome = TrustGrantOutcome {
            evaluation_digest: hash_bytes(b"evaluation"),
            result: EvaluationResult::Allow,
            evaluated_at,
        };
        // Mirror the §18.1 step 11 wiring so any future drift in the mapping
        // is caught here.
        let proof = AuthorityProof {
            kind: String::from(TRUSTGRANT_EVALUATION_KIND),
            evaluation_digest: outcome.evaluation_digest,
            result: EvaluationResult::Allow,
            evaluated_at: outcome.evaluated_at,
        };
        assert_eq!(proof.evaluated_at, evaluated_at);
    }

    fn sample_intent() -> Intent {
        use statechronicle_domain::ids::IntentId;
        use statechronicle_domain::intent::{Nonce, Operation};
        use statechronicle_domain::subject::SubjectId;

        Intent::new(
            TenantId(String::from("acme.game.alpha")),
            IntentId::new(String::from("int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2")).unwrap(),
            Operation::new(String::from("asset.transfer")).unwrap(),
            SubjectId(String::from("account:example:player_123")),
            ResourceId(String::from("asset:sword_001")),
            Some(StateType::UniqueAsset),
            41,
            BTreeMap::new(),
            None,
            DateTime::parse_from_rfc3339("2026-07-14T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            None,
            Nonce::from_bytes(vec![1]).unwrap(),
        )
    }

    fn held_projection(state_type: StateType, subject: &str) -> StateProjection {
        use statechronicle_core::digest::ContentDigest;
        use statechronicle_domain::ids::{CommitId, EventId};

        StateProjection {
            tenant_id: TenantId(String::from("acme.game.alpha")),
            resource_id: ResourceId(String::from("balance:gold")),
            state_type,
            version: 3,
            last_event_id: EventId::new(String::from("evt_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            last_commit_id: CommitId::new(String::from("cmt_01JZ8X5HN3C4PXG5A9FGEWQF5W")).unwrap(),
            state_hash: ContentDigest::new([0u8; 32]),
            state: ResourceState::from_legacy_json(
                state_type,
                serde_json::json!({
                    "subject": subject,
                    "balance": "100",
                    "unit": "gold_minor",
                }),
            )
            .unwrap(),
        }
    }

    #[test]
    fn holder_for_prefers_projection_subject_over_actor() {
        let intent = sample_intent();
        // The projection's subject is authoritative once a resource exists,
        // even when it differs from the acting actor.
        let current = held_projection(StateType::FungibleBalance, "account:example:player_456");
        let holder = holder_for(Some(&current), &intent, StateType::FungibleBalance).unwrap();
        assert_eq!(holder.0, "account:example:player_456");
    }

    #[test]
    fn holder_for_falls_back_to_actor_on_create() {
        let intent = sample_intent();
        let holder = holder_for(None, &intent, StateType::FungibleBalance).unwrap();
        assert_eq!(holder, intent.actor);
    }

    #[test]
    fn holder_for_is_none_for_owner_based_types() {
        let intent = sample_intent();
        let current = held_projection(StateType::FungibleBalance, "account:example:player_123");
        assert!(holder_for(Some(&current), &intent, StateType::UniqueAsset).is_none());
        assert!(holder_for(None, &intent, StateType::Listing).is_none());
    }

    #[test]
    fn is_transfer_operation_detects_transfers_only() {
        assert!(is_transfer_operation(
            &Operation::new(String::from("stack.transfer")).unwrap()
        ));
        assert!(is_transfer_operation(
            &Operation::new(String::from("balance.transfer")).unwrap()
        ));
        assert!(!is_transfer_operation(
            &Operation::new(String::from("stack.debit")).unwrap()
        ));
        assert!(!is_transfer_operation(
            &Operation::new(String::from("asset.transfer")).unwrap()
        ));
    }

    #[test]
    fn destination_holder_reads_to_subject() {
        let inputs = BTreeMap::from([(String::from("to_subject"), serde_json::json!("bob"))]);
        assert_eq!(destination_holder(&inputs).unwrap().0, "bob");
        assert!(destination_holder(&BTreeMap::new()).is_err());
    }
}

#[path = "evaluation.rs"]
mod evaluation;

impl Executor {
    fn evaluation(&self) -> evaluation::Evaluation<'_> {
        evaluation::Evaluation {
            state_index: self.ports.state_index.as_ref(),
            tenant_store: self.ports.tenant_store.as_ref(),
            trustgrant: &self.ports.trustgrant,
            profiles: &self.preparation.profiles,
            executor: &self.preparation.executor,
            now: self.preparation.now.as_ref(),
            event_id_fn: self.preparation.event_id_fn.as_ref(),
            intent_verifier: &self.preparation.intent_verifier,
        }
    }
}

/// Required domain policy, identity and callbacks for protected preparation.
///
/// Callbacks must not perform durable writes or external effect dispatch.
/// The verifier must resolve trusted keys and verify canonical intent bytes;
/// key metadata is independently checked by each player preparation call.
pub struct PreparationConfig {
    /// Registered domain semantics and execution policies.
    pub profiles: ProfileRegistry,
    /// The executor identity recorded on every emitted event.
    pub executor: SubjectId,
    /// The injected wall clock used for expiry checks and `created_at`.
    pub now: Box<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    /// The injected event-id generator. The executor never invents randomness:
    /// event ids are supplied by the composition root (a counter, a ULID
    /// service, or a sharded sequence). The function must return a valid
    /// `evt_`-prefixed id (the `EventId` newtype enforces this at construction).
    pub event_id_fn: Box<dyn Fn() -> EventId + Send + Sync>,
    /// Verifies a present detached intent signature against the BCS canonical
    /// bytes of the intent body (protocol §18.1 step 4, ADR-004 §5). The
    /// composition root wires `key_id` → public-key resolution into this
    /// closure.
    pub intent_verifier: IntentVerifier,
}

/// Read capabilities needed to prepare an authenticated mutation.
///
/// Implementations and injected callbacks must obey the no-persistence/no-dispatch
/// contract of [`Preparer::prepare_player`]. A read can become stale before commit.
/// This bundle does not require an intent store or symbolic transaction manager.
pub struct PreparationPorts {
    /// Current derived resource and subject projections.
    pub state_index: Box<dyn StateIndex + Send + Sync>,
    /// Tenant existence within the deployment.
    pub tenant_store: Box<dyn TenantStore + Send + Sync>,
    /// Delegated-authority evaluators, aggregated by the owning profile policy.
    pub trustgrant: Vec<Box<dyn TrustGrantPort + Send + Sync>>,
}

/// Authenticated preparation using only read capabilities.
///
/// Produces uncommitted plans, never replay receipts. Shares all ingress,
/// transition and event construction with [`Executor`]'s protected preparation.
/// No legacy execution, intent marker or transaction-manager API is exposed.
/// Commit through an authoritative ledger and recover by original identity.
///
/// ```compile_fail
/// # fn check(preparer: statechronicle_executor::preparation::Preparer) {
/// preparer.execute(todo!());
/// # }
/// ```
pub struct Preparer {
    ports: PreparationPorts,
    config: PreparationConfig,
}

impl Preparer {
    /// Compose required read ports and explicit domain/security callbacks.
    pub const fn new(ports: PreparationPorts, config: PreparationConfig) -> Self {
        Self { ports, config }
    }

    fn evaluation(&self) -> evaluation::Evaluation<'_> {
        evaluation::Evaluation {
            state_index: self.ports.state_index.as_ref(),
            tenant_store: self.ports.tenant_store.as_ref(),
            trustgrant: &self.ports.trustgrant,
            profiles: &self.config.profiles,
            executor: &self.config.executor,
            now: self.config.now.as_ref(),
            event_id_fn: self.config.event_id_fn.as_ref(),
            intent_verifier: &self.config.intent_verifier,
        }
    }

    /// Prepare a signed request with key policy, principal binding and authorization.
    ///
    /// Uses the exact gates and callback order of [`Executor::prepare_player`].
    /// Reads/callbacks must not write durable state or dispatch external effects.
    /// Clock and local event-id counters may advance. Success grants no durable
    /// authority or state freshness; commit must enforce current authorization,
    /// concurrency, canonical head and complete atomic effects.
    ///
    /// # Errors
    /// Returns ingress, key, policy, state or profile refusal with no owned plan.
    pub async fn prepare_player(
        &self,
        validated: &ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
    ) -> Result<PreparedMutation, ExecutorError> {
        self.evaluation()
            .prepare_player(validated, principal, authorizer, key_registry)
            .await
    }

    /// Prepare one complete authenticated single-tenant batch without persistence.
    ///
    /// Uses [`Executor::prepare_player_batch`]'s ingress, identity and group gates.
    /// No successful subset escapes on refusal. Reads and callbacks obey the same
    /// effect restrictions as [`Self::prepare_player`]; this supplies no atomic commit.
    ///
    /// # Errors
    /// Refuses invalid batch identity/size, ingress, transition or group invariants.
    pub async fn prepare_player_batch(
        &self,
        items: &[PlayerBatchItem<'_>],
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
    ) -> Result<PreparedBatch, ExecutorError> {
        self.evaluation()
            .prepare_player_batch(items, authorizer, key_registry)
            .await
    }

    /// Prepare one authenticated single-tenant settlement as an owned batch.
    ///
    /// Every member passes the same ingress and preparation gates as
    /// [`Self::prepare_player_batch`], with coordinated baseline resource routes
    /// enabled. The complete event set must additionally pass the existing
    /// settlement validator: declared value legs and bundle membership cannot
    /// escape as a successful subset. No claim, transaction or sink is invoked.
    ///
    /// Bind the result with `SignedBatch` in the public facade and commit it
    /// through one supported ledger transaction. Preparation alone supplies no
    /// durable authority, freshness guarantee or cross-tenant atomicity.
    /// Reads/callbacks obey [`Self::prepare_player`]'s effect restrictions.
    /// # Errors
    /// Refuses ingress, scope, profile, transfer, value-leg or bundle failures.
    pub async fn prepare_player_settlement(
        &self,
        items: &[PlayerBatchItem<'_>],
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
    ) -> Result<PreparedBatch, ExecutorError> {
        self.evaluation()
            .prepare_player_settlement(items, authorizer, key_registry)
            .await
    }
}
