# Compiler API comparison checkpoint

Immutable original `c418e98b91720188158c531cd5c2c4b8ab71cbcf` and candidate
`64c5350b8ba0a674c81e00a43160deed036557f1` were independently archived and
compiled with the same Rust 1.102.0-nightly (2026-11-13 build), rustdoc JSON
format 61, all features, host target, locked dependencies and offline resolution.
Both manifests passed all thirteen library packages and the before/after source
and lockfile hash guards. No production source changes belong to this checkpoint.
Nightly supplies metadata; it does not replace stable/MSRV qualification.
Nightly redundant-link warnings remain in the retained logs.

`compiler-*/manifest.json` records exact commands, compiler identity, revision,
archive and source hashes, exits and uncompressed artifact hashes. Each package
retains compressed raw rustdoc JSON, normalized API JSON and build output.
The collector and normalizer used are copied into each lane. Reproduce with:

```sh
python3 scripts/collect_rustdoc_api.py <immutable-commit> <new-output-directory>
python3 -m unittest discover -s scripts -p test_rustdoc_api_inventory.py -v
python3 docs/0.3/evidence/api/verify.py
python3 docs/0.3/evidence/api/review.py
```

`comparison.json` records **86 added, zero removed and 22 changed export
records**. These are named module-tree records, not a count of public concepts:
methods/fields/implementations are nested, and reexports can name the same
underlying definition more than once. External reexports retain compiler target
paths; their definitions are available in the corresponding crate catalog.
Private-module local reexports retain their target declarations. Glob imports
are represented as import records rather than flattened names.

| Crate | Added records | Changed records | Review of existing declarations |
| --- | ---: | ---: | --- |
| core | 0 | 0 | Identical inventory |
| domain | 0 | 0 | Identical, including compiler-expanded string trait impls |
| intent | 0 | 0 | Identical inventory |
| executor | 9 | 1 | Executor gains three preparation forwarding methods |
| commit | 20 | 1 | `projections_for` replaces named input lifetime with elision |
| accumulator | 2 | 0 | Shared event-tree owner exported |
| proof | 15 | 1 | ProofService gains current/historical verification methods |
| profiles | 2 | 10 | Default `evaluate`, native override/authoring helper; eight builtin rules inherit `execution_policy` instead of repeating its implementation |
| profile | 0 | 0 | Identical inventory |
| portable | 0 | 0 | Identical inventory |
| ports | 9 | 1 | LedgerTransaction gains default `retain_batch` |
| index | 5 | 8 | Existing type records gain tracing Instrument/WithSubscriber blanket impls from dependency graph changes |
| umbrella | 24 | 0 | Mutation entry point, aliases and command macro |

`structural-diff.json` and retained `review.py` align methods and impl blocks
for review; they never rewrite the raw or normalized snapshots. Splitting or
joining inherent impl blocks can otherwise make a textual diff noisy. All
non-additive structural differences are the lifetime spelling above, removal of
redundant builtin execution-policy overrides, and the provided-method lists that
also reflect the new evaluation default. The method remains on ProfileRules.
The full additions are retained for public-concept-budget review; “additive”
does not establish usability or eliminate potential downstream name collisions.

The five compiler-backed tests include successful compilation after removing a
public export, six independent contract mutations (field type, reexported field,
enum order, bound, trait default, repr), hidden public items, semantic attributes,
macro-generated impl visibility, compiler ID renumbering/order invariance, and
rejection of unsupported/private metadata. The retained test log passes. The
verifier independently checks all 26 raw/normalized pairs, recomputes catalogs,
checks actual Git archives/source inputs and reconstructs the export comparison.

This is review evidence, **not a SemVer checker or completed release audit**.
It covers one compiler, target and feature configuration; private implementation
behavior and arbitrary downstream macro invocations are outside the catalog.
Docs/spans and lint/codegen attributes are intentionally omitted. Canonical
paths do not establish dependency type identity across package versions.
Frozen external implementers, exhaustive enum matches, vectors, behavioral
parity, stable/MSRV builds and the final requirement audit remain separate gates.
