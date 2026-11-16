# Profile validation cost evidence

Candidate `e8500d6` versus immediate control `89e2494`. Both exact archived
production revisions use identical dependency versions, compiler, fixture and
benchmark source. The original-release trace is an independent behavior oracle;
performance compares the immediate pre-extraction code to isolate this change.

Seven alternating pairs pinned to CPU 24, 10,000 warmup calls per case, 31
samples of 50,000 calls. Cases and projections are constructed before timing.
The allocation counter wraps the system allocator; identical per-case allocation
call counts and requested bytes were observed in every pair. This measures
public pure validation only, excluding preparation, reduction, authorization,
durability and proof workflows. p95 is a percentile of sample means, not request
latency. This is a shared host with uncontrolled load and CPU frequency.

First attempt overlapped local qualification processes; it remains retained as
`concurrent-builds`. The repeated comparison ran after those processes finished.
`same-binary` executes the identical candidate binary under both timing labels;
it is an order/noise control and never a source comparison.

| Case | Median delta | p95 sample mean delta | Median paired median delta |
| --- | ---: | ---: | ---: |
| FungibleBalance/balance.debit/2/3/0 | -4.58% | -34.41% | -2.81% |
| FungibleBalance/balance.debit/2/2/0 | -9.09% | +12.97% | -8.11% |
| FungibleBalance/balance.debit/2/5/0 | -4.35% | -36.98% | -4.58% |
| ConsumableStack/stack.consume/2/3/0 | -1.66% | -9.12% | -2.22% |
| ConsumableStack/stack.reserve/2/5/0 | -0.01% | -0.36% | -0.00% |
| UniqueAsset/asset.lock/1/0 | -2.05% | +27.96% | -1.60% |
| UniqueAsset/asset.lock/0/0 | +6.08% | +31.95% | +3.46% |
| Entitlement/entitlement.activate/12/0 | +0.06% | +14.01% | -0.72% |
| Listing/listing.cancel/3/0 | -0.52% | +37.65% | -0.06% |
| Escrow/escrow.release/2/0 | -2.65% | +28.90% | -3.54% |

Timing disposition: inconclusive for tail performance; no universal speedup or
performance-gate pass is claimed. The aggregate unborn asset refusal median
rose 6.08% in the repeated run, but paired median deltas range from -3.3% to
+28.1%, with a median of +3.5%; the first attempt aggregate was +2.30%.
Candidate pair four is slower across unrelated operations, indicating host/order
interference. All samples remain retained. The identical-binary control has
median label differences within 0.83% but a zero-refusal p95 difference of
15.05%, confirming that sample tails need further isolation. Inspection of the
candidate binary shows the shared status adapter inlined: no additional
`single_from` symbol or dispatch layer. The authoritative `require_from` and
exact refusal allocation behavior remain shared and unchanged. This does not
rule out instruction-layout costs. The F8 isolated performance campaign must
resolve any repeatable important-path regression before release.

Reproduce from repository root using `python3 <path>/run.py` after copying
`check.rs` to `target/profile-guards/bench/check.rs`; the runner writes only
under that target directory. Run `repeat.py` and `same-binary.py` only after
preserving earlier output in the subdirectories named by those scripts.
No benchmark output replaces the original behavior oracle.
