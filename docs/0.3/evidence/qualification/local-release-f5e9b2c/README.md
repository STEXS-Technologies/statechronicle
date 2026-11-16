# Complete local driver checkpoint

Candidate: `f5e9b2c69ddcb8054124c53901f0d1f79308022c`.
`bash scripts/run_release_checks.sh` exited **0** in an isolated detached
worktree, on Rust 1.100.0 (also the declared MSRV), in 258.85 seconds. All
4,699 tracked file hashes were unchanged before/after. The active checkout was
not used as the test source. Builds used an external target directory,
`RUSTC_WRAPPER=` and `CARGO_BUILD_JOBS=8`. No GitHub action was requested.

The retained compressed driver log reaches `release checks passed` after:

- Root and conformance formatting; three release-driver orchestration tests.
- 26 conformance, 897 finite root and 16 root doctest executions (**939 total**).
- Strict root/conformance Clippy and rustdoc, root all-target/all-feature check,
  and supported bare-metal compilation.
- Both isolated tutorial consumers: metadata, tests, exact CLI, strict Clippy
  and formatting, all five checks passing per tutorial.
- Native consumer contracts: 18 checks, including intended compiler refusals
  with required diagnostics/source attribution and restored positive builds.
- All four active lockfile audits denying warnings; cargo-deny advisories,
  licenses, bans and sources all passing.
- Every one of the 25 manifest fuzz targets built and run with nightly ASAN
  instrumentation, 100-run / five-second stop conditions, empty initial corpora,
  and a recorded actual random seed. All 25 completed 100 runs normally.
- Tracked lockfile-drift and whitespace checks.

`manifest.json` records exact revision/compiler/environment, all input hashes,
exit, elapsed time and collected artifact hashes. `runner.py` reproduces the
worktree run from the current HEAD into a fresh evidence directory; it does not
check out or modify the active branch. `nested/` retains the two tutorial and
native-contract manifests and logs produced inside that isolated worktree.

From the repository root, run:

```sh
python3 docs/0.3/evidence/qualification/local-release-f5e9b2c/verify.py
python3 docs/0.3/evidence/qualification/local-release-f5e9b2c/negative.py
```

The verifier compares the input hash map with an actual archive of the pinned
Git commit, checks 32 top-level recorded artifact hashes, validates nested
revision/status/expected exits and log hashes, and checks exact manifest fuzz
target coverage, seed/instrumentation markers and normal completion counts.
`verification.json` retains individual Rust result counts and hashes of all
three nested manifests. The negative control changes a tutorial metadata exit
in a temporary evidence copy to 501; verification exits 1 with the intended
`nested exit metadata` diagnostic. The original evidence is unchanged.

This is **the complete local driver**, not complete roadmap qualification.
Fuzz smoke is not an extended campaign; existing targets do not directly call
the new ClaimVerifier or protected durable receipt APIs. Adding and exercising
those campaigns remains open. This run also does not replace frozen cross-version
fixtures, full semantic parity, exact coverage, matched performance/build-cost
qualification, package/registry checks, public API review or requirement closure.
The package version remains 0.2.0 pending the roadmap's release gates. Tagging,
publishing, GitHub CI and PR creation remain separately controlled.
