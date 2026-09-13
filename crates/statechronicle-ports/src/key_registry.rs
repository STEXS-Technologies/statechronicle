//! Tenant-scoped signing-key lifecycle and trust-resolution port.
//!
//! Cryptographic signature verification alone is insufficient for a game
//! backend: a key must also be owned by the authenticated actor, scoped to the
//! tenant/operation, inside its validity window, and not revoked.  This port
//! keeps that policy out of the protocol and lets a KMS/HSM-backed composition
//! root provide the authoritative registry.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use statechronicle_domain::intent::{KeyId, Operation};
use statechronicle_domain::subject::SubjectId;
use statechronicle_domain::tenant::TenantId;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

/// Lifecycle state of a registered key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    /// Key may authenticate new requests within its validity interval.
    Active,
    /// Key is retained for historical verification but cannot authenticate
    /// new requests.
    Retired,
    /// Key is suspected compromised and must be rejected everywhere policy
    /// requires current trust.
    Revoked,
}

/// Metadata needed to bind a key to an actor and policy scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRecord {
    /// Stable key identifier carried by the detached signature.
    pub key_id: KeyId,
    /// Tenant that owns the key.
    pub tenant: TenantId,
    /// Actor to which the key is bound, if it is an actor key.
    pub subject: Option<SubjectId>,
    /// Operations permitted for this key. Empty means no operation is
    /// permitted; callers must never interpret it as wildcard.
    pub operations: Vec<Operation>,
    /// Earliest time at which the key can be used.
    pub valid_from: DateTime<Utc>,
    /// Latest time at which the key can be used.
    pub valid_until: Option<DateTime<Utc>>,
    /// Current lifecycle state.
    pub status: KeyStatus,
}

impl KeyRecord {
    /// Returns whether this record is valid for one actor/operation/time.
    pub fn permits(
        &self,
        tenant: &TenantId,
        subject: &SubjectId,
        operation: &Operation,
        at: DateTime<Utc>,
    ) -> bool {
        self.tenant == *tenant
            && self.subject.as_ref() == Some(subject)
            && self.status == KeyStatus::Active
            && at >= self.valid_from
            && self.valid_until.is_none_or(|until| at < until)
            && self.operations.iter().any(|allowed| allowed == operation)
    }
}

/// Errors returned by a key registry.
#[derive(Debug, Error)]
pub enum KeyRegistryError {
    /// Registry has no record for the requested key.
    #[error("key not found")]
    NotFound,
    /// Key exists but cannot be used for this request.
    #[error("key is not trusted for this context")]
    NotTrusted,
    /// Registry/KMS is unavailable; callers must fail closed.
    #[error("key registry unavailable: {0}")]
    Unavailable(String),
}

/// Authoritative key registry supplied by the deployment.
#[async_trait]
pub trait KeyRegistry: Send + Sync {
    /// Resolves and policy-checks an actor key for a new intent.
    async fn resolve_intent_key(
        &self,
        tenant: &TenantId,
        subject: &SubjectId,
        key_id: &KeyId,
        operation: &Operation,
        at: DateTime<Utc>,
    ) -> Result<KeyRecord, KeyRegistryError>;

    /// Checks whether a commit-signing key is trusted at a historical time.
    /// Historical verification may permit retired keys but must still reject
    /// unknown or revoked keys according to deployment policy.
    async fn resolve_commit_key(
        &self,
        tenant: &TenantId,
        key_id: &KeyId,
        at: DateTime<Utc>,
    ) -> Result<KeyRecord, KeyRegistryError>;
}

/// Thread-safe in-process key metadata registry for development and
/// controlled single-process deployments.
///
/// This type stores trust metadata only; cryptographic public-key material is
/// resolved separately by the caller's verifier. Production deployments
/// should back the same [`KeyRegistry`] contract with an auditable KMS/HSM
/// registry. The lifecycle methods are atomic and refuse accidental duplicate
/// registration or invalid validity windows.
#[derive(Clone, Default)]
pub struct MemoryKeyRegistry {
    records: Arc<RwLock<BTreeMap<String, KeyRecord>>>,
}

