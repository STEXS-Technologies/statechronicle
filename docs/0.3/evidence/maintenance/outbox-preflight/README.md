# One immutable outbox validation pass

Production source: `52e8e6fa81167f94998b31ff0980c5549a8d8019`.
Runner dimension guards: `f99c37d3561c63d6fc4c7251d1b13fc365d0acd1`.
Control: `79baccfdfae964f8007dc086aa8a2f0d25f022b8`.

## Deleted work and invariant ownership

`commit::persist::validate_effects_with_binding` previously called
`validate_outbox_records` over the complete borrowed slice, then called
`validate_outbox_record` again for every same immutable row while checking its
commit/tenant binding. The second call repeated delivery-key validation, payload
hashing and size validation. There is no await or callback between those passes.
It cannot add a new refusal after the identical records have already validated.

The extraction deletes only that second call. `validate_outbox_records` remains
the ordered payload/key/size/duplicate-key owner, followed by the existing
scope-binding loop. The original payload-before-scope refusal precedence is
retained, including competing malformed later rows and earlier wrong scope.
Signature checks, authorization, event/projection roots, ledger writes and
recovered-record validation remain unchanged. Fresh untrusted ledger readback
has a separate trust boundary and still requires its own checks.

This removes one syntax line, six lexical tokens and one parsed try branch.
Whole production is still 121,090 tokens and 1,950 parsed branches; this small
cleanup does not satisfy the broader footprint target. No public declaration,
wire format, dependency or crate is added.

## Independent behavior and qualification

The external fixture uses only public original-release APIs and the unchanged
original test harness. It covers 18 outbox shapes, four claim dispositions, and
zero/single/pairwise faults at 12 callback boundaries: 5,688 unique cases. Cases
include payload/key/size failures, duplicate keys, tenant/commit mismatch,
competing projection/outbox failures, empty valid payloads and later bad rows.

- [Final positive comparison](20261116T140328Z-3140629/manifest.json): actual
  original release, preceding control and candidate produce byte-identical
  outcomes and callback order. All builds, strict Clippy and transcripts pass.
  Complete case dimensions and uniqueness are checked under optimized Python.
- [Final digest-bypass control](20261116T140438Z-3173020/manifest.json): removing
  the sole remaining digest comparison in an isolated candidate archive yields
  552 different rows. Both applications build and run; the runner exits 1 as
  intended under optimized Python. Altered source is never checked in.
- [Immutable-source root qualification](qualification-52e8e6f/manifest.json):
  896 finite tests, 16 doctests, formatting, strict workspace Clippy and strict
  rustdoc pass, with archived source/lock drift guards.
- [Frozen external consumer](../../compat/20261116T140542Z-3200915/manifest.json):
  legacy/additive, portable and bare-metal checks pass on `f99c37d`. Its library
  source is byte-identical to `52e8e6f`.
- [Parser metrics](metrics-52e8e6f.json) use the same pinned parser as preceding
  checkpoints. Exact scope/limitations remain in the metrics file.

The first external-consumer attempt
[failed before dependency resolution](../../compat/20261116T140459Z-3182360/manifest.json)
because the ambient sccache wrapper timed out. The successful retry explicitly
clears `RUSTC_WRAPPER`; that setup failure is not a product regression or a pass.

## Matched performance and allocations

[Native timing](paired-52e8e6f/manifest.json) uses identical consumer source,
resolved locks/features and Rust 1.100.0, pinned CPU 8, seven verified alternating
lane orders and no discarded samples. All six actions perform real signature,
authorization, event/projection/outbox preflight and then deliberately refuse at
`begin`. Each workload verifies the expected refusal before timing; this measures preflight,
not durable commit or network latency. No compilation or qualification runs from
this session overlap native timing.

| Outbox payloads | Median paired change | p95 batch-average change |
| --- | ---: | ---: |
| None | −6.69% | −7.44% |
| One empty | +1.84% | −1.06% |
| One 1 KiB | −2.32% | −0.99% |
| One 64 KiB | −29.82% | −26.64% |
| Eight 64 KiB | −47.13% | −43.65% |
| One 1 MiB | −47.43% | −46.15% |

These are medians of seven paired observations; p95 values describe 31 batch
averages, not individual-request tails. Shared-host and code-layout variance
remain visible in unchanged/empty workloads. The larger-payload results are
consistent with deleting one of two payload hashes, not a universal speedup.
The dedicated benchmark binary shrinks 1,072 bytes and its `.text` section shrinks
320 bytes; [raw section logs](paired-52e8e6f/binary-sections.json) retain hashes.
This is one consumer binary, not a whole-distribution size claim.

[Four Valgrind probes](allocations-52e8e6f/manifest.json) pass without memory
errors using those exact binaries. With no rows both lanes allocate 510,661
times; full-process bytes differ by two, which this probe does not attribute. For one
64 KiB row, control allocates 267,396 times/20,212,016 bytes and candidate
262,713 times/19,988,795 bytes. The 4,683-allocation reduction is three allocations
per each of 1,561 preflight actions (including one validation action and ten
warmups). These totals include process/fixture setup and are not per-request
heap telemetry. Digest string construction explains why repeated hashing also
repeated allocations. Valgrind timings are excluded from native latency results.

## Retained attempts and remaining gates

`20261116T135639Z-2939068` was the initial equal trace, but its long-key case was
257 bytes rather than the actual 512-byte boundary, and fault slot 11 was unused.
`20261116T135715Z-2960613` corrects both before production changes. The corrected
fixture uses 513 bytes and a separate key-trust sentinel. Neither initial attempt
is used to claim the final complete dimensions.

`20261116T135828Z-2999941` and `20261116T135829Z-3000488` were inadvertently launched
concurrently against a shared binary output directory; their outcomes are
retained but excluded from qualification because build/execute isolation was not
proved. Sequential reruns `20261116T135911Z-3022945` and
`20261116T140044Z-3067489` agree, and the final guarded runs above are also
sequential. Future same-output-directory parity jobs must run sequentially.

A separate [audit of earlier built-in benchmark order](../builtin-profiles/order-audit.json)
corrects prior alternation claims without rewriting raw results. This timing
runner uses an explicit conditional and verifies command order in its checkpoint.

Reproduce characterization using `scripts/check_proof_service_parity.py
--fixture outbox --revision c418e98 --revision 79baccf --revision f99c37d`.
Use `--negative-control-ignore-outbox-digest` with control/candidate to reproduce
the expected failure. The paired/Valgrind runners are retained with exact hashes
and phases. Broader semantic duplication, exact-candidate coverage/fuzz,
concurrent adapter performance, packaging and release gates remain open.

The final [checker driver](driver-f99c37d/manifest.json) records actual top-level exits under optimized Python: positive 0 and digest-bypass negative 1, with 552 differences. Its two child manifests retain the final sequential reruns and their raw logs.
