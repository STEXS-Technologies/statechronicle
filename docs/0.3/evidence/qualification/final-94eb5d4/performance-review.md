# Accepted performance and build-cost disposition

Compatibility and signed-history latency/allocation campaigns, portable size and
cold-build records are terminal and independently verified. Their costs are
accepted within the measured library scope and limitations below. All seventeen
final evidence rechecks pass; this review supplies the manual cost disposition.

## Method and decision rules

The four-lane comparison uses actual original 0.2, original Rust with refreshed
dependencies, the pre-completion coordinator and the candidate. Registry versions,
checksums and features match for the latter three; the original commit crate
retains its necessary SHA-2 edge in the dependency control. All lanes use Rust
1.100, identical consumer code, CPU affinity, warmups and seven rotated/reversed
cycles. The separate two-lane canonical API comparison uses earlier coherent 0.3
because original 0.2 has no equivalent receipt/expected-claim contract.

The 22-path table uses actual individual action times. The earlier 31-batch
averages remain a separate measurement; their percentiles are not request tails.
No samples or timer costs are discarded/subtracted. Shared-host conditions,
interrupts and other users prevent a hardware isolation guarantee. Percent changes
are medians of the seven paired cycle ratios, not ratios of the medians.

Review triggers are repeatable >5% median or >10% p95 regressions, not arbitrary
automatic failures. Native/portable new allocations are a hard failure. A source
ownership benefit must be specific; added safety or capability costs are reported
rather than hidden by a single universal speedup figure.

## All 22 compatibility paths

Candidate absolute values are medians of seven per-cycle medians/p95s. Delta
columns are paired medians against dependency-matched original Rust.

| Workload | Candidate median ns | Candidate p95 ns | Median delta % | p95 delta % |
|---|---:|---:|---:|---:|
| cross_tenant_trade_plan_including_seed_setup | 326,662 | 345,608 | +0.42 | +0.73 |
| trade_state_proof_verification | 43,764 | 47,112 | +0.35 | -1.03 |
| checkpoint_resume_256_events_chunk_32_and_root | 10,335,342 | 10,668,906 | -0.90 | -6.45 |
| control | 20 | 20 | +0.00 | +0.00 |
| native_validate_reduce | 20 | 20 | +0.00 | +0.00 |
| hosted_typed_check_apply | 491 | 531 | -1.96 | -1.85 |
| portable_validate_reduce_hash | 91 | 101 | +0.00 | +1.00 |
| canonical_commit_bytes | 240 | 271 | +0.00 | +3.45 |
| sign_commit | 24,928 | 26,552 | +0.04 | -0.04 |
| historical_state_proof | 43,665 | 47,101 | +0.39 | -0.04 |
| historical_absence_proof | 43,133 | 46,169 | -0.35 | -1.05 |
| current_state_proof_reference_read_adapter | 44,045 | 46,720 | +0.16 | -1.20 |
| historical_state_proof_reference_read_adapter | 43,634 | 47,011 | -0.21 | -0.51 |
| stale_head_refusal_reference_read_adapter | 271 | 291 | +8.00 | +4.07 |
| wrong_key_refusal | 24,417 | 26,732 | +0.20 | -0.22 |
| replay_two_event_state_root | 57,020 | 60,477 | +0.16 | +0.53 |
| authenticated_prepare_including_adapter_setup | 38,043 | 41,981 | -0.26 | +0.74 |
| authenticated_two_intent_batch_to_refusing_sink_including_setup | 78,892 | 83,912 | +0.00 | -2.29 |
| two_intent_planning_including_adapter_setup | 75,987 | 81,437 | -0.33 | +0.37 |
| verified_durable_write_reference_memory_adapter | 79,654 | 84,143 | -0.06 | -0.17 |
| verified_exact_retry_reference_memory_adapter | 39,336 | 43,334 | +0.10 | +1.00 |
| ambiguous_ack_reference_memory_adapter | 79,735 | 84,824 | -0.51 | +0.06 |

