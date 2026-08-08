//! Shared wire-map input keys.
//!
//! The opaque string keys used in the profile-defined `inputs` map and in
//! projected state payloads (protocol §11.1, §20). These are deliberately
//! **plain string constants, not newtypes**: they are wire-format map keys that
//! must stay byte-identical to the protocol literals, and profiles read and
//! write them through [`crate::registry::input_str`] /
//! [`crate::registry::state_str`]. Grouping them here removes literal drift and
//! lets the executor reference the same keys as the profiles.

/// The current owner/actor input key.
pub const FROM_OWNER: &str = "from_owner";
/// The new owner input key.
pub const TO_OWNER: &str = "to_owner";
/// The destination subject input key.
pub const TO_SUBJECT: &str = "to_subject";
/// The owner-consent boolean input key.
pub const AUTHORIZED_BY_OWNER: &str = "authorized_by_owner";
/// The pending trade identifier input key.
pub const TRADE_ID: &str = "trade_id";
/// The stack quantity input/payload key.
pub const QUANTITY: &str = "quantity";
/// The fungible/meter amount input key.
pub const AMOUNT: &str = "amount";
/// The denomination input/payload key.
pub const UNIT: &str = "unit";
/// The fungible balance payload key.
pub const BALANCE: &str = "balance";
/// The subject payload/input key.
pub const SUBJECT: &str = "subject";
/// The marketplace buyer input key.
pub const BUYER: &str = "buyer";
/// The marketplace seller input key.
pub const SELLER: &str = "seller";
/// The transferable flag payload key.
pub const TRANSFERABLE: &str = "transferable";
/// The status payload key.
pub const STATUS: &str = "status";
/// The meter remaining payload key.
pub const REMAINING: &str = "remaining";
/// The meter maximum payload key.
pub const MAXIMUM: &str = "maximum";
/// The conversion target denomination input key.
pub const TO_UNIT: &str = "to_unit";
