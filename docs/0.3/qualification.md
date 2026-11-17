# Development API qualification

The coherence refactor completed library qualification before delivery commit
`2827f0f`. The tested library source is `e880a2a`; the qualification record
establishes equivalent library inputs across the final tooling fixes. This is
qualification of that source snapshot, not automatic certification of later
changes. Workspace versions remain `0.2.0`; versioning, tagging and publication
are separate release actions.

## Completed checks

All seventeen final verification gates passed. The audit accounted for 307
roadmap clauses: 306 verified, with version/tag/publication recorded separately.

| Area | Recorded result |
| --- | --- |
| Tests and static checks | 955 indexed test executions in the local driver and standalone tutorials; strict lint, formatting, rustdoc, dependency policy and portable compilation passed |
| Original-release compatibility | Frozen hosted, portable and bare-metal consumers passed; original wire and signing fixtures were preserved |
| Packages | All thirteen unpacked crate archives passed isolated tests, doctests and lint |
| Hosted line coverage | Full 18,913/20,365 (92.8701%); filtered 15,493/16,410 (94.4119%), above the unchanged 92.4% filtered ratchet |
| Semantic parity | 348,621 distinct original/current characterization cases matched, including errors and callback order |
| Property and fuzz campaigns | Three property seeds; all 27 ASAN fuzz targets ran for at least 3,600 reported seconds each, with no crash artifacts |
| Adapter and recovery behavior | Concurrent history, corrupt receipts, unsupported capabilities and selected file-journal process-termination cases exercised; deliberately broken controls were detected |
| Performance and allocation | Matched baseline/control/candidate comparisons, cold builds and signed-history verification/rebuild measurements reviewed with explicit cost acceptance |

## Accepted costs and limits

- Stale-head refusal measured 251 to 271 ns (+8% paired median). Head checks were
  retained. Trade-proof, durable-preflight and projection-rebuild allocations
  increased to preserve their required validation and identity contracts.
- Complete verification of a 256-commit signed history measured about 2.491 s
  and 42,320 allocations totalling 26.99 MB. Verified checkpoint resume measured
  about 10.584 ms. These are host-local observations, not service capacity or
  linear-scaling guarantees; cumulative allocation is not peak memory.
- Finite fuzzing, line coverage and selected crash tests do not prove universal
  safety, branch coverage, power-loss durability or production backend behavior.
  Applications own authentication, trust policies, keys and storage qualification.
- Compiler catalogs reported 86 added, zero removed and 22 changed named export
  records. Selected external consumers passed; this does not establish every
  possible downstream source or dependency combination.

## Historical record and reproduction

The [complete qualification record](https://github.com/STEXS-Technologies/statechronicle/blob/2827f0fe9c3f3945aae2f1a46ffb79670bbd06f7/docs/0.3/evidence/qualification/final-94eb5d4/README.md)
and [raw evidence archive](https://github.com/STEXS-Technologies/statechronicle/tree/2827f0fe9c3f3945aae2f1a46ffb79670bbd06f7/docs/0.3/evidence)
remain available at an immutable Git revision. The archive includes failed and
superseded attempts; those attempts are not passing evidence. Some historical
verifiers require generated workspace artifacts and are not standalone offline
checks for a fresh clone.

Use the maintained [release checks](release-checks.md), [package checks](package-checks.md)
and [coverage instructions](coverage.md) for new changes. Their generated output
under `docs/0.3/evidence/` is ignored by Git. Retain each new run's logs and
manifests as build/release artifacts with its exact revision and toolchain.
The frozen `evidence/maintenance/profile-guards/original-manifest.json` remains
tracked because the package checker verifies its byte-identical packaged copy.