### Reviewed differences

**Stale-head refusal: accepted measured overhead.** Actual individual medians
are 251 ns for dependency-matched original Rust and 271 ns for candidate; the
median paired change is +8.00%. All seven median deltas are positive, with five
above 5%, so this is treated as a real review trigger. Paired p95 change is
+4.07%. The batch-average matrix independently observes +11.85% median and
+10.64% p95. The completion control is already 270 ns and the candidate differs
from it by +0.37% median / -3.44% p95 in individual timing.

Source review shows the old compatibility service now forwards to the same
generic `CommitReader` head checker used by the minimum read-only `ClaimVerifier`.
The complete head tuple, error construction and ordered checks are preserved;
the final 2,352-case original/current service traces agree exactly. The concrete
benefits are one authoritative head/lookup interpretation and an externally
compiled proof consumer that requires no write-capable store. The observed
difference accompanies that shared capability boundary; these campaigns do not
isolate one machine instruction or prove all 20 ns is attributable to one helper.
We accept this bounded early-refusal cost for those specific ownership/capability
benefits. This is not a claim of performance neutrality or permission to weaken
head checks.

**Hosted typed check/apply tail: no repeatable p95 regression established.**
The batch-average completion comparison triggers +14.87% p95, with +3.31% median.
The independent individual-action campaign has equal candidate/control p95s
(531 ns), -1.96% paired median and 0.00% paired p95. Individual p99 paired delta
is +14.26% against completion control; this remains visible as a small-helper
tail observation, not relabeled as zero. The measured full authenticated and
planning paths below do not show a corresponding important-path regression.
We accept the observations with their timer/shared-host limitations and make
no universal p99 guarantee.

The larger individual-action single/refusing-batch/planning/trade campaign
also stays inside median/p95 review triggers: candidate vs completion paired
median deltas are +0.21%, -0.06%, +0.20%, -0.21%; corresponding p95 deltas are
+1.34%, -0.56%, -0.74%, -0.39%. It uses 3,100 samples per cycle except trade
(1,240), supplementing the default-count table. No further implementation
change is justified by these measured important-path observations.

## Canonical protected mutation and expected claims

These actions use prebuilt signed plans and fixed consumer-owned in-memory
adapters. Preparation, BCS/signing and profile components have separate matched
measurements. Network, disk, database locks and production authorization policy
are not included. The eight-thread action includes thread creation/join and
eight same-identity attempts; it verifies exactly one complete retained commit
and recovery, not eight successful receipts or generic database scaling.

| Workload | Candidate median ns | Paired median delta % | Paired p95 delta % |
|---|---:|---:|---:|
| control | 20 | +0.00 | +0.00 |
| durable_commit_including_empty_adapter | 123,559 | -0.88 | -1.67 |
| durable_replay | 80,666 | -1.68 | -2.68 |
| durable_recovery | 41,310 | -0.94 | +1.28 |
| durable_denied | 36,351 | +0.06 | +0.02 |
| durable_conflicting_identity | 4,058 | +0.50 | +0.24 |
| durable_unknown_and_recovery | 125,383 | -1.19 | -2.17 |
| durable_eight_concurrent_exact_retries_including_threads | 987,118 | -1.38 | -1.71 |
| claim_current_ownership | 44,727 | +0.02 | +1.28 |
| claim_historical_ownership | 44,736 | +0.13 | +0.63 |
| claim_current_absence | 44,386 | +0.18 | +1.05 |
| claim_wrong_trusted_key | 25,399 | +0.16 | +2.76 |
| claim_missing_canonical_head | 44,787 | +0.07 | +0.72 |

All thirteen median/p95 comparisons are inside the review triggers. All 26
allocation probes and independent raw/source verification pass. Process totals
include fixed setup; the earlier canonical executable
names have different lengths, so sub-byte-per-action control offsets must not
be advertised as a universal per-operation memory guarantee.

