# Native command authoring

`TypedProfile::encode_command(&command)` returns the registered operation and
canonical intent input map. Callers no longer assemble `profile`, `schema_version`
and `command` JSON fields or separately derive the operation. The adapter uses
its native profile's identity, schema, operation discriminator and bounded BCS
encoder, and rejects operations absent from its static whitelist.

```rust,ignore
let command = ConfigurationCommand::Create { enabled: true };
let (operation, inputs) = CONFIGURATION.encode_command(&command)?;
// Put these in the intent, then use authenticated preparation and atomic commit.
```

The returned pair is ordinary unsigned data, with no authority or durability
claim. Lifecycle rules require current state and therefore run during evaluation,
not encoding. A command that is encodable can still be refused. Existing ingress
continues to check the signed operation against the decoded command discriminator,
identity/schema, canonicality, bounds and state invariants. Encoding does not
replace any of those checks.

The pure-planning `custom_domains` example replaces its manual metadata/BCS and
operation assembly with one call and uses combined evaluation. It explicitly
labels its planning scope. The frozen compatibility example stays unchanged.
A new external consumer overlay compares the generated operation and exact map
with legacy assembly for both Configuration and Equipment native models. Local
fixtures cover refused-but-encodable commands, unregistered operations, zero and
undersized payload limits, and the exact accepted size boundary.

## One operation mapping per command enum

The opt-in `command_operations!` macro generates the existing discriminator
function and a `LazyLock<[Operation; N]>` from one declaration:

```rust,ignore
use statechronicle::{command_operations, Operation};
command_operations! {
    fn configuration_operation(command: &ConfigurationCommand);
    static CONFIG_OPERATIONS: Operation;
    ConfigurationCommand::Create { .. } => "configuration.create",
    ConfigurationCommand::SetEnabled { .. } => "configuration.set_enabled",
}
// In the ordinary StateProfile implementation:
fn operation(&self, command: &Self::Command) -> &'static str {
    configuration_operation(command)
}
// Existing adapter setup, unchanged:
// TypedProfile::new(Configuration, &*CONFIG_OPERATIONS)
```

This is a hosted convenience for ordinary Rust enums, not a new profile trait or
serialization format. The manual API remains supported. The existing manual
prototype repeats each operation name in a match and a lazy static list; a
function accepting those two items cannot prove that the list covers the match.
Rust has no general reflection over payload-carrying enum variants. A small
optional declarative macro removes that repetition without a proc macro,
mandatory command trait, runtime registry mutation or another adapter type.

Use explicit variant patterns with ignored payloads. Rust checks match
exhaustiveness; adding a variant then requires an operation decision. Guards and
unreachable patterns are rejected. A wildcard fallback or payload-specific
patterns can weaken the author's intended policy; the macro cannot prove that
chosen operation permissions are appropriate. It does not infer domain/schema,
validate authority, change BCS variant order or prove transition correctness.
The static list follows declaration order. Several variants may deliberately
share a name; registry and decoded command-operation checks still apply.

The existing Configuration and Equipment example now uses this mapping. Its
seven operation literals appear once each instead of twice; model semantics,
codec implementations, profile/schema identities and adapter setup are unchanged.
No heap vector is introduced: the generated fixed array has the same shape and
one-time string materialization as the former handwritten lazy array. There is
no new branch or table lookup in the adapter: the generated discriminator is an
ordinary match. This is a boilerplate reduction, not a measured speedup.

An external consumer overlay compiles the actual migrated example and compares
all nine representative command values with the frozen original models: operation
order, canonical envelope bytes, unborn evaluation results and wrong-operation
refusals. It checks unknown domain/schema and rejects empty names and reserved
operation prefixes through normal registry validation. Compile-fail doctests
cover omitted variants, guards and duplicate patterns. Existing frozen consumers
and wire fixtures remain untouched. Broader signed execution/replay/proof
composition, compile-fail ergonomics and final performance gates remain separate
roadmap work; this does not declare all F3/F4/F7 requirements complete.
