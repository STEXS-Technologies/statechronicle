#!/usr/bin/env bash
set -euo pipefail

# Reproducible local equivalent of the CI PostgreSQL integration gate.
# Requires Docker with a locally available postgres:16-alpine image.
container_name="statechronicle-postgres-gate"
port="${STATECHRONICLE_POSTGRES_TEST_PORT:-55432}"
url="host=127.0.0.1 port=${port} user=postgres password=postgres dbname=statechronicle"

cleanup() {
  docker rm -f "${container_name}" >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup

docker run --detach --name "${container_name}" \
  --env POSTGRES_PASSWORD=postgres \
  --env POSTGRES_DB=statechronicle \
  --publish "${port}:5432" \
  postgres:16-alpine >/dev/null

for _ in $(seq 1 30); do
  if pg_isready -h 127.0.0.1 -p "${port}" -U postgres -d statechronicle >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
pg_isready -h 127.0.0.1 -p "${port}" -U postgres -d statechronicle >/dev/null

STATECHRONICLE_POSTGRES_URL="${url}" \
  cargo test -p statechronicle-postgres --test integration --all-features --locked -- --test-threads=1
