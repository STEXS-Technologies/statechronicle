# Legacy signature latency

Measured on the exact v0.3.0 commit
`9ed64a5a5ece51fa2cd5929119cbe21e1e34f3f4` with Rust 1.100.0,
the release profile, and the tagged lockfile. The benchmark source was
copied into an otherwise clean detached checkout. It uses one fixed key,
one 1,024-byte message, and one representative commit.

On Linux, copy `crates/statechronicle-commit/examples/signature_baseline.rs`
to the same path in the tagged checkout, then run:

```sh
cargo build --release --locked -p statechronicle-commit --example signature_baseline
taskset -c 0 target/release/examples/signature_baseline
```

Each run takes 100 warmup calls followed by 10,000 individually timed
calls per workload. Timings include timer overhead. Commit signing
includes copying the key ID, canonicalizing the commit and forming the
signed envelope. Commit verification includes canonicalization.

| Workload | Run | p50 (ns) | p95 (ns) | p99 (ns) |
| --- | ---: | ---: | ---: | ---: |
| Ed25519 sign | 1 | 21165 | 34473 | 55175 |
| Ed25519 sign | 2 | 21189 | 37168 | 60616 |
| Ed25519 sign | 3 | 21182 | 35055 | 59613 |
| Ed25519 verify | 1 | 43666 | 67457 | 112554 |
| Ed25519 verify | 2 | 43652 | 63317 | 82538 |
| Ed25519 verify | 3 | 43622 | 62936 | 82810 |
| Commit sign | 1 | 18985 | 19858 | 48892 |
| Commit sign | 2 | 18972 | 19840 | 38718 |
| Commit sign | 3 | 18988 | 23305 | 51956 |
| Commit verify | 1 | 42544 | 63327 | 88278 |
| Commit verify | 2 | 42611 | 73221 | 163710 |
| Commit verify | 3 | 42558 | 63298 | 96073 |

These were shared-runner observations. Tail variability makes paired
baseline and candidate runs on the same quiet runner important for
regression qualification. Allocation measurements and matched proof
and protected durable workflow measurements remain open.
