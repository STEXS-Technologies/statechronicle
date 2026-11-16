# Private checked profile preparation stages

Retained production: `4c55fe681d75d2d5f0868f27cd0d2663d964eba3`.
Control: `04dd9461e2c87e2a98fb34ce5210f83e7679aeb2`.
Prototype: `a52d53fcc5c8710f3ddc21cecce77f11bd308dff`.
This directory retains the prototype's source identifier in its name; the
retained candidate is the corrected `4c55fe6` revision above.

## Ownership change

`evaluation::PreparedProfile` has private fields and one constructor. It keeps
the immutable intent reference, resolved static rules/policy, state type and
owned observed projection together after tenant, route, addressing, state-type,
version, access and custom identity/schema gates pass. It does not contain a
signature, principal, marker capability or durable receipt.

Its authority method preserves evaluator order and error precedence. Its reducer
uses the same checked state/profile and produces existing `PreparedEventStates`;
its consuming event method performs primary and companion construction. The
common `Evaluation::execute_inner` retains envelope/crypto/client-freshness and
legacy marker policy. Both Executor and the read-only Preparer use that owner;
coordinators retain their explicit group/atomicity checks.

No policy registration, crypto, wire, public API, dependency or durability change
is included. The context is a private record of checked profile facts, not an
authentication seal or a new transaction authority. Existing clock and event-ID
provenance and before-hash behavior are preserved.

The first prototype returned a vector from the event stage. Source review found
that this moved vector allocation before the compatibility marker claim. The
retained version returns an event/optional-companion pair and allocates the result
vector after the claim, preserving original order. Prototype measurements and
successful tests remain recorded; they do not qualify the retained revision.
An initial development Clippy run found two collapsible ifs and a redundant
return after the split; these were corrected before the first committed checks.

## Qualification

The [final three-lane matrix](../executor-order/20261116T112806Z-1498271/manifest.json)
compiles actual original release `c418e98`, control and retained candidate
independently, then compares all 23,544 rows exactly. Each full trace SHA-256 is
`b7853af8b100d4761755f2e7277a9181ebeb4acef184464b65afc9b53f7d89d1`.
Errors, canonical events, marker claims, callbacks and IDs match. Strict consumer
Clippy and unchanged source/locks pass in all lanes. The [prototype matrix](
../executor-order/20261116T112312Z-1301926/manifest.json) independently passes too.
The original expiry negative controls are documented with the underlying fixture
in [executor-order/README.md](../executor-order/README.md).

[Exact retained qualification](qualification-4c55fe6/manifest.json) passes
formatting, **884 finite tests, 16 doctests**, all-target strict Clippy (including
fuzz compilation), strict rustdoc, whitespace and guarded-source identity.
The [external consumer qualification](../../compat/20261116T112923Z-1550149/manifest.json)
passes original frozen tests, portable tests and bare-metal compilation. The
[prototype qualification](qualification/manifest.json) and [prototype external
checks](../../compat/20261116T112518Z-1387072/manifest.json) are separate.
No unbounded fuzz test, physical crash or exhaustive state space is claimed.

## Footprint and measurements

Pinned Python 3.13/tree-sitter 0.25.2/tree-sitter-rust 0.24.2 metrics and their
runner are retained beside this file. Retained whole-production changes versus
control: **+2,374 raw bytes, +65 syntax lines, +374 tokens, +1 parsed branch**.
The three additional public-syntax items have `pub(super)` visibility inside a
private module; they are not exported API. The common `execute_inner` body moves
from 1,791 to 367 tokens; four focused methods contain the moved decisions.
Whole-production totals are 1,504,215 raw bytes, 16,601 syntax lines, 121,626
tokens, 1,985 parsed branches and 1,448 public-syntax items. These are parser
counts, not executable complexity or macro-expanded API. This is an ownership
improvement with modest source growth, not F6 source-reduction credit.

The [retained matched benchmark](bench-4c55fe6-20261116T112823Z/manifest.json)
uses byte-identical frozen benchmark source, compiler and resolved locks, seven
alternating control/candidate pairs and fixed CPU affinity. The three legacy
workflows include adapter setup; the batch sink intentionally refuses persistence.

| Workload | Median paired median change | Median paired p95 change |
| --- | ---: | ---: |
| Authenticated single preparation | +0.185% / +70.4 ns | +2.036% |
| Two-intent authenticated batch to refusing sink | +0.461% / +366.6 ns | +2.085% |
| Two-intent planning batch | +0.121% / +92.2 ns | -0.168% |

Shared-host pair ranges are wide and signed: -0.79%…+34.89%,
-18.18%…+28.42% and -14.55%…+1.08%, respectively. Other qualification jobs were
active on the same host. These observations establish neither a speedup nor
universal runtime neutrality or the complete performance gate. The retained
consumer binary grows from 4,079,112 to 4,088,576 bytes (+9,464, about 0.232%);
source/archive and binary hashes are recorded. Build paths differ between lanes.
All observations, including outliers, remain in the summary and compressed logs.

[Valgrind totals](allocations-4c55fe6/manifest.json) match exactly for all three
full bounded processes: 101,835 allocations / 8,401,657 bytes; 130,381 /
10,449,594; and 122,625 / 9,638,986, respectively. This includes fixed startup,
setup, ten warmups and 31 samples of five actions; it is not a per-operation or
arbitrary-input allocation guarantee. Copied executable paths have equal length,
and binaries match the benchmark hashes. The [prototype benchmark](
bench-a52d53f-20261116T112429Z/manifest.json) and [prototype allocations](
allocations/manifest.json) are retained separately.

Full F2 still needs the remaining mutation-entry and settlement composition
review and broader ordering evidence. F6 footprint and F8 final performance,
crash/fuzz, packaging and release gates remain open.
