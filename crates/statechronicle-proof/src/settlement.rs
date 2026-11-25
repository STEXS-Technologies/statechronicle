//! Independently requested historical settlement facts, authenticated by events.
//!
//! This verifier deliberately does not promote the unsigned `TradeSummary` or
//! `latest_event` metadata into authority. Supply full ordered commit event
//! batches and expectations obtained independently from the proof provider.
use std::collections::{BTreeMap, BTreeSet};

use crate::{error::ProofError, verify::verify_commit_signature_with_key};
use chrono::{DateTime, Utc};
use ed25519_dalek::VerifyingKey;
use statechronicle_accumulator::event_tree::event_merkle_root;
use statechronicle_commit::sign_v1::verify_commit_v1_with_policy;
use statechronicle_core::{
    amount::Amount,
    canonicalize::{canonicalize, canonicalize_and_digest},
    digest::{ContentDigest, hash_bytes},
    limits::{
        MAX_COMMIT_BYTES, MAX_EVENT_BATCH_BYTES, MAX_EVENT_BYTES, MAX_EVENTS_PER_COMMIT,
        MAX_ID_LENGTH, check_size,
    },
    signature_wire::MAX_V1_ENVELOPE_BYTES,
};
use statechronicle_domain::{
    commit::{COMMIT_SCHEMA, Commit, ScopeKind},
    event::{EVENT_SCHEMA, Event},
    ids::{EventId, IntentId},
    resource::ResourceId,
    resource_state::ResourceState,
    signed::Signed,
    subject::SubjectId,
    tenant::TenantId,
};
use statechronicle_ports::signature_policy::{SignatureAuthority, SignaturePolicy, SignatureUse};

/// Independently pinned signed event identity. `commit_digest` hashes the full
/// canonical commit body, not an untrusted commit ID or reference alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSettlementEvent {
    pub tenant: TenantId,
    pub resource: ResourceId,
    pub event_id: EventId,
    pub intent_id: IntentId,
    pub commit_digest: ContentDigest,
}

/// An asset's requested historical transfer out of the named trade hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedAssetSettlement {
    pub event: ExpectedSettlementEvent,
    pub from: SubjectId,
    pub to: SubjectId,
}

/// An independently requested payment, including both signed event identities.
/// Payment events do not contain a trade ID: the caller explicitly requests
/// this payment alongside the asset facts. This does not prove an unsigned
/// intent's value declaration or a cross-tenant atomic relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSettlementPayment {
    pub debit: ExpectedSettlementEvent,
    pub credit: ExpectedSettlementEvent,
    pub from: SubjectId,
    pub to: SubjectId,
    pub amount: Amount,
    pub unit: String,
}

/// The complete set of historical facts the caller wants authenticated.
/// An empty payment list requests no payment facts; it does not prove none occurred.
/// The asset list must be nonempty. Do not derive expectations from a summary
/// supplied by the same untrusted party supplying the witnesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSettlement {
    pub trade_id: String,
    pub assets: Vec<ExpectedAssetSettlement>,
    pub payments: Vec<ExpectedSettlementPayment>,
}

/// Untrusted full, ordered event batch and its enclosing signed commit.
/// This is a borrowed verifier input, not a new wire representation.
pub struct SettlementWitness<'witness> {
    pub commit: &'witness Signed<Commit>,
    pub events: &'witness [Event],
}

/// Authenticated historical facts only. No current ownership, canonical-head
/// freshness, full lifecycle completeness or distributed atomicity is implied.
/// Private construction and no deserialization prevent manufacturing a receipt.
///
/// ```compile_fail
/// use statechronicle_proof::settlement::VerifiedSettlement;
/// let receipt = VerifiedSettlement { expected: todo!() };
/// ```
#[derive(Debug)]
pub struct VerifiedSettlement {
    expected: ExpectedSettlement,
}
impl VerifiedSettlement {
    /// Returns only the independently requested and authenticated facts.
    pub const fn claim(&self) -> &ExpectedSettlement {
        &self.expected
    }
}

