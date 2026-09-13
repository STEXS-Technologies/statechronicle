#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${root_dir}"
iterations="${STATECHRONICLE_BENCH_ITERATIONS:-10}"
min_runs_per_sec="${STATECHRONICLE_BENCH_MIN_RUNS_PER_SEC:-0}"
if ! [[ "${iterations}" =~ ^[1-9][0-9]*$ ]]; then
  echo "STATECHRONICLE_BENCH_ITERATIONS must be a positive integer" >&2
  exit 2
fi
if ! [[ "${min_runs_per_sec}" =~ ^[0-9]+$ ]]; then
  echo "STATECHRONICLE_BENCH_MIN_RUNS_PER_SEC must be a non-negative integer" >&2
  exit 2
fi

echo "building release examples"
cargo build --release -p statechronicle --examples --locked --quiet

for scenario in inventory currency marketplace trade_bundle trade_value trade_cross_tenant; do
  binary="target/release/examples/${scenario}"
  if [[ ! -x "${binary}" ]]; then
    echo "missing release example: ${binary}" >&2
    exit 1
  fi
  start_ns="$(date +%s%N)"
  for _ in $(seq 1 "${iterations}"); do
    "${binary}" >/dev/null
  done
  end_ns="$(date +%s%N)"
  elapsed_ns=$((end_ns - start_ns))
  if (( elapsed_ns <= 0 )); then
    echo "clock did not advance while benchmarking ${scenario}" >&2
    exit 1
  fi
  runs_per_sec=$((iterations * 1000000000 / elapsed_ns))
  if (( runs_per_sec < min_runs_per_sec )); then
    echo "${scenario} throughput ${runs_per_sec}/s is below required ${min_runs_per_sec}/s" >&2
    exit 1
  fi
  printf '%-18s %8d runs  %12.3f ms total  %10d runs/s\n' \
    "${scenario}" "${iterations}" \
    "$((elapsed_ns / 1000000)).$(( (elapsed_ns / 1000) % 1000 ))" \
    "${runs_per_sec}"
done

echo "economy benchmark passed"