impl MemoryKeyRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one active/retired key record.
    ///
    /// # Errors
    ///
    /// Returns [`KeyRegistryError::NotTrusted`] for malformed metadata or a
    /// duplicate key ID, and [`KeyRegistryError::Unavailable`] if the lock is
    /// poisoned.
    pub fn register(&self, record: KeyRecord) -> Result<(), KeyRegistryError> {
        if record.tenant.0.is_empty()
            || (record.subject.is_some() && record.operations.is_empty())
            || record
                .valid_until
                .is_some_and(|until| until <= record.valid_from)
            || record.status == KeyStatus::Revoked
        {
            return Err(KeyRegistryError::NotTrusted);
        }
        let mut records = self.records.write().map_err(|_poisoned| {
            KeyRegistryError::Unavailable(String::from("registry lock poisoned"))
        })?;
        if records.contains_key(record.key_id.as_str()) {
            return Err(KeyRegistryError::NotTrusted);
        }
        records.insert(record.key_id.as_str().to_owned(), record);
        Ok(())
    }

    /// Marks a key retired. Retired keys remain available for historical
    /// commit verification but cannot authenticate new intents.
    ///
    /// # Errors
    ///
    /// Returns [`KeyRegistryError::NotFound`] when no such key exists.
    pub fn retire(&self, key_id: &KeyId) -> Result<(), KeyRegistryError> {
        self.set_status(key_id, KeyStatus::Retired)
    }

    /// Revokes a key. Revocation is terminal for this in-process registry.
    ///
    /// # Errors
    ///
    /// Returns [`KeyRegistryError::NotFound`] when no such key exists.
    pub fn revoke(&self, key_id: &KeyId) -> Result<(), KeyRegistryError> {
        self.set_status(key_id, KeyStatus::Revoked)
    }

    /// Returns a metadata snapshot for operator/audit tooling.
    ///
    /// # Errors
    ///
    /// Returns [`KeyRegistryError::Unavailable`] if the registry lock is
    /// poisoned.
    pub fn get(&self, key_id: &KeyId) -> Result<Option<KeyRecord>, KeyRegistryError> {
        let records = self.records.read().map_err(|_poisoned| {
            KeyRegistryError::Unavailable(String::from("registry lock poisoned"))
        })?;
        Ok(records.get(key_id.as_str()).cloned())
    }

    fn set_status(&self, key_id: &KeyId, status: KeyStatus) -> Result<(), KeyRegistryError> {
        let mut records = self.records.write().map_err(|_poisoned| {
            KeyRegistryError::Unavailable(String::from("registry lock poisoned"))
        })?;
        let record = records
            .get_mut(key_id.as_str())
            .ok_or(KeyRegistryError::NotFound)?;
        if record.status == KeyStatus::Revoked && status != KeyStatus::Revoked {
            // Revocation is terminal. A stale operator or compromised control
            // plane must not be able to downgrade a revoked key back to a
            // historically trusted state.
            return Err(KeyRegistryError::NotTrusted);
        }
        record.status = status;
        Ok(())
    }
}

#[async_trait]
impl KeyRegistry for MemoryKeyRegistry {
    async fn resolve_intent_key(
        &self,
        tenant: &TenantId,
        subject: &SubjectId,
        key_id: &KeyId,
        operation: &Operation,
        at: DateTime<Utc>,
    ) -> Result<KeyRecord, KeyRegistryError> {
        let record = self.get(key_id)?.ok_or(KeyRegistryError::NotFound)?;
        if record.permits(tenant, subject, operation, at) {
            Ok(record)
        } else {
            Err(KeyRegistryError::NotTrusted)
        }
    }