fn demand(condition: bool, detail: &str) -> Result<(), ProofError> {
    if condition {
        Ok(())
    } else {
        Err(ProofError::InvalidState(detail.into()))
    }
}

struct AuthenticatedSettlementWitness<'witness> {
    commit: &'witness Commit,
    tenant: &'witness TenantId,
    digest: ContentDigest,
    events: &'witness [Event],
}

fn validate_settlement_request(
    expected: &ExpectedSettlement,
    witness_count: usize,
) -> Result<BTreeSet<(&str, &str)>, ProofError> {
    demand(
        !expected.trade_id.is_empty() && !expected.assets.is_empty(),
        "settlement requires a trade ID and assets",
    )?;
    check_size("trade ID", MAX_ID_LENGTH, expected.trade_id.len())?;
    check_size(
        "settlement assets",
        MAX_EVENTS_PER_COMMIT,
        expected.assets.len(),
    )?;
    check_size(
        "settlement payments",
        MAX_EVENTS_PER_COMMIT / 2,
        expected.payments.len(),
    )?;
    check_size(
        "settlement events",
        MAX_EVENTS_PER_COMMIT,
        expected
            .assets
            .len()
            .saturating_add(expected.payments.len().saturating_mul(2)),
    )?;
    check_size("settlement witnesses", MAX_EVENTS_PER_COMMIT, witness_count)?;
    let mut pins = BTreeSet::new();
    let mut requested_events = BTreeSet::new();
    let mut assets = BTreeSet::new();
    for asset in &expected.assets {
        demand(
            assets.insert((&asset.event.tenant.0, &asset.event.resource.0)),
            "duplicate requested asset",
        )?;
    }
    for event in expected
        .assets
        .iter()
        .map(|a| &a.event)
        .chain(expected.payments.iter().flat_map(|p| [&p.debit, &p.credit]))
    {
        demand(
            requested_events.insert((&event.tenant.0, &event.event_id.0)),
            "duplicate requested event",
        )?;
        pins.insert((event.tenant.0.as_str(), event.commit_digest.as_str()));
    }
    Ok(pins)
}

