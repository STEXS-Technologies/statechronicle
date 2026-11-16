# Frozen external release consumer

The Rust tests, support adapters, native domains and JSON vectors here are
byte-for-byte copies from the original release. `provenance.json` records their
original paths and hashes. Do not update expectations using candidate code.
The separate Cargo manifest is deliberately outside the root workspace and
imports only `statechronicle` plus ordinary third-party consumer dependencies.

Run `RUSTC_WRAPPER= CARGO_BUILD_JOBS=8 python3 scripts/refactor_compat.py` from
the repository root. This runs the same frozen consumer against the original
release and candidate source, in separate directories, with each revision's
lockfile as the dependency seed. The fixture is not a new independent model;
it preserves released behavior and original native-domain, planning, durable
adapter, recovery, proof and legacy import tests. A new independent adapter
conformance harness and exhaustive API/feature/portable consumers remain
separate requirements.

The runner first resolves the external manifest offline against the supplied
lockfile, permits Cargo to add the consumer package/remove unused workspace
packages, and rejects any new third-party version/checksum not present in that
revision's original lockfile. Tests then run with `--locked --offline`.
No network resolution, GitHub workflow, publishing or fixture regeneration is
part of this command.

Use `--portable` to include a second independent consumer with no-std native
state, fixed-capacity canonical encoding and transition refusal checks. It runs
on the host and compiles independently for `riscv32imafc-unknown-none-elf`.
`--lane baseline` or `--lane candidate` can qualify one lane after a localized
change without rerunning an unaffected lane. The default runs both.

`tests/exhaustive.rs` freezes exhaustive matches of the eight central released
enums. `tests/wire_compat.rs` is a new reader of original-version signed history;
its goldens come from `ci/wire-0.2/`, never from the candidate. These new tests and
the no-std consumer are not represented as byte-for-byte released source.
