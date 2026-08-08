//! Trade proof assembly and verification (Phase 2 of the trade completion).
//!
//! A trade proof ties a settled trade's per-tenant state proofs together. There
//! is deliberately **no cross-tenant root**: each settled asset is proven by a
//! per-tenant [`ResourceStateProof`] under that tenant's own commit and
//! verifying key, and `trade_id` is the semantic binding that a caller uses to
//! join the legs. This keeps every leg independently verifiable with the
//! existing single-tenant [`crate::verify::verify_bundle`] pipeline while
//! leaving the cross-leg join to the caller's trust of the `trade_id` linkage.
//!
//! [`build_trade_proof`] is pure and assembling; [`verify_trade_proof`] is pure
//! and fail-closed.

use std::collections::BTreeMap;

use ed25519_dalek::VerifyingKey;
use statechronicle_domain::commit::Commit;
use statechronicle_domain::proof::ResourceStateProof;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::tenant::TenantId;
use statechronicle_domain::trade::{
    TRADE_PROOF_SCHEMA, TradeProof, TradeProofLeg, TradeRecord, TradeSummary,
};

use crate::bundle::{derive_state_key, owner_of};
use crate::error::ProofError;
use crate::verify::verify_bundle;

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
/// `commits_by_tenant` maps each tenant id string to the signed settle commit
/// and verifying key for that tenant. For every leg, each state proof is
/// verified through the existing [`verify_bundle`] pipeline (schema, tenant
/// scope, commit reference, commit signature over the BCS body, sparse Merkle
/// inclusion, claimed-state hash) under that tenant's key, then checked
/// structurally against the summary:
///
/// * `summary.trade_id` equals `proof.trade_id`;
/// * the proof carries exactly as many legs as the summary declares sides, and
///   each leg exactly as many state proofs as that side declares settled assets
///   ([`ProofError::TradeProofTruncated`] otherwise);
/// * each proven resource is one of the summary's settled assets for that
///   tenant;
/// * each settle proof's claimed owner equals the summary's `to_owner` for
///   that tenant;
/// * each settle proof's `latest_event.operation` is `trade.settle`.
///
/// # Errors
///
/// Returns [`ProofError::UnsupportedSchema`], [`ProofError::KeyNotFound`] when
/// no commit/key is supplied for a leg's tenant, [`ProofError::TradeMissingSide`]
/// when a leg has no summary side, [`ProofError::TradeProofTruncated`] when the
/// proof carries fewer legs than sides or fewer state proofs than a side's
/// settle assets, [`ProofError::TradeIdMismatch`],
/// [`ProofError::TradeOperation`], [`ProofError::ResourceMismatch`],
/// [`ProofError::SubjectMismatch`], or every [`ProofError`] variant of
/// [`verify_bundle`], in fail-closed order.
pub fn verify_trade_proof(
    proof: &TradeProof,
    commits_by_tenant: &BTreeMap<String, (Signed<Commit>, VerifyingKey)>,
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

    for leg in &proof.legs {
        let Some((signed, verifying_key)) = commits_by_tenant.get(&leg.tenant.0) else {
            return Err(ProofError::KeyNotFound(leg.tenant.0.clone()));
        };
        let Some(side) = proof
            .summary
            .sides
            .iter()
            .find(|side| side.tenant == leg.tenant)
        else {
            return Err(ProofError::TradeMissingSide(leg.tenant.0.clone()));
        };

        for state_proof in &leg.state_proofs {
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
            serde_json::json!({
                "owner": "account:example:player_456",
                "status": "active",
            }),
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
}