## Cold target builds and binary size

Three samples per revision use new target directories, warm registry/filesystem
caches and the same fixed tiny facade application. The approved dependency
refresh is part of this total comparison; its effect is not attributed solely
to refactoring. The cold target artifact records prove actual fresh compilation.

| Metric | Original median (range) | Candidate median (range) | Delta % |
|---|---:|---:|---:|
| wall_seconds | 7.45 (7.32–7.6) | 7.5 (7.38–7.75) | +0.67 |
| max_rss_kib | 299,072 (297,996–299,460) | 303,080 (301,632–305,028) | +1.34 |
| binary_bytes | 333,392 (333,392–333,392) | 332,864 (332,864–332,864) | -0.16 |
| text_bytes | 251,237 (251,237–251,237) | 250,789 (250,789–250,789) | -0.18 |
| compiled_packages | 74 (74–74) | 64 (64–64) | -13.51 |

**Accepted build costs.** Wall-time ranges overlap; three shared-host samples
do not establish statistical equivalence. Compiler maximum RSS increases about
4 MiB (+1.34%) with non-overlapping observed ranges; this is explicitly accepted
for the protected preparation, receipts/batches, minimum read capability and
verified-history owners documented in the +13,336-token source growth review.
The increase is measured, not claimed to be baseline variance. Compiled packages
fall from 74 to 64 with the dependency cleanup/refresh; this is not a pure
refactor speedup claim. The tiny linked binary shrinks by 528 bytes and text by
448 bytes; it is not a complete protocol-workflow footprint.

The broader same-workload matrix binary grows from 4,001,544 original bytes
to 4,123,376 candidate bytes; dependency-matched original is 3,993,296 and
completion control 4,099,616. That is about +3.26% against matched original and
+0.58% against completion. These actual consumer-specific code-generation costs
are accepted alongside the documented added typed capabilities; the tiny
consumer reduction does not conceal them. No universal downstream binary-size
or runtime-memory bound is inferred.

## Portable core code-size supplement

The same frozen no-std Counter consumer is freshly built for
`riscv32imafc-unknown-none-elf`, release mode, one codegen unit, no LTO/debug info,
using Rust 1.100 and each revision's own approved lockfile.
`portable-size-verification.json` independently checks source/archive/consumer
hashes, fresh compiler artifact records and GNU object-section output.

| Archive | Original rlib bytes | Candidate rlib bytes | Original text/rodata bytes | Candidate text/rodata bytes |
|---|---:|---:|---:|---:|
| statechronicle-profile | 8,198 | 8,198 | 0 | 0 |
| statechronicle-portable | 126,146 | 125,170 | 8,708 | 8,068 |
| Frozen portable Counter consumer | 18,926 | 18,706 | 1,009 | 1,009 |

The portable/profile Rust implementations are unchanged; dependency refresh and
code generation affect the portable archive. A metadata-only generic profile
object does not mean that monomorphized profile execution costs zero. Rlibs
include metadata and unreferenced objects; these are portable core artifact costs,
not linked firmware flash/RAM guarantees. No allocator, runtime or serializer
was added. The first successful build's artifact-discovery failure is retained
in `portable-size-failed-artifact-layout/`; only the corrected campaign is
qualified. Code-size reduction is not attributed solely to refactoring.

## Reviewed compatibility allocation paths

All 22 workloads now have 176 independently verified zero/1000-action probes (`performance-verification.json`). The earlier 168-probe preflight remains a partial historical check. Signed-history qualification and aggregate acceptance now pass within the documented limits. The old shared-control figures are unqualified; the following values use each workload’s own setup-only process.

