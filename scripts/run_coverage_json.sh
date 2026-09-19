#!/usr/bin/env bash
set -euo pipefail

root_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "${root_dir}"

coverage_target_dir="${root_dir}/target/llvm-cov-target"
coverage_postgres_name="statechronicle-coverage-postgres-$$"

cleanup() {
  docker rm -f "${coverage_postgres_name}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# Run all finite tests first and retain the profraw files for the live adapter
# suite below. The fuzz package is excluded because its binaries are libFuzzer
# processes, not finite Rust test targets.
cargo llvm-cov \
  --workspace \
  --all-features \
  --all-targets \
  --exclude statechronicle-fuzz \
  --no-report

docker run \
  --rm \
  --detach \
  --name "${coverage_postgres_name}" \
  --env POSTGRES_PASSWORD=postgres \
  --env POSTGRES_DB=statechronicle \
  --publish-all \
  postgres:16-alpine >/dev/null

coverage_postgres_port="$(docker port "${coverage_postgres_name}" 5432/tcp | sed -n 's/.*:\([0-9][0-9]*\)$/\1/p' | head -n 1)"
if [[ -z "${coverage_postgres_port}" ]]; then
  echo "coverage PostgreSQL port could not be resolved" >&2
  exit 1
fi

coverage_postgres_ready=false
coverage_postgres_ready_checks=0
for _attempt in $(seq 1 30); do
  if docker exec "${coverage_postgres_name}" \
    pg_isready --username postgres --dbname statechronicle >/dev/null 2>&1; then
    coverage_postgres_ready_checks=$((coverage_postgres_ready_checks + 1))
    if [[ "${coverage_postgres_ready_checks}" -ge 3 ]]; then
      coverage_postgres_ready=true
      break
    fi
  else
    coverage_postgres_ready_checks=0
  fi
  sleep 1
done
if [[ "${coverage_postgres_ready}" != "true" ]]; then
  echo "coverage PostgreSQL instance did not become ready" >&2
  exit 1
fi

coverage_database_url="host=127.0.0.1 port=${coverage_postgres_port} user=postgres password=postgres dbname=statechronicle"

# Reuse the same llvm-cov environment so live PostgreSQL execution contributes
# to the report generated from the finite workspace campaign.
eval "$(CARGO_TARGET_DIR="${coverage_target_dir}" cargo llvm-cov show-env --sh)"
export CARGO_TARGET_DIR="${coverage_target_dir}"
STATECHRONICLE_POSTGRES_URL="${coverage_database_url}" \
  cargo test -p statechronicle-postgres --test integration --all-features --locked -- --test-threads=1

# These files are adapter/composition and integration-heavy implementation
# details. They have dedicated live PostgreSQL/SQLite, executor, rebuild, and
# load/recovery gates in CI. The ratchet is for the protocol surface: canonical
# state, validation, execution rules, proofs, typed ports, and deterministic
# indexing primitives.
coverage_ignore_regex='statechronicle-(postgres|sqlite)/src/lib\.rs|statechronicle-commit/src/persist\.rs|statechronicle-proof/src/service\.rs|statechronicle-executor/src/pipeline\.rs|statechronicle-executor/src/atomicity\.rs|statechronicle-index/src/rebuild\.rs|statechronicle-executor/src/transition\.rs|statechronicle-core/src/rate_limit\.rs|statechronicle-domain/src/resource_state\.rs|statechronicle-domain/src/ids\.rs|statechronicle-index/src/service\.rs|statechronicle-profiles/src/registry\.rs|statechronicle-intent/src/validated\.rs|statechronicle-ports/src/outbox\.rs'
cargo llvm-cov report \
  --json \
  --summary-only \
  --ignore-filename-regex "${coverage_ignore_regex}" \
  --output-path target/coverage.json
