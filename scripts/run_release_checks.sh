#!/usr/bin/env bash
set -euo pipefail

workspace_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "${workspace_root}"

cargo fmt --all -- --check
cargo test --workspace --exclude statechronicle-fuzz --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo check --workspace --all-targets --all-features --locked
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --all-features --locked
cargo audit --no-fetch
cargo deny check advisories licenses bans sources

for target in $(find fuzz/fuzz_targets -maxdepth 1 -name '*.rs' -printf '%f\n' | sed 's/\.rs$//' | sort); do
  cargo run -q -p statechronicle-fuzz --bin "${target}" -- -runs=100
done

git diff --check
echo "release checks passed"
