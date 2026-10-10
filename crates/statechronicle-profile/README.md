# statechronicle-profile

Allocation-free `no_std` transition contracts for consumer-defined StateChronicle domains.

Implement `StateProfile` with native state and command types, a stable domain ID,
schema version, maximum payload size, command-to-operation binding, validation,
reduction, and state invariants. Implementations must be deterministic and have
no external side effects. The contract has no dependencies and needs no allocator.

The `statechronicle-profiles::custom::TypedProfile` adapter connects this portable
contract to the standard execution, persistence, signed-commit, and proof APIs.
The standard adapter uses Serde and BCS and requires `std`.

See [domain extension and migration guide](../../docs/DESIGN/DOMAIN_EXTENSIONS.md).
