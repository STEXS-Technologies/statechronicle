//! Resource identity.
//!
//! `ResourceId` identifies a resource within a tenant's namespace (protocol §6).

use serde::{Deserialize, Serialize};

/// Identifies a resource (asset, balance, stack, entitlement, meter, listing,
/// custody record, ...) within a tenant's namespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceId(pub String);
