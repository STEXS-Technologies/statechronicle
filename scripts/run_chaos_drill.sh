#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${root_dir}"

iterations="${STATECHRONICLE_CHAOS_ITERATIONS:-5}"
base_port="${STATECHRONICLE_POSTGRES_TEST_PORT:-55532}"
if ! [[ "${iterations}" =~ ^[1-9][0-9]*$ ]]; then
  echo "STATECHRONICLE_CHAOS_ITERATIONS must be a positive integer" >&2
  exit 2
fi
if ! [[ "${base_port}" =~ ^[1-9][0-9]*$ ]] || (( base_port + iterations > 65535 )); then
  echo "STATECHRONICLE_POSTGRES_TEST_PORT must leave room for all iterations" >&2
  exit 2
fi

for iteration in $(seq 1 "${iterations}"); do
  port=$((base_port + iteration - 1))
  container="statechronicle-chaos-${iteration}"
  url="host=127.0.0.1 port=${port} user=postgres password=postgres dbname=statechronicle"
  cleanup() { docker rm -f "${container}" >/dev/null 2>&1 || true; }
  cleanup
  trap cleanup EXIT

  echo "[${iteration}/${iterations}] start isolated PostgreSQL"
  docker run --detach --name "${container}" \
    --env POSTGRES_PASSWORD=postgres --env POSTGRES_DB=statechronicle \
    --publish "${port}:5432" postgres:16-alpine >/dev/null
  for _ in $(seq 1 40); do
    pg_isready -h 127.0.0.1 -p "${port}" -U postgres -d statechronicle >/dev/null 2>&1 && break
    sleep 1
  done
  pg_isready -h 127.0.0.1 -p "${port}" -U postgres -d statechronicle >/dev/null

  echo "[${iteration}/${iterations}] run tests while injecting SIGKILL"
  log="${TMPDIR:-/tmp}/statechronicle-chaos-${iteration}.log"
  set +e
  STATECHRONICLE_POSTGRES_URL="${url}" \
    cargo test -p statechronicle-postgres --test integration --all-features --locked \
    -- --test-threads=4 >"${log}" 2>&1 &
  test_pid=$!
  sleep "${STATECHRONICLE_CHAOS_KILL_DELAY:-1}"
  docker kill --signal KILL "${container}" >/dev/null 2>&1
  wait "${test_pid}"
  first_status=$?
  set -e
  echo "fault-injected run exit=${first_status} (expected nonzero or interrupted)"

  docker start "${container}" >/dev/null
  for _ in $(seq 1 40); do
    pg_isready -h 127.0.0.1 -p "${port}" -U postgres -d statechronicle >/dev/null 2>&1 && break
    sleep 1
  done
  pg_isready -h 127.0.0.1 -p "${port}" -U postgres -d statechronicle >/dev/null

  echo "[${iteration}/${iterations}] replay schema migration and verify recovery"
  docker exec -i "${container}" psql -v ON_ERROR_STOP=1 -U postgres -d statechronicle \
    < docs/OPERATIONS/postgres_schema.sql >/dev/null
  STATECHRONICLE_POSTGRES_URL="${url}" \
    cargo run -q -p statechronicle-postgres --example verify_integrity --all-features --locked

  replay_db="statechronicle_replay_${iteration}"
  docker exec "${container}" createdb -U postgres "${replay_db}" >/dev/null
  replay_url="host=127.0.0.1 port=${port} user=postgres password=postgres dbname=${replay_db}"
  STATECHRONICLE_POSTGRES_URL="${replay_url}" \
    cargo test -p statechronicle-postgres --test integration --all-features --locked \
    -- --test-threads=4 --quiet
  STATECHRONICLE_BENCH_ITERATIONS=1 ./scripts/run_economy_bench.sh >/dev/null
  cleanup
  trap - EXIT
done

echo "chaos drill passed (${iterations} forced database restarts)"
