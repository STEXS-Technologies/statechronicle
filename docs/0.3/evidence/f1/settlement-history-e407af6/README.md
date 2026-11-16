# Historical settlement qualification checkpoint

Production source: `e407af6d5ac55b08f3659235fd6a3fe8a2b04218`.
Conformance lock correction: `b5257af` (identical production crates tree).

Archived source passes 862 finite all-target/all-feature workspace tests and
nine doctests on Rust 1.85.0 and 1.100.0. The fresh stable target also passes
strict all-target/all-feature Clippy and strict rustdoc. The root historical
trade integration has six functions, including genuine two-tenant asset/payment
facts, equal textual commit IDs, two assets in different commits, substituted
expectations, omitted/reordered/duplicate events, malformed signed transitions,
wrong signatures and independent key-policy refusal. A compile-fail doctest
checks that callers cannot manufacture the private receipt.

The frozen external consumer/wire matrix, new candidate overlay and portable
consumer/bare-metal compilation pass under MSRV. See
[the independent compatibility manifest](../../compat/20261116T064037Z-1717409/manifest.json).
Its immutable original fixtures and wire provenance remain unchanged.

`manifest.json` retains every archived command and exit code. It includes the
initial stable shared-target failure (a stale accumulator artifact lacked the
new module) and the successful fresh-target retry. At `e407af6`, the isolated
conformance lock was stale after removing the normal SHA-256 edge; this failure
is retained. `lock-correction-manifest.json` independently qualifies the corrected
`b5257af` archive: nine conformance functions pass on stable and MSRV, and strict
Clippy passes. The correction removes exactly one dependency edge, changes no
third-party package version/checksum, and changes no production source. These
are precise checkpoint claims, not a final exact-release F8 campaign.

All worktree logs remain explicitly unqualified, including an accidentally
unbounded fuzz binary launched by all-target testing and terminated with exit
143, the initial unborn-recipient refusal, and strict-lint corrections. The
finite qualified commands explicitly exclude `statechronicle-fuzz`; no fuzz
campaign is inferred from those tests.

## Production cost

The same pinned tree-sitter measurement at the parent and candidate shows
+15,466 source bytes, +305 syntax lines, +2,073 lexical tokens, +54 branch nodes
and +30 public syntax items. Full compressed measurements and limits are in
`metrics*.json.gz`, `parent-metrics.json.gz` and `metrics-delta.json`. The new
expected-event/payment inputs and private receipt enforce a previously absent
authority boundary; this is a justified additive cost, not a source reduction.
The overall F6 flat/reduced target remains unmet.

`root-cost/` retains the exact benchmark source, runner, resolved locks, binary
hashes, seven alternating paired runs and allocation profiles. The immediate
parent and candidate share compiler and third-party versions/checksums. All
1/3/256-event roots agree. Median paired latency deltas are +1.69% (+11.87 ns),
+2.04% (+43.25 ns), and +1.01% (+1,697.85 ns); paired p95 sample-average deltas
are +1.87%, +1.87% and +0.29%. This is a measured small cost on a shared host,
not a speedup or isolated release acceptance. Full batches make the new verifier
perform additional checks; this benchmark does not measure that new contract.

Matched-width command-line allocation controls supersede an initial
mixed-width attempt; both are retained. Control-subtracted allocation totals
match between parent and candidate. No portable allocation regression is
inferred merely from this hosted benchmark; the separate portable checks pass.

## Reproduction and limits

The retained `qualify*.py` runners document exact archives, commands and safe
explicit environment overrides. Run in a disposable checkout at their recorded
source revision; output paths are relative to the repository. The first runner
stops at the retained failed stable attempt; `qualify-stable.py` records the
fresh-target retry. The correction runner starts from the corrected revision.
Never overwrite these retained evidence files to imply a failed attempt passed.

For the root benchmark, archive the parent and candidate to
`target/settlement-history/<full e407af6 SHA>/parent` and `/candidate`, copy
`root-cost/main.rs` to `target/settlement-history/root-bench/main.rs`, then run
`root-cost/run.py` and `root-cost/allocation-recheck.py` in the disposable clone.
Use the same Rust compiler, frame-pointer configuration, CPU affinity and
Valgrind version from the manifest. Original and matched-width allocation logs
are retained separately.

This verifies independently requested historical signed facts, not canonical
chain membership, current ownership, whole lifecycle completeness, unsigned
value declarations, physical truth or cross-tenant atomicity. Service witness
composition, broader adversarial properties, new-contract performance, full
fuzz/coverage/crash/package/API review and remaining F2–F8 gates remain open.
