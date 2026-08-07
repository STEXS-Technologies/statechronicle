//! Accumulator error type.
//!
//! Failures from checkpoint construction, built with `thiserror`. The
//! per-tenant sparse Merkle tree itself is total. Its API never errors.

use thiserror::Error;

/// Errors reported by accumulator composition.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AccumulatorError {
    /// A checkpoint root was requested with no tenant roots.
    #[error("no tenant roots provided; a checkpoint requires at least one (tenant_id, root) pair")]
    EmptyTenantRoots,

    /// The same tenant appeared more than once in a checkpoint input.
    #[error("duplicate tenant root entry for `{0}`")]
    DuplicateTenant(String),
}
