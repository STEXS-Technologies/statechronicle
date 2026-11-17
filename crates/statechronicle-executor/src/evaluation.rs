//! Shared evaluation, borrowed read capabilities, and explicit legacy markers.
use super::*;
#[path = "prepared_profile.rs"]
mod prepared_profile;
use prepared_profile::PreparedProfile;

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
        self.prepare_batch(items, authorizer, key_registry, false)
            .await
    }

    pub(super) async fn prepare_player_settlement(
        &self,
        items: &[PlayerBatchItem<'_>],
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
    ) -> Result<PreparedBatch, ExecutorError> {
        self.prepare_batch(items, authorizer, key_registry, true)
            .await
    }

    async fn prepare_batch(
        &self,
        items: &[PlayerBatchItem<'_>],
        authorizer: &dyn Authorizer,
        key_registry: &dyn KeyRegistry,
        settlement: bool,
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
                .execute_inner(&request, settlement, IntentHandling::Prepare)
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
        if settlement {
            // Existing validators consume flat slices. Keep this bounded copy
            // local to the explicit coordinator; ordinary batches do not pay it.
            let events: Vec<_> = prepared.events().cloned().collect();
            settlement::validate_settle(
                &events,
                prepared.mutations().iter().map(PreparedMutation::validated),
            )?;
        }
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

    pub(super) async fn execute_inner(
        &self,
        validated: &ValidatedIntent,
        allow_coordinated: bool,
        intent_handling: IntentHandling<'_>,
    ) -> Result<Vec<Event>, ExecutorError> {
        let intent = &validated.intent;
        let tenant = &intent.tenant_id;
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
        if let Some(proof) = &intent.authority
            && let Some(primary) = self.trustgrant.first()
        {
            primary
                .check_revocation_freshness(proof)
                .await
                .map_err(map_trustgrant_error)?;
        }

        let (event, companion) = PreparedProfile::load(self, intent, allow_coordinated)
            .await?
            .events(self)
            .await?;

        // Claim only after authentication, authority, profile, transition, and
        // complete event construction have passed. This prevents malformed or
        // unauthorized requests from poisoning the idempotency namespace.
        // Durable implementations must still make this claim atomic with the
        // event/commit write within the ledger transaction.
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
