//! Trade proof assembly and verification (Phase 2 of the trade completion).
//!
//! A trade proof ties a settled trade's per-tenant state proofs together. There
//! is deliberately **no cross-tenant root**: each settled asset is proven by a
//! per-tenant [`ResourceStateProof`](statechronicle_domain::proof::ResourceStateProof) under that tenant's own commit and
//! verifying key, and `trade_id` is the semantic binding that a caller uses to
//! join the legs. This keeps every leg independently verifiable with the
//! existing single-tenant [`crate::verify::verify_bundle`] pipeline while
//! leaving the cross-leg join to the caller's trust of the `trade_id` linkage.
//!
//! [`build_trade_proof`](crate::trade::build_trade_proof) is pure and assembling; [`verify_trade_proof`](crate::trade::verify_trade_proof) is pure
//! and fail-closed.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use ed25519_dalek::VerifyingKey;
use statechronicle_domain::commit::Commit;
use statechronicle_domain::ids::CommitId;
use statechronicle_domain::proof::ResourceStateProof;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use statechronicle_domain::trade::{
    TRADE_PROOF_SCHEMA, TRADE_PROOF_V1_SCHEMA, TradeAssetStateProofV1, TradeProof, TradeProofLeg,
    TradeProofV1, TradeRecord, TradeSummary,
};
use statechronicle_ports::signature_policy::SignaturePolicy;

use crate::bundle::{derive_state_key, owner_of};
use crate::error::ProofError;
use crate::verify::verify_bundle;

/// Caller-pinned verification authority for one tenant/commit identity.
pub enum ScopedTradeCommitV1 {
    /// Legacy commit and key selected independently by the caller.
    Legacy {
        signed: Box<Signed<Commit>>,
        verifying_key: VerifyingKey,
    },
    /// Exact signed-record digest pin and independently configured authority.
    V1 {
        pin: crate::claim::ExpectedCommitV1,
        authority: SubjectId,
    },
}

