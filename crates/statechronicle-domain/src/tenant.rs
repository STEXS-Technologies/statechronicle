//! Tenant identity and isolation scope.
//!
//! `TenantId` roots a tenant-scoped history and determines its isolation mode
//! (hard or logical, protocol §8).

use serde::{Deserialize, Serialize};

/// Identifies an isolated StateChronicle scope (game, studio, marketplace,
/// organization, world, customer, or environment).
///
/// Referenced as `stc://<tenant_id>/<resource_type>/<resource_id>`
/// (protocol §6).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TenantId(pub String);
