//! Mandatory authentication and authorization boundary for mutations.
//!
//! The executor's profile authority checks are not an identity system.  A
//! game-facing composition root must authenticate a principal, bind it to the
//! intent actor, and then make an explicit allow/deny decision before calling
//! a durable ledger transaction.

use async_trait::async_trait;
use statechronicle_domain::intent::Intent;
use statechronicle_domain::resource::ResourceId;
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use thiserror::Error;

/// Authenticated identity supplied by the game session/token verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    /// Canonical actor identity.  This value, not client input, is authoritative.
    pub subject: SubjectId,
    /// Credential/session identifier for audit correlation.
    pub credential_id: String,
    /// Tenant the credential is scoped to.
    pub tenant: TenantId,
}

/// Typed authorization request bound to all security-sensitive dimensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationContext<'context> {
    /// Authenticated principal.
    pub principal: &'context AuthenticatedPrincipal,
    /// Tenant being mutated.
    pub tenant: &'context TenantId,
    /// Intent whose actor/resource/operation must be checked.
    pub intent: &'context Intent,
    /// Resource targeted by the mutation.
    pub resource: &'context ResourceId,
}

/// Authorization failure.  Implementations should avoid exposing policy
/// details to untrusted callers while retaining a correlation id in logs.
#[derive(Debug, Error)]
pub enum AuthorizationError {
    /// The principal is not authenticated or has expired.
    #[error("authentication failed")]
    Unauthenticated,
    /// The principal is authenticated but not allowed for this mutation.
    #[error("authorization denied")]
    Denied,
    /// The policy service could not produce a safe decision.
    #[error("authorization service unavailable: {0}")]
    Unavailable(String),
    /// The request contains an identity or scope mismatch.
    #[error("authorization context mismatch: {0}")]
    ContextMismatch(String),
}

/// Mandatory default-deny authorization port.
#[async_trait]
pub trait Authorizer: Send + Sync {
    /// Authorize one intent after authenticating the principal.
    async fn authorize(&self, context: AuthorizationContext<'_>) -> Result<(), AuthorizationError>;
}

/// Safe default for compositions that have not installed a policy engine.
/// Every mutation is rejected until an explicit authorizer is wired.
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyAllAuthorizer;

#[async_trait]
impl Authorizer for DenyAllAuthorizer {
    async fn authorize(
        &self,
        _context: AuthorizationContext<'_>,
    ) -> Result<(), AuthorizationError> {
        Err(AuthorizationError::Denied)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deny_all_is_fail_closed() {
        let authorizer = DenyAllAuthorizer;
        let principal = AuthenticatedPrincipal {
            subject: SubjectId(String::from("account:player")),
            credential_id: String::from("session-1"),
            tenant: TenantId(String::from("game")),
        };
        let intent = Intent::new(
            principal.tenant.clone(),
            statechronicle_domain::ids::IntentId::new(String::from(
                "int_01JZ8WJ1V6MJ6Y3Z6Z9CA8B2K2",
            ))
            .unwrap(),
            statechronicle_domain::intent::Operation::from_static("asset.transfer"),
            principal.subject.clone(),
            ResourceId(String::from("asset:sword")),
            Some(statechronicle_domain::state_type::StateType::UniqueAsset),
            0,
            std::collections::BTreeMap::new(),
            None,
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            None,
            statechronicle_domain::intent::Nonce::from_bytes(vec![1]).unwrap(),
        );
        let denied = authorizer
            .authorize(AuthorizationContext {
                principal: &principal,
                tenant: &principal.tenant,
                resource: &intent.resource_id,
                intent: &intent,
            })
            .await;
        assert!(matches!(denied, Err(AuthorizationError::Denied)));
    }
}
