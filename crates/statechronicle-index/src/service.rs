//! Async trade service over the read-side ports.
//!
//! [`TradeService`] is the composition layer for the trade read side: it
//! ingests committed batches into the trade index ([`Self::ingest_batch`]),
//! rebuilds the index from a raw batch stream ([`Self::rebuild`]), and serves
//! ordered history ([`Self::get_history`]) and portable trade proofs
//! ([`Self::get_proof`]). All verification is delegated to the pure builder
//! ([`crate::build`]) and the pure proof verifiers; the service itself holds no
//! logic.

use std::collections::BTreeMap;

use ed25519_dalek::VerifyingKey;
use statechronicle_domain::commit::Commit;
use statechronicle_domain::event::Event;
use statechronicle_domain::signed::Signed;
use statechronicle_domain::tenant::TenantId;
use statechronicle_domain::trade::{TradeHistory, TradeProof, TradeRecord};
use statechronicle_ports::commit_store::CommitStore;
use statechronicle_ports::event_store::EventStore;
use statechronicle_ports::proof_index::ProofIndex;
use statechronicle_ports::trade_index::TradeIndex;
use statechronicle_proof::trade::{build_trade_proof, verify_trade_proof};

use crate::build::{self, IngestBatch};
use crate::error::TradeServiceError;
use crate::history::reconstruct_history;

/// Resolves a tenant's Ed25519 verifying key (used for defense-in-depth proof
/// verification at query time).
type KeyResolver = Box<dyn Fn(&TenantId) -> Option<VerifyingKey> + Send + Sync>;

/// Backend-agnostic port set used by [`TradeService`].
///
/// Holds the driven read-side ports as `Send + Sync` trait objects so the
/// service composes in any runtime.
pub struct TradePorts {
    /// Trade record index, keyed by `trade_id`.
    pub trade_index: Box<dyn TradeIndex + Send + Sync>,
    /// Append-only event store.
    pub event_store: Box<dyn EventStore + Send + Sync>,
    /// Proof index serving state proofs at a commit.
    pub proof_index: Box<dyn ProofIndex + Send + Sync>,
    /// Append-only signed commit store.
    pub commit_store: Box<dyn CommitStore + Send + Sync>,
}

/// Async trade service over the trade read-side ports.
pub struct TradeService {
    ports: TradePorts,
    key_for_tenant: KeyResolver,
}

impl TradeService {
    /// Constructs a trade service from its ports and a per-tenant verifying-key
    /// resolver (used by [`Self::get_proof`]'s defense-in-depth verification).
    pub fn new<F>(ports: TradePorts, key_for_tenant: F) -> Self
    where
        F: Fn(&TenantId) -> Option<VerifyingKey> + Send + Sync + 'static,
    {
        Self {
            ports,
            key_for_tenant: Box::new(key_for_tenant),
        }
    }

    /// Returns the service's port set.
    pub const fn ports(&self) -> &TradePorts {
        &self.ports
    }

    /// Applies a batch to the trade index and upserts every affected record.
    ///
    /// # Errors
    ///
    /// Returns [`TradeServiceError::Index`] when the pure builder rejects the
    /// batch and [`TradeServiceError::TradeIndex`] when the index cannot be
    /// reached.
    pub async fn ingest_batch(&self, batch: &IngestBatch) -> Result<(), TradeServiceError> {
        let mut state: BTreeMap<String, TradeRecord> = BTreeMap::new();
        // Seed with any existing records for the trade ids this batch touches,
        // so incremental ingestion merges into (rather than clobbers) them.
        for trade_id in build::batch_trade_ids(batch) {
            if let Some(record) = self
                .ports
                .trade_index
                .get_trade(&trade_id)
                .await
                .map_err(|err| TradeServiceError::TradeIndex(err.to_string()))?
            {
                state.insert(trade_id, record);
            }
        }
        build::apply(&mut state, batch)?;
        for record in state.values() {
            self.ports
                .trade_index
                .put_trade(record)
                .await
                .map_err(|err| TradeServiceError::TradeIndex(err.to_string()))?;
        }
        Ok(())
    }

    /// Rebuilds the trade index from a raw batch stream and upserts every
    /// record.
    ///
    /// The event/commit ports expose no scan or replay method, so the
    /// composition root supplies the raw batches (in commit order); this replays
    /// them through the pure builder into a fresh index and upserts the result.
    /// Replaying the same stream yields exactly the same index as incremental
    /// [`Self::ingest_batch`] calls.
    ///
    /// # Errors
    ///
    /// Returns [`TradeServiceError::Index`] when the pure builder rejects a
    /// batch and [`TradeServiceError::TradeIndex`] when the index cannot be
    /// reached.
    pub async fn rebuild(&self, batches: &[IngestBatch]) -> Result<(), TradeServiceError> {
        let mut state: BTreeMap<String, TradeRecord> = BTreeMap::new();
        for batch in batches {
            build::apply(&mut state, batch)?;
        }
        for record in state.values() {
            self.ports
                .trade_index
                .put_trade(record)
                .await
                .map_err(|err| TradeServiceError::TradeIndex(err.to_string()))?;
        }
        Ok(())
    }

