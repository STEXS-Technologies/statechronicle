//! Port trait for the trade read-side index (Phase 2).
//!
//! Stores and serves the accumulated [`TradeRecord`](statechronicle_domain::trade::TradeRecord) projections of the trade
//! read-side vertical slice, keyed by `trade_id`. Purely additive: the
//! read-side consumes events/commits through the existing event/commit/proof
//! ports and publishes its derived records through this new port. There are no
//! implementations inside this crate.

use async_trait::async_trait;
use statechronicle_domain::trade::TradeRecord;
use thiserror::Error;

/// Errors produced by the trade index port.
#[derive(Debug, Error)]
pub enum TradeIndexError {
    /// No trade record is stored for the requested id.
    #[error("no trade record available for the requested trade id")]
    NotFound,
    /// The backing index could not be reached or resolved.
    #[error("trade index unavailable: {0}")]
    Unavailable(String),
}

/// Backend-agnostic trade index port (no implementations in this crate).
///
/// The production adapter lives in the consuming platform's composition root;
/// the trade read-side ships an in-memory fake in its integration tests. Async
/// via `#[async_trait]` (boxed futures keep the port dyn-compatible).
#[async_trait]
pub trait TradeIndex: Sync + Send {
    /// Upserts a trade record by its id.
    ///
    /// # Errors
    ///
    /// Returns [`TradeIndexError::Unavailable`] when the backing index cannot
    /// be reached.
    async fn put_trade(&self, trade: &TradeRecord) -> Result<(), TradeIndexError>;

    /// Fetches a trade record by id.
    ///
    /// # Errors
    ///
    /// Returns [`TradeIndexError::NotFound`] when no record is stored and
    /// [`TradeIndexError::Unavailable`] when the backing index cannot be
    /// reached.
    async fn get_trade(&self, trade_id: &str) -> Result<Option<TradeRecord>, TradeIndexError>;
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            TradeIndexError::NotFound.to_string(),
            "no trade record available for the requested trade id"
        );
        assert_eq!(
            TradeIndexError::Unavailable(String::from("db down")).to_string(),
            "trade index unavailable: db down"
        );
    }
}
