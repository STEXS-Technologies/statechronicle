# Rust version support

The library's minimum supported Rust version (MSRV) is **1.88.0**. All thirteen
library crates inherit `rust-version = "1.88"` from the workspace. The standalone
native-domain tutorial and adapter conformance workspace declare the same minimum.
The file-ledger tutorial requires **1.89.0** for the standard library's
file-locking API; this is an example-adapter requirement, not a library dependency.

`rust-toolchain.toml` pins **1.100.0** for repository development and release
tooling. Cargo does not apply a dependency repository's toolchain pin to an
application that depends on its crates. Use `cargo +1.88.0 ...` to select the
minimum compiler when working inside this checkout. Sanitizer fuzz campaigns
use nightly separately.

## Why 1.88?

The current locked dependency graph includes SHA-2 0.11.1 and Proptest 1.12.0,
both declaring Rust 1.88. The code also uses let chains and slice `as_chunks`,
which are supported by this compiler. Supporting 1.85 would require additional
source and dependency changes; it is not claimed by this branch.

## Minimum-version validation

The CI `msrv` job selects Rust 1.88.0 explicitly and checks:

- All finite workspace tests and examples, plus doctests, with all features.
- The standalone native-domain tutorial tests on 1.88 and file-ledger tutorial
  tests on its own 1.89 minimum.
- The isolated adapter conformance suite.
- Independent `statechronicle-profile` and `statechronicle-portable` builds for
  `riscv32imafc-unknown-none-elf`.
- Unchanged lockfiles throughout the run.

Local validation passed 897 workspace tests, 16 doctests, 26 conformance tests
and 4 native-tutorial tests on Rust 1.88.0. All workspace targets, including fuzz
binaries, passed a stable compile check; both portable cores compiled for bare
metal. The file-ledger tutorial passed all 12 tests on Rust 1.89.0, including
its process-termination/reopen tests. No lockfiles changed.

Run the same commands locally after installing Rust 1.88.0 and 1.89.0:

```sh
cargo +1.88.0 test --workspace --exclude statechronicle-fuzz --all-targets --all-features --locked
cargo +1.88.0 test --workspace --exclude statechronicle-fuzz --doc --all-features --locked
cargo +1.88.0 test --manifest-path ci/conformance/Cargo.toml --all-targets --locked
cargo +1.88.0 test --manifest-path tutorials/minimal-native-domain/Cargo.toml --all-targets --locked
cargo +1.89.0 test --manifest-path tutorials/durable-custom-ledger/Cargo.toml --all-targets --locked
rustup target add --toolchain 1.88.0 riscv32imafc-unknown-none-elf
cargo +1.88.0 check -p statechronicle-profile --target riscv32imafc-unknown-none-elf --locked
cargo +1.88.0 check -p statechronicle-portable --target riscv32imafc-unknown-none-elf --locked
```

These minimum-version checks supplement the pinned development-toolchain release
checks. They do not replace extended fuzzing, package qualification or production
adapter validation. Dependency updates must continue to pass the MSRV job;
raising the declared minimum requires an explicit compatibility change.
