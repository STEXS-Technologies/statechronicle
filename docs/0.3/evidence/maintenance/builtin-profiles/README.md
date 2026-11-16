# Built-in profile field ownership

Control: `d83ae03`; retained source: `7abe9d885eeaf008e524d70e51165fa3b137c62f`.
The reducer imports the existing registry `input_str`, `state_amount` and
`state_bool` helpers and deletes its equivalent implementations. Eight redundant
built-in `execution_policy` overrides and their unused imports are removed;
they duplicated the existing `ProfileRules` default body. The trait default
remains the policy-selection owner, forwarding to the existing resource policy.
Operation ordering, reduction behavior, public API, dependencies and wire
representations are unchanged.

## Review of the eight built-in rule sets

| Profile | Shared ownership and retained distinction |
| --- | --- |
| Unique asset | Required input strings share the registry owner; owner/status and trade gates remain ordered and operation-specific. |
| Paid unique asset | Existing delegation to unique-asset validation remains, followed by paid consent/ownership overlays. Hard-delete and paid restoration retain their separate gates. |
| Consumable stack | Required input strings and typed quantity reads share the registry owner. Credit accepting zero remains distinct from positive-only balance credit. |
| Fungible balance | Typed balance reads and required strings share the same owner. Burn authorization, destination and conversion checks retain their order. |
| Entitlement | Typed transferability reads share the registry owner; grant/transfer and lifecycle policy remain separate. |
| Meter | Typed remaining/maximum reads and required strings share the registry owner; creation bounds, consumption, refill and clamp semantics remain separate. |
| Listing | Seller input validation shares the registry owner. Listing lifecycle and paired economic policy remain explicit. |
| Escrow | Buyer/seller input validation shares the registry owner. Escrow lifecycle remains distinct from listing lifecycle. |

All eight built-ins now inherit the existing execution-policy default; their
state types, policies and native custom-domain behavior are unchanged.

Three apparent duplicates are deliberately retained. Reducer `require_current`
returns `InvalidInput`, while rule validation returns `InvalidTransition`.
Reducer amount parsing reports float-shaped input as a specific `InvalidInput`,
while rule validation reports `FloatForbidden`. Reducer string-field lookup
does not expose listing/escrow status as registry validation does. Mismatched
projection/payload combinations can observe that last distinction through public
reduction calls. A blanket helper import would change compatibility behavior.

Built-in states are already typed; these accessors do not deserialize state.
Checking and reduction still have distinct responsibilities, and public `apply`
can be called separately from `check`. This extraction does not introduce a
second prepared-command framework or claim to eliminate every repeated input
parse. The native hosted adapter's separate one-decode work is unchanged.

## Independent characterization

The new external [fixture](../../../../../ci/characterization/profiles.rs)
uses only public APIs available in the original release. It retains the existing
14,364 validation cases and expands to **296,604 uniquely named cases** across
all eight built-in profiles, malformed and missing inputs, amount boundaries,
empty identifiers, both transferability values, lifecycle statuses and
mismatched typed payloads. Each row records check-then-apply stage, direct and
trait reduction, successful state digests and exact Debug/Display errors.
Transfer cases also record unborn/existing destination reduction.

- [Final positive campaign](20261116T130228Z-486762/manifest.json): original
  `c418e98`, unchanged control and candidate produce identical full transcripts.
  Each independently archived application resolves without registry drift,
  builds, passes strict Clippy and runs. Guards remain active under
  `PYTHONOPTIMIZE=1`; row count, uniqueness, locks and source hashes are checked.
- [Optimized-Python negative control](20261116T125320Z-145157/manifest.json):
  changes only the archived reducer's float error to the registry variant.
  Both binaries execute successfully, but **1,430 differing rows** make the
  runner fail. The altered source never enters production. This control uses
  field-only `fb3d5a4`; its reducer owner, runner and both fixture files are
  byte-identical in final `7abe9d8`.
- [First attempt](20261116T125018Z-16590/manifest.json): the external fixture's
  nested `if` failed strict Clippy before a transcript ran. The correction is
  fixture-only. [Earlier positive](20261116T125057Z-41710/manifest.json) and
  [negative](20261116T125147Z-78337/manifest.json) attempts are retained as well.

Reproduce from the repository root:

```sh
PYTHONOPTIMIZE=1 python3 scripts/check_proof_service_parity.py --fixture profiles \
  --revision c418e98 --revision d83ae03 --revision 7abe9d8
PYTHONOPTIMIZE=1 python3 scripts/check_proof_service_parity.py --fixture profiles \
  --revision fb3d5a4 --revision fb3d5a4 --negative-control-share-amount-errors
```

