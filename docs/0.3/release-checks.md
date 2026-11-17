# Local release checks

Run from a committed checkout with Rust 1.100.0, Python 3.12+, rustup and the
Cargo audit, deny and fuzz tools installed:

```sh
bash scripts/run_release_checks.sh
```

Rustup may install the selected embedded target and nightly fuzz compiler;
Cargo and advisory checks may require network access. The script does not
publish packages or dispatch GitHub workflows.

Before isolated offline checks, `cargo fetch --locked` populates the complete
dependency graph, including platform-specific packages that a Linux-only build
does not download. The tutorial verifiers then run offline with unchanged locks.
CI uploads their logs and the native-contract logs even when verification fails.

The driver runs finite workspace tests and doctests, strict Clippy, compilation,
rustdoc, formatting, isolated tutorials, native-domain contracts, adapter
conformance, embedded checks, dependency policy and bounded fuzz smoke. It checks
lockfile drift and whitespace before reporting success. Isolated tutorial and
package checks archive committed source, so commit source changes before running.

Fuzz smoke enumerates every manifest target and uses ASAN with 100-run / 5-second
libFuzzer limits. These are fuzzer stop conditions, not external wall-clock
deadlines; initialization may take longer. Smoke does not replace an extended
campaign.

## Additional qualification

- [Coverage collection and ratchet](coverage.md).
- [Isolated crate archive tests](package-checks.md).
- [Original-release compatibility](release-compatibility.md), public API review,
  matched performance measurements and extended property/fuzz/crash campaigns.
- Production adapter, key and deployment qualification by the consuming application.

Dedicated canonical API fuzz campaigns use immutable source archives, independent
seeds and deliberately insecure controls that must fail at the intended runtime
assertion:

```sh
python3 scripts/check_claim_fuzz.py --candidate <commit> --output <new-directory>
python3 scripts/check_fuzz_campaign.py --candidate <commit> --target fuzz_durable_recovery --output <new-directory>
```

`python3 scripts/test_release_driver.py` checks fail-fast orchestration with fake
external tools. It is part of the driver and does not substitute for real Rust,
advisory or fuzz runs.

Generated evidence directories are ignored by Git. Retain manifests, commands,
source hashes and raw logs as build/release artifacts. See the
[qualification summary](qualification.md) for the completed candidate and its
historical record. A passing local driver alone does not establish all release
gates or authorize versioning, tagging or publication.
