# Initial workload cost model

Original source: `c418e98b91720188158c531cd5c2c4b8ab71cbcf`. Evidence:
`evidence/compat/20261116T022511Z-3136726/`. These are initial shared-host
observations, not release regression dispositions. Three process rounds ran
on one pinned CPU; each round contains 31 sample averages after warmups. All
raw samples and sample p95/p99 values are retained. Sample-average quantiles
are not per-request tail latency. No machine-independent throughput target is
inferred from these values.

| Workload | Median of round medians (ns/op) | Round-median range (ns/op) |
| --- | ---: | ---: |
| `control` | 0.20 | 0.20–0.20 |
| `native_validate_reduce` | 0.95 | 0.94–0.95 |
| `hosted_typed_check_apply` | 479.52 | 476.02–560.98 |
| `portable_validate_reduce_hash` | 77.95 | 77.85–78.05 |
| `canonical_commit_bytes` | 204.40 | 201.29–205.00 |
| `sign_commit` | 25261.80 | 25166.60–25275.90 |
| `historical_state_proof` | 42258.70 | 42157.50–42394.00 |
| `historical_absence_proof` | 41908.00 | 41645.50–41913.00 |
| `wrong_key_refusal` | 24326.10 | 24302.00–24430.20 |
| `replay_two_event_state_root` | 53301.10 | 53138.70–53391.20 |
| `authenticated_prepare_including_adapter_setup` | 38241.80 | 37989.40–38464.40 |
| `two_intent_planning_including_adapter_setup` | 77016.80 | 76696.20–77648.00 |
| `verified_durable_write_reference_memory_adapter` | 78064.80 | 77549.80–78072.80 |
| `verified_exact_retry_reference_memory_adapter` | 39624.60 | 39297.80–39851.00 |
| `ambiguous_ack_reference_memory_adapter` | 78261.40 | 78042.80–78373.60 |

External allocation probes (1000 operations, identical setup and output):

| Path | Added allocations/op | Added bytes/op |
| --- | ---: | ---: |
| native | 0 | 0 |
| portable | 0 | 0 |
| hosted | 23 | 362 |
| proof | 29 | 41128 |

Both control and portable runs allocated exactly 4,201 times / 241,073 bytes
for process and fixture setup; the 1000 portable transitions added zero.
The hosted profile observation includes original check/apply duplication.
The proof path includes canonical proof-size validation and signed-envelope
verification; allocation ownership must be audited before optimization.

## Remaining comparisons and limitations

- Freeze a starting-point dependency-control measurement before changing production code.
- Add explicit current-head and stale-head workloads, authenticated batches,
  larger/checkpointed replay and timed cross-tenant/trade workloads.
- Original economy correctness scenarios already include cross-tenant settlement;
  they do not establish per-request latency distributions.
- Establish matched-run noise thresholds per workload; tiny native/control
  measurements require absolute timing floors, not percentage-only decisions.
- Final qualification must alternate comparator/candidate execution, record
  source and binary hashes, and separate safety-related new work from equal-work comparisons.
- Reference memory-ledger measurements do not qualify any real database,
  process-kill recovery, physical durability or production concurrency.

Reproduce with `RUSTC_WRAPPER= CARGO_BUILD_JOBS=8 python3 scripts/refactor_compat.py --lane baseline --measure`.

## Starting-point dependency control

The same fifteen workloads and five allocation probes passed against
`55b5be8` in `evidence/compat/20261116T022951Z-3197563/`. Its production Rust
source is byte-identical to the original release; dependency versions differ.
This captures a dependency control before production refactoring. The portable
allocation guard passed. The runs were not alternated across revisions, so
observed timing differences are not an accepted refactor performance delta.
Use this starting control, in addition to the original release, for matched
comparisons after changing code.

## Expanded pre-refactor workload baseline

`evidence/compat/20261116T024032Z-3384369/` captures twenty-two workloads
against both the immutable original release and starting dependency control
`55b010d`. All twenty build, latency and allocation checks passed. Production
Rust sources remain unchanged. These are three sequential process rounds per
lane on the same pinned CPU, not an alternated candidate comparison.

The added fixtures exercise public original-library methods. Trade setup
follows the frozen original two-tenant integration scenario, including actual
asset mint/lock, wallet creation, settlement planning and trade-index assembly.
Its proof workload verifies the resulting settled asset proof; it does not
authenticate every summary field or demonstrate durable cross-tenant atomicity.
The authenticated batch verifies both requests and reaches a sink that checks
the event bindings and deliberately refuses persistence. The checkpoint workload
generates 256 independent native configuration creates, signs their enclosing
commit, rebuilds 32 events, reopens in-memory handles, resumes in chunks of 32,
and checks the final state root. Signature verification and history generation
are setup outside that timed rebuild. This does not measure a process crash.

New workload medians and observed process-round median ranges (ns/op):

| Workload | Original median | Original range | Dependency-control median | Control range |
| --- | ---: | ---: | ---: | ---: |
| `historical_state_proof_reference_read_adapter` | 42988.10 | 42698.60–46656.20 | 42868.90 | 42680.60–43023.20 |
| `current_state_proof_reference_read_adapter` | 43032.20 | 42777.80–45938.80 | 42972.10 | 42570.30–43148.40 |
| `stale_head_refusal_reference_read_adapter` | 239.50 | 238.50–269.50 | 251.50 | 237.50–264.50 |
| `authenticated_two_intent_batch_to_refusing_sink_including_setup` | 80111.00 | 78798.40–82984.60 | 78938.60 | 77986.80–78960.60 |
| `cross_tenant_trade_plan_including_seed_setup` | 328695.50 | 323080.00–329242.00 | 320214.00 | 319783.50–321682.00 |
| `trade_state_proof_verification` | 43207.60 | 42688.50–43345.90 | 42779.80 | 42728.60–42872.90 |
| `checkpoint_resume_256_events_chunk_32_and_root` | 9949100.00 | 9948273.50–11003163.50 | 9894384.50 | 9788304.00–9906583.00 |

