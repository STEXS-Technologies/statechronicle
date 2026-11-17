//! Checked scope/profile state shared by every ordinary evaluation lane.
use super::*;
use statechronicle_profiles::registry::ProfileRules;

// This value records scope, route, observed-state, version and profile/schema
// gates. It is not an authentication token or a durable reservation. Only load
// constructs it; event generation consumes the same immutable intent and state.
pub(super) struct PreparedProfile<'intent> {
    intent: &'intent Intent,
    state_type: StateType,
    rules: &'static dyn ProfileRules,
    execution_policy: &'static dyn ExecutionPolicy,
    current: Option<StateProjection>,
}

impl<'intent> PreparedProfile<'intent> {
    pub(super) async fn load(
        evaluation: &Evaluation<'_>,
        intent: &'intent Intent,
        allow_coordinated: bool,
    ) -> Result<Self, ExecutorError> {
        let tenant = &intent.tenant_id;
        let resource = &intent.resource_id;
        let operation = &intent.operation;
        // §18.1 step 5: tenant scope.
        conflict::check_tenant_scope(intent)?;
        let tenant_exists = evaluation
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
        let rules = evaluation.profiles.resolve(state_type, &intent.inputs)?;
        let execution_policy = rules.execution_policy();
        execution_policy.validate_route(intent, allow_coordinated)?;
        let mut current = match execution_policy.primary_subject(intent) {
            Some(subject) => evaluation
                .state_index
                .get_subject_state(tenant, subject, resource)
                .await
                .map_err(|err| ExecutorError::Store(err.to_string()))?,
            None => evaluation
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
                current = evaluation
                    .state_index
                    .get_subject_state(tenant, &holder, resource)
                    .await
                    .map_err(|err| ExecutorError::Store(err.to_string()))?;
            }
        }

        if let Some(projection) = &current
            && (projection.state_type != state_type || projection.state.state_type() != state_type)
        {
            return Err(ExecutorError::TransitionInvalid(String::from(
                "state type mismatch",
            )));
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
        Ok(Self {
            intent,
            state_type,
            rules,
            execution_policy,
            current,
        })
    }

    async fn authority(
        &self,
        evaluation: &Evaluation<'_>,
    ) -> Result<Option<AuthorityProof>, ExecutorError> {
        let intent = self.intent;
        let tenant = &intent.tenant_id;
        let resource = &intent.resource_id;
        let operation = &intent.operation;
        let rules = self.rules;
        let required = rules.requires_authority(operation);
        let policy = rules.authority_policy(operation);

        if intent.authority.is_none() && required {
            // Authority-required operations MUST carry a binding (protocol
            // §11.2, ADR-006 §36 Q5 / deferral item 4).
            Err(ExecutorError::AuthorityMissing {
                operation: String::from(operation.as_str()),
            })
        } else if intent.authority.is_none() {
            // Not authority-required and no binding: proceed WITHOUT authority.
            // The profile's transition and consent rules govern this path
            // (profile-owned fallback; the event carries no authority).
            Ok(None)
        } else {
            // Authority is present: evaluate every member of the deployment's
            // authority set and aggregate under the profile's policy. An empty
            // set with authority present fails closed.
            if evaluation.trustgrant.is_empty() {
                return Err(ExecutorError::AuthorityUnavailable(String::from(
                    "no authority configured",
                )));
            }
            let mut outcomes: Vec<TrustGrantOutcome> = Vec::new();
            let mut denied = false;
            let mut stale = false;
            let mut unavailable: Option<String> = None;
            for trustgrant in evaluation.trustgrant {
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
                .unwrap_or_else(|| (evaluation.now)());
            let proof = AuthorityProof {
                kind: String::from(TRUSTGRANT_EVALUATION_KIND),
                evaluation_digest,
                result: EvaluationResult::Allow,
                evaluated_at,
            };
            // Freshness of the aggregate proof (the stalest member's
            // `evaluated_at`), checked against the primary authority.
            if let Some(primary) = evaluation.trustgrant.first() {
                primary
                    .check_revocation_freshness(&proof)
                    .await
                    .map_err(map_trustgrant_error)?;
            }
            Ok(Some(proof))
        }
    }

    fn reduce(&self) -> Result<PreparedEventStates, ExecutorError> {
        let intent = self.intent;
        let current = self.current.as_ref();
        let rules = self.rules;
        // §18.1 steps 9–10: profile rules, then deterministic after-state.
        // Typed adapters share decoded arguments; default rules retain both calls.
        let after_state = rules
            .evaluate(current, &intent.operation, &intent.inputs)
            .map_err(|error| match error {
                ProfileEvaluationError::Check(error) => ExecutorError::from(error),
                ProfileEvaluationError::Apply(error) => {
                    ExecutorError::TransitionInvalid(error.to_string())
                }
            })?;
        if after_state.state_type() != self.state_type {
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
        PreparedEventStates::new(current, after_state, self.execution_policy)
    }

    pub(super) async fn events(
        self,
        evaluation: &Evaluation<'_>,
    ) -> Result<(Event, Option<Event>), ExecutorError> {
        let intent = self.intent;
        let tenant = &intent.tenant_id;
        let resource = &intent.resource_id;
        let execution_policy = self.execution_policy;
        let authority = self.authority(evaluation).await?;
        let states = self.reduce()?;
        let current = &self.current;
        let event = evaluation.emit_event(intent, resource, states, authority.clone());

        tracing::debug!(event_id = %event.event_id.as_str(), "emitted event");

        // Construct a profile-declared companion mutation, then claim the
        // intent only after every event has been validated and constructed.
        let companion = if let Some(target) = execution_policy.companion_target(intent)? {
            let destination_current = match &target.subject {
                Some(subject) => {
                    evaluation
                        .state_index
                        .get_subject_state(tenant, subject, &target.resource)
                        .await
                }
                None => {
                    evaluation
                        .state_index
                        .get_state(tenant, &target.resource)
                        .await
                }
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
                evaluation.emit_event(intent, &target.resource, destination_states, authority);

            tracing::debug!(
                event_id = %destination_event.event_id.as_str(),
                "emitted companion event"
            );
            Some(destination_event)
        } else {
            None
        };

        Ok((event, companion))
    }
}