| Workload | Candidate allocations/action | Candidate bytes/action | Allocation delta vs matched original | Byte delta vs matched original |
|---|---:|---:|---:|---:|
| cross_tenant_trade_plan_including_seed_setup | 903.000 | 62,028.000 | +0.000 | +0.000 |
| trade_state_proof_verification | 34.000 | 41,782.000 | +4.000 | +592.000 |
| checkpoint_resume_256_events_chunk_32_and_root | 14,372.000 | 976,252.000 | +512.000 | +61,064.000 |
| control | 0.000 | 0.000 | +0.000 | +0.000 |
| native_validate_reduce | 0.000 | 0.000 | +0.000 | +0.000 |
| hosted_typed_check_apply | 23.000 | 362.000 | +0.000 | +0.000 |
| portable_validate_reduce_hash | 0.000 | 0.000 | +0.000 | +0.000 |
| canonical_commit_bytes | 9.000 | 839.000 | +0.000 | +0.000 |
| sign_commit | 18.000 | 1,217.000 | +0.000 | +0.000 |
| historical_state_proof | 29.000 | 41,128.000 | +0.000 | +0.000 |
| historical_absence_proof | 26.000 | 41,585.000 | +0.000 | +0.000 |
| current_state_proof_reference_read_adapter | 42.000 | 41,657.000 | +0.000 | +0.000 |
| historical_state_proof_reference_read_adapter | 39.000 | 41,538.000 | +0.000 | +0.000 |
| stale_head_refusal_reference_read_adapter | 15.000 | 573.000 | +0.000 | +0.000 |
| wrong_key_refusal | 27.000 | 41,185.000 | +0.000 | +0.000 |
| replay_two_event_state_root | 6.000 | 1,040.000 | +0.000 | +0.000 |
| authenticated_prepare_including_adapter_setup | 133.000 | 7,791.000 | -7.000 | -131.000 |
| authenticated_two_intent_batch_to_refusing_sink_including_setup | 306.000 | 20,190.000 | -14.000 | -262.000 |
| two_intent_planning_including_adapter_setup | 259.000 | 15,290.000 | -14.000 | -262.000 |
| verified_durable_write_reference_memory_adapter | 250.000 | 29,106.000 | +1.000 | +15.000 |
| verified_exact_retry_reference_memory_adapter | 181.009 | 12,900.176 | +1.000 | +15.000 |
| ambiguous_ack_reference_memory_adapter | 252.000 | 29,180.000 | +1.000 | +15.000 |

All completed candidate counts equal completion-control counts. The original and dependency-control lanes also agree. Counts include setup inside actions where named; fractional values include amortized retained fixture trace/capacity growth, rather than a universal per-request memory guarantee.

**Trade proof: accepted +4 allocations / +592 bytes per action.** Correct matched controls measure original 30 / 41,190 and candidate 34 / 41,782. The earlier 29-to-33 absolute figures were incorrect and are superseded. Unique summary tenant/asset and proof leg/resource coverage prevents duplicated witnesses from disguising missing coverage. Temporary borrowed-key BTreeSet owners implement that additional check; this is a measured safety cost, not a reason to drop it. Unsigned trade summary/history metadata remains explicitly outside this state-proof contract.

**Legacy durable helper: accepted +1 allocation / +15 bytes.** Source review identifies the owned tenant returned by the shared `validate_request` preflight (`tenant.clone()`), replacing the original borrowed scope. This fixture’s tenant string is 15 bytes. The owner is reused for legacy writes and verified canonical recovery checks with common ordered intent/effect validation; canonical receipt/batch authority remains separate. The measured write/replay/ACK-loss increment is bounded and their important-path median/p95 comparisons remain inside review triggers. The shared owner benefit, rather than stronger checks attributed to the clone itself, justifies retaining this cost.

**Legacy reconstruction: accepted +512 allocations / +61,064 bytes for 256 events.**
The completed paired probes measure 13,860 / 915,188 in original and dependency
control versus 14,372 / 976,252 in completion and candidate. Continuity now uses
full projection addresses including holders, while event identities are scoped
by tenant. This prevents independent holders from merging and equal event-ID
text across different tenants from colliding; same-address forks and same-tenant
duplicates still fail before writes. The fixture validates 512 event entries across its initial/resumed chunks and
full-stream check; its additional tenant identity copies and larger composite
BTree keys account for additional storage work, without claiming instruction-
level attribution of every byte. Current regression tests exercise those exact
identity distinctions. This allocation cost is accepted for the corrected
identity contract; the old-compatible reconstruction latency median/p95 remains
inside review triggers. This is separate from full signed-history authentication.

