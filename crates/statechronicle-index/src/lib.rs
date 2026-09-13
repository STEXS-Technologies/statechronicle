//! The trade read-side vertical slice (Phase 2 of the trade completion).
//!
//! This crate owns the trade read side end-to-end: a pure, deterministic index
//! builder ([`build`]) that projects committed trade events and settle intents
//! into [`TradeRecord`](statechronicle_domain::trade::TradeRecord)s, history reconstruction ([`history`]), and the async
//! composition layer over the trade/event/proof/commit ports ([`service`]).
//!
//! The builder is deterministic by construction: it uses `BTreeMap`/`BTreeSet`
//! only, sorted iteration, no wall clock, no RNG, and no `HashMap` in any
//! output, so applying an event stream incrementally yields exactly the same
//! index as replaying the raw stream from scratch.

#![deny(unsafe_code)]
#![allow(clippy::must_use_candidate)]

/// The pure trade index builder.
pub mod build;

/// History reconstruction over a trade record.
pub mod history;

/// Async trade service over the read-side ports.
pub mod service;

/// Deterministic projection rebuild from canonical events.
pub mod rebuild;

/// Index and service error types.
pub mod error;
