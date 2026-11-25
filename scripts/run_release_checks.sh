#!/usr/bin/env bash
set -euo pipefail

workspace_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "${workspace_root}"

python3 scripts/check_advisory_exceptions.py
python3 scripts/test_advisory_exceptions.py
cargo fmt --all -- --check
python3 scripts/test_release_driver.py
# Dev-only adapter harness is an isolated workspace; root --workspace omits it.
cargo fmt --manifest-path ci/conformance/Cargo.toml --all -- --check
cargo test --manifest-path ci/conformance/Cargo.toml --all-targets --locked
cargo clippy --manifest-path ci/conformance/Cargo.toml --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --manifest-path ci/conformance/Cargo.toml --no-deps --locked
cargo fmt --manifest-path ci/signature-conformance/Cargo.toml --all -- --check
cargo test --manifest-path ci/signature-conformance/Cargo.toml --release --locked
cargo clippy --manifest-path ci/signature-conformance/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path ci/wire-0.3/consumer/Cargo.toml --all-targets --locked
cargo test --workspace --exclude statechronicle-fuzz --all-targets --all-features --locked
cargo test --workspace --exclude statechronicle-fuzz --doc --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo check --workspace --all-targets --all-features --locked
bash scripts/check_embedded.sh
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --all-features --locked
# Metadata resolves every target, including packages Linux builds do not fetch.
# Populate the complete locked graph before the isolated offline consumers run.
cargo fetch --locked
# Standalone applications are separate workspaces and must compile outside ours.
python3 scripts/check_tutorials.py --tutorial minimal-native-domain
python3 scripts/check_tutorials.py --tutorial durable-custom-ledger
# Compiler refusals and dynamic schema checks use a fresh external consumer.
python3 scripts/check_native_contracts.py
# Unsoundness and other advisory warnings must fail, including isolated locks.
for lockfile in Cargo.lock ci/conformance/Cargo.lock ci/signature-conformance/Cargo.lock \
  ci/wire-0.3/consumer/Cargo.lock \
  tutorials/minimal-native-domain/Cargo.lock tutorials/durable-custom-ledger/Cargo.lock; do
  cargo audit --file "${lockfile}" --deny warnings
done
cargo deny check advisories licenses bans sources

rustup toolchain install nightly --profile minimal
fuzz_host_target="$(rustc +nightly -vV | sed -n 's/^host: //p')"
test -n "${fuzz_host_target}"
# Capture list failure before parsing: process substitution would hide its exit.
fuzz_target_list="$(cargo +nightly fuzz list)"
test -n "${fuzz_target_list}"
mapfile -t fuzz_targets <<< "${fuzz_target_list}"
for target in "${fuzz_targets[@]}"; do
  # `cargo run` builds a normal binary without sanitizer-coverage. Use the
  # cargo-fuzz toolchain so this gate actually exercises coverage-guided,
  # ASan-instrumented fuzz targets.
  cargo +nightly fuzz run "${target}" --target "${fuzz_host_target}" -- -runs=100 -max_total_time=5
done

# Builds (including cargo-fuzz) must not silently resolve a different graph.
git diff --exit-code -- Cargo.lock ci/conformance/Cargo.lock ci/signature-conformance/Cargo.lock \
  ci/wire-0.3/consumer/Cargo.lock \
  tutorials/minimal-native-domain/Cargo.lock tutorials/durable-custom-ledger/Cargo.lock
git diff --check
echo "release checks passed"