fn verify_settlement_facts(
    expected: &ExpectedSettlement,
    witnesses: &[AuthenticatedSettlementWitness<'_>],
    pins: &BTreeSet<(&str, &str)>,
) -> Result<VerifiedSettlement, ProofError> {
    let mut events = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut bytes = 0usize;
    for witness in witnesses {
        let commit = witness.commit;
        let tenant = witness.tenant;
        let digest = &witness.digest;
        demand(
            pins.contains(&(tenant.0.as_str(), digest.as_str())),
            "unrequested settlement commit",
        )?;
        demand(
            seen.insert((tenant.0.clone(), digest.clone())),
            "duplicate settlement witness",
        )?;
        check_size(
            "settlement commit",
            MAX_COMMIT_BYTES,
            canonicalize(commit)?.len(),
        )?;
        check_size(
            "settlement commit events",
            MAX_EVENTS_PER_COMMIT,
            witness.events.len(),
        )?;
        demand(
            commit.event_count == witness.events.len() as u64,
            "settlement event count mismatch",
        )?;
        for event in witness.events {
            let size = canonicalize(event)?.len();
            check_size("settlement event", MAX_EVENT_BYTES, size)?;
            bytes = bytes
                .checked_add(size)
                .ok_or_else(|| ProofError::InvalidState("settlement byte count overflow".into()))?;
            check_size("settlement event batch", MAX_EVENT_BATCH_BYTES, bytes)?;
            demand(
                event.schema == EVENT_SCHEMA && event.tenant_id == *tenant,
                "settlement event scope/schema mismatch",
            )?;
            demand(
                event.before.version.checked_add(1) == Some(event.after.version),
                "settlement event version mismatch",
            )?;
            demand(
                canonicalize_and_digest(&event.before.state)? == event.before.state_hash
                    && canonicalize_and_digest(&event.after.state)? == event.after.state_hash,
                "settlement state hash mismatch",
            )?;
            demand(
                events
                    .insert(
                        (tenant.0.clone(), event.event_id.0.clone()),
                        (event, digest.clone()),
                    )
                    .is_none(),
                "duplicate witnessed event identity",
            )?;
        }
        demand(
            event_merkle_root(witness.events)?.as_ref() == Some(&commit.event_merkle_root),
            "settlement event root mismatch",
        )?;
    }
    demand(seen.len() == pins.len(), "missing settlement witness")?;
    let find = |pin: &ExpectedSettlementEvent| -> Result<&Event, ProofError> {
        let (event, digest) = events
            .get(&(pin.tenant.0.clone(), pin.event_id.0.clone()))
            .ok_or_else(|| ProofError::InvalidState("missing requested settlement event".into()))?;
        demand(
            event.resource_id == pin.resource
                && event.intent_id == pin.intent_id
                && *digest == pin.commit_digest,
            "settlement event identity mismatch",
        )?;
        Ok(event)
    };
    for asset in &expected.assets {
        let event = find(&asset.event)?;
        demand(
            event.operation.as_str() == "trade.settle",
            "requested asset event is not trade.settle",
        )?;
        let (ResourceState::UniqueAsset(before), ResourceState::UniqueAsset(after)) =
            (&event.before.state, &event.after.state)
        else {
            return Err(ProofError::InvalidState(
                "settlement asset state kind mismatch".into(),
            ));
        };
        demand(
            event.before.version != 0
                && before.trade_id.as_deref() == Some(expected.trade_id.as_str())
                && before.status.as_str() == "trade_held"
                && after.status.as_str() == "active"
                && after.trade_id.is_none()
                && before.owner == asset.from
                && after.owner == asset.to
                && asset.from != asset.to,
            "requested trade/asset ownership transition mismatch",
        )?;
    }
    for payment in &expected.payments {
        let debit = find(&payment.debit)?;
        let credit = find(&payment.credit)?;
        demand(
            payment.debit.tenant == payment.credit.tenant
                && payment.debit.resource == payment.credit.resource
                && payment.debit.intent_id == payment.credit.intent_id
                && payment.debit.commit_digest == payment.credit.commit_digest
                && payment.from != payment.to
                && !payment.amount.is_zero()
                && !payment.unit.is_empty(),
            "invalid requested payment pair",
        )?;
        let group_count = events
            .values()
            .filter(|(e, d)| {
                e.tenant_id == debit.tenant_id
                    && e.intent_id == debit.intent_id
                    && *d == payment.debit.commit_digest
            })
            .count();
        demand(group_count == 2, "payment intent is not exactly two events")?;
        for (event, subject, is_debit) in
            [(debit, &payment.from, true), (credit, &payment.to, false)]
        {
            let (ResourceState::FungibleBalance(before), ResourceState::FungibleBalance(after)) =
                (&event.before.state, &event.after.state)
            else {
                return Err(ProofError::InvalidState(
                    "payment balance state kind mismatch".into(),
                ));
            };
            let delta = if is_debit {
                before.balance.checked_sub(after.balance)
            } else {
                after.balance.checked_sub(before.balance)
            };
            let existing_holder = before.subject == *subject && before.unit == payment.unit;
            let unborn_credit = !is_debit
                && event.before.version == 0
                && event.before.state
                    == statechronicle_profiles::resource::policy(
                        statechronicle_domain::state_type::StateType::FungibleBalance,
                    )
                    .unborn_state(&event.after.state)
                    .map_err(|error| ProofError::InvalidState(error.to_string()))?;
            demand(
                event.operation.as_str() == "balance.transfer"
                    && event.actor == payment.from
                    && (if event.before.version == 0 {
                        unborn_credit
                    } else {
                        existing_holder
                    })
                    && after.subject == *subject
                    && after.unit == payment.unit
                    && delta == Some(payment.amount),
                "requested payment transition mismatch",
            )?;
        }
    }
    Ok(VerifiedSettlement {
        expected: expected.clone(),
    })
}

/// Authenticates the exact requested historical asset settlements and payments.
///
/// `resolve_key` must independently authorize the signer for the expected
/// tenant and historical commit (including executor, key ID and time policy).
/// A supplied verifying key is never inferred from the witness. This function
/// itself always performs strict Ed25519 verification after policy resolution.
/// Every used commit is pinned by its full body digest; no live store is read.
/// This establishes historical signed facts under those pins, not canonical
/// chain membership or current state. Use the current-claim verifier separately.
///
/// Bounds: at most 256 requested events and witnesses, 256 events per witness,
/// 64 KiB per event, 4 MiB aggregate event bytes, 1 MiB per commit body.
///
/// # Errors
/// Returns structural/size, signature/key-policy or canonicalization errors
/// when any requested identity, root, scope, state hash or transition differs.
pub fn verify_settlement_history(
    expected: &ExpectedSettlement,
    witnesses: &[SettlementWitness<'_>],
    mut resolve_key: impl FnMut(&TenantId, &Signed<Commit>) -> Result<VerifyingKey, ProofError>,
) -> Result<VerifiedSettlement, ProofError> {
    let pins = validate_settlement_request(expected, witnesses.len())?;
    let mut prepared = Vec::with_capacity(witnesses.len());
    let mut seen = BTreeSet::new();
    for witness in witnesses {
        let signed = witness.commit;
        let commit = &signed.body;
        check_size(
            "settlement commit",
            MAX_COMMIT_BYTES,
            canonicalize(commit)?.len(),
        )?;
        let tenant = commit
            .scope
            .tenant_id
            .as_ref()
            .ok_or_else(|| ProofError::CommitScope("missing tenant".into()))?;
        demand(
            commit.schema == COMMIT_SCHEMA && commit.scope.kind == ScopeKind::Tenant,
            "unsupported settlement commit scope/schema",
        )?;
        let digest = canonicalize_and_digest(commit)?;
        demand(
            pins.contains(&(tenant.0.as_str(), digest.as_str())),
            "unrequested settlement commit",
        )?;
        demand(
            seen.insert((tenant.0.clone(), digest.clone())),
            "duplicate settlement witness",
        )?;
        let key = resolve_key(tenant, signed)?;
        verify_commit_signature_with_key(signed, &key)?;
        prepared.push(AuthenticatedSettlementWitness {
            commit,
            tenant,
            digest,
            events: witness.events,
        });
    }
    demand(seen.len() == pins.len(), "missing settlement witness")?;
    verify_settlement_facts(expected, &prepared, &pins)
}

/// Explicit wire format of a pinned settlement commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SettlementEvidenceFormat {
    /// Legacy BCS `Signed<Commit>` record.
    Legacy,
    /// Exact versioned v1 signed envelope.
    V1,
}

/// Independent pin for one exact signed settlement commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSettlementCommit {
    /// Tenant scope.
    pub tenant: TenantId,
    /// Digest of the canonical commit body used by requested event facts.
    pub body_digest: ContentDigest,
    /// Explicit wire format; no fallback is attempted.
    pub format: SettlementEvidenceFormat,
    /// Digest of the exact signed record bytes (canonical BCS for legacy).
    pub signed_record_digest: ContentDigest,
    /// Independently configured authority for this tenant's v1 commit chain.
    /// Absent for legacy records; required for v1 records.
    pub commit_authority: Option<SubjectId>,
}

