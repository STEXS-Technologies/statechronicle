//! Port trait for TrustGrant authority evaluation (ADR-003).
//!
//! The executor calls this port during the execution pipeline (§18.1 step 8)
//! and fails closed unless the evaluation result is `allow` and fresh (§18.2).
//! The evaluation itself runs behind the port, in an adapter owned by the
//! consuming platform's composition root (stexs). StateChronicle ships no
//! implementation and has no compile-time dependency on the trustgrant crate.
//!
//! The port references **only statechronicle-domain types** — it is
//! dependency-free by construction (deliberately unlike trustgrant-ports, which
//! imports its own sibling crates).

use statechronicle_domain::authority::{AuthorityProof, TrustGrantOutcome};
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;
use trait_variant::make;

/// Errors produced by the TrustGrant authority port.
#[derive(Debug, Error)]
pub enum TrustGrantError {
    /// The authority evaluation was denied.
    #[error("authority evaluation denied")]
    Denied,
    /// The authority source could not be reached or resolved.
    #[error("authority source unavailable: {0}")]
    Unavailable(String),
    /// The authority proof is stale or revoked.
    #[error("authority proof stale or revoked")]
    Stale,
}

/// Backend-agnostic TrustGrant evaluator port (no implementations in this
/// crate).
///
/// Production adapter lives in the stexs composition root; StateChronicle v0
/// uses an in-memory fake. Async via `trait_variant::make` (stexs convention).
#[make(Send)]
pub trait TrustGrantEvaluator: Sync {
    /// Evaluate whether `actor` may perform `operation` on `resource` in
    /// `scope`.
    ///
    /// Returns an outcome whose digest is bound into the event's authority
    /// block by the executor.
    ///
    /// # Errors
    ///
    /// Returns [`TrustGrantError::Denied`] when the evaluation result is not
    /// `allow`, and [`TrustGrantError::Unavailable`] when the authority source
    /// cannot be resolved.
    async fn evaluate(
        &self,
        scope: &TenantId,
        actor: &SubjectId,
        operation: &str,
        resource: &ResourceId,
    ) -> Result<TrustGrantOutcome, TrustGrantError>;

    /// Check revocation freshness for an authority proof.
    ///
    /// # Errors
    ///
    /// Returns [`TrustGrantError::Stale`] when the proof is no longer fresh.
    async fn check_revocation_freshness(
        &self,
        proof: &AuthorityProof,
    ) -> Result<(), TrustGrantError>;
}