**Authenticated preparation: observed allocation reduction.** Single preparation removes seven allocations / 131 bytes; two-intent planning/refusing-batch paths remove fourteen / 262. These measurements support the shared native decode/evaluation simplification. They do not establish a universal speedup or permit removal of output validation. Native and supported portable action paths remain zero-allocation in every lane.

## Retained compatibility-allocation attempts

The first all-path allocation attempt timed out at 180 seconds on 256-event
reconstruction; two original lanes later timed out at 1200 seconds on contended
CPUs. All failure manifests and available raw logs remain retained. A resumed
campaign was intentionally interrupted after review identified mismatched fixed
setup. Its shared-control arithmetic is unqualified. Corrected fresh binaries
support zero actions at each selected workload boundary; all 22 workloads and
four lanes complete both zero and 1000 actions (176 probes). Independent
source/build/log verification and native/portable zero-allocation checks pass.
Together with direct and canonical campaigns, all 222 earlier allocation probes
are verified. Latency retains its separately measured, unchanged binaries.
Amortized counts may be fractional when a reused in-memory adapter retains trace
or read capacity; these fixture counts are not universal per-request bounds.

## Complete signed-history method and costs

Original 0.2 has no equivalent verified-history authority contract. The matched
control is the earlier coherent 0.3 implementation; both freshly compiled public
consumers use the same signed histories and matching registry versions, checksums
and features. Fixtures contain 1, 32 or 256 unique-resource mint commits, with
proper sequence, parent, signer policy, event roots, prior/next state roots and
independent historical pins. All three sizes are generated and verified before
each selected allocation action, identically in both lanes.

Authentication constructs witnesses, validates the complete signed chain from
genesis and owns the verified result. The separate resume action starts with an
already-verified source, creates a fresh memory sink, loses the ACK after the
first persisted checkpoint, resumes through a new shared handle and checks every
row, the terminal checkpoint and an independently computed final root. These
memory adapters do not establish physical backend durability.

Fourteen timing processes complete seven alternated cycles on CPU6, selected
before measurement. Each action has three warmups and 100 individual samples for
one commit or 31 samples for 32/256 commits; there are 1000 timer controls per
process. No observations or timer costs are discarded/subtracted. Raw log hashes,
fixture identities, sample distributions, percentiles and paired arithmetic have
been checked. Absolute values below are medians of seven cycle statistics;
changes are medians of seven paired ratios.

| Workload | Candidate median ns | Candidate p95 ns | Paired median delta % | Paired p95 delta % |
|---|---:|---:|---:|---:|
| verify_signed_history_1 | 78,431 | 81,999 | +0.04 | -0.17 |
| checkpoint_ack_loss_resume_and_root_1 | 41,931 | 42,422 | -0.17 | -0.94 |
| verify_signed_history_32 | 39,854,270 | 40,471,861 | -0.72 | +0.50 |
| checkpoint_ack_loss_resume_and_root_32 | 1,292,649 | 1,306,066 | +0.13 | +0.15 |
| verify_signed_history_256 | 2,490,579,148 | 2,536,078,291 | -0.05 | -0.42 |
| checkpoint_ack_loss_resume_and_root_256 | 10,584,105 | 10,745,467 | -0.26 | +0.42 |

