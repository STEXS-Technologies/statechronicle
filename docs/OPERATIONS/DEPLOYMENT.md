# Deployment example and drills

StateChronicle is a library, so the application owns the HTTP server, worker
processes, secret manager, and network policy. PostgreSQL is the recommended
single transactional database for player-facing mutations.

## Local deployment example

```bash
export STATECHRONICLE_POSTGRES_PASSWORD='use-a-secret-manager-in-real-deployments'
docker compose -f deploy/docker-compose.postgres.yml up -d --wait
export STATECHRONICLE_POSTGRES_URL="host=127.0.0.1 port=5432 user=statechronicle password=${STATECHRONICLE_POSTGRES_PASSWORD} dbname=statechronicle"
cargo run -p statechronicle-postgres --example verify_integrity --all-features
```

The Compose example applies the checked-in baseline schema only when the
volume is initialized. For an existing volume, apply schema migrations through
the normal migration job before restarting the service; never rely on a
container restart to mutate an already-populated database.

In production, bind PostgreSQL to a private network, use TLS, rotate the
password through the deployment secret manager, and run the verified startup
constructor before accepting mutations. Do not expose the database port to
the public internet.

## Reproducible drills

Run the complete local drill (SQLite recovery tests followed by an isolated
PostgreSQL container and live adapter tests):

```bash
./scripts/run_deployment_drill.sh
```

The drill intentionally exercises rollback, idempotency races, lease takeover,
canonical-head races, concurrent schema installation, integrity scans, file
reopen, and partial-transaction/crash behavior. Preserve its output as release
evidence. It does not replace production KMS/HSM, backup-restore, load/soak,
or alerting game-days.

For a repeatable contention/soak campaign, increase the iteration count as
appropriate for the release window:

```bash
STATECHRONICLE_LOAD_ITERATIONS=5 ./scripts/run_load_drill.sh
```

This executes the real multi-connection SQLite load and crash tests and the
live PostgreSQL race suite on a fresh database per iteration. Record the
iteration count, host resources, timings, and any retry/lock metrics alongside
the release evidence.

Run the release-mode economy baseline across inventory, currency, marketplace,
and trade scenarios:

```bash
STATECHRONICLE_BENCH_ITERATIONS=10 ./scripts/run_economy_bench.sh
```

This measures the pure protocol/example path only; database, network, signer,
quota, and broker latency must be measured again in the deployed service.

For forced database-failure chaos testing, run:

```bash
STATECHRONICLE_CHAOS_ITERATIONS=10 ./scripts/run_chaos_drill.sh
```

Each iteration kills PostgreSQL with `SIGKILL` during the live test suite,
restarts it, replays the transactional schema migration (as a deployment
migration job would), verifies the interrupted database in place, and runs the
complete race suite against a fresh replay database. Preserve the migration,
integrity output, and interrupted test log as evidence.

Run every core fuzz target concurrently for one hour with:

```bash
./scripts/run_fuzz_parallel.sh
```

Set `STATECHRONICLE_FUZZ_SECONDS` for a different budget. Each target gets an
independent log and crash-artifact directory; the script returns nonzero if
any target finds a crash or exits unexpectedly.
