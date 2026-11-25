//! Deployment-owned policy resolution for versioned signatures.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use statechronicle_core::{
    signature_hybrid::{HYBRID_SIGNATURE_BYTES, HYBRID_SUITE_ID, HybridVerifier},
    signature_operation::SignatureContext,
    signature_provider::{KeySetId, SignatureError, SignaturePurpose, SignatureSuiteId, Verifier},
};
use statechronicle_domain::{intent::Operation, subject::SubjectId, tenant::TenantId};

use crate::key_registry::KeyRegistryError;

/// Subject and authority scope established independently of the signature.
#[derive(Debug, Clone, Copy)]
pub enum SignatureAuthority<'scope> {
    /// Authenticated actor requesting a tenant operation.
    ActorIntent {
        /// Tenant selected by the request's trusted context.
        tenant: &'scope TenantId,
        /// Actor authenticated by the host.
        actor: &'scope SubjectId,
        /// Operation whose authority is being checked.
        operation: &'scope Operation,
    },
    /// Configured service authority for a tenant's commit chain.
    TenantCommit {
        /// Tenant of the expected commit chain.
        tenant: &'scope TenantId,
        /// Tenant service authority selected by the host.
        authority: &'scope SubjectId,
    },
    /// Configured service authority for a global checkpoint chain.
    GlobalCheckpoint {
        /// Global checkpoint authority selected by the host.
        authority: &'scope SubjectId,
    },
}

impl SignatureAuthority<'_> {
    /// Returns the protocol purpose implied by this authority.
    pub const fn purpose(self) -> SignaturePurpose {
        match self {
            Self::ActorIntent { .. } => SignaturePurpose::ActorIntent,
            Self::TenantCommit { .. } => SignaturePurpose::TenantCommit,
            Self::GlobalCheckpoint { .. } => SignaturePurpose::GlobalCheckpoint,
        }
    }
}

/// Policy mode chosen by the protocol entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureUse {
    /// A new write or authentication request requiring current trust.
    Active,
    /// Verification of an independently pinned historical record.
    Historical,
}

/// Signature policy lookup keyed by authority, purpose, and time.
#[derive(Debug, Clone, Copy)]
pub struct SignaturePolicyQuery<'scope> {
    /// Authenticated or independently configured authority.
    pub authority: SignatureAuthority<'scope>,
    /// Whether the protocol operation requires current trust.
    pub usage: SignatureUse,
    /// Signed body timestamp, to be confirmed by successful verification.
    pub signed_at: DateTime<Utc>,
    /// Trusted clock reading or verification reference time.
    pub now: DateTime<Utc>,
}

/// The version and cryptographic binding permitted by the resolved policy.
pub enum ResolvedSignaturePolicy {
    /// Existing v0 Ed25519 policy and key-registry path.
    Legacy,
    /// Versioned verification under one selected key set and provider.
    V1(ResolvedSignatureV1),
}

/// V1 key and provider resolved together by the trusted deployment adapter.
pub struct ResolvedSignatureV1 {
    verifier: Arc<dyn Verifier>,
    suite: SignatureSuiteId,
    key_set_id: KeySetId,
    signature_len: usize,
}

impl ResolvedSignatureV1 {
    /// Builds a decision for one non-hybrid provider selected by the registry.
    ///
    /// # Errors
    /// The reserved hybrid suite requires [Self::hybrid].
    pub fn new(
        verifier: Arc<dyn Verifier>,
        key_set_id: KeySetId,
        signature_len: usize,
    ) -> Result<Self, SignatureError> {
        let suite = verifier.suite().clone();
        if suite.as_str() == HYBRID_SUITE_ID {
            return Err(SignatureError::UnsupportedSuite);
        }
        Ok(Self {
            verifier,
            suite,
            key_set_id,
            signature_len,
        })
    }

    /// Builds an ALL-required policy decision for the reserved hybrid suite.
    pub fn hybrid(verifier: Arc<HybridVerifier>, key_set_id: KeySetId) -> Self {
        Self {
            suite: verifier.suite().clone(),
            verifier,
            key_set_id,
            signature_len: HYBRID_SIGNATURE_BYTES,
        }
    }

    /// Returns the cryptographic binding for the selected typed operation.
    pub const fn context<'policy>(
        &'policy self,
        purpose: SignaturePurpose,
        body_schema: &'policy str,
    ) -> SignatureContext<'policy> {
        SignatureContext {
            suite: &self.suite,
            key_set_id: &self.key_set_id,
            purpose,
            body_schema,
            signature_len: self.signature_len,
        }
    }

    /// Returns the provider bound to the selected key set.
    pub fn verifier(&self) -> &dyn Verifier {
        self.verifier.as_ref()
    }
}

/// Resolves the authorized signature format and key set for a protocol context.
///
/// Implementations select policy without consulting the envelope suite, key ID,
/// or format. New operations require an active key at the trusted clock time
/// as well as validity at the signature timestamp. Historical operations require
/// an independently pinned record and explicit history policy. Retirement
/// never authorizes new writes, and historical revocation is denied unless the
/// deployment has explicitly allowed it. Resolution failure is terminal.
#[async_trait]
pub trait SignaturePolicy: Send + Sync {
    /// Selects the only authorized policy for a trusted protocol context.
    async fn resolve(
        &self,
        query: SignaturePolicyQuery<'_>,
    ) -> Result<ResolvedSignaturePolicy, KeyRegistryError>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use statechronicle_core::signature_provider::VerificationRequest;

    use super::*;

    struct ClaimedHybrid(SignatureSuiteId);

    impl Verifier for ClaimedHybrid {
        fn suite(&self) -> &SignatureSuiteId {
            &self.0
        }

        fn verify(&self, _request: VerificationRequest<'_>) -> Result<(), SignatureError> {
            Ok(())
        }
    }

    #[test]
    fn generic_provider_cannot_claim_the_reserved_hybrid_suite() {
        let provider = ClaimedHybrid(SignatureSuiteId::new(HYBRID_SUITE_ID).unwrap());
        assert!(matches!(
            ResolvedSignatureV1::new(
                Arc::new(provider),
                KeySetId([1; 32]),
                HYBRID_SIGNATURE_BYTES,
            ),
            Err(SignatureError::UnsupportedSuite)
        ));
    }
}
