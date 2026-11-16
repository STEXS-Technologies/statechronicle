# Standalone native tutorial qualification

Candidate `e794d37bbda8f5a7c0ea9c9fa90181297bc66965` adds the first standalone
native-domain application. It defines configuration and equipment types once,
registers command operations through the exhaustive mapping, signs canonical
commands, and prepares an authenticated two-domain batch. It imports the public
umbrella crate plus declared application/runtime dependencies, with no workspace
test harness, intent store or transaction manager. Restricted default-deny host
policy replaces delegated grants for the two creation routes; development keys,
principal, clock, identity generation and empty reads are explicitly labeled.

Both isolated lanes pass: [stable](minimal-native-domain-20261116T092434Z-781209/manifest.json)
and [Rust 1.85.0](minimal-native-domain-20261116T092434Z-781205/manifest.json).
The runner archives committed source, copies the application outside the repository,
changes only its umbrella path to a sibling archive, and runs locked/offline
metadata, all-target/all-feature tests, the exact CLI output contract, strict
Clippy, and application-package formatting. The committed tutorial lockfile stays
byte-identical and adds no registry packages absent from the source lockfile.
This demonstrates a build using only declared application dependencies, not a
published-package/registry qualification.

Four tests per compiler cover both prepared native states and transition
recomputation; domain/state substitution and altered commitments; unknown session,
actor, operation and resource policy; batch refusal; missing or modified signatures;
key revocation; and schema/command-operation substitution. An uncommitted event
and domain transition check are never presented as a canonical history proof.
The expected CLI states and explicit uncommitted message are retained as an
output fixture. Documentation explains the authoring/encoding/ingress owners,
optional delegated authority, scope of host trust and remaining durable steps.

The initial exact candidate `a29178d` passed tests, CLI and Clippy on both
compilers. Its runner used `cargo fmt --all`, which also visited library path
dependencies and their test modules. Those tests reference frozen CI fixtures
intentionally omitted from this isolated library archive. Both format failures
are retained. The correction scopes formatting to the application package;
application executable source is unchanged. `worktree-preflights` contains the
initial exploratory logs, which are not claimed as exact-source qualification.

`native-tutorial-summary.json` records checks, hashes and the source comparison.
All library Rust, library manifests/root lockfile, fuzz and workflow files are
byte-identical to preceding checkpoint `4985c55`; the prior 878-test/15-doctest
library qualification is not rerun or relabeled as a new root run. No source
shrink or performance equivalence is claimed. The application adds 424 physical
Rust lines including comments/module declarations and 262 lines of tests; these
are tutorial source, separately from the production-library metrics. Its manifest
and own lockfile are committed rather than relying on workspace test settings.

Reproduce from the exact candidate with Python 3.12+, installed Rust toolchains
and populated Cargo caches:

```sh
python3 scripts/check_tutorials.py --toolchain stable
python3 scripts/check_tutorials.py --toolchain 1.85.0
```

This advances F3/F7 usability. It does not complete the durable-custom-ledger
example, native durable replay/proof journey, current/historical serving,
migration/operator documentation, broader F4 composition or F8 release gates.
Fixed development credentials/IDs and empty projections are not suitable for
production ingestion, durable retries, physical crash recovery or signing policy.
No GitHub workflow dispatch, PR, tag or publication is part of this check.
