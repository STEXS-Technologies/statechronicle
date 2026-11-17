//! Candidate-only external consumer of the exact archived protected batch API.
//! Shares public-API integration scenarios without changing frozen consumers.
mod common;
#[path = "../../../../source/crates/statechronicle/tests/durable_batch.rs"]
mod batch;
