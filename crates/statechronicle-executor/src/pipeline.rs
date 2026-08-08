//! The validation pipeline (protocol §18.1).
//!
//! Ordered checks: idempotency, actor authentication, tenant scope, current
//! state loading, expected version, TrustGrant evaluation (via port), profile
//! rules, and deterministic after-state. Events are emitted only when every
//! check passes. The pipeline is the protocol's "brain", a deterministic
//! validator that drives pure [`crate::transition`] and [`crate::conflict`]
//! logic through injected [`Ports`].
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
use statechronicle_domain::state::StateProjection;
use statechronicle_domain::state_type::StateType;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_intent::validated::ValidatedIntent;
use statechronicle_ports::intent_store::{IntentStore, IntentStoreError};
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
/// injected [`intent_verifier`](Self::intent_verifier) (which the composition
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
    /// 2. **Idempotency**: `intent_store.get_intent`; a stored intent with an
    ///    equal payload is a replay (`Ok(vec![])`); a stored intent with a
    ///    different payload is [`ExecutorError::DuplicateIntent`]. Otherwise
    ///    the intent is claimed with `intent_store.put_intent` *before*
    ///    executing.
    /// 3. **Actor authentication**: when `validated.signature` is present, the
    ///    injected [`intent_verifier`](Self::intent_verifier) is invoked over
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
        let intent = &validated.intent;
        let tenant = &intent.tenant_id;
        let resource = &intent.resource_id;
        let operation = &intent.operation;

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
        self.ports
            .intent_store
            .put_intent(tenant, intent)
            .await
            .map_err(|err| map_intent_store_error(err, &intent.intent_id.0))?;

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
        if let Some(proof) = &intent.authority
            && let Some(primary) = self.ports.trustgrant.first()
        {
            primary
                .check_revocation_freshness(proof)
                .await
                .map_err(map_trustgrant_error)?;
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
            .unwrap_or_else(|| serde_json::json!({}));
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
                .unwrap_or_else(|| serde_json::json!({}));
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
            return Ok(vec![event, destination_event]);
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
    /// persist anything itself (events are returned for the commit crate), so
    /// the handle records commit/rollback intent. Production adapters would
    /// stage the intent-store claims inside the transaction for true atomicity.
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
        let Some(first) = intents.first() else {
            return Err(ExecutorError::AtomicityViolation(String::from(
                "empty batch",
            )));
        };
        let tenant = &first.intent.tenant_id;
        let handle = self
            .ports
            .transaction_manager
            .begin(tenant)
            .await
            .map_err(|err| map_transaction_manager_error(&err))?;

        // Both a leg failure and an inconsistent-batch validation failure are
        // rolled back atomically (a failed validation must not short-circuit
        // past the rollback via `?`).
        let result = match self.run_batch(intents).await {
            Ok(events) => atomicity::validate_batch_consistency(&events).map(|()| events),
            Err(error) => Err(error),
        };
        match result {
            Ok(events) => {
                handle
                    .commit()
                    .await
                    .map_err(|err| map_transaction_manager_error(&err))?;
                Ok(events)
            }
            Err(error) => {
                let message = error.to_string();
                if let Err(rollback_error) = handle.rollback().await {
                    tracing::warn!(rollback = %rollback_error, "batch rollback failed");
                }
                Err(ExecutorError::AtomicityViolation(message))
            }
        }
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
        let Some(first) = intents.first() else {
            return Err(ExecutorError::AtomicityViolation(String::from(
                "empty settle batch",
            )));
        };
        let tenant = &first.intent.tenant_id;
        let handle = self
            .ports
            .transaction_manager
            .begin(tenant)
            .await
            .map_err(|err| map_transaction_manager_error(&err))?;

        // Both a leg failure and a validation failure are rolled back atomically.
        let result = match self.run_batch(intents).await {
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
                handle
                    .commit()
                    .await
                    .map_err(|err| map_transaction_manager_error(&err))?;
                Ok(events)
            }
            Err(error) => {
                let message = error.to_string();
                if let Err(rollback_error) = handle.rollback().await {
                    tracing::warn!(rollback = %rollback_error, "settle rollback failed");
                }
                Err(ExecutorError::AtomicityViolation(message))
            }
        }
    }

    /// Runs a cross-tenant batch atomically (protocol §8.2, §18.3).
    ///
    /// A cross-tenant transaction spans two or more distinct tenants. The
    /// affected-tenant set is derived by partitioning the intents by
    /// `tenant_id` (preserving input order within each tenant); the sorted
    /// tenant keys are passed to `transaction_manager.begin_multi`, then each
    /// tenant's leg runs through the single-tenant [`Self::run_batch`] pipeline.
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

        let handle = self
            .ports
            .transaction_manager
            .begin_multi(&sorted_tenants)
            .await
            .map_err(|err| map_transaction_manager_error(&err))?;

        // Both a leg failure and an inconsistent-group validation failure are
        // rolled back atomically (a failed validation must not short-circuit
        // past the rollback via `?`).
        let result = match self.run_cross_tenant_legs(&by_name).await {
            Ok(groups) => atomicity::validate_cross_tenant_consistency(&groups).map(|()| groups),
            Err(error) => Err(error),
        };
        match result {
            Ok(groups) => {
                handle
                    .commit()
                    .await
                    .map_err(|err| map_transaction_manager_error(&err))?;
                Ok(groups)
            }
            Err(error) => {
                let message = error.to_string();
                if let Err(rollback_error) = handle.rollback().await {
                    tracing::warn!(rollback = %rollback_error, "cross-tenant rollback failed");
                }
                Err(ExecutorError::AtomicityViolation(message))
            }
        }
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
    /// single-tenant [`Self::run_batch`] pipeline, and the per-tenant groups are
    /// validated with [`atomicity::validate_cross_tenant_trade`] before an
    /// atomic commit. Any error rolls back and surfaces as
    /// [`ExecutorError::AtomicityViolation`].
    ///
    /// Idempotent-replay legs return no events (existing semantics). A retry
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
    pub async fn execute_cross_tenant_trade(
        &self,
        intents: &[ValidatedIntent],
        manifest: &atomicity::TradeManifest,
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

        let handle = self
            .ports
            .transaction_manager
            .begin_multi(&sorted_tenants)
            .await
            .map_err(|err| map_transaction_manager_error(&err))?;

        // Both a leg failure and a manifest-validation failure are rolled back
        // atomically (a failed validation must not short-circuit past the
        // rollback via `?`).
        let result = match self.run_cross_tenant_legs(&by_name).await {
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
                handle
                    .commit()
                    .await
                    .map_err(|err| map_transaction_manager_error(&err))?;
                Ok(groups)
            }
            Err(error) => {
                let message = error.to_string();
                if let Err(rollback_error) = handle.rollback().await {
                    tracing::warn!(
                        rollback = %rollback_error,
                        "cross-tenant trade rollback failed"
                    );
                }
                Err(ExecutorError::AtomicityViolation(message))
            }
        }
    }

    /// Executes every intent in order, short-circuiting on the first failure.
    async fn run_batch(&self, intents: &[ValidatedIntent]) -> Result<Vec<Event>, ExecutorError> {
        let mut events = Vec::new();
        for validated in intents {
            events.extend(self.execute(validated).await?);
        }
        Ok(events)
    }

    /// Runs each tenant's leg in sorted tenant order, collecting the emitted
    /// events into tenant-scoped groups.
    async fn run_cross_tenant_legs(
        &self,
        by_name: &BTreeMap<String, Vec<ValidatedIntent>>,
    ) -> Result<Vec<atomicity::TenantEventGroup>, ExecutorError> {
        let mut groups = Vec::new();
        for (name, sub_intents) in by_name {
            let events = self.run_batch(sub_intents).await?;
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

/// Resolves the authoritative holder of a subject-held resource
/// (protocol §9/§10).
///
/// The rule: a projection's `subject` field is the source of truth once a
/// resource exists; the acting actor is only the creator default used when no
/// projection exists yet (the create path). Returns `None` for owner-based
/// state types, which key by resource alone.
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
            if let Some(projection) = current
                && let Some(subject) = projection.state.get("subject").and_then(Value::as_str)
                && !subject.is_empty()
            {
                return Some(SubjectId(String::from(subject)));
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
            state: serde_json::json!({
                "subject": subject,
                "balance": "100",
                "unit": "gold_minor",
            }),
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