    async fn resolve_commit_key(
        &self,
        tenant: &TenantId,
        key_id: &KeyId,
        at: DateTime<Utc>,
    ) -> Result<KeyRecord, KeyRegistryError> {
        let record = self.get(key_id)?.ok_or(KeyRegistryError::NotFound)?;
        let valid_window =
            at >= record.valid_from && record.valid_until.is_none_or(|until| at < until);
        if record.tenant == *tenant && valid_window && record.status != KeyStatus::Revoked {
            Ok(record)
        } else {
            Err(KeyRegistryError::NotTrusted)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn permits_requires_exact_scope_and_active_window() {
        let tenant = TenantId(String::from("game"));
        let subject = SubjectId(String::from("account:alice"));
        let operation = Operation::from_static("asset.transfer");
        let record = KeyRecord {
            key_id: KeyId::new(String::from("key_01JZ8X2XRE5ZYW5V9R7VDQBSH4")).unwrap(),
            tenant: tenant.clone(),
            subject: Some(subject.clone()),
            operations: vec![operation.clone()],
            valid_from: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            valid_until: None,
            status: KeyStatus::Active,
        };
        let at = DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(record.permits(&tenant, &subject, &operation, at));
        assert!(!record.permits(&TenantId(String::from("other")), &subject, &operation, at));
    }

    #[tokio::test]
    async fn memory_registry_rotates_and_revokes_keys() {
        let registry = MemoryKeyRegistry::new();
        let tenant = TenantId(String::from("game"));
        let subject = SubjectId(String::from("account:alice"));
        let operation = Operation::from_static("asset.transfer");
        let key_id = KeyId::new(String::from("key_01JZ8X2XRE5ZYW5V9R7VDQBSH5")).unwrap();
        let record = KeyRecord {
            key_id: key_id.clone(),
            tenant: tenant.clone(),
            subject: Some(subject.clone()),
            operations: vec![operation.clone()],
            valid_from: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            valid_until: None,
            status: KeyStatus::Active,
        };
        registry.register(record).unwrap();
        let at = DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(
            registry
                .resolve_intent_key(&tenant, &subject, &key_id, &operation, at)
                .await
                .is_ok()
        );
        registry.retire(&key_id).unwrap();
        assert!(matches!(
            registry
                .resolve_intent_key(&tenant, &subject, &key_id, &operation, at)
                .await,
            Err(KeyRegistryError::NotTrusted)
        ));
        assert!(
            registry
                .resolve_commit_key(&tenant, &key_id, at)
                .await
                .is_ok()
        );
        registry.revoke(&key_id).unwrap();
        assert!(matches!(
            registry.resolve_commit_key(&tenant, &key_id, at).await,
            Err(KeyRegistryError::NotTrusted)
        ));
        assert!(matches!(
            registry.retire(&key_id),
            Err(KeyRegistryError::NotTrusted)
        ));
        assert_eq!(
            registry.get(&key_id).unwrap().unwrap().status,
            KeyStatus::Revoked
        );
    }

    #[tokio::test]
    async fn memory_registry_accepts_scoped_commit_key_without_operations() {
        let registry = MemoryKeyRegistry::new();
        let tenant = TenantId(String::from("game"));
        let key_id = KeyId::new(String::from("key_01JZ8X2XRE5ZYW5V9R7VDQBSH6")).unwrap();
        registry
            .register(KeyRecord {
                key_id: key_id.clone(),
                tenant: tenant.clone(),
                subject: None,
                operations: Vec::new(),
                valid_from: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                valid_until: None,
                status: KeyStatus::Active,
            })
            .unwrap();
        let at = DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(
            registry
                .resolve_commit_key(&tenant, &key_id, at)
                .await
                .is_ok()
        );
        assert!(matches!(
            registry
                .resolve_intent_key(
                    &tenant,
                    &SubjectId(String::from("account:alice")),
                    &key_id,
                    &Operation::from_static("asset.transfer"),
                    at,
                )
                .await,
            Err(KeyRegistryError::NotTrusted)
        ));
    }
}
