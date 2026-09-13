#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${root_dir}"

iterations="${STATECHRONICLE_LOAD_ITERATIONS:-5}"
base_port="${STATECHRONICLE_POSTGRES_TEST_PORT:-55432}"
if ! [[ "${iterations}" =~ ^[1-9][0-9]*$ ]]; then
  echo "STATECHRONICLE_LOAD_ITERATIONS must be a positive integer" >&2
  exit 2
fi
if ! [[ "${base_port}" =~ ^[1-9][0-9]*$ ]] || (( base_port + iterations > 65535 )); then
  echo "STATECHRONICLE_POSTGRES_TEST_PORT must leave room for all iterations" >&2
  exit 2
fi

for iteration in $(seq 1 "${iterations}"); do
  echo "[${iteration}/${iterations}] SQLite contention/recovery campaign"
  for test_name in \
    bounded_multi_tenant_claim_load_isolated \
    thirty_two_concurrent_claims_have_one_winner \
    process_crash_before_commit_releases_reservation \
    dropped_transaction_leaves_no_partial_ledger_rows; do
    cargo test -p statechronicle-sqlite --lib --all-features --locked \
      "${test_name}" --quiet
  done

  echo "[${iteration}/${iterations}] PostgreSQL live race campaign"
  STATECHRONICLE_POSTGRES_TEST_PORT="$((base_port + iteration - 1))" \
    ./scripts/run_postgres_integration.sh
done

echo "load/recovery drill passed (${iterations} iteration(s))"
