# Paired legacy signature latency at 6ab3b09

Seven matched pairs compare exact tag `v0.3.0`
(`9ed64a5a5ece51fa2cd5929119cbe21e1e34f3f4`) with
`6ab3b093f8227aa1eceaec0c4c93e1826bb637b1`. Both lanes used identical
benchmark source, the Rust 1.100.0 release profile, and their locked
dependencies. Runs alternated lane order and pinned both binaries to
logical CPU 0 of a shared Intel Xeon Platinum 8573C runner. Each workload
records 10,000 individually timed operations after 100 warmup calls.

| Workload | Baseline p50 / p95 / p99 (ns) | Branch p50 / p95 / p99 (ns) | Median paired change |
| --- | --- | --- | --- |
| ed25519_sign_1024b | 35312 / 55089 / 122691 | 21228 / 39624 / 86011 | +0.0% / -5.7% / -7.0% |
| ed25519_verify_1024b | 56189 / 92324 / 178203 | 51951 / 87472 / 162229 | +0.0% / -1.9% / -5.2% |
| commit_sign | 31247 / 51155 / 127747 | 19040 / 23072 / 68900 | +0.3% / -5.4% / -6.3% |
| commit_verify | 54355 / 90848 / 176945 | 43682 / 73430 / 151631 | -0.9% / -6.7% / -4.5% |

Each absolute value is the median of the seven per-run percentiles.
Each paired change is the median of the seven candidate/baseline changes,
calculated within pairs. Shared-runner contention was visible and the
absolute lane medians are sensitive to it. The paired medians showed no
consistent regression in these legacy workloads. The tested raw
signature and legacy commit-signing source files match the release.
Repeat on an isolated runner for final release qualification.

[All per-run values](paired-signature-latency-6ab3b09.csv)
