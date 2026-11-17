# Performance qualification checkpoint

Current qualification is tracked in the [final release audit](../qualification/final-94eb5d4/README.md), with matched original/dependency/control/candidate measurements, individual-action samples and explicit regression decisions. The campaigns below retain their original checkpoint scope.

Candidate: `a889f172a65bf203a0dde989d372dd111ae4b73e`. Production Rust remains unchanged from `1e995abc29af266b487654f32fcf321c4b335e7a`.

The retained campaigns compare the original `c418e98` release source, that source with refreshed dependencies, the pre-completion coordinator `404dda5`, and the candidate. All use Rust 1.100.0, CPU affinity, rotated/reversed lane order, seven cycles, and retained raw logs. This is shared-host evidence, with no discarded observations.

- [Four-lane matrix](matrix-20261116T123514Z-3594158/manifest.json): 22 workloads plus four extended workloads at twenty times the original iteration count. Percentiles describe 31 batch averages, not individual requests.
- [Individual requests](per-request-20261116T123934Z-3779170/manifest.json): 3,100 observations per single/refusing/planning workload per process, 1,240 for trade; nearest-rank p95/p99. Median empty-timer observation is 20 ns, without subtraction.
- [Allocation probes](allocations-20261116T124253Z-3908094/manifest.json): all twenty Valgrind probes passed. Native and portable totals equal their lane control. Hosted control-subtracted totals are 23 allocations/362 bytes per action; proof totals are 29 allocations/41,128 bytes per action, identical across lanes. These are bounded fixture observations including fixed setup, not universal allocation guarantees.
- [Initial failed matrix](matrix-20261116T123358Z-3544678/manifest.json): original source still requires the commit crate SHA-2 runtime edge. The first dependency-control build removed it and failed before timing. The corrected control retains that edge. Registry versions, checksums and resolved features match the newer lanes; local dependency edges are explicitly recorded, not claimed identical.

Against the pre-completion control, individual-request refusing-batch median paired p95 changes by **−0.70%**, versus **+2.56%** for planning and **−0.40%** for trade. Refusing-batch p95 changes span approximately −5.77% to +5.23%; trade includes +341.74% and +1,155.60% cycle outliers. The extended batch-average refusing-batch p95 changes by +7.05%. These different measurements do not establish a consistent individual-request refusing-batch regression, and do not establish universal performance neutrality. No production optimization follows from this campaign.

Each directory retains its exact runner, source/lock provenance, subprocess statuses, hashes and raw observations. Run matrix and per-request runners from the repository root; the latter references the retained matrix manifest. Allocation reproduction requires the matrix binaries at their recorded target paths, or updating its source campaign paths after rebuilding a new matrix. Timing and allocation instrumentation are separate.

Full performance qualification remains open, including canonical facade absolute costs, concurrent storage workloads, planning tail variability and explicit acceptance budgets. Existing narrower compatibility write workloads do not prove equivalence of stronger canonical receipt guarantees. No coverage, package-release, physical crash durability or release-completion claim is made here.
