//! Candidate-only external consumer. Reuse the exact archived candidate's
//! public-API integration scenarios and adapter without duplicating fixtures.
//! This relative path matches refactor_compat.py's isolated source/consumer tree.
mod common;
#[path = "../../../../source/crates/statechronicle/tests/durable_recovery.rs"]
mod recovery;