/// Complete independently pinned expected settlement evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSettlementEvidence {
    /// Requested asset/payment facts.
    pub facts: ExpectedSettlement,
    /// Exact signed evidence pins, one per witnessed commit.
    pub commits: Vec<ExpectedSettlementCommit>,
}

/// Full historical commit event evidence in an explicitly selected format.
pub enum SettlementEvidence<'evidence> {
    /// Legacy signed body and complete ordered events.
    Legacy {
        /// Original signed legacy commit.
        commit: &'evidence Signed<Commit>,
        /// Complete commit event list.
        events: &'evidence [Event],
    },
    /// Exact v1 envelope and complete ordered events.
    V1 {
        /// Original canonical v1 commit envelope bytes.
        commit_record: &'evidence [u8],
        /// Complete commit event list.
        events: &'evidence [Event],
    },
}

/// Verify explicitly pinned legacy and/or v1 settlement commit evidence.
/// Legacy records use the supplied independent key resolver. V1 records use
/// the shared commit policy verifier in Historical mode and never fall back to
/// legacy decoding. Settlement event and transition rules are shared with the
/// compatibility [`verify_settlement_history`] entrypoint.
///
/// # Errors
/// Refuses missing/duplicate pins, format substitution, invalid signatures,
/// malformed scopes, event/root errors, or settlement transition mismatches.
pub async fn verify_settlement_history_evidence(
    expected: &ExpectedSettlementEvidence,
    witnesses: &[SettlementEvidence<'_>],
    policy: &dyn SignaturePolicy,
    now: DateTime<Utc>,
    mut resolve_legacy_key: impl FnMut(&TenantId, &Signed<Commit>) -> Result<VerifyingKey, ProofError>,
) -> Result<VerifiedSettlement, ProofError> {
    check_size(
        "settlement evidence pins",
        MAX_EVENTS_PER_COMMIT,
        expected.commits.len(),
    )?;
    check_size(
        "settlement witnesses",
        MAX_EVENTS_PER_COMMIT,
        witnesses.len(),
    )?;
    demand(
        expected.commits.len() == witnesses.len(),
        "settlement pin/witness count mismatch",
    )?;
    let requested_pins = validate_settlement_request(&expected.facts, witnesses.len())?;
    for pin in &expected.commits {
        demand(
            (pin.format == SettlementEvidenceFormat::V1) == pin.commit_authority.is_some(),
            "v1 evidence requires a pinned authority and legacy evidence forbids one",
        )?;
    }
    // Bound all signed records and events before canonical hashing or signature
    // work begins.
    let mut evidence_bytes = 0usize;
    for witness in witnesses {
        let (record_size, events) = match witness {
            SettlementEvidence::Legacy { commit, events } => (
                bcs::serialized_size(*commit)
                    .map_err(|error| ProofError::InvalidState(error.to_string()))?,
                *events,
            ),
            SettlementEvidence::V1 {
                commit_record,
                events,
            } => (commit_record.len(), *events),
        };
        check_size(
            "settlement signed commit",
            MAX_V1_ENVELOPE_BYTES,
            record_size,
        )?;
        check_size(
            "settlement commit events",
            MAX_EVENTS_PER_COMMIT,
            events.len(),
        )?;
        evidence_bytes = evidence_bytes.checked_add(record_size).ok_or_else(|| {
            ProofError::InvalidState(String::from("settlement evidence byte count overflow"))
        })?;
        for event in events {
            let event_size = bcs::serialized_size(event)
                .map_err(|error| ProofError::InvalidState(error.to_string()))?;
            check_size("settlement event", MAX_EVENT_BYTES, event_size)?;
            evidence_bytes = evidence_bytes.checked_add(event_size).ok_or_else(|| {
                ProofError::InvalidState(String::from("settlement evidence byte count overflow"))
            })?;
        }
        check_size(
            "settlement evidence bytes",
            MAX_EVENT_BATCH_BYTES,
            evidence_bytes,
        )?;
    }
    let mut pins = BTreeSet::new();
    for pin in &expected.commits {
        demand(
            pins.insert((
                pin.tenant.0.clone(),
                pin.body_digest.clone(),
                pin.format,
                pin.signed_record_digest.clone(),
            )),
            "duplicate signed settlement pin",
        )?;
    }
    let mut used = BTreeSet::new();
    let mut decoded: Vec<(Commit, &[Event])> = Vec::with_capacity(witnesses.len());
    for witness in witnesses {
        let (body, events) = match witness {
            SettlementEvidence::Legacy { commit, events } => {
                let body_digest = canonicalize_and_digest(&commit.body)?;
                let signed_bytes = bcs::to_bytes(*commit)
                    .map_err(|error| ProofError::InvalidState(error.to_string()))?;
                let signed_digest = hash_bytes(&signed_bytes);
                let tenant = commit
                    .body
                    .scope
                    .tenant_id
                    .as_ref()
                    .ok_or_else(|| ProofError::CommitScope(String::from("missing tenant")))?;
                demand(
                    commit.body.schema == COMMIT_SCHEMA
                        && commit.body.scope.kind == ScopeKind::Tenant,
                    "unsupported settlement commit scope/schema",
                )?;
                let pin = (
                    tenant.0.clone(),
                    body_digest,
                    SettlementEvidenceFormat::Legacy,
                    signed_digest,
                );
                demand(
                    pins.contains(&pin),
                    "legacy settlement evidence does not match an exact pin",
                )?;
                demand(used.insert(pin), "duplicate settlement evidence")?;
                let key = resolve_legacy_key(tenant, commit)?;
                verify_commit_signature_with_key(commit, &key)?;
                (commit.body.clone(), *events)
            }
            SettlementEvidence::V1 {
                commit_record,
                events,
            } => {
                let record_digest = hash_bytes(commit_record);
                let mut matching_pins = expected.commits.iter().filter(|pin| {
                    pin.format == SettlementEvidenceFormat::V1
                        && pin.signed_record_digest == record_digest
                });
                let trusted_pin = matching_pins.next().ok_or_else(|| {
                    ProofError::InvalidState(String::from(
                        "v1 signed settlement record has no exact authority pin",
                    ))
                })?;
                demand(
                    matching_pins.next().is_none(),
                    "ambiguous v1 signed settlement record pin",
                )?;
                let pinned_tenant = &trusted_pin.tenant;
                let authority = trusted_pin.commit_authority.as_ref().ok_or_else(|| {
                    ProofError::InvalidState(String::from("v1 settlement authority pin is missing"))
                })?;
                let body = verify_commit_v1_with_policy(
                    commit_record,
                    SignatureAuthority::TenantCommit {
                        tenant: pinned_tenant,
                        authority,
                    },
                    SignatureUse::Historical,
                    now,
                    policy,
                )
                .await
                .map_err(|error| ProofError::CommitSignature(error.to_string()))?;
                let tenant = body
                    .scope
                    .tenant_id
                    .as_ref()
                    .ok_or_else(|| ProofError::CommitScope(String::from("missing tenant")))?;
                demand(
                    tenant == pinned_tenant,
                    "v1 settlement commit tenant changed during verification",
                )?;
                let body_digest = canonicalize_and_digest(&body)?;
                let pin = (
                    tenant.0.clone(),
                    body_digest,
                    SettlementEvidenceFormat::V1,
                    record_digest,
                );
                demand(
                    pins.contains(&pin),
                    "v1 settlement evidence does not match an exact pin",
                )?;
                demand(used.insert(pin), "duplicate settlement evidence")?;
                (body, *events)
            }
        };
        decoded.push((body, events));
    }
    demand(
        used.len() == pins.len(),
        "missing exact signed settlement evidence",
    )?;
    let mut prepared = Vec::with_capacity(decoded.len());
    for (body, events) in &decoded {
        let tenant = body
            .scope
            .tenant_id
            .as_ref()
            .ok_or_else(|| ProofError::CommitScope(String::from("missing tenant")))?;
        prepared.push(AuthenticatedSettlementWitness {
            commit: body,
            tenant,
            digest: canonicalize_and_digest(body)?,
            events,
        });
    }
    verify_settlement_facts(&expected.facts, &prepared, &requested_pins)
}