Allocation deltas are unchanged: native/portable zero, hosted 23 allocations
and 362 bytes/op, historical proof 29 allocations and 41,128 bytes/op. Added
read adapters increase shared fixture setup to 4,231 allocations, equally in
control and portable probes; the hard zero-added-allocation guard passes.
Earlier fixture build failures and the intermediate eighteen-workload run are
retained separately and are not substituted for this successful run.

The raw reports retain all 31 sample averages in every round. Three sequential
rounds establish initial observed spread, not a qualified statistical noise
bound. For final disposition, alternate comparator/candidate process order and
use paired per-workload differences. Investigate repeatable median/p95 changes
above the roadmap's 5%/10% review triggers; if observed paired noise overlaps
those triggers, gather more rounds before accepting or rejecting a regression.
For tiny native/control timings, report absolute nanosecond differences beside
percentages. Safety-related additional work needs its own measured disposition.
This policy records uncertainty rather than inventing an automatic threshold
from three samples.

## First F1 trade-scope checkpoint

Candidate `260bb1e` passed all ten workload/allocation checks in
`evidence/compat/20261116T025005Z-3704572/`. Its three trade state-proof process
medians were 43145.40, 42666.50, 42723.60 ns/op. All 22 workload
assertions passed, and portable transitions still added zero allocations or
bytes relative to setup. The trade workload invokes the legacy pure entry point
and measures its added tenant-binding check; it does not measure the new scoped
service cache with coincident identifiers. These sequential samples are an
initial checkpoint, not the final alternated performance disposition.

## Independent-claim checkpoint scope

The existing twenty-two-workload suite passed all ten measurement checks for
`6be864f` in `evidence/compat/20261116T031539Z-345814/`. Portable transitions
still add zero allocations/bytes. These fixed original workloads exercise
legacy proof helpers and shared commit lookup; they do not measure the new
`VerifiedClaim` construction or its owned-payload copy added in `8f05cce`. The
final comparison matrix must measure those new guarantees separately and
include their allocation cost, rather than treating a `Result<()>` verifier as
equal work. No final performance disposition is claimed for the new route.


## Shared event-stage checkpoint and open stale-head timing finding

Exact candidate `e30fdda` and immutable original release pass all twenty
measurement checks in `evidence/compat/20261116T042553Z-2255276/` on Rust
1.100. All 22 fixed legacy workloads run three CPU-pinned processes with 31
sample averages each. Native and portable operations each add zero allocations
or bytes over their process/setup control. Hosted custom execution remains
23 allocations / 362 bytes per operation; the legacy proof allocation probe
remains 29 allocations / 41,128 bytes. These do not measure owned expected
claims or protected signed-plan/readback costs.

The initial sequential samples crossed the roadmap's median review trigger
for hosted custom execution (+6.13%) and stale-head refusal (+8.12%). Rather
than dropping samples or declaring a pass, seven alternating original/candidate
pairs were run using the same verified binary hashes and pinned CPU. The
reproducible script is `evidence/f2/event-preparation/paired_benchmark.py`;
commands, raw sample arrays, checksums and summary are retained in the above
measurement directory's `alternating/` folder.

Hosted custom execution has +3.84% median paired change (+18.43 ns), with
wide positive and negative pair deltas; its median paired p95 change is +8.76%.
This is shared-host noise evidence, not final release disposition. Trade state
verification has -0.22% median paired change (-92.1 ns), which does not establish
a speedup or isolate the allocation cost of new coverage checks.

**Open investigation:** stale-head refusal has +13.78% median paired change
(+33 ns), with six positive pairs and one equal pair; median paired p95 change
is +56.09%, with substantial outliers. This crosses both review triggers and
is not accepted as final performance qualification. The timed compatibility
route loads the original commit, then refuses its stale head before crypto.
The cumulative candidate routes lookup through the shared async `load_commit`
helper; an extra async layer/code generation is a hypothesis to test, not an
established cause. The original and candidate also have different dependency
lockfiles. A controlled same-dependency comparison is still needed before
attributing this to any one extraction or deciding its disposition. No checks
were removed or weakened to hide the observation.

The event-stage extraction itself reduces three duplicate sites to one each
and does not introduce allocations. The measured workload matrix covers the
cumulative candidate, not an isolated before/after of just that extraction.
New canonical safety APIs, final alternating noise bounds and this open
refusal-path finding still need the F8 exact-candidate performance gate.

## Proof lookup mitigation checkpoint

The exact combined candidate `abd7245` repeats the stale-head investigation with
seven alternating pairs against both the pre-fix same-dependency candidate and
the frozen original. Observed paired median deltas are respectively -10 ns
(-3.71%) and +12 ns (+4.88%); paired p95 deltas are -5.02% and +1.14%.
All samples remain recorded, including substantial host noise. This mitigates
the earlier +33 ns observation without declaring the final F8 gate passed.
See [controlled investigation and limitations](proof-lookup-performance.md).
