#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${root_dir}"
duration="${STATECHRONICLE_FUZZ_SECONDS:-3600}"
if ! [[ "${duration}" =~ ^[1-9][0-9]*$ ]]; then
  echo "STATECHRONICLE_FUZZ_SECONDS must be a positive integer" >&2
  exit 2
fi

host_target="$(rustc +nightly -vV | sed -n 's/^host: //p')"
test -n "${host_target}"
cargo +nightly fuzz build --target "${host_target}"
log_root="$(mktemp -d /tmp/statechronicle-fuzz-parallel.XXXXXX)"
pids=()
targets=()
for target in $(find fuzz/fuzz_targets -maxdepth 1 -name '*.rs' -printf '%f\n' | sed 's/\.rs$//' | sort); do
  targets+=("${target}")
  "target/${host_target}/release/${target}" -max_total_time="${duration}" \
    -artifact_prefix="${log_root}/${target}-" >"${log_root}/${target}.log" 2>&1 &
  pids+=("$!")
done
echo "started ${#pids[@]} targets for ${duration}s; logs: ${log_root}"

status=0
for i in "${!pids[@]}"; do
  if wait "${pids[$i]}"; then
    rc=0
  else
    rc=$?
    status=1
  fi
  printf '%s %s\n' "${rc}" "${targets[$i]}" >>"${log_root}/status"
done
cat "${log_root}/status"
exit "${status}"