/// Verifies mixed-format trade asset proofs against an independently supplied
/// trade summary and caller-pinned `(tenant, commit_id)` evidence. V1 envelopes
/// use Historical signature policy. Signed canonical settlement facts (the
/// trade summary, event linkage and atomic settlement semantics) require
/// separate witnesses; these asset proofs establish only resulting asset
/// states and structural coverage.
///
/// # Errors
/// Returns expectation, coverage, caller-pin, commit signature, state proof,
/// ownership, event-operation, and missing-evidence errors.
pub async fn verify_trade_proof_v1(
    proof: &TradeProofV1,
    expected: &TradeSummary,
    commits: &BTreeMap<(String, String), ScopedTradeCommitV1>,
    policy: &dyn SignaturePolicy,
    now: DateTime<Utc>,
) -> Result<(), ProofError> {
    if proof.schema != TRADE_PROOF_V1_SCHEMA {
        return Err(ProofError::UnsupportedSchema(proof.schema.clone()));
    }
    if proof.trade_id != expected.trade_id {
        return Err(ProofError::TradeIdMismatch {
            expected: expected.trade_id.clone(),
            actual: proof.trade_id.clone(),
        });
    }
    crate::verify::check_proof_size(proof)?;
    if proof.summary != *expected {
        return Err(ProofError::InvalidState(String::from(
            "trade proof summary does not match the independently expected summary",
        )));
    }
    for leg in &proof.legs {
        let Some(side) = expected.sides.iter().find(|side| side.tenant == leg.tenant) else {
            return Err(ProofError::TradeMissingSide(leg.tenant.0.clone()));
        };
        if leg.state_proofs.len() != side.settle_assets.len() {
            return Err(ProofError::TradeProofTruncated {
                expected: side.settle_assets.len(),
                actual: leg.state_proofs.len(),
            });
        }
    }
    if proof.legs.len() != expected.sides.len() {
        return Err(ProofError::TradeProofTruncated {
            expected: expected.sides.len(),
            actual: proof.legs.len(),
        });
    }
    validate_unique_coverage(
        expected,
        proof.legs.iter().map(|leg| {
            (
                &leg.tenant,
                leg.state_proofs.iter().map(|witness| match witness {
                    TradeAssetStateProofV1::Legacy(p) => &p.resource_id,
                    TradeAssetStateProofV1::V1(p) => &p.resource_id,
                }),
            )
        }),
    )?;

    for leg in &proof.legs {
        let side = expected
            .sides
            .iter()
            .find(|side| side.tenant == leg.tenant)
            .ok_or_else(|| ProofError::TradeMissingSide(leg.tenant.0.clone()))?;
        for witness in &leg.state_proofs {
            let (tenant, resource, commit_id) = match witness {
                TradeAssetStateProofV1::Legacy(p) => {
                    (&p.tenant_id, &p.resource_id, &p.commit.commit_id)
                }
                TradeAssetStateProofV1::V1(p) => {
                    (&p.tenant_id, &p.resource_id, &p.commit.commit_id)
                }
            };
            if tenant != &leg.tenant {
                return Err(ProofError::TenantMismatch {
                    expected: leg.tenant.0.clone(),
                    actual: tenant.0.clone(),
                });
            }
            if !side.settle_assets.iter().any(|asset| asset == resource) {
                return Err(ProofError::ResourceMismatch {
                    expected: side.to_owner.clone(),
                    actual: resource.0.clone(),
                });
            }
            let Some(evidence) = commits.get(&(leg.tenant.0.clone(), commit_id.0.clone())) else {
                return Err(ProofError::KeyNotFound(commit_id.0.clone()));
            };
            match (witness, evidence) {
                (
                    TradeAssetStateProofV1::Legacy(p),
                    ScopedTradeCommitV1::Legacy {
                        signed,
                        verifying_key,
                    },
                ) => {
                    let key = derive_state_key(p)?;
                    if p.commit.signature != signed.signature {
                        return Err(ProofError::InvalidState(String::from(
                            "trade proof commit signature does not match independently pinned evidence",
                        )));
                    }
                    verify_bundle(p, signed, verifying_key, &key)?;
                    let owner = owner_of(&p.claimed_state)?;
                    if owner != side.to_owner {
                        return Err(ProofError::SubjectMismatch {
                            expected: side.to_owner.clone(),
                            actual: owner,
                        });
                    }
                }
                (TradeAssetStateProofV1::V1(p), ScopedTradeCommitV1::V1 { pin, authority }) => {
                    if pin.commit_id() != commit_id {
                        return Err(ProofError::CommitRefMismatch {
                            expected: pin.commit_id().0.clone(),
                            actual: commit_id.0.clone(),
                        });
                    }
                    let expected_claim = crate::claim::ExpectedClaim::ownership(
                        leg.tenant.clone(),
                        resource.clone(),
                        SubjectId(side.to_owner.clone()),
                    );
                    crate::verify::verify_resource_state_v1(
                        p,
                        &expected_claim,
                        pin,
                        authority,
                        policy,
                        now,
                    )
                    .await?;
                }
                _ => {
                    return Err(ProofError::InvalidState(String::from(
                        "proof format does not match caller-pinned commit evidence",
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Assembles a portable trade proof from a trade record and per-tenant state
/// proofs of the settled assets.
///
/// The summary is derived deterministically from the record (sides + value
/// legs); each supplied `(tenant, state_proofs)` group becomes one
/// [`TradeProofLeg`] pinning the tenant's settle commit. Groups with no state
/// proofs are omitted (a tenant that only moved fungible value contributes no
/// settle leg). Legs are emitted in sorted tenant order for deterministic
/// output.
///
/// # Errors
///
/// Returns [`ProofError::TradeMissingSide`] when a supplied proof group's
/// tenant has no corresponding settle side in the record. The assembly is
/// otherwise infallible: cryptographic validity is checked by the caller via
/// [`verify_trade_proof`].
pub fn build_trade_proof(
    record: &TradeRecord,
    proofs: Vec<(TenantId, Vec<ResourceStateProof>)>,
) -> Result<TradeProof, ProofError> {
    let summary = TradeSummary {
        trade_id: record.trade_id.clone(),
        status: record.status,
        sides: record.sides.clone(),
        value_legs: record.value_legs.clone(),
    };

    let mut legs = Vec::new();
    for (tenant, state_proofs) in proofs {
        // Each proof group must correspond to a settle side in the record; this
        // keeps the leg's commit authoritative and rejects orphan proof groups.
        if !record.sides.iter().any(|side| side.tenant == tenant) {
            return Err(ProofError::TradeMissingSide(tenant.0));
        }
        let Some(first) = state_proofs.first() else {
            continue;
        };
        legs.push(TradeProofLeg {
            tenant,
            commit: first.commit.clone(),
            state_proofs,
        });
    }
    legs.sort_by(|left, right| left.tenant.0.cmp(&right.tenant.0));

    Ok(TradeProof {
        schema: String::from(TRADE_PROOF_SCHEMA),
        trade_id: record.trade_id.clone(),
        summary,
        legs,
    })
}

/// Verifies a trade proof fail-closed.
///
/// `commits_by_id` maps each commit id string to the signed settle commit and
/// verifying key for the tenant that produced it. This compatibility form
/// cannot represent coincident commit identifiers from different tenants; use
/// [`verify_trade_proof_scoped`] for a multi-tenant verification context.
/// Each state proof is resolved
/// against **the commit its own `commit_ref` names** (a tenant may settle two
/// assets of one trade in two different commits, so each leg's proofs may pin
/// distinct commits), then verified through the existing [`verify_bundle`]
/// pipeline (schema, tenant scope, commit reference, commit signature over the
/// BCS body, sparse Merkle inclusion, claimed-state hash) under that commit's
/// tenant key, and checked structurally against the summary:
///
/// * `summary.trade_id` equals `proof.trade_id`;
/// * the proof carries exactly as many legs as the summary declares sides, and
///   each leg exactly as many state proofs as that side declares settled assets
///   ([`ProofError::TradeProofTruncated`] otherwise);
/// * summary tenants/assets and proof tenants/resources are unique, so equal
///   counts and per-resource membership establish complete structural coverage;
/// * each proven resource is one of the summary's settled assets for that
///   tenant;
/// * each settle proof's claimed owner equals the summary's `to_owner` for
///   that tenant;
/// * each settle proof's `latest_event.operation` is `trade.settle`.
///
/// The summary and event-operation metadata are not authenticated by these
/// state proofs. Equal trade IDs only establish internal agreement, not the
/// caller's expected trade. Previous owner, value legs, settle event IDs and
/// representative/per-asset summary commit references are not settlement
/// witnesses. Callers must establish that linkage independently. This API does
/// not establish current canonicality or cross-tenant atomic settlement.
///
/// # Errors
///
/// Returns [`ProofError::UnsupportedSchema`], [`ProofError::KeyNotFound`] when
/// no commit/key is supplied for a state proof's own commit,
/// [`ProofError::TradeMissingSide`] when a leg has no summary side,
/// [`ProofError::TradeProofTruncated`] when the proof carries fewer legs than
/// sides or fewer state proofs than a side's settle assets,
/// [`ProofError::InvalidState`] for duplicate tenants/assets/resources,
/// [`ProofError::TradeIdMismatch`], [`ProofError::TradeOperation`],
/// [`ProofError::ResourceMismatch`], [`ProofError::SubjectMismatch`], or every
/// [`ProofError`] variant of [`verify_bundle`], in fail-closed order.
pub fn verify_trade_proof(
    proof: &TradeProof,
    commits_by_id: &BTreeMap<String, (Signed<Commit>, VerifyingKey)>,
) -> Result<(), ProofError> {
    verify_trade_proof_with(proof, |_, commit| commits_by_id.get(commit.as_str()))
}

/// Verifies a trade's settled asset states with tenant-scoped trusted commits.
///
/// Map keys are `(tenant_id, commit_id)`, matching the commit store's identity
/// scope. Every state proof must belong to its declared trade leg's tenant.
/// Signer keys must be independently trusted by the caller. This verifies
/// signed asset state, not the authenticity of unsigned summary, operation or
/// previous-owner metadata, nor current canonicality or atomic settlement.
///
/// # Errors
///
/// Returns the same failures as [`verify_trade_proof`], including
/// [`ProofError::TenantMismatch`] for a proof moved between tenant legs.
pub fn verify_trade_proof_scoped(
    proof: &TradeProof,
    commits: &BTreeMap<(String, String), (Signed<Commit>, VerifyingKey)>,
) -> Result<(), ProofError> {
    verify_trade_proof_with(proof, |tenant, commit| {
        commits.get(&(tenant.0.clone(), commit.0.clone()))
    })
}

fn verify_trade_proof_with<'context>(
    proof: &TradeProof,
    resolve: impl Fn(&TenantId, &CommitId) -> Option<&'context (Signed<Commit>, VerifyingKey)>,
) -> Result<(), ProofError> {
    if proof.schema != TRADE_PROOF_SCHEMA {
        return Err(ProofError::UnsupportedSchema(proof.schema.clone()));
    }
    if proof.summary.trade_id != proof.trade_id {
        return Err(ProofError::TradeIdMismatch {
            expected: proof.summary.trade_id.clone(),
            actual: proof.trade_id.clone(),
        });
    }

    // Cardinality guards: a proof must carry exactly as many state proofs per
    // leg as its side declares settled assets, and exactly as many legs as the
    // summary declares settle sides. Without these, a proof whose summary
    // claims two settled assets but carries only one genuine proof (or omits a
    // whole side) would verify with a subset of the settled state. An orphan
    // leg (no matching summary side) is reported as [`ProofError::TradeMissingSide`]
    // first, so callers still distinguish a tenant-level mismatch from a
    // truncation.
    for leg in &proof.legs {
        let Some(side) = proof
            .summary
            .sides
            .iter()
            .find(|side| side.tenant == leg.tenant)
        else {
            return Err(ProofError::TradeMissingSide(leg.tenant.0.clone()));
        };
        if leg.state_proofs.len() != side.settle_assets.len() {
            return Err(ProofError::TradeProofTruncated {
                expected: side.settle_assets.len(),
                actual: leg.state_proofs.len(),
            });
        }
    }
    if proof.legs.len() != proof.summary.sides.len() {
        return Err(ProofError::TradeProofTruncated {
            expected: proof.summary.sides.len(),
            actual: proof.legs.len(),
        });
    }

    validate_unique_coverage(
        &proof.summary,
        proof.legs.iter().map(|leg| {
            (
                &leg.tenant,
                leg.state_proofs.iter().map(|witness| &witness.resource_id),
            )
        }),
    )?;

    for leg in &proof.legs {
        let Some(side) = proof
            .summary
            .sides
            .iter()
            .find(|side| side.tenant == leg.tenant)
        else {
            return Err(ProofError::TradeMissingSide(leg.tenant.0.clone()));
        };

        for state_proof in &leg.state_proofs {
            if state_proof.tenant_id != leg.tenant {
                return Err(ProofError::TenantMismatch {
                    expected: leg.tenant.0.clone(),
                    actual: state_proof.tenant_id.0.clone(),
                });
            }
            // Resolve this proof against the commit its OWN commit_ref names,
            // not the leg's representative commit: a tenant may settle two
            // assets of one trade in two different commits, so each proof must
            // be verified under the commit that pinned it.
            let Some((signed, verifying_key)) = resolve(&leg.tenant, &state_proof.commit.commit_id)
            else {
                return Err(ProofError::KeyNotFound(String::from(
                    state_proof.commit.commit_id.as_str(),
                )));
            };
            let state_key = derive_state_key(state_proof)?;
            verify_bundle(state_proof, signed, verifying_key, &state_key)?;

            // Structural checks against the summary.
            if !side
                .settle_assets
                .iter()
                .any(|asset| asset == &state_proof.resource_id)
            {
                return Err(ProofError::ResourceMismatch {
                    expected: side.to_owner.clone(),
                    actual: state_proof.resource_id.0.clone(),
                });
            }
            let owner = owner_of(&state_proof.claimed_state)?;
            if owner != side.to_owner {
                return Err(ProofError::SubjectMismatch {
                    expected: side.to_owner.clone(),
                    actual: owner,
                });
            }
            if state_proof.latest_event.operation.as_str() != "trade.settle" {
                return Err(ProofError::TradeOperation(String::from(
                    state_proof.latest_event.operation.as_str(),
                )));
            }
        }
    }
    Ok(())
}

/// Count equality plus membership only proves coverage when both declarations
/// and supplied witnesses are unique. Borrow keys to avoid cloning identifiers.
fn validate_unique_coverage<'proof>(
    summary: &TradeSummary,
    legs: impl IntoIterator<
        Item = (
            &'proof TenantId,
            impl IntoIterator<Item = &'proof statechronicle_domain::resource::ResourceId>,
        ),
    >,
) -> Result<(), ProofError> {
    let mut tenants = BTreeSet::new();
    for side in &summary.sides {
        if !tenants.insert(side.tenant.0.as_str()) {
            return Err(ProofError::InvalidState(String::from(
                "duplicate trade summary tenant",
            )));
        }
        let mut assets = BTreeSet::new();
        for asset in &side.settle_assets {
            if !assets.insert(asset) {
                return Err(ProofError::InvalidState(String::from(
                    "duplicate trade summary asset",
                )));
            }
        }
    }
    tenants.clear();
    for (tenant, witnesses) in legs {
        if !tenants.insert(tenant.0.as_str()) {
            return Err(ProofError::InvalidState(String::from(
                "duplicate trade proof tenant",
            )));
        }
        let mut resources = BTreeSet::new();
        for resource in witnesses {
            if !resources.insert(resource) {
                return Err(ProofError::InvalidState(String::from(
                    "duplicate trade proof resource",
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::shadow_unrelated
)]
mod tests {
    use super::*;
    use statechronicle_core::signature::Signature;
    use statechronicle_domain::ids::{CommitId, EventId};
    use statechronicle_domain::intent::{KeyId, Operation, SignatureAlg, SignatureBlock};
    use statechronicle_domain::proof::EventRef;
    use statechronicle_domain::proof::{CommitRef, SparseMerkleProof};
    use statechronicle_domain::resource::ResourceId;
    use statechronicle_domain::trade::{TradeSide, TradeStatus, TradeValueLeg};
    use statechronicle_ports::{
        key_registry::KeyRegistryError,
        signature_policy::{ResolvedSignaturePolicy, SignaturePolicyQuery},
    };

    struct NoPolicy;
    #[async_trait::async_trait]
    impl SignaturePolicy for NoPolicy {
        async fn resolve(
            &self,
            _query: SignaturePolicyQuery<'_>,
        ) -> Result<ResolvedSignaturePolicy, KeyRegistryError> {
            Err(KeyRegistryError::Unavailable(String::from(
                "not used by structural tests",
            )))
        }
    }

    fn commit_ref() -> CommitRef {
        CommitRef {
            commit_id: CommitId::new(String::from("cmt_00000000000000000001")).unwrap(),
            sequence: 1,
            state_root: statechronicle_core::digest::ContentDigest::new([0u8; 32]),
            signature: SignatureBlock {
                alg: SignatureAlg::Ed25519,
                key_id: KeyId::new(String::from("did:key:z6Mk...#test")).unwrap(),
                sig: Signature::from_bytes([0u8; 64]),
            },
        }
    }

    fn state_proof(tenant: &str, asset: &str) -> ResourceStateProof {
        ResourceStateProof::new(
            TenantId(String::from(tenant)),
            ResourceId(String::from(asset)),
            statechronicle_domain::resource_state::ResourceState::from_legacy_json(
                statechronicle_domain::state_type::StateType::UniqueAsset,
                serde_json::json!({
                    "owner": "account:example:player_456",
                    "status": "active",
                }),
            )
            .unwrap(),
            commit_ref(),
            SparseMerkleProof::new(
                Vec::new(),
                statechronicle_core::digest::ContentDigest::new([0u8; 32]),
            ),
            EventRef {
                event_id: EventId::new(String::from("evt_00000000000000000001")).unwrap(),
                operation: Operation::from_static("trade.settle"),
            },
            None,
        )
    }

    fn side(tenant: &str, assets: &[&str]) -> TradeSide {
        TradeSide {
            tenant: TenantId(String::from(tenant)),
            settle_assets: assets
                .iter()
                .map(|asset| ResourceId(String::from(*asset)))
                .collect(),
            from_owner: String::from("account:example:player_123"),
            to_owner: String::from("account:example:player_456"),
            settle_commit: commit_ref(),
            settle_commits_by_asset: BTreeMap::from([(
                ResourceId(String::from(assets[0])),
                commit_ref(),
            )]),
            settle_event_ids: Vec::new(),
        }
    }

    fn proof(sides: Vec<TradeSide>, legs: Vec<TradeProofLeg>) -> TradeProof {
        TradeProof {
            schema: String::from(TRADE_PROOF_SCHEMA),
            trade_id: String::from("trade_001"),
            summary: TradeSummary {
                trade_id: String::from("trade_001"),
                status: TradeStatus::Settled,
                sides,
                value_legs: Vec::<TradeValueLeg>::new(),
            },
            legs,
        }
    }

    #[tokio::test]
    async fn mixed_v1_trade_checks_independent_summary_and_scoped_pins() {
        let side = side("acme.game.alpha", &["asset:sword", "asset:shield"]);
        let summary = TradeSummary {
            trade_id: String::from("trade_001"),
            status: TradeStatus::Settled,
            sides: vec![side.clone()],
            value_legs: Vec::new(),
        };
        let legacy = state_proof("acme.game.alpha", "asset:sword");
        let v1 = statechronicle_domain::proof::ResourceStateProofV1::new(
            legacy.tenant_id.clone(),
            ResourceId(String::from("asset:shield")),
            legacy.claimed_state.clone(),
            statechronicle_domain::proof::CommitRefV1 {
                commit_id: legacy.commit.commit_id.clone(),
                sequence: legacy.commit.sequence,
                state_root: legacy.commit.state_root.clone(),
                signed_commit_record: vec![0],
            },
            legacy.state_inclusion_proof.clone(),
            legacy.latest_event.clone(),
            None,
        );
        let proof = statechronicle_domain::trade::TradeProofV1 {
            schema: String::from(TRADE_PROOF_V1_SCHEMA),
            trade_id: summary.trade_id.clone(),
            summary: summary.clone(),
            legs: vec![statechronicle_domain::trade::TradeProofLegV1 {
                tenant: side.tenant.clone(),
                state_proofs: vec![
                    TradeAssetStateProofV1::Legacy(legacy),
                    TradeAssetStateProofV1::V1(v1),
                ],
            }],
        };
        let mut wrong_summary = summary.clone();
        wrong_summary.sides[0].to_owner = String::from("account:other");
        assert!(matches!(
            verify_trade_proof_v1(
                &proof,
                &wrong_summary,
                &BTreeMap::new(),
                &NoPolicy,
                chrono::Utc::now()
            )
            .await,
            Err(ProofError::InvalidState(_))
        ));
        let missing_result = verify_trade_proof_v1(
            &proof,
            &summary,
            &BTreeMap::new(),
            &NoPolicy,
            chrono::Utc::now(),
        )
        .await;
        assert!(
            matches!(missing_result, Err(ProofError::KeyNotFound(_))),
            "{missing_result:?}"
        );
        let unrelated_scope = BTreeMap::from([(
            (
                String::from("other.tenant"),
                String::from("cmt_00000000000000000001"),
            ),
            ScopedTradeCommitV1::V1 {
                pin: crate::claim::ExpectedCommitV1::new(
                    commit_ref().commit_id,
                    statechronicle_core::digest::hash_bytes(b"record"),
                ),
                authority: SubjectId(String::from("service:authority")),
            },
        )]);
        assert!(matches!(
            verify_trade_proof_v1(
                &proof,
                &summary,
                &unrelated_scope,
                &NoPolicy,
                chrono::Utc::now()
            )
            .await,
            Err(ProofError::KeyNotFound(_))
        ));
        let mut v1_first = proof.clone();
        v1_first.legs[0].state_proofs.swap(0, 1);
        let commit_id = commit_ref().commit_id;
        let v1_evidence = BTreeMap::from([(
            (String::from("acme.game.alpha"), commit_id.0.clone()),
            ScopedTradeCommitV1::V1 {
                pin: crate::claim::ExpectedCommitV1::new(
                    commit_id,
                    statechronicle_core::digest::hash_bytes(&[0]),
                ),
                authority: SubjectId(String::from("service:authority")),
            },
        )]);
        assert!(matches!(
            verify_trade_proof_v1(
                &v1_first,
                &summary,
                &v1_evidence,
                &NoPolicy,
                chrono::Utc::now()
            )
            .await,
            Err(ProofError::CommitSignature(_)) | Err(ProofError::Core(_))
        ));
    }

    /// (a) A proof whose leg carries fewer state proofs than the side declares
    /// settled assets must be rejected as truncated.
    #[test]
    fn truncated_state_proofs_rejected() {
        let side = side("acme.game.alpha", &["asset:sword", "asset:shield"]);
        let leg = TradeProofLeg {
            tenant: TenantId(String::from("acme.game.alpha")),
            commit: commit_ref(),
            // Only one state proof for a side that declares two settled assets.
            state_proofs: vec![state_proof("acme.game.alpha", "asset:sword")],
        };
        let p = proof(vec![side], vec![leg]);
        assert!(matches!(
            verify_trade_proof(&p, &BTreeMap::new()),
            Err(ProofError::TradeProofTruncated {
                expected: 2,
                actual: 1
            })
        ));
    }

    /// (b) A proof with fewer legs than the summary declares sides must be
    /// rejected as truncated (a whole side omitted).
    #[test]
    fn truncated_legs_rejected() {
        let sides = vec![
            side("acme.game.alpha", &["asset:sword"]),
            side("acme.game.beta", &["asset:shield"]),
        ];
        // Only one leg for a summary declaring two sides.
        let leg = TradeProofLeg {
            tenant: TenantId(String::from("acme.game.alpha")),
            commit: commit_ref(),
            state_proofs: vec![state_proof("acme.game.alpha", "asset:sword")],
        };
        let p = proof(sides, vec![leg]);
        assert!(matches!(
            verify_trade_proof(&p, &BTreeMap::new()),
            Err(ProofError::TradeProofTruncated {
                expected: 2,
                actual: 1
            })
        ));
    }

    /// (c) A complete-shaped proof (legs == sides, proofs == settle_assets) is
    /// not rejected for truncation: it proceeds past the cardinality guards to
    /// verification (here failing on the missing tenant key, not on truncation).
    #[test]
    fn complete_proof_not_rejected_for_truncation() {
        let side = side("acme.game.alpha", &["asset:sword"]);
        let leg = TradeProofLeg {
            tenant: TenantId(String::from("acme.game.alpha")),
            commit: commit_ref(),
            state_proofs: vec![state_proof("acme.game.alpha", "asset:sword")],
        };
        let p = proof(vec![side], vec![leg]);
        let err = verify_trade_proof(&p, &BTreeMap::new()).unwrap_err();
        assert!(!matches!(err, ProofError::TradeProofTruncated { .. }));
    }

    #[test]
    fn trade_proof_rejects_orphan_groups_and_omits_empty_groups() {
        let record = TradeRecord {
            trade_id: String::from("trade_001"),
            status: TradeStatus::Settled,
            sides: vec![side("acme.game.alpha", &["asset:sword"])],
            events: Vec::new(),
            value_legs: Vec::new(),
        };

        let orphan = build_trade_proof(
            &record,
            vec![(TenantId(String::from("acme.game.beta")), Vec::new())],
        );
        assert!(matches!(
            orphan,
            Err(ProofError::TradeMissingSide(tenant)) if tenant == "acme.game.beta"
        ));

        let empty = build_trade_proof(
            &record,
            vec![(TenantId(String::from("acme.game.alpha")), Vec::new())],
        )
        .unwrap();
        assert!(empty.legs.is_empty());
        assert_eq!(empty.summary.trade_id, empty.trade_id);
    }

    #[test]
    fn trade_proof_rejects_schema_and_identity_mismatches_before_crypto() {
        let side = side("acme.game.alpha", &["asset:sword"]);
        let leg = TradeProofLeg {
            tenant: TenantId(String::from("acme.game.alpha")),
            commit: commit_ref(),
            state_proofs: vec![state_proof("acme.game.alpha", "asset:sword")],
        };

        let mut wrong_schema = proof(vec![side.clone()], vec![leg.clone()]);
        wrong_schema.schema = String::from("statechronicle.trade.invalid");
        assert!(matches!(
            verify_trade_proof(&wrong_schema, &BTreeMap::new()),
            Err(ProofError::UnsupportedSchema(_))
        ));

        let mut wrong_id = proof(vec![side], vec![leg]);
        wrong_id.trade_id = String::from("trade_other");
        assert!(matches!(
            verify_trade_proof(&wrong_id, &BTreeMap::new()),
            Err(ProofError::TradeIdMismatch { .. })
        ));
    }
}
