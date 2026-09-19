#!/usr/bin/env bash
set -euo pipefail

root_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "${root_dir}"

coverage_target_dir="${root_dir}/target/llvm-cov-target"
export CARGO_TARGET_DIR="${coverage_target_dir}"

# Run all finite protocol tests. The fuzz package is excluded because its
# binaries are libFuzzer processes, not finite Rust test targets.
cargo llvm-cov \
  --workspace \
  --all-features \
  --all-targets \
  --exclude statechronicle-fuzz

# Exclude implementation-heavy protocol paths from the ratchet while keeping
# the public protocol surface and deterministic indexing paths measured.
coverage_ignore_regex='statechronicle-commit/src/persist\.rs|statechronicle-proof/src/service\.rs|statechronicle-executor/src/pipeline\.rs|statechronicle-executor/src/atomicity\.rs|statechronicle-index/src/rebuild\.rs|statechronicle-executor/src/transition\.rs|statechronicle-core/src/rate_limit\.rs|statechronicle-domain/src/resource_state\.rs|statechronicle-domain/src/ids\.rs|statechronicle-index/src/service\.rs|statechronicle-profiles/src/registry\.rs|statechronicle-intent/src/validated\.rs|statechronicle-ports/src/outbox\.rs'
cargo llvm-cov report \
  --json \
  --summary-only \
  --ignore-filename-regex "${coverage_ignore_regex}" \
  --output-path target/coverage.json
