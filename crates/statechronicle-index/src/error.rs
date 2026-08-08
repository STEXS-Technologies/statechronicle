//! Index and trade-service error types.
//!
//! [`IndexError`] is produced by the pure builder; [`TradeServiceError`] by the
//! async service as it maps port and builder failures into a single type.

use statechronicle_domain::error::DomainError;
use statechronicle_proof::error::ProofError;

/// Errors produced by the pure trade index builder ([`crate::build::apply`]).
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// A domain newtype (id, operation) rejected a value.
    #[error(transparent)]
    Domain(#[from] DomainError),

    /// A trade event's body carries no `trade_id`.
    #[error("trade event is missing its trade_id: {0}")]
    MissingTradeId(String),

    /// A batch commit is not tenant-scoped.
    #[error("trade batch commit is not tenant-scoped: {0}")]
    NonTenantCommit(String),

    /// A value-leg declaration or `balance.transfer` pair is malformed.
    #[error("malformed trade value leg or pair: {0}")]
    MalformedValue(String),

    /// A recorded event reference could not be resolved during history
    /// reconstruction.
    #[error("missing event `{0}` during history reconstruction")]
    MissingEvent(String),
}

/// Errors produced by the async trade service ([`crate::service::TradeService`]).
#[derive(Debug, thiserror::Error)]
pub enum TradeServiceError {
    /// The pure builder rejected a batch.
    #[error(transparent)]
    Index(#[from] IndexError),

    /// A domain newtype rejected a value.
    #[error(transparent)]
    Domain(#[from] DomainError),

    /// Proof assembly or verification failed.
    #[error(transparent)]
    Proof(#[from] ProofError),

    /// The event store could not be reached.
    #[error("event store unavailable: {0}")]
    EventStore(String),

    /// The trade index could not be reached.
    #[error("trade index unavailable: {0}")]
    TradeIndex(String),

    /// The proof index could not be reached.
    #[error("proof index unavailable: {0}")]
    ProofIndex(String),

    /// The commit store could not be reached.
    #[error("commit store unavailable: {0}")]
    CommitStore(String),

    /// A recorded event id could not be resolved from the event store.
    #[error("missing event `{0}` in event store during history reconstruction")]
    MissingEvent(String),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn index_error_display_messages() {
        assert_eq!(
            IndexError::MissingTradeId(String::from("evt_1")).to_string(),
            "trade event is missing its trade_id: evt_1"
        );
        assert_eq!(
            IndexError::MalformedValue(String::from("bad amount")).to_string(),
            "malformed trade value leg or pair: bad amount"
        );
    }

    #[test]
    fn service_error_display_messages() {
        assert_eq!(
            TradeServiceError::EventStore(String::from("down")).to_string(),
            "event store unavailable: down"
        );
        assert_eq!(
            TradeServiceError::MissingEvent(String::from("evt_9")).to_string(),
            "missing event `evt_9` in event store during history reconstruction"
        );
    }
}
