# Completed library release qualification

Qualification starts from `94eb5d4`; `0799afe` fixes only the package checker's
shared build-cache contamination. `candidate-equivalence.json` proves identical
library/test/fuzz/conformance/tutorial/manifests/locks across those commits and `e880a2a` (which only separates Cargo stderr from characterization JSON stdout).
Subsequent documentation/evidence commits must preserve those tested scopes.
No version bump, tag or registry publication is part of this qualification.
The complete final gate recheck passes; the requirement audit records the reviewed
library scope and consumer/backend/measurement limits.

| Gate | Current evidence | Status |
|---|---|---|
| Full stable/MSRV local driver | `../local-release-94eb5d4/manifest.json`, `verification.json` | Passed; pinned stable and declared MSRV are both 1.100.0; 939 driver test executions plus 16 standalone tutorial tests (955 indexed total), and 27 ASAN smoke targets |
| Frozen original/current hosted/portable/bare-metal consumer | `../../compat/20261116T180227Z-1914823/manifest.json` | Passed, both lanes |
| Fresh unpacked package builds/tests/docs/lint | `../../packages/20261116T180422Z-1975357/manifest.json` | Passed, thirteen archives |
| Initial cached package attempt | `../../packages/20261116T180227Z-1914820/manifest.json` | Failed; retained; fresh isolation fixes the driver |
| Hosted coverage and unchanged ratchet | `../../coverage/hosted-94eb5d4/manifest.json` | Passed; full 92.8701%, filtered 94.4119% |
| Compiler API source/catalog validation | `verify_api.py`, `api-comparison.json`, `api-structural-diff.json` | Passed; 86 added, zero removed, 22 changed records; final catalogs equal earlier reviewed catalogs |
| Consumer API review | `api-review.md`, external canonical benchmark source/build logs | Reviewed; both external lanes compile, concurrent retry smoke passes |
| Source ownership and footprint | `maintenance-review.md`, both metrics JSON files | Measured; growth and retained distinct contracts explicitly justified |
| Concurrent mixed history/reference controls | `../../conformance/concurrent-mixed-0799afe/` | Positive/static checks pass; original ineffective deadlock control retained as failed; corrected fresh control detects runtime timeout |
| File process termination/restart | Full driver's nested durable tutorial tests; `process-lock-control/lock-negative.json` | Passed positive tests and intended OS-lock-bypass runtime rejection |
| Canonical claim/durable seeded controls | `../../fuzz/claim-final-0799afe/`, `../../fuzz/durable-final-0799afe/` | Both pass; two seeded positives each and two intended runtime rejections each; retained source/artifact verifier passes |
| Strict actual-original proof-service trace parity | `../../maintenance/proof-service/20261116T182114Z-2235511/manifest.json` | Passed; all 2,352 cases match exactly, including errors and callback order |
| Semantic string/executor/profile/settlement/outbox parity | Final fresh parity manifests under `evidence/maintenance/` and `evidence/f2/executor-order/` | Passed; independent raw trace/source/build-log verification confirms 348,621 distinct cases |
| Explicit requirement inventory | `requirement-inventory.json`, `supplemental-requirements.json`, `design-requirements.json` | 138 numbered clauses + 84 supplemental clauses + 85 literal H1–H8/work-map/discipline clauses (307 rows, overlap retained); 306 clauses reviewed and verified; F8-9 version/tag/publication remains a separate release action |
| Extended all-target fuzz | `../../fuzz/extended-94eb5d4/manifest.json` | Passed: all 27 independently seeded targets completed at least 3600 reported seconds each, zero crash artifacts, fresh ASAN build |
| Repeated seeded property targets | `properties/manifest.json` | Passed; three seeds, nine target binaries and 171 test executions; default case budget 1024, explicit target overrides preserved |
| Matched original/dependency/control/candidate timings | `after-fuzz.json` | Passed; independently verified matched four-lane matrix and individual-action observations |
| Canonical facade/concurrent retry timings and allocations | `canonical-performance/manifest.json` | Passed; seven alternated cycles and 26 allocation probes; acceptance recorded in `performance-review.md` |
| Cold-build cost and binary/dependency graph | `after-fuzz.json`, `repeat-build-costs.json` | Passed; six fresh builds, independently verified in `build-cost-verification.json` |
| All 22 compatibility paths: individual latency and allocations | `complete-performance.json` | Latency passed; timeout attempts and invalid shared-setup arithmetic retained. Fresh corrected allocation binaries pass 88 full 1000-action probes plus 88 workload-matched zero-action setup controls; independent performance verification passes all 222 allocation probes across campaigns. |
| Complete signed-history verification and verified checkpoint resume | `after-history.json`, `history-parallel-completion.json`, `history-performance-verification.json` | Passed; fourteen timing processes and eighteen allocation probes, including full ten-action 256-commit authentication in both lanes. All six allocation pairs match; latency and absolute scaling costs are accepted in `performance-review.md`. |
| Signed-history original deadline and initial final-check queue | `after-history-failed-900-seconds.json`, `history-performance/manifest-failed-900-seconds.json`, `await-final-history-failed-900-seconds.json` | Failed and retained; no success inferred from the timed-out allocation process. The completed guarded integration supersedes the failed queue with the full positive scope. |
| Performance acceptance and complete requirement audit | `performance-review.md`, `requirement-audit.json`, `final-gate-recheck.json` | Accepted; all seventeen final gates pass and all 307 literal roadmap rows are accounted for (306 verified, one separate publication action). Branch commit/push is the delivery action for this changeset. |

`after_fuzz.py` completed its orchestration: it verified live fuzz PIDs while
waiting, then ran the matched matrix, individual request timing, Valgrind
allocation probes, canonical facade timing/allocation probes, and baseline/
candidate cold consumer builds. Raw observations are retained without rejection
or subtraction of timer costs. Its completed results pass independent review. Failed attempts remain separately
retained and unqualified.

The baseline matrix uses equal old-protocol workloads plus a refreshed-dependency
control. The canonical benchmark uses equal public APIs on an earlier coherent
0.3 checkpoint and current code, because original 0.2 lacks those receipt/claim
authority contracts. Its synchronous consumer-owned fake does not establish
physical durability or general database scalability. The selected file tutorial
SIGKILL tests do not establish power-loss behavior. Production backend/key/service
qualification remains consumer-owned, as specified by the repository scope.

## Evidence retention and reproduction

The commit retains source fixtures, drivers, manifests, raw compressed logs,
measurements, review dispositions and packaged source archives. Generated native
executables, target directories, Python caches, live logs and the internal
session notebook are excluded. Binary hashes record the artifacts actually
measured; they do not imply the executable bytes are included in Git.

The independent verifiers were run in this qualification workspace and some
inspect its generated binaries or consumer directories, whose absolute paths
remain in the provenance records. They are not a self-contained offline check
for a fresh clone. Reproduction requires the recorded Git revisions, toolchains,
registry dependencies and a fresh run of the retained campaign drivers into new
output directories. Compare the resulting contracts and measurements with their
stated scope and host conditions; timestamps, paths, timings and rebuilt binary
hashes are not promised to be identical across environments.
