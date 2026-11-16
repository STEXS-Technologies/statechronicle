# Current hosted line coverage

Exact source: `8faaa14f82aed6f305c0b3352a0f09b4f1f91e85`.
The [runner](runner.py) extracts an immutable archive outside the checkout,
collects coverage with the committed driver, retains both raw reports, runs the
real ratchet and rejects any source or lockfile drift. Rust is 1.100.0;
cargo-llvm-cov is 0.8.7. Commands, compiler, hashes, durations and actual exit
codes are retained in [the manifest](manifest.json). Collection and ratchet
both exited 0. The four coverage-parser tests and 897 finite workspace tests
ran successfully. Fuzz processes are excluded; this is not a doctest collection.

| Scope | Covered lines | Measured lines | Coverage |
| --- | ---: | ---: | ---: |
| Full LLVM report | 18,913 | 20,365 | 92.8701203% |
| Historical filtered ratchet | 15,493 | 16,410 | 94.4119439% |

The required filtered minimum remains 92.4%. Its tolerance, thirteen-file
exclusion expression, parser tests and collector/ratchet scripts are byte-identical
to the previous coverage checkpoint `4b133d9`. The full and filtered reports are
both retained; the filtered number must not be presented as whole-workspace
coverage. The independent [negative ratchet](negative-manifest.json) supplies a
95% minimum in a separate test-only baseline; the unchanged checker exits 1 with
its coverage-regression diagnostic. The committed baseline is never modified.

LLVM attributes the new shared string-trait template to validated_string.rs,
which reports 21/21 measured lines. It is not added to the exclusion expression.
A shared macro's line attribution is not per-instantiation branch coverage;
these reports do not establish independent coverage of all generated branches.
The separate 10,305-case external characterization tests all nine string types.
Changes in source layout, macro attribution and tests mean prior percentages
are not a matched before/after coverage improvement measurement.

This covers the root hosted workspace under its existing instrumented line
scope. The isolated dev conformance package and standalone tutorials have
separate test campaigns; their line coverage is not silently included here.
It does not establish branch/MC/DC coverage, no-alloc execution, extended fuzz,
physical crash safety or complete final-candidate release qualification. Later
library changes require appropriate new collection before a final release claim.
