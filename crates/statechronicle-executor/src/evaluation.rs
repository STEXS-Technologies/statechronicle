//! Shared evaluation, borrowed read capabilities, and explicit legacy markers.
use super::*;

pub(super) struct Evaluation<'ports> {
    pub state_index: &'ports dyn StateIndex,
    pub tenant_store: &'ports dyn TenantStore,
    pub trustgrant: &'ports [Box<dyn TrustGrantPort + Send + Sync>],
    pub profiles: &'ports ProfileRegistry,
    pub executor: &'ports SubjectId,
    pub now: &'ports (dyn Fn() -> DateTime<Utc> + Send + Sync),
    pub event_id_fn: &'ports (dyn Fn() -> EventId + Send + Sync),
    pub intent_verifier: &'ports IntentVerifier,
}

impl Evaluation<'_> {
    pub(super) async fn prepare_player(
        &self,
        validated: &ValidatedIntent,
        principal: &AuthenticatedPrincipal,
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
    ) -> Result<PreparedMutation, ExecutorError> {
        let signature = player_signature(validated)?;
        check_player_key(validated, principal, signature, key_registry).await?;
        // Public compatibility fields are not a structural-validation seal.
        let request = ValidatedIntent::try_from_intent(
            validated.intent.clone(),
            validated.signature.clone(),
        )?;
        authorize_principal(&request, principal, authorizer).await?;
        let events = self
            .execute_inner(&request, false, IntentHandling::Prepare)
            .await?;
        Ok(PreparedMutation::new(request, principal, events)?)
    }

    pub(super) async fn prepare_player_batch(
        &self,
        items: &[PlayerBatchItem<'_>],
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
    ) -> Result<PreparedBatch, ExecutorError> {
        if items.is_empty() {
            return Err(ExecutorError::AtomicityViolation(String::from(
                "empty batch",
            )));
        }
        check_size("batch_intents", MAX_EVENTS_PER_COMMIT, items.len())?;
        let mut requests = Vec::with_capacity(items.len());
        for item in items {
            let signature = player_signature(item.validated)?;
            authorize_principal(item.validated, item.principal, authorizer).await?;
            check_player_key(item.validated, item.principal, signature, key_registry).await?;
            requests.push(ValidatedIntent::try_from_intent(
                item.validated.intent.clone(),
                item.validated.signature.clone(),
            )?);
        }
        let mut identities = BTreeSet::new();
        let tenant = &requests
            .first()
            .ok_or_else(|| ExecutorError::AtomicityViolation(String::from("empty batch")))?
            .intent
            .tenant_id;
        for request in &requests {
            if &request.intent.tenant_id != tenant || !identities.insert(&request.intent.intent_id)
            {
                return Err(ExecutorError::AtomicityViolation(String::from(
                    "mixed tenant scopes or duplicate intent identity in prepared batch",
                )));
            }
        }
        let mut mutations = Vec::with_capacity(requests.len());
        for (request, item) in requests.into_iter().zip(items) {
            let events = self
                .execute_inner(&request, false, IntentHandling::Prepare)
                .await?;
            mutations.push(PreparedMutation::new(request, item.principal, events)?);
        }
        let prepared = PreparedBatch::new(mutations)?;
        self.validate_profile_groups(
            |identity| {
                prepared
                    .mutations()
                    .iter()
                    .map(PreparedMutation::validated)
                    .find(|request| &request.intent.intent_id == identity)
            },
            prepared.events(),
        )?;
        Ok(prepared)
    }

    fn emit_event(
        &self,
        intent: &Intent,
        resource: &ResourceId,
        states: PreparedEventStates,
        authority: Option<AuthorityProof>,
    ) -> Event {
        Event::new(
            intent.tenant_id.clone(),
            (self.event_id_fn)(),
            intent.intent_id.clone(),
            intent.operation.clone(),
            resource.clone(),
            intent.actor.clone(),
            states.before,
            states.after,
            authority,
            self.executor.clone(),
            (self.now)(),
        )
    }

    #[allow(clippy::collapsible_if)]
    pub(super) async fn execute_inner(
        &self,
        validated: &ValidatedIntent,
        allow_coordinated: bool,
        intent_handling: IntentHandling<'_>,
    ) -> Result<Vec<Event>, ExecutorError> {
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

        // Legacy idempotency is a compatibility behavior. Preparation cannot
        // treat an independent intent marker as a committed ledger result.
        if let Some(store) = intent_handling.marker_store() {
            // §18.1 step 3: idempotency. Replays succeed; conflicting intents fail.
            let existing = store
                .get_intent(tenant, &intent.intent_id)
                .await
                .map_err(|err| map_intent_store_error(err, &intent.intent_id.0))?;
            if let Some(existing) = existing {
                conflict::check_idempotency_existing(&existing, intent)?;
                tracing::debug!(intent_id = %intent.intent_id.as_str(), "idempotent replay");
                return Ok(Vec::new());
            }
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
            if let Some(primary) = self.trustgrant.first() {
                primary
                    .check_revocation_freshness(proof)
                    .await
                    .map_err(map_trustgrant_error)?;
            }
        }

        // §18.1 step 5: tenant scope.
        conflict::check_tenant_scope(intent)?;
        let tenant_exists = self
            .tenant_store
            .tenant_exists(tenant)
            .await
            .map_err(|err| ExecutorError::Store(err.to_string()))?;
        if !tenant_exists {
            return Err(ExecutorError::TenantNotFound {
                tenant: tenant.0.clone(),
            });
        }

        // Resolve profile-owned storage addressing before loading the state.
        let state_type = intent.state_type.ok_or(ExecutorError::StateTypeRequired)?;
        let rules = self.profiles.resolve(state_type, &intent.inputs)?;
        let execution_policy = rules.execution_policy();
        execution_policy.validate_route(intent, allow_coordinated)?;
        let mut current = match execution_policy.primary_subject(intent) {
            Some(subject) => self
                .state_index
                .get_subject_state(tenant, subject, resource)
                .await
                .map_err(|err| ExecutorError::Store(err.to_string()))?,
            None => self
                .state_index
                .get_state(tenant, resource)
                .await
                .map_err(|err| ExecutorError::Store(err.to_string()))?,
        };

        // A profile may make the existing state's address authoritative over
        // the initial lookup's creation default.
        if let Some(holder) = execution_policy.current_subject(intent, current.as_ref()) {
            let queried = execution_policy.primary_subject(intent);
            if queried.is_some() && Some(&holder) != queried {
                current = self
                    .state_index
                    .get_subject_state(tenant, &holder, resource)
                    .await
                    .map_err(|err| ExecutorError::Store(err.to_string()))?;
            }
        }

        if let Some(projection) = &current {
            if projection.state_type != state_type || projection.state.state_type() != state_type {
                return Err(ExecutorError::TransitionInvalid(String::from(
                    "state type mismatch",
                )));
            }
        }

        // §18.1 step 7 + §18.2 conflict gates.
        conflict::check_expected_version(intent, current.as_ref())?;
        execution_policy.check_access(intent, current.as_ref())?;

        // §18.1 step 8: TrustGrant authority. The active profile's rule set is
        // resolved here (before the gate) so `requires_authority` and
        // `authority_policy` can drive the mandatory-binding rule and the
        // aggregation policy; `rules.check` still runs at step 9.
        if state_type == StateType::Custom {
            let identity = ProfileId::new(String::from(rules.profile_id()))?;
            if !rules.allowed_operations().contains(operation) {
                return Err(ExecutorError::TransitionInvalid(String::from(
                    "unregistered operation",
                )));
            }
            if let Some(projection) = &current {
                let ResourceState::Custom(custom) = &projection.state else {
                    return Err(ExecutorError::TransitionInvalid(String::from(
                        "custom state required",
                    )));
                };
                if custom.domain != identity || custom.schema_version != rules.schema_version() {
                    return Err(ExecutorError::TransitionInvalid(String::from(
                        "domain/schema mismatch",
                    )));
                }
            }
        }
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
            if self.trustgrant.is_empty() {
                return Err(ExecutorError::AuthorityUnavailable(String::from(
                    "no authority configured",
                )));
            }
            let mut outcomes: Vec<TrustGrantOutcome> = Vec::new();
            let mut denied = false;
            let mut stale = false;
            let mut unavailable: Option<String> = None;
            for trustgrant in self.trustgrant {
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
            if let Some(primary) = self.trustgrant.first() {
                primary
                    .check_revocation_freshness(&proof)
                    .await
                    .map_err(map_trustgrant_error)?;
            }
            Some(proof)
        };

        // §18.1 steps 9–10: profile rules, then deterministic after-state.
        // Typed adapters share decoded arguments; default rules retain both calls.
        let after_state = rules
            .evaluate(current.as_ref(), operation, &intent.inputs)
            .map_err(|error| match error {
                ProfileEvaluationError::Check(error) => ExecutorError::from(error),
                ProfileEvaluationError::Apply(error) => {
                    ExecutorError::TransitionInvalid(error.to_string())
                }
            })?;
        if after_state.state_type() != state_type {
            return Err(ExecutorError::TransitionInvalid(String::from(
                "reducer state type mismatch",
            )));
        }

        if let ResourceState::Custom(custom) = &after_state {
            let identity = ProfileId::new(String::from(rules.profile_id()))?;
            if custom.domain != identity || custom.schema_version != rules.schema_version() {
                return Err(ExecutorError::TransitionInvalid(String::from(
                    "reducer domain/schema mismatch",
                )));
            }
        }

        // §18.1 step 11: emit the event.
        let states = PreparedEventStates::new(current.as_ref(), after_state, execution_policy)?;
        let event = self.emit_event(intent, resource, states, authority.clone());

        tracing::debug!(event_id = %event.event_id.as_str(), "emitted event");

        // Construct a profile-declared companion mutation, then claim the
        // intent only after every event has been validated and constructed.
        let companion = if let Some(target) = execution_policy.companion_target(intent)? {
            let destination_current = match &target.subject {
                Some(subject) => {
                    self.state_index
                        .get_subject_state(tenant, subject, &target.resource)
                        .await
                }
                None => self.state_index.get_state(tenant, &target.resource).await,
            }
            .map_err(|err| ExecutorError::Store(err.to_string()))?;
            let source = current.as_ref().ok_or_else(|| {
                ExecutorError::TransitionInvalid(String::from(
                    "companion transition requires an existing primary state",
                ))
            })?;
            let destination_after_state = execution_policy
                .apply_companion(intent, source, destination_current.as_ref())
                .map_err(|error| ExecutorError::TransitionInvalid(error.to_string()))?;

            let destination_states = PreparedEventStates::new(
                destination_current.as_ref(),
                destination_after_state,
                execution_policy,
            )?;
            let destination_event =
                self.emit_event(intent, &target.resource, destination_states, authority);

            tracing::debug!(
                event_id = %destination_event.event_id.as_str(),
                "emitted companion event"
            );
            Some(destination_event)
        } else {
            None
        };

        // Claim only after authentication, authority, profile, transition, and
        // complete event construction have passed. This prevents malformed or
        // unauthorized requests from poisoning the idempotency namespace.
        // Durable implementations must still make this claim atomic with the
        // event/commit write (see TODO.md P0-1/P0-2).
        if let IntentHandling::LegacyClaim(store) = intent_handling {
            store
                .put_intent(tenant, intent)
                .await
                .map_err(|err| map_intent_store_error(err, &intent.intent_id.0))?;
        }
        Ok(match companion {
            Some(destination) => vec![event, destination],
            None => vec![event],
        })
    }

    pub(super) fn validate_profile_groups<'event>(
        &self,
        intent_for: impl Fn(&IntentId) -> Option<&'event ValidatedIntent>,
        events: impl IntoIterator<Item = &'event Event>,
    ) -> Result<(), ExecutorError> {
        let mut identities = BTreeSet::new();
        let mut groups = BTreeMap::new();
        let mut tenant = None;
        for event in events {
            if !identities.insert(&event.event_id) {
                return Err(ExecutorError::AtomicityViolation(String::from(
                    "duplicate event id in batch",
                )));
            }
            if tenant.is_some_and(|first| first != &event.tenant_id) {
                return Err(ExecutorError::AtomicityViolation(String::from(
                    "mixed tenant scopes in batch",
                )));
            }
            tenant = Some(&event.tenant_id);
            groups
                .entry(&event.intent_id)
                .or_insert_with(Vec::new)
                .push(event);
        }
        for (identity, group) in groups {
            let validated = intent_for(identity).ok_or_else(|| {
                ExecutorError::AtomicityViolation(String::from("event has no accepted intent"))
            })?;
            let intent = &validated.intent;
            let state_type = intent.state_type.ok_or(ExecutorError::StateTypeRequired)?;
            self.profiles
                .resolve(state_type, &intent.inputs)?
                .execution_policy()
                .validate_events(&group)?;
        }
        Ok(())
    }
}
