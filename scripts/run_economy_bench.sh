#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${root_dir}"
iterations="${STATECHRONICLE_BENCH_ITERATIONS:-10}"
throughput_iterations="${STATECHRONICLE_BENCH_THROUGHPUT_ITERS:-1000000}"
minimum_ops_per_sec="${STATECHRONICLE_BENCH_MIN_OPS_PER_SEC:-1000000}"
if ! [[ "${iterations}" =~ ^[1-9][0-9]*$ ]]; then
  echo "STATECHRONICLE_BENCH_ITERATIONS must be a positive integer" >&2
  exit 2
fi
if ! [[ "${throughput_iterations}" =~ ^[1-9][0-9]*$ ]]; then
  echo "STATECHRONICLE_BENCH_THROUGHPUT_ITERS must be a positive integer" >&2
  exit 2
fi
if ! [[ "${minimum_ops_per_sec}" =~ ^[0-9]+$ ]]; then
  echo "STATECHRONICLE_BENCH_MIN_OPS_PER_SEC must be a non-negative integer" >&2
  exit 2
fi

echo "building release examples and hot-path benchmark"
cargo build --release -p statechronicle --example throughput --examples --locked --quiet

STATECHRONICLE_THROUGHPUT_ITERS="${throughput_iterations}" \
  STATECHRONICLE_THROUGHPUT_MIN_OPS_PER_SEC="${minimum_ops_per_sec}" \
  target/release/examples/throughput

for scenario in inventory currency marketplace trade_bundle trade_value trade_cross_tenant; do
  binary="target/release/examples/${scenario}"
  if [[ ! -x "${binary}" ]]; then
    echo "missing release example: ${binary}" >&2
    exit 1
  fi
  for _ in $(seq 1 "${iterations}"); do
    "${binary}" >/dev/null
  done
  printf '%-18s %8d correctness runs\n' "${scenario}" "${iterations}"
done

echo "economy benchmark passed"
