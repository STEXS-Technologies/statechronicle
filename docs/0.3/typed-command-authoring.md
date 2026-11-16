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

Static operation registration remains explicit. This helper removes repeated
wire assembly, not the separate registration/discriminator declarations. Choosing
an exhaustive typed registration design still requires a useful manual prototype
and a before/after consumer comparison; no macro, mutable registry, new portable
trait requirement or serialization schema is introduced here.