All six paired median/p95 comparisons are inside the review triggers. Full
256-commit authentication costs about 2.491 seconds, versus about 10.584 ms for
already-verified resume/root. Authentication grows about 62.5 times from 32 to
256 fresh-resource commits; resume grows about 8.2 times. `verify_history`
accumulates updates and calls `verify_state_root_continuity` at each commit.
That helper reconstructs prior and extended roots, including copying/sorting
accumulated updates. Increasing prefix work explains why this fixture cannot
support a linear-time authentication claim. There is no new candidate/control
regression in these observations. Absolute authentication cost and nonlinear
scaling remain deployment-sizing limits; the resume cost must not be presented
as the full authentication cost or generic production throughput.

## Signed-history allocation scope and retained attempts

Each lane requires nine probes: setup controls for 1000, 100 and 10 actions,
plus authentication and checkpoint/resume/root at each of the three history
sizes. One-commit actions use 1000 iterations, 32-commit actions use 100, and
256-commit actions use 10. The exact measured binaries have equal-length names,
128-byte arguments and count-matched control environments. Subtraction is
within each lane and action count. Process exit skips destruction, so these
probes do not qualify leak freedom.

The original control's 256-commit authentication process hit its actual
900-second deadline. Its failed manifest, outer stack trace and failed queued
checker are retained. The original driver did not separately archive partial
Valgrind output for that timeout; no successful allocation total or zero-error
result is inferred from it. All fourteen positive timing processes and seven
completed control allocation probes remain unchanged. Only the two unfinished
control probes were rerun with their full ten actions on distinct cores 6/8 and
3600-second budgets; both completed successfully. The resumed driver retains partial output on a new timeout.

Candidate allocation measurement uses an explicit parallel handoff. Four
completed CPU2 probes remain unchanged. Before SIGTERM, the reason for stopping
the active one-commit checkpoint probe was recorded in
`parallel-handoff-review.json`. Its negative exit, partial log and failed manifest
remain unqualified. All five remaining probes completed in full on cores 2/3/4/5/7,
whose distinct physical-core topology is recorded. This avoids serializing
repeated setup after timing is already complete. Allocation execution times are
not additional latency observations. No source, fixture, action count or
verification requirement is reduced.

All eighteen probes pass. All six completed action pairs have independently
checked raw-log hashes, zero-error summaries and count-matched setup subtraction:

| Action pair | Control allocations/action | Candidate allocations/action | Control bytes/action | Candidate bytes/action |
|---|---:|---:|---:|---:|
| Authenticate 1 commit (1000 actions) | 154 | 154 | 17,732 | 17,732 |
| Authenticate 32 commits (100 actions) | 4,237 | 4,237 | 745,025 | 745,025 |
| Authenticate 256 commits (10 actions) | 42,320 | 42,320 | 26,992,521 | 26,992,521 |
| Checkpoint/resume/root 1 commit (1000 actions) | 116 | 116 | 18,949 | 18,949 |
| Checkpoint/resume/root 32 commits (100 actions) | 2,322 | 2,322 | 144,582 | 144,582 |
| Checkpoint/resume/root 256 commits (10 actions) | 18,337 | 18,337 | 1,153,422 | 1,153,422 |

Both full ten-action 256-commit authentication probes completed. The guarded
integration and both independent lane checks pass, followed by all seventeen
final evidence gates. `history-performance-verification.json` binds the original
timing/source provenance, eighteen positive allocation records and retained
failed/interrupted attempts. Allocation counts match in all six action pairs.

**Accepted signed-history costs.** The full authentication fixture allocates
about 26.99 MB cumulatively per 256-commit action; this is total allocated bytes,
not peak retained memory. Its roughly 2.491-second median and nonlinear prefix
work remain explicit sizing limits. The already-verified resume action measures
a different contract. The matched earlier coherent implementation has the same
allocation counts, and all six latency median/p95 comparisons stay inside the
review triggers. These results justify accepting this implementation without
weakening signature, pin, event/root or chain checks. They do not establish
linear authentication, arbitrary history capacity or production backend SLOs.
The original timeout and deliberate handoff remain failed/unqualified attempts;
only the complete positive integrated scope is accepted.
