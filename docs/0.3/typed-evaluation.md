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

## Matched performance checkpoint

`evidence/f3/typed-performance/20261116T045923Z-3200086/` measures exact
candidate `15aff1b` (production identical to `d1550a9`) against `63b9a47`.
The control's complete typed adapter source equals frozen original `c418e98b`.
Both builds use Rust 1.100, byte-identical source and resolved consumer lockfiles,
CPU 0, the frozen Configuration model, identical inputs and result assertions.
The benchmark alone substitutes legacy check/apply for the unavailable combined
call in the control. No production or frozen consumer file is patched.

Seven alternating rounds retain all raw samples for each of three lanes:
control legacy, candidate legacy and candidate combined. Each process collects
31 averages of 10,000 operations per case, after warmup. The candidate's retained
legacy path is a useful additional control; its allocation totals match exactly.

| Workload | Combined vs control median paired change | Absolute change | Paired p95 change |
| --- | ---: | ---: | ---: |
| Create configuration | -38.25% | -150.66 ns | -38.77% |
| Update configuration | -42.37% | -215.41 ns | -30.21% |
| Malformed command refusal | +0.75% | +1.03 ns | +1.66% |

Within the candidate binary, combined versus legacy median paired changes are
-40.68%, -48.31% and -2.82% respectively. Candidate legacy versus control legacy
changes by +4.39%, +1.25% and +1.90%; shared-host variability remains visible.
These figures cover a fixed small model, including result assertions; they are
not a universal speedup and do not qualify the complete F8 workload matrix.
The model's own reducer still calls its own validation, as before.

Valgrind observes 1,000 operations per case, with identical setup and fixed-width
arguments in matched no-action controls. Successful create saves **7 allocations
and 131 bytes per operation**; update saves **10 allocations and 166 bytes**;
malformed command allocation totals are unchanged. Control-subtracted raw totals
retain a fixed two-allocation/45-byte residual over each 1,000-operation process,
so the JSON reports fractional per-operation totals rather than rounding them
away. Exact legacy/combined differences above cancel that fixed residual.
All profiler runs pass with memory errors configured to fail the command.
This hosted measurement says nothing new about portable allocation guarantees;
the independently qualified portable path is unchanged.

Reproduce with `RUSTC_WRAPPER= CARGO_BUILD_JOBS=8 python3 scripts/refactor_typed_bench.py`
on the desired committed candidate, with locally installed offline dependencies
and Valgrind. The runner archives source, forbids dirty production, checks full
resolved-lock equality, records binary/source/fixture hashes and saves every
failed command before stopping. The exact runner used here is retained beside
the raw evidence. The benchmark fixture is separate from frozen compatibility
fixtures under `ci/performance/`.
