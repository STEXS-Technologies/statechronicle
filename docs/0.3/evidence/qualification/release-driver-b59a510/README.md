# Local release-driver qualification

Candidate `b59a510984a1966ef353eb1fbac01f0132ec68b5` adds missing workspace
doctests, both isolated external tutorials, warning-denying audits of all four
active locks, explicit nightly setup, manifest-based nonempty fuzz enumeration,
libFuzzer time/run limits and a final tracked-lock diff gate. Library Rust,
production/isolated manifests and locks, fuzz and tutorial source are byte
unchanged from `3980f13`. No workflow file or trigger changes.

The real local command in the manifest exited 0 and printed success only after
all stages. Its 916 Rust test passes comprise 884 workspace finite tests,
16 workspace doctests and 16 isolated conformance finite tests. Both separately
linked tutorial manifests pass tests, exact output, strict Clippy, formatting,
lock stability and registry-package checks. Root/conformance lint/docs/format,
embedded builds, four `cargo audit --deny warnings` calls and cargo-deny passed.
All 25 cargo-fuzz ASAN smoke runs completed; the manifest extracts their actual
random seeds, commands, run counts and binary hashes from retained output.

Some targets initialize thousands of existing corpus inputs and stop before
new mutations because the requested run count was already exceeded. This is
replay/smoke evidence, not a long fuzz campaign. Initial corpus data is pinned
through the preceding dependency checkpoint's archive/hash; each random seed
and actual libFuzzer result remains recorded. The full log is retained compressed.

Three fake-tool orchestration tests pass within the real command. The coverage
case requires the new stages; the failure case independently rejects each of
29 external boundaries in a two-target fake manifest; the empty-manifest case
must fail before fuzz execution/success. A failed listing that emits partial
output is also exercised by the per-boundary failure case. Fake tools test shell
control flow, not actual protocol, audit or backend behavior.

The same test source against the old script failed two named controls: omitted
workspace doctests and false success with no fuzz source directory. Negative
baseline and successful preflight logs remain separately labelled; preflights
are not substituted for the committed-source real run. Graph updates are AST-only.
No full release, package, matched performance, coverage or crash qualification
claim follows. No GitHub CI run or PR was requested.
