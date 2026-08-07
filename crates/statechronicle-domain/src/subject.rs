//! Subject identity for actors and services.
//!
//! `SubjectId` identifies the actor or service authorizing a transition (for
//! example the commit authority `service:statechronicle...`).

use serde::{Deserialize, Serialize};

/// Identifies a user, account, service, organization, game, device, or
/// authority (protocol §7).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubjectId(pub String);
