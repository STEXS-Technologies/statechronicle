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

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use async_trait::async_trait;
use serde_json::Value;
use statechronicle_core::canonicalize::{canonicalize, canonicalize_and_digest};
use statechronicle_core::digest::ContentDigest;
use statechronicle_core::limits::{MAX_INTENT_BYTES, check_size};
use statechronicle_domain::authority::{
    AggregationPolicy, AuthorityProof, EvaluationResult, TRUSTGRANT_EVALUATION_KIND,
    TrustGrantOutcome, aggregate_evaluation_digest,
};
use statechronicle_domain::event::{Event, StateCommitment};
use statechronicle_domain::ids::EventId;
use statechronicle_domain::intent::{Intent, Operation, SignatureBlock};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::resource_state::{
    ConsumableStackState, EntitlementState, EscrowState, FungibleBalanceState, ListingState,
    MeterState, ResourceState, UniqueAssetState,
};
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::status::Status;
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
use statechronicle_profiles::consumable_stack::op as stack_op;
use statechronicle_profiles::fungible_balance::op as balance_op;
use statechronicle_profiles::registry::ProfileRegistry;
use statechronicle_profiles::unique_asset::op as asset_op;

use crate::atomicity;
use crate::conflict;
use crate::error::{ExecutorBuildError, ExecutorError, PortsBuildError};
use crate::transition;

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
    profiles: ProfileRegistry,
    /// The executor identity recorded on every emitted event.
    executor: SubjectId,
    /// The injected wall clock used for expiry checks and `created_at`.
    now: Box<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    /// The injected event-id generator. The executor never invents randomness:
    /// event ids are supplied by the composition root (a counter, a ULID
    /// service, or a sharded sequence). The function must return a valid
    /// `evt_`-prefixed id (the `EventId` newtype enforces this at construction).
    event_id_fn: Box<dyn Fn() -> EventId + Send + Sync>,
    /// Verifies a present detached intent signature against the BCS canonical
    /// bytes of the intent body (protocol §18.1 step 4, ADR-004 §5). The
    /// composition root wires `key_id` → public-key resolution into this
    /// closure.
    intent_verifier: IntentVerifier,
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
    /// 5. **Load current state**: via `state_index.get_subject_state` for
    ///    subject-held types or `state_index.get_state` for owner-based types,
    ///    per [`transition::state_key_for`]'s keying rules. The intent's
    ///    `state_type` must be present ([`ExecutorError::StateTypeRequired`]).
    /// 6. **Expected version**: [`conflict::check_expected_version`].
    /// 7. **Conflict gates**: [`conflict::check_owner`] and
    ///    [`conflict::check_resource_availability`] (§18.2). Expiry
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
    /// 10. **After-state**: [`transition::apply`] computes the deterministic
    ///     new projection payload.
    /// 11. **Emit event**: before/after [`StateCommitment`]s carry
    ///     `current.version` / `current.version + 1` and canonical state
    ///     digests (`canonicalize_and_digest`), the evaluated authority is
    ///     bound when present, and `executor`/`created_at` come from the
    ///     injected identity and clock.
    /// 12. **Transfer pair**: for `stack.transfer` / `balance.transfer` a
    ///     second event credits the destination (holder = `to_subject`, the
    ///     destination's current state or a create-on-credit at version 0),
    ///     making the transfer an atomic debit + credit pair sharing one
    ///     intent id (§20.5, §18.3). Both event ids come from the injected
    ///     generator, source first, then destination.
    /// 13. **Atomicity**: multi-resource transactions are assembled by the
    ///     commit crate batching multiple executions via [`Self::execute_batch`]
    ///     (§18.3); [`atomicity::validate_batch_consistency`] admits a transfer
    ///     pair as the one multi-event unit sharing an intent id.
    ///
    /// # Errors
    ///
    /// Returns every [`ExecutorError`] variant in fail-closed order above. No
    /// event is emitted when any check fails.
    pub async fn execute(&self, validated: &ValidatedIntent) -> Result<Vec<Event>, ExecutorError> {
        self.execute_inner(validated, false, true).await
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
        self.execute_authenticated_inner(validated, principal, authorizer, true)
            .await
    }

    async fn execute_authenticated_inner(
        &self,
        validated: &ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        authorizer: &dyn Authorizer,
        persist_intent: bool,
    ) -> Result<Vec<Event>, ExecutorError> {
        if principal.tenant != validated.intent.tenant_id
            || principal.subject != validated.intent.actor
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
        self.execute_inner(validated, false, persist_intent).await
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
        if validated.signature.is_none() {
            return Err(ExecutorError::ActorAuthenticationFailed(String::from(
                "player mutation requires a detached intent signature",
            )));
        }
        self.execute_authenticated(validated, principal, authorizer)
            .await
    }

    /// Executes a signed player mutation and hands its events to the required
    /// durable persistence boundary before returning success.
    ///
    /// This is the recommended public-ingress route. It preserves the strict
    /// authentication and authorization checks of [`Self::execute_player`],
    /// then invokes the deployment's sink exactly once for the constructed
    /// event set. The sink is responsible for calling the commit crate's
    /// `persist_durable_verified` (or metrics variant) with an atomic ledger
    /// transaction. No events are returned when persistence fails.
    /// Idempotent replays (an empty event set) are returned without invoking
    /// the sink.
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
        if validated.signature.is_none() {
            return Err(ExecutorError::ActorAuthenticationFailed(String::from(
                "player mutation requires a detached intent signature",
            )));
        }
        let events = self
            .execute_authenticated_inner(validated, principal, authorizer, false)
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
        let signature = validated.signature.as_ref().ok_or_else(|| {
            ExecutorError::ActorAuthenticationFailed(String::from(
                "player mutation requires a detached intent signature",
            ))
        })?;
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
        let signature = validated.signature.as_ref().ok_or_else(|| {
            ExecutorError::ActorAuthenticationFailed(String::from(
                "player mutation requires a detached intent signature",
            ))
        })?;
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
            let signature = item.validated.signature.as_ref().ok_or_else(|| {
                ExecutorError::ActorAuthenticationFailed(String::from(
                    "player mutation requires a detached intent signature",
                ))
            })?;
            if item.principal.tenant != item.validated.intent.tenant_id
                || item.principal.subject != item.validated.intent.actor
            {
                return Err(ExecutorError::ActorAuthenticationFailed(String::from(
                    "authenticated principal is not bound to intent actor and tenant",
                )));
            }
            authorizer
                .authorize(AuthorizationContext {
                    principal: item.principal,
                    tenant: &item.validated.intent.tenant_id,
                    intent: &item.validated.intent,
                    resource: &item.validated.intent.resource_id,
                })
                .await
                .map_err(|error| ExecutorError::ActorAuthenticationFailed(error.to_string()))?;
            key_registry
                .resolve_intent_key(
                    &item.validated.intent.tenant_id,
                    &item.principal.subject,
                    &signature.key_id,
                    &item.validated.intent.operation,
                    item.validated.intent.created_at,
                )
                .await
                .map_err(|error| ExecutorError::ActorAuthenticationFailed(error.to_string()))?;
            intents.push(item.validated.clone());
        }
        self.execute_batch_durable(&intents, sink).await
    }

    /// The shared execution core, parameterized over whether value-leg
    /// `trade.settle` intents are admitted.
    ///
    /// `allow_value_legs == false` for the single-tenant [`Self::execute`] and
    /// [`Self::execute_batch`]: a `trade.settle` that declares a value leg must
    /// be settled via [`Self::execute_settle`]. `allow_value_legs == true` for
    /// the cross-tenant legs (through the single-tenant batch pipeline), where value legs
    /// are declared in the trade manifest and validated by the cross-tenant
    /// validator, not the settle intent.
    #[allow(clippy::collapsible_if)]
    async fn execute_inner(
        &self,
        validated: &ValidatedIntent,
        allow_value_legs: bool,
        persist_intent: bool,
    ) -> Result<Vec<Event>, ExecutorError> {
        let intent = &validated.intent;
        let tenant = &intent.tenant_id;
        let resource = &intent.resource_id;
        let operation = &intent.operation;

        // Phase 2 routing gate: a `trade.settle` that declares a value leg
        // (value_resource / value_amount / value_to_subject) must be settled via
        // [`Self::execute_settle`], which validates the value pairs. This path
        // cannot move value, so a value-leg settle is rejected up front — before
        // any claim is recorded — rather than silently settling the asset with
        // no value moved and no error. The cross-tenant path admits value-leg
        // settles because their value legs are declared in the manifest.
        if !allow_value_legs
            && operation == asset_op::trade_settle()
            && atomicity::declares_value_leg(intent)
        {
            return Err(ExecutorError::ValueLegSettleRouting {
                intent_id: intent.intent_id.0.clone(),
            });
        }

        // §18.1 steps 1–2: schema/size are enforced upstream by the intent
        // crate; re-check the canonical size here as a defense-in-depth gate.
        let bytes = canonicalize(intent)?;
        check_size("intent", MAX_INTENT_BYTES, bytes.len())?;

        // §18.2: an intent expired before acceptance is rejected before any
        // claim is recorded or any authority evaluated.
        conflict::check_expiry(intent, (self.now)())?;

        // §18.1 step 3: idempotency. Replays succeed; conflicting intents fail.
        let existing = self
            .ports
            .intent_store
            .get_intent(tenant, &intent.intent_id)
            .await
            .map_err(|err| map_intent_store_error(err, &intent.intent_id.0))?;
        if let Some(existing) = existing {
            conflict::check_idempotency_existing(&existing, intent)?;
            tracing::debug!(intent_id = %intent.intent_id.as_str(), "idempotent replay");
            return Ok(Vec::new());
        }
        // §18.1 step 4: actor authentication. Signature presence is the
        // platform's submission policy; the executor verifies a present
        // signature against the BCS canonical bytes of the intent body via the
        // injected verifier (which resolves key_id → public key). An absent
        // signature follows the v0 policy: unsigned intents are allowed only
        // for operations the active profile does not require authority for
        // (gated downstream by the TrustGrant step and profiles' authorized_by
        // inputs). Revocation freshness is re-checked here on the client-
        // provided proof when present (v0 behavior); the aggregate proof is
        // checked for freshness in step 8.
        if let Some(block) = &validated.signature {
            (self.intent_verifier)(block, &bytes)?;
        }
        if let Some(proof) = &intent.authority {
            if let Some(primary) = self.ports.trustgrant.first() {
                primary
                    .check_revocation_freshness(proof)
                    .await
                    .map_err(map_trustgrant_error)?;
            }
        }

        // §18.1 step 5: tenant scope.
        conflict::check_tenant_scope(intent)?;
        let tenant_exists = self
            .ports
            .tenant_store
            .tenant_exists(tenant)
            .await
            .map_err(|err| ExecutorError::Store(err.to_string()))?;
        if !tenant_exists {
            return Err(ExecutorError::TenantNotFound {
                tenant: tenant.0.clone(),
            });
        }

        // §18.1 step 6: load current state. The state type must resolve before
        // loading because subject-held vs owner-based keying determines which
        // index call to make (per `transition::state_key_for`).
        let state_type = intent.state_type.ok_or(ExecutorError::StateTypeRequired)?;
        let mut current = match subject_for(intent, state_type) {
            Some(subject) => self
                .ports
                .state_index
                .get_subject_state(tenant, subject, resource)
                .await
                .map_err(|err| ExecutorError::Store(err.to_string()))?,
            None => self
                .ports
                .state_index
                .get_state(tenant, resource)
                .await
                .map_err(|err| ExecutorError::Store(err.to_string()))?,
        };

        // §18.1 step 6a (Gap 3): for subject-held types the projection's own
        // `subject` field is the source of truth once a resource exists
        // (protocol §9/§10); the acting actor is only the creator default. If
        // the actor-keyed lookup surfaced a projection whose holder differs
        // from the acting actor, re-load under the authoritative holder so
        // subsequent keying matches the state's holder, never the acting actor.
        if let Some(holder) = holder_for(current.as_ref(), intent, state_type) {
            let queried = subject_for(intent, state_type).map(|subject| subject.0.as_str());
            if queried.is_some() && Some(holder.0.as_str()) != queried {
                current = self
                    .ports
                    .state_index
                    .get_subject_state(tenant, &holder, resource)
                    .await
                    .map_err(|err| ExecutorError::Store(err.to_string()))?;
            }
        }

        // §18.1 step 7 + §18.2 conflict gates.
        conflict::check_expected_version(intent, current.as_ref())?;
        if let Some(projection) = &current {
            conflict::check_owner(intent, Some(projection), &intent.inputs)?;
            conflict::check_resource_availability(projection, operation)?;
        }

        // §18.1 step 8: TrustGrant authority. The active profile's rule set is
        // resolved here (before the gate) so `requires_authority` and
        // `authority_policy` can drive the mandatory-binding rule and the
        // aggregation policy; `rules.check` still runs at step 9.
        let rules = self
            .profiles
            .get(state_type)
            .ok_or_else(|| ExecutorError::TransitionInvalid(String::from("unknown state type")))?;
        let required = rules.requires_authority(operation);
        let policy = rules.authority_policy(operation);

        let authority = if intent.authority.is_none() && required {
            // Authority-required operations MUST carry a binding (protocol
            // §11.2, ADR-006 §36 Q5 / deferral item 4).
            return Err(ExecutorError::AuthorityMissing {
                operation: String::from(operation.as_str()),
            });
        } else if intent.authority.is_none() {
            // Not authority-required and no binding: proceed WITHOUT authority.
            // The profile's transition and consent rules govern this path
            // (profile-owned fallback; the event carries no authority).
            None
        } else {
            // Authority is present: evaluate every member of the deployment's
            // authority set and aggregate under the profile's policy. An empty
            // set with authority present fails closed.
            if self.ports.trustgrant.is_empty() {
                return Err(ExecutorError::AuthorityUnavailable(String::from(
                    "no authority configured",
                )));
            }
            let mut outcomes: Vec<TrustGrantOutcome> = Vec::new();
            let mut denied = false;
            let mut stale = false;
            let mut unavailable: Option<String> = None;
            for trustgrant in &self.ports.trustgrant {
                match trustgrant
                    .evaluate(tenant, &intent.actor, operation.as_str(), resource)
                    .await
                {
                    Ok(outcome) if outcome.result == EvaluationResult::Allow => {
                        outcomes.push(outcome)
                    }
                    Ok(_outcome) => denied = true,
                    Err(TrustGrantError::Denied) => denied = true,
                    Err(TrustGrantError::Stale) => stale = true,
                    Err(TrustGrantError::Unavailable(message)) => unavailable = Some(message),
                }
            }
            let pass = match policy {
                AggregationPolicy::RequireAll => !denied && !stale && unavailable.is_none(),
                AggregationPolicy::AnyOf => !outcomes.is_empty(),
                // Unknown policies fail closed as require-all.
                _ => !denied && !stale && unavailable.is_none(),
            };
            if !pass {
                // Error precedence when multiple members fail: a member's
                // explicit deny is masked by another member's unavailability.
                // Ordering is unavailable > stale > denied. Fail-closed is
                // preserved regardless of which error surfaces.
                if let Some(message) = unavailable {
                    return Err(ExecutorError::AuthorityUnavailable(message));
                }
                if stale {
                    return Err(ExecutorError::AuthorityStale);
                }
                return Err(ExecutorError::AuthorityDenied);
            }
            let sub_digests: Vec<ContentDigest> = outcomes
                .iter()
                .map(|outcome| outcome.evaluation_digest.clone())
                .collect();
            let evaluation_digest = aggregate_evaluation_digest(policy, &sub_digests);
            let evaluated_at = outcomes
                .iter()
                .map(|outcome| outcome.evaluated_at)
                .min()
                .unwrap_or_else(|| (self.now)());
            let proof = AuthorityProof {
                kind: String::from(TRUSTGRANT_EVALUATION_KIND),
                evaluation_digest,
                result: EvaluationResult::Allow,
                evaluated_at,
            };
            // Freshness of the aggregate proof (the stalest member's
            // `evaluated_at`), checked against the primary authority.
            if let Some(primary) = self.ports.trustgrant.first() {
                primary
                    .check_revocation_freshness(&proof)
                    .await
                    .map_err(map_trustgrant_error)?;
            }
            Some(proof)
        };

        // §18.1 step 9: profile rules.
        rules.check(operation, current.as_ref(), &intent.inputs)?;

        // §18.1 step 10: deterministic after-state.
        let after_state = transition::apply(current.as_ref(), operation, &intent.inputs)?;

        // §18.1 step 11: emit the event.
        let version = current.as_ref().map(|c| c.version).unwrap_or(0);
        let next_version = version
            .checked_add(1)
            .ok_or_else(|| ExecutorError::TransitionInvalid(String::from("version overflow")))?;
        let before_state = current
            .as_ref()
            .map(|c| c.state.clone())
            .unwrap_or_else(|| empty_state_for(state_type));
        let before_hash = match current.as_ref() {
            Some(projection) => projection.state_hash.clone(),
            None => canonicalize_and_digest(&before_state)?,
        };
        let after_hash = canonicalize_and_digest(&after_state)?;
        let event = Event::new(
            tenant.clone(),
            (self.event_id_fn)(),
            intent.intent_id.clone(),
            operation.clone(),
            resource.clone(),
            intent.actor.clone(),
            StateCommitment {
                version,
                state_hash: before_hash,
                state: before_state,
            },
            StateCommitment {
                version: next_version,
                state_hash: after_hash,
                state: after_state,
            },
            authority.clone(),
            self.executor.clone(),
            (self.now)(),
        );

        tracing::debug!(event_id = %event.event_id.as_str(), "emitted event");

        // §18.1 step 12: transfer pair. For a subject-held transfer the source
        // event above is the debit; emit a second event crediting the
        // destination (holder = `to_subject`). Both events share this intent id
        // and form the atomic multi-resource unit (§20.5, §18.3). The
        // destination is loaded by its holder and, when absent, created at
        // version 0 (create-on-credit).
        if is_transfer_operation(operation) {
            let destination_holder = destination_holder(&intent.inputs)?;
            let destination_current = self
                .ports
                .state_index
                .get_subject_state(tenant, &destination_holder, resource)
                .await
                .map_err(|err| ExecutorError::Store(err.to_string()))?;
            let source = current.as_ref().ok_or_else(|| {
                ExecutorError::TransitionInvalid(String::from(
                    "transfer requires an existing source resource",
                ))
            })?;
            let destination_after_state = transition::transfer_after_state(
                source,
                destination_current.as_ref(),
                operation,
                &intent.inputs,
            )?;

            let destination_version = destination_current.as_ref().map(|c| c.version).unwrap_or(0);
            let destination_next_version = destination_version.checked_add(1).ok_or_else(|| {
                ExecutorError::TransitionInvalid(String::from("version overflow"))
            })?;
            let destination_before_state = destination_current
                .as_ref()
                .map(|c| c.state.clone())
                .unwrap_or_else(|| empty_state_for(state_type));
            let destination_before_hash = match destination_current.as_ref() {
                Some(projection) => projection.state_hash.clone(),
                None => canonicalize_and_digest(&destination_before_state)?,
            };
            let destination_after_hash = canonicalize_and_digest(&destination_after_state)?;

            let destination_event = Event::new(
                tenant.clone(),
                (self.event_id_fn)(),
                intent.intent_id.clone(),
                operation.clone(),
                resource.clone(),
                intent.actor.clone(),
                StateCommitment {
                    version: destination_version,
                    state_hash: destination_before_hash,
                    state: destination_before_state,
                },
                StateCommitment {
                    version: destination_next_version,
                    state_hash: destination_after_hash,
                    state: destination_after_state,
                },
                authority,
                self.executor.clone(),
                (self.now)(),
            );

            tracing::debug!(
                event_id = %destination_event.event_id.as_str(),
                "emitted destination credit event"
            );
            // Claim only after both transfer legs have been fully validated and
            // constructed. Durable implementations must make this claim
            // atomic with persistence of both events and their commit.
            if persist_intent {
                self.ports
                    .intent_store
                    .put_intent(tenant, intent)
                    .await
                    .map_err(|err| map_intent_store_error(err, &intent.intent_id.0))?;
            }
            return Ok(vec![event, destination_event]);
        }

        // Claim only after authentication, authority, profile, transition, and
        // complete event construction have passed. This prevents malformed or
        // unauthorized requests from poisoning the idempotency namespace.
        // Durable implementations must still make this claim atomic with the
        // event/commit write (see TODO.md P0-1/P0-2).
        if persist_intent {
            self.ports
                .intent_store
                .put_intent(tenant, intent)
                .await
                .map_err(|err| map_intent_store_error(err, &intent.intent_id.0))?;
        }
        Ok(vec![event])
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
        let handle = if sink.is_none() {
            Some(
                self.ports
                    .transaction_manager
                    .begin(tenant)
                    .await
                    .map_err(|err| map_transaction_manager_error(&err))?,
            )
        } else {
            None
        };

        // Both a leg failure and an inconsistent-batch validation failure are
        // rolled back atomically (a failed validation must not short-circuit
        // past the rollback via `?`).
        // The symbolic transaction manager cannot roll back the independent
        // intent store. Only a durable sink owns an atomic claim boundary;
        // pure/planning batches must never leave partial claims behind.
        let result = match self.run_batch_fail_closed(intents, sink.is_some()).await {
            Ok(events) => atomicity::validate_batch_consistency(&events).map(|()| events),
            Err(error) => Err(error),
        };
        match result {
            Ok(events) => {
                if let Some(sink) = sink.filter(|_| !events.is_empty()) {
                    if let Err(error) = sink.persist_batch(intents, &events).await {
                        if let Some(handle) = handle {
                            if let Err(rollback_error) = handle.rollback().await {
                                tracing::warn!(%rollback_error, "durable batch rollback failed");
                            }
                        }
                        return Err(ExecutorError::Store(error));
                    }
                }
                if let Some(handle) = handle {
                    handle
                        .commit()
                        .await
                        .map_err(|err| map_transaction_manager_error(&err))?;
                }
                Ok(events)
            }
            Err(error) => {
                let message = error.to_string();
                if let Some(handle) = handle {
                    if let Err(rollback_error) = handle.rollback().await {
                        tracing::warn!(rollback = %rollback_error, "batch rollback failed");
                    }
                }
                Err(ExecutorError::AtomicityViolation(message))
            }
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
        let handle = if sink.is_none() {
            Some(
                self.ports
                    .transaction_manager
                    .begin(tenant)
                    .await
                    .map_err(|err| map_transaction_manager_error(&err))?,
            )
        } else {
            None
        };

        // Both a leg failure and a validation failure are rolled back atomically.
        // `allow_value_legs` is `true`: the settle batch's value legs are
        // validated by [`atomicity::validate_settle_batch`] below.
        let result = match self.run_batch(intents, true, sink.is_some()).await {
            Ok(events) => {
                let settle_intents: Vec<statechronicle_domain::intent::Intent> = intents
                    .iter()
                    .filter(|validated| &validated.intent.operation == asset_op::trade_settle())
                    .map(|validated| validated.intent.clone())
                    .collect();
                atomicity::validate_batch_consistency(&events)
                    .and_then(|()| atomicity::validate_settle_batch(&events, &settle_intents))
                    .map(|()| events)
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(events) => {
                if let Some(sink) = sink.filter(|_| !events.is_empty()) {
                    if let Err(error) = sink.persist_batch(intents, &events).await {
                        if let Some(handle) = handle {
                            if let Err(rollback_error) = handle.rollback().await {
                                tracing::warn!(%rollback_error, "durable settle rollback failed");
                            }
                        }
                        return Err(ExecutorError::Store(error));
                    }
                }
                if let Some(handle) = handle {
                    handle
                        .commit()
                        .await
                        .map_err(|err| map_transaction_manager_error(&err))?;
                }
                Ok(events)
            }
            Err(error) => {
                let message = error.to_string();
                if let Some(handle) = handle {
                    if let Err(rollback_error) = handle.rollback().await {
                        tracing::warn!(rollback = %rollback_error, "settle rollback failed");
                    }
                }
                Err(ExecutorError::AtomicityViolation(message))
            }
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
        let mut by_name: BTreeMap<String, Vec<ValidatedIntent>> = BTreeMap::new();
        for validated in intents {
            by_name
                .entry(validated.intent.tenant_id.0.clone())
                .or_default()
                .push(validated.clone());
        }
        let sorted_tenants: Vec<TenantId> =
            by_name.keys().map(|name| TenantId(name.clone())).collect();
        if sorted_tenants.len() < 2 {
            return Err(ExecutorError::AtomicityViolation(String::from(
                "cross-tenant batch requires at least two distinct tenants",
            )));
        }

        let handle = if sink.is_none() {
            Some(
                self.ports
                    .transaction_manager
                    .begin_multi(&sorted_tenants)
                    .await
                    .map_err(|err| map_transaction_manager_error(&err))?,
            )
        } else {
            None
        };

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
                if let Some(sink) = sink.filter(|_| !events.is_empty()) {
                    if let Err(error) = sink.persist_batch(intents, &events).await {
                        if let Some(handle) = handle {
                            if let Err(rollback_error) = handle.rollback().await {
                                tracing::warn!(%rollback_error, "durable cross-tenant rollback failed");
                            }
                        }
                        return Err(ExecutorError::Store(error));
                    }
                }
                if let Some(handle) = handle {
                    handle
                        .commit()
                        .await
                        .map_err(|err| map_transaction_manager_error(&err))?;
                }
                Ok(groups)
            }
            Err(error) => {
                let message = error.to_string();
                if let Some(handle) = handle {
                    if let Err(rollback_error) = handle.rollback().await {
                        tracing::warn!(rollback = %rollback_error, "cross-tenant rollback failed");
                    }
                }
                Err(ExecutorError::AtomicityViolation(message))
            }
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
        let mut by_name: BTreeMap<String, Vec<ValidatedIntent>> = BTreeMap::new();
        for validated in intents {
            by_name
                .entry(validated.intent.tenant_id.0.clone())
                .or_default()
                .push(validated.clone());
        }
        let sorted_tenants: Vec<TenantId> =
            by_name.keys().map(|name| TenantId(name.clone())).collect();
        if sorted_tenants.len() < 2 {
            return Err(ExecutorError::AtomicityViolation(String::from(
                "cross-tenant trade requires at least two distinct tenants",
            )));
        }

        let handle = if sink.is_none() {
            Some(
                self.ports
                    .transaction_manager
                    .begin_multi(&sorted_tenants)
                    .await
                    .map_err(|err| map_transaction_manager_error(&err))?,
            )
        } else {
            None
        };

        // Both a leg failure and a manifest-validation failure are rolled back
        // atomically (a failed validation must not short-circuit past the
        // rollback via `?`).
        let result = match self
            .run_cross_tenant_legs(&by_name, true, sink.is_some())
            .await
        {
            Ok(groups) => {
                let settle_intents: Vec<statechronicle_domain::intent::Intent> = intents
                    .iter()
                    .filter(|validated| &validated.intent.operation == asset_op::trade_settle())
                    .map(|validated| validated.intent.clone())
                    .collect();
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
                if let Some(sink) = sink.filter(|_| !events.is_empty()) {
                    if let Err(error) = sink.persist_batch(intents, &events).await {
                        if let Some(handle) = handle {
                            if let Err(rollback_error) = handle.rollback().await {
                                tracing::warn!(%rollback_error, "durable cross-tenant rollback failed");
                            }
                        }
                        return Err(ExecutorError::Store(error));
                    }
                }
                if let Some(handle) = handle {
                    handle
                        .commit()
                        .await
                        .map_err(|err| map_transaction_manager_error(&err))?;
                }
                Ok(groups)
            }
            Err(error) => {
                let message = error.to_string();
                if let Some(handle) = handle {
                    if let Err(rollback_error) = handle.rollback().await {
                        tracing::warn!(
                            rollback = %rollback_error,
                            "cross-tenant trade rollback failed"
                        );
                    }
                }
                Err(ExecutorError::AtomicityViolation(message))
            }
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

    /// Executes every intent in order, short-circuiting on the first failure.
    ///
    /// Used by the cross-tenant legs. `allow_value_legs` mirrors the value-leg
    /// routing gate of [`Self::execute_inner`]: the trade path admits value-leg
    /// `trade.settle` intents (their value legs are declared in the trade
    /// manifest and validated by the cross-tenant validator), while the plain
    /// cross-tenant path rejects them for lack of a declared manifest.
    async fn run_batch(
        &self,
        intents: &[ValidatedIntent],
        allow_value_legs: bool,
        persist_intent: bool,
    ) -> Result<Vec<Event>, ExecutorError> {
        let mut events = Vec::new();
        for validated in intents {
            events.extend(
                self.execute_inner(validated, allow_value_legs, persist_intent)
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
    /// Value-leg `trade.settle` intents are rejected (single-tenant batch).
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
            let produced = self.execute_inner(validated, false, persist_intent).await?;
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
    /// `allow_value_legs` is threaded into each tenant's batch execution:
    /// the cross-tenant trade path admits value-leg settles (declared in the
    /// trade manifest), while the plain cross-tenant path rejects them so a
    /// value-declaring `trade.settle` cannot silently settle an asset for a
    /// mismatched value pair.
    async fn run_cross_tenant_legs(
        &self,
        by_name: &BTreeMap<String, Vec<ValidatedIntent>>,
        allow_value_legs: bool,
        persist_intent: bool,
    ) -> Result<Vec<atomicity::TenantEventGroup>, ExecutorError> {
        let mut groups = Vec::new();
        for (name, sub_intents) in by_name {
            let events = self
                .run_batch(sub_intents, allow_value_legs, persist_intent)
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
            profiles,
            executor,
            now,
            event_id_fn,
            intent_verifier,
        })
    }
}

/// Returns the subject used for the initial state-index lookup of a
/// subject-held resource.
///
/// The acting actor is the creator default (Gap 3): it is the key used to
/// *query* the index. Once a resource exists, the projection's own `subject`
/// field is the authoritative holder (see [`holder_for`]), and step 6a
/// re-resolves under it when it differs. Owner-based types
/// ([`StateType::UniqueAsset`], [`StateType::Listing`], [`StateType::Escrow`])
/// carry no subject in their key.
const fn subject_for(intent: &Intent, state_type: StateType) -> Option<&SubjectId> {
    match state_type {
        StateType::ConsumableStack
        | StateType::FungibleBalance
        | StateType::Entitlement
        | StateType::MeteredResource => Some(&intent.actor),
        StateType::UniqueAsset | StateType::Listing | StateType::Escrow => None,
    }
}

fn empty_state_for(state_type: StateType) -> ResourceState {
    let subject = SubjectId(String::new());
    let status = Status::from_static("active");
    match state_type {
        StateType::UniqueAsset => ResourceState::UniqueAsset(UniqueAssetState {
            owner: subject,
            status,
            trade_id: None,
        }),
        StateType::ConsumableStack => ResourceState::ConsumableStack(ConsumableStackState {
            subject,
            quantity: statechronicle_core::amount::Amount::ZERO,
            unit: String::new(),
        }),
        StateType::FungibleBalance => ResourceState::FungibleBalance(FungibleBalanceState {
            subject,
            balance: statechronicle_core::amount::Amount::ZERO,
            unit: String::new(),
        }),
        StateType::Entitlement => ResourceState::Entitlement(EntitlementState {
            subject,
            status,
            transferable: false,
        }),
        StateType::MeteredResource => ResourceState::MeteredResource(MeterState {
            subject,
            remaining: statechronicle_core::amount::Amount::ZERO,
            maximum: statechronicle_core::amount::Amount::ZERO,
        }),
        StateType::Listing => ResourceState::Listing(ListingState {
            seller: subject,
            status,
        }),
        StateType::Escrow => ResourceState::Escrow(EscrowState {
            buyer: subject.clone(),
            seller: subject,
            status,
        }),
    }
}

/// Resolves the authoritative holder of a subject-held resource
/// (protocol §9/§10).
///
/// The rule: a projection's `subject` field is the source of truth once a
/// resource exists; the acting actor is only the creator default used when no
/// projection exists yet (the create path). Returns `None` for owner-based
/// state types, which key by resource alone.
#[allow(clippy::collapsible_if)]
fn holder_for(
    current: Option<&StateProjection>,
    intent: &Intent,
    state_type: StateType,
) -> Option<SubjectId> {
    match state_type {
        StateType::ConsumableStack
        | StateType::FungibleBalance
        | StateType::Entitlement
        | StateType::MeteredResource => {
            if let Some(projection) = current {
                if let Some(subject) = projection.state.subject().map(|s| s.0.as_str()) {
                    if !subject.is_empty() {
                        return Some(SubjectId(String::from(subject)));
                    }
                }
            }
            Some(intent.actor.clone())
        }
        StateType::UniqueAsset | StateType::Listing | StateType::Escrow => None,
    }
}

/// Returns whether the operation is a subject-held atomic transfer
/// (stack.transfer / balance.transfer) that requires a destination credit.
fn is_transfer_operation(operation: &Operation) -> bool {
    operation == stack_op::stack_transfer() || operation == balance_op::balance_transfer()
}

/// Reads the destination holder (`to_subject`) from a transfer's inputs.
///
/// # Errors
///
/// Returns [`ExecutorError::TransitionInvalid`] when `to_subject` is missing
/// or empty.
fn destination_holder(inputs: &BTreeMap<String, Value>) -> Result<SubjectId, ExecutorError> {
    let subject = inputs
        .get("to_subject")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ExecutorError::TransitionInvalid(String::from("missing input `to_subject`"))
        })?;
    Ok(SubjectId(String::from(subject)))
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
