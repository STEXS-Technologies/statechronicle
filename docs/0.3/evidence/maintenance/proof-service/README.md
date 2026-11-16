# Compatibility proof orchestration

Retained production candidate: `a818f9bd884bc6bd9754376d44cbfad811289b6a`.
Control: `b1257d7faecb7b9b3f27afdcada6d924df25c347` (production identical
to `d7167a1`). Prototype: `2b73ab7`. Compiler: Rust 1.100.0.

`ProofService::verify` now derives the state key and delegates to its existing
`verify_with_key` owner. The canonical variant keeps its original direct
sequence after a measured refusal-cost concern. No new helper, public type,
method, dependency, signature, wire format or cryptographic operation is added.
The independently expected current/historical claim APIs are unchanged.

## Independent original-release characterization

`ci/characterization/proof_service.rs` runs against separately archived library
revisions. It uses the immutable original wire corpus and records full Debug
and Display errors, success, and the exact tenant/commit lookup and head-read
order. Unused projection/proof/snapshot reads and all writes panic. Positive
controls require the genuine proof to verify; empty-subject derived-key cases
must refuse before any read. The matrix has 2,352 rows:

- Four compatibility methods: derived/explicit state key, ordinary/canonical.
- Fourteen valid/tampered proof or stored-signature cases.
- Found/missing/unavailable commit lookup; seven head configurations.
- Correct/different caller-supplied state keys (derived-key methods ignore that input).

The final run at `20261116T105936Z-326221/` passes exact comparison across the
actual original `v0.2.0` (`c418e98`), control and retained candidate. Each lane's
complete trace has SHA-256
`5fa3aab6fe076b026f28fc4fa9f6efad9a1bbdb3d1169deb181b95f4a3fbaea9`.
The prototype also passes at `20261116T105612Z-196422/`.

Reproduce with:

```sh
python3 scripts/check_proof_service_parity.py \
  --revision c418e98b91720188158c531cd5c2c4b8ab71cbcf \
  --revision b1257d7faecb7b9b3f27afdcada6d924df25c347 \
  --revision a818f9bd884bc6bd9754376d44cbfad811289b6a
```

Requires cached locked dependencies and Rust 1.100.0. No expected output is
regenerated into the frozen corpus. This characterizes compatibility acceptance,
including unsigned reference metadata those legacy helpers do not authenticate;
it does not promote them to the independent `ExpectedClaim` verification API.

## Measurements and retained non-equivalence

Seven alternating pairs use identical compiler, resolved consumer lockfiles,
fixed read adapter and workloads. The frozen benchmark changes only three calls
to exercise the affected derived-key methods. Every sample is retained.

| Candidate / path | Median paired median change | Median paired p95 change |
| --- | ---: | ---: |
| Prototype ordinary verification | +0.25% | +0.85% |
| Prototype canonical verification | +0.34% | -13.60% |
| Prototype stale-head refusal | +5.79% / +18.1 ns | +10.07% |
| Retained ordinary verification | -0.31% / -143.2 ns | -0.30% |
| Retained canonical verification | -2.04% | -16.45% |
| Retained stale-head refusal | -0.28% / -0.9 ns | -2.94% |

The prototype's stale-head median increases in all seven pairs. That raised the
roadmap's review trigger; the additional nested async delegation is not retained
on that short refusal path. The existing canonical orchestration stays direct
and shares its lookup, head comparison and pure bundle-verification owners.
The ordinary route's small paired changes do not show a repeatable regression.

These are noisy shared-host observations, not a speedup or final performance
qualification. Retained stale-head median pairs range from -32.92% to +43.58%;
canonical p95 pairs include a +3543.79% outlier. No sample is discarded. The
benchmark consumer binary is 4,078,864 bytes in control and 4,080,168 in the
retained lane (+1,304 / 0.032%); no binary shrink or allocation equivalence is
claimed. Each consumer archive and binary is hashed in its manifest. The
prototype and retained campaigns have separate `bench-*` directories.

The pinned production parser records a retained deletion of 147 raw bytes,
four syntax lines, 28 tokens and one parsed branch node, with public syntax
items unchanged. Whole-workspace retained totals are 16,536 syntax lines,
121,252 tokens, 1,984 branch nodes and 1,445 public syntax items. These are
syntax counts, not executable complexity or the completed 10–20% hotspot target.
Compressed before/prototype/retained reports and parser provenance are retained.

## Qualification and failed attempts

`qualification-a818f9b/` passes formatting, 884 finite hosted workspace tests,
16 doctests, strict all-target/all-feature Clippy (including fuzz compilation),
strict rustdoc and guarded source/lock/frozen-fixture immutability checks.
The fuzz binaries are excluded from finite test execution. The five-check
[frozen external consumer/portable/bare-metal matrix](../../compat/20261116T110216Z-413482/manifest.json)
also passes at the same candidate. Its original fixture provenance is validated
by the compatibility runner.

Two initial development runs (`105314` and `105400`) failed because the fixture's
unused subject-read stub had reversed parameter types. They remain unqualified.
The `105422` run uses `ae3d94b`, the dependency-patched starting point with Dalek
2, rather than the original release. Its exact comparison fails in 32 stored-
signature cases solely on a diagnostic suffix; typed errors, refusal and port
order agree. The actual original release and current branch both use Dalek 3.
This failure is retained, not normalized into a pass or called original-release
qualification. A prototype benchmark run failed before its first observation
because copying the binary lost executable permission; the corrected runner
preserves permissions and uses a separate evidence directory.

This checkpoint does not close executor error/effect parity, extended fuzz/crash
campaigns, fresh candidate coverage, the full performance matrix, package/release
version qualification or the broader F1–F8 roadmap. Existing original vectors
and all active lockfiles remain unchanged.