The second command must fail. This is bounded characterization, not an
exhaustive proof, a new canonical-facade audit or concurrent backend testing.

## Qualification and footprint

[Archived root qualification](qualification-7abe9d8/manifest.json) passes all
**884 finite tests, 16 doctests, strict all-target/all-feature Clippy, formatting
and strict rustdoc** on Rust 1.100.0, against the exact retained library source.
[Original/control/candidate executor characterization](../../f2/executor-order/20261116T130321Z-530529/manifest.json)
also preserves all 23,544 error/effect/callback rows through nine compatibility
routes. It covers asset and subject-held transfer policies plus native domains,
not every possible profile or canonical storage backend. Fuzz targets are compiled, not executed without bounds.

[Control](control-metrics.json) and [final candidate](final-metrics.json) use the
same pinned parser. Production loses **68 syntax lines, 555 tokens and 10
parsed branches**, with no public declaration change. The field-helper extraction alone shrinks the registry/reducer pair
3.10% in tokens; this does not establish the roadmap's broader hotspot
target. Whole production remains above the original release at 120,696 tokens
and 1,948 parsed branches. Raw tracked Rust bytes decrease by 3,062, including
comments; the figures are not compiled instruction counts.

The following initial measurements qualify the interim field-only source
`fb3d5a4`, before the policy-default cleanup. They are retained separately from
the final candidate measurements. [Interim root qualification](qualification-cd85a39/manifest.json)
and [positive parity](20261116T125230Z-109663/manifest.json) remain historical evidence.

The [matched timing campaign](paired-20261116T125348Z-164088/manifest.json)
uses identical consumer locks/features, Rust 1.100.0, seven alternating cycles,
all 22 frozen workloads and four extended workloads at twenty times the
iteration count. Binary size decreases from 4,112,616 to 4,111,144 bytes.
CPU 0 results are highly variable: checkpoint median paired change is +49.47%,
refusing-batch p95 is +41.19% in the default regime and −3.86% in the extended
regime. Raw observations and every outlier remain retained. Percentiles are
of 31 batch averages, not individual-request latency.

[Allocation probes](allocations-20261116T125508Z-219192/manifest.json) pass all
ten Valgrind processes, with identical full-process allocation counts/bytes
between control and candidate for control, native, portable, hosted and proof
fixtures. Control/native/portable each allocate 4,224 times and 243,897 bytes;
hosted totals are 27,224/605,897 and proof totals 33,224/41,371,897. These include
startup and 1,000 actions; no universal allocation-free claim follows.

The [predeclared CPU 8 repeat](repeat-20261116T125631Z-278672/manifest.json)
uses the same immutable interim binaries, with all 22 default workloads and five
extended workloads. Checkpoint median changes are −0.03% and −0.12%; refusing
batch p95 changes are +1.08% and −0.85%. The CPU 0 checkpoint concern does not
repeat here. Both campaigns remain retained, without discarded observations.

## Final candidate measurements

[Matched CPU 8 campaign](paired-20261116T130401Z-565324/manifest.json) measures
`7abe9d8` against `d83ae03`, with identical resolved locks/features and all 22
workloads plus four extended workloads. [Exact final external compatibility](
../../compat/20261116T130517Z-627088/manifest.json) passes the frozen consumer,
portable tests and bare-metal build. [Final allocations](
allocations-20261116T130516Z-626071/manifest.json) preserve all ten process totals
listed above exactly.

The final binary grows **5,456 bytes (0.133%)** to 4,117,512 bytes, against this
campaign's 4,112,056-byte control. The [ELF section observations](binary-sections.json)
show `.text` decreases by 496 bytes; raw file size and code size are different
measurements. No general binary-size improvement is claimed for the combined
cleanup.

Single/batch/trade extended median changes are −0.68%, −2.74%, +0.41% and
+1.12%, respectively. Two default observations trigger further review:
portable median +6.27% and wrong-key p95 +27.74%. The
[retained repeat](repeat-20261116T130653Z-711434/manifest.json) repeats all 22
default workloads and additionally measures those two paths with twenty times
the iterations. Default portable remains +8.94% median/+18.78% p95 and wrong-key
p95 remains +16.56%; extended portable is +2.03%/−0.50% and extended wrong-key is
−0.09%/+1.32%. Default portable medians are roughly 79–115 ns. These observations
are sensitive to the measurement regime; the default increases are not erased
or described as a speedup. No portable source, feature, dependency or allocation
contract changes. Full individual-request and portable performance qualification
remains open; this is a reviewed bounded extraction, not universal neutrality.

Timing variability, broader performance budgets, canonical facade costs,
concurrency, final coverage and complete release qualification remain open.
No blanket speedup, performance neutrality or full F3/F6/F8 pass is claimed.
