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
