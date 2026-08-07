//! Intent parsing and validation for StateChronicle.
//!
//! Follows the trustgrant stage separation: a client submits a `RawIntent`,
//! which is parsed and validated into a `ValidatedIntent` before it enters the
//! execution pipeline.

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

/// The unvalidated `RawIntent` document as submitted by a client.
pub mod raw;

/// The validated `ValidatedIntent` produced by the validation stage.
pub mod validated;

/// Parsing of raw intent payloads.
pub mod parse;

/// Validation of parsed intents.
pub mod validate;

/// Intent processing error type.
pub mod error;
