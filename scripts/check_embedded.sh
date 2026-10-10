#!/usr/bin/env bash
set -euo pipefail

workspace_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "${workspace_root}"

# Check actual portable execution and verification, not just a trait-only crate.
# Compile each core independently so host feature unification cannot hide std.
embedded_target="riscv32imafc-unknown-none-elf"
embedded_toolchain="${STATECHRONICLE_EMBEDDED_TOOLCHAIN:-stable}"
rustup toolchain install "${embedded_toolchain}" --profile minimal
rustup target add --toolchain "${embedded_toolchain}" "${embedded_target}"
for portable_crate in statechronicle-profile statechronicle-portable; do
  cargo "+${embedded_toolchain}" check -p "${portable_crate}" \
    --target "${embedded_target}" --locked
done
