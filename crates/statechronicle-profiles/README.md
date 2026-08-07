# statechronicle-profiles

## What it is

The baseline resource profiles and their rule sets: unique assets, paid unique
assets, consumable stacks, fungible balances, entitlements, meters, and
marketplace listings/escrow. `ProfileRules` is the single gate every state
transition passes through.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §20 | Profiles | Profile-based specialization of transition rules |
| §20.1 | Baseline Resource Profile | Shared rule baseline for all state types |
| §20.2 | Unique Asset Profile | Singleton resources with ownership rules |
| §20.3 | Paid Unique Asset Profile | Durable paid ownership, no-hard-delete invariants |
| §20.4 | Consumable Stack Profile | Stacked quantities consumed by use |
| §20.5 | Fungible Balance Profile | Divisible balances with atomic debit + credit transfer |
| §20.6 | Entitlement Profile | Grantable rights scoped to a subject |
| §20.7 | Meter Profile | Usage meters accrued and settled against entitlements |
| §20.9 | Economy and Marketplace Profile | Listings and escrow for atomic purchase settlement |

## Key types

- `registry::{ProfileRegistry, ProfileRules}`: profile resolution and the
  transition gate.
- `unique_asset::UniqueAssetRules`.
- `paid_unique_asset` (overlay over unique asset).
- `consumable_stack`, `fungible_balance`, `entitlement`, `meter`, `marketplace`.
- `error`: profile rule error type.

## How it's used

`ProfileRegistry::baseline()` registers every built-in profile by state type.
The executor resolves the rule set for an intent's state type and runs every
transition through it.

```rust
use statechronicle_profiles::registry::ProfileRegistry;
let registry = ProfileRegistry::baseline();
```

## Dependencies

`statechronicle-core`, `statechronicle-domain`, `serde_json`. Dev-only:
`proptest`.

## Tests

`tests/`: `registry.rs`, `property.rs`, plus inline unit tests for each
profile's transition table and fail-closed invariants.

## Where it fits

A pure rules layer consumed by `statechronicle-executor` at runtime. The
umbrella crate re-exports `ProfileRegistry` and `ProfileRules`.
