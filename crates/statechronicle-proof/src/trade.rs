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
/// when a leg has no summary side, [`ProofError::TradeIdMismatch`],
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