    /// Returns the ordered history of a trade.
    ///
    /// Resolves each recorded event through the event store (per tenant) and
    /// reconstructs the history in the record's canonical event order. Value
    /// declarations are already pre-derived into the record, so no intent-store
    /// read happens at query time.
    ///
    /// # Errors
    ///
    /// Returns [`TradeServiceError::TradeIndex`], [`TradeServiceError::EventStore`],
    /// [`TradeServiceError::MissingEvent`], or [`TradeServiceError::Index`].
    pub async fn get_history(
        &self,
        trade_id: &str,
    ) -> Result<Option<TradeHistory>, TradeServiceError> {
        let Some(record) = self
            .ports
            .trade_index
            .get_trade(trade_id)
            .await
            .map_err(|err| TradeServiceError::TradeIndex(err.to_string()))?
        else {
            return Ok(None);
        };
        let mut events_by_id: BTreeMap<(String, String), Event> = BTreeMap::new();
        for event_ref in &record.events {
            let Some(event) = self
                .ports
                .event_store
                .event_by_id(&event_ref.tenant_id, &event_ref.event_id)
                .await
                .map_err(|err| TradeServiceError::EventStore(err.to_string()))?
            else {
                return Err(TradeServiceError::MissingEvent(
                    event_ref.event_id.0.clone(),
                ));
            };
            events_by_id.insert(
                (event_ref.tenant_id.0.clone(), event_ref.event_id.0.clone()),
                event,
            );
        }
        let history = reconstruct_history(&record, &events_by_id)?;
        Ok(Some(history))
    }

    /// Returns a portable, verified trade proof for a settled trade.
    ///
    /// Fetches a state proof per settle asset per tenant at the settle commit,
    /// assembles the [`TradeProof`], then verifies it before returning (defense
    /// in depth) under each tenant's verifying key.
    ///
    /// # Errors
    ///
    /// Returns [`TradeServiceError::TradeIndex`], [`TradeServiceError::ProofIndex`],
    /// [`TradeServiceError::CommitStore`], or every [`ProofError`](statechronicle_proof::error::ProofError)
    /// variant surfaced through [`TradeServiceError::Proof`].
    pub async fn get_proof(&self, trade_id: &str) -> Result<Option<TradeProof>, TradeServiceError> {
        let Some(record) = self
            .ports
            .trade_index
            .get_trade(trade_id)
            .await
            .map_err(|err| TradeServiceError::TradeIndex(err.to_string()))?
        else {
            return Ok(None);
        };
        let mut proofs: Vec<(
            TenantId,
            Vec<statechronicle_domain::proof::ResourceStateProof>,
        )> = Vec::new();
        for side in &record.sides {
            let mut state_proofs = Vec::new();
            for asset in &side.settle_assets {
                // Fail closed: a settle asset with no stored state proof must
                // not be silently dropped from the trade proof, or the proof
                // would claim a settlement it cannot back.
                let proof = self
                    .ports
                    .proof_index
                    .get_state_proof(&side.tenant, asset, Some(&side.settle_commit.commit_id))
                    .await
                    .map_err(|err| TradeServiceError::ProofIndex(err.to_string()))?
                    .ok_or_else(|| {
                        TradeServiceError::ProofIndex(format!(
                            "no state proof stored for settle asset `{}` in tenant `{}` at commit `{}`",
                            asset.0,
                            side.tenant.0,
                            side.settle_commit.commit_id.as_str()
                        ))
                    })?;
                state_proofs.push(proof);
            }
            if !state_proofs.is_empty() {
                proofs.push((side.tenant.clone(), state_proofs));
            }
        }
        let proof = build_trade_proof(&record, proofs)?;
        self.verify_inline(&proof).await?;
        Ok(Some(proof))
    }

    /// Verifies a trade proof by loading each tenant's commit and resolving its
    /// verifying key.
    ///
    /// # Errors
    ///
    /// Returns [`TradeServiceError::CommitStore`] when a leg's commit is not
    /// stored or unreachable, [`TradeServiceError::Proof`] when verification
    /// fails or no key resolves for a tenant.
    async fn verify_inline(&self, proof: &TradeProof) -> Result<(), TradeServiceError> {
        let mut commits_by_tenant: BTreeMap<String, (Signed<Commit>, VerifyingKey)> =
            BTreeMap::new();
        for leg in &proof.legs {
            if commits_by_tenant.contains_key(&leg.tenant.0) {
                continue;
            }
            let Some(signed) = self
                .ports
                .commit_store
                .commit_by_id(&leg.tenant, &leg.commit.commit_id)
                .await
                .map_err(|err| TradeServiceError::CommitStore(err.to_string()))?
            else {
                return Err(TradeServiceError::CommitStore(format!(
                    "commit `{}` not found for tenant `{}`",
                    leg.commit.commit_id.as_str(),
                    leg.tenant.0
                )));
            };
            let Some(key) = (self.key_for_tenant)(&leg.tenant) else {
                return Err(TradeServiceError::Proof(
                    statechronicle_proof::error::ProofError::KeyNotFound(leg.tenant.0.clone()),
                ));
            };
            commits_by_tenant.insert(leg.tenant.0.clone(), (signed, key));
        }
        verify_trade_proof(proof, &commits_by_tenant)?;
        Ok(())
    }
}
