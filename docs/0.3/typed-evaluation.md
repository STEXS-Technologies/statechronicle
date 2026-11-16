# Hosted typed transition evaluation

The executor now calls `ProfileRules::evaluate` for its adjacent profile-check
and reduction steps. The default implementation calls the existing `check` then
`apply` methods, stopping on check refusal. Existing implementations need no
changes. `ProfileEvaluationError::{Check, Apply}` preserves the failure stage:
check errors retain the executor's structured profile mapping; reducer errors
retain the legacy `TransitionInvalid` rendering. No trust gate moved.

`TypedProfile` overrides this method to decode the native command and current
state once into locally owned values. It checks operation registration, signed
operation/command equality, domain/schema, payload bounds, canonical BCS and
current-state invariants before command validation. It then reduces, validates
the new state's invariants and canonically encodes bounded output. The local
values never escape as an unchecked token, and native state/command types do
not need `Clone`. Direct `check`, direct `apply`, and historical transition
verification retain their original contracts and validation.

Removing repeated interpretation is justified by the existing `StateProfile`
contract: serialization, validation and reduction must be deterministic and
must not depend on mutable context. Between the original check and apply there
was no executor action or mutable state access. The same immutable inputs and
current state were decoded and validated twice. The combined typed path retains
one validation of the decoded current state, one command validation and one
validation of the reduced state. Stateful consumer implementations that depend
on invocation count violate that existing contract.

Two characterization tests run against unchanged original production and lockfile
before the optimization. The successful update records two command decodes, two
current-state decodes, two command validations and three state validations.
The combined update records one command decode, one current-state decode, one
command validation and two state validations. Create has no current-state decode.
A differential matrix compares states and exact failure stages with the legacy
check/apply composition for absent/current/malformed/misbound states and valid,
refused, unknown-operation, malformed, oversized and wrong-schema commands.
An independent default-rule fixture checks short-circuiting and an executor
fixture checks the two legacy error mappings. These are callback-count and
semantic tests; timing and allocation evidence must be measured separately.

This adds one compatible default trait method and a two-stage error type. It
leaves the portable `StateProfile` trait and fixed-buffer executor unchanged,
adds no dependency, and changes no wire type. Registration ergonomics, full
pipeline decomposition, adapter conformance and final qualification remain open.
