# Deployment and release drills

StateChronicle is a protocol library. The application owns the storage
implementation, HTTP or RPC server, worker processes, secret manager, and
network policy. Implement the contracts in `statechronicle-ports` at the
composition boundary and keep durable writes, projections, and outbox delivery
under the application's transaction policy.

## Release checks

Run the production release checks locally:

```bash
./scripts/run_release_checks.sh
```

The checks cover formatting, workspace tests, Clippy with warnings denied,
documentation warnings, dependency policy, fuzz smoke tests, and the coverage
ratchet. They do not replace production load, backup-restore, key-management,
or alerting game-days.

## Performance evidence

Run the release-mode economy baseline across inventory, currency, marketplace,
and trade scenarios:

```bash
STATECHRONICLE_BENCH_ITERATIONS=10 ./scripts/run_economy_bench.sh
```

This runs the examples as correctness smoke tests and measures one million
in-process pure-protocol operations on the optimized hot path. Measure storage,
network, signer, quota, and broker latency in the deployed application.

## Fuzzing

Run every core fuzz target concurrently for one hour with:

```bash
./scripts/run_fuzz_parallel.sh
```

Set `STATECHRONICLE_FUZZ_SECONDS` for a different budget. Each target gets an
independent log and crash-artifact directory; the script returns nonzero if a
target finds a crash or exits unexpectedly.
