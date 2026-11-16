# Legacy executor error and effect characterization

The external consumer `ci/characterization/executor.rs` is compiled independently
against actual original release `c418e98b91720188158c531cd5c2c4b8ab71cbcf`
and candidate `f5b922fffcf9248b4ce0002c8103089b37b6139e`.
No production source, original golden or frozen consumer is changed.
The two native domain implementations come from the original Git object and
must match the frozen consumer provenance hash.

## Exact committed result

[Final manifest](20261116T111808Z-1103420/manifest.json): both lanes pass offline
resolution, locked release compilation, strict Clippy and all 23,544 trace rows.
The complete serialized trace SHA-256 is
`b7853af8b100d4761755f2e7277a9181ebeb4acef184464b65afc9b53f7d89d1`
in both lanes. Each manifest records source and archive hashes, resolved lock,
compiler, commands, terminal exits, binary hashes and compressed full output.
No new registry package or post-run source/lock mutation is allowed.

The matrix is six templates × nine entry points × 436 fault masks. Masks cover
zero faults, all 29 single faults and every pair. All 54 no-fault controls must
emit events. Templates cover asset mint/transfer, balance and stack debit/credit,
and both frozen native domains. The lanes cover ordinary, authenticated, player,
registry, compatibility durable sink, and three legacy batch entry points.
Batch controls prepend an independent valid mint to expose earlier-item effects.

Faults cover absent signatures, principal/policy/registry refusal, expiry,
marker read/replay/conflict, crypto refusal, client and aggregate freshness,
tenant scope/existence/read, absent type, state read/type/version, absent authority,
three evaluator refusals, malformed inputs, companion reads, marker writes,
sinks, version overflow and symbolic transaction rollback/commit failure.
Each row freezes complete Debug/Display errors or canonical event-vector digest,
ordered port/clock/ID callbacks, canonical claimed intents and consumed IDs.
Rollback failures are injected alongside primary failures: the preserved primary
error is intentional; a rollback fault alone does not cause rollback.

## Negative controls and development attempts

[Clock-preserving negative control](20261116T111827Z-1114958/manifest.json)
removes only expiry refusal in the temporary candidate archive, preserving its
clock callback. Both lanes compile and finish; strict comparison fails with
**1,410 changed rows**, all expired cases and all with changed outcomes. Full
expected/actual differences and the exact mutation hashes are retained.
The worktree and original archive remain unchanged.

The earlier [whole-check removal](20261116T111729Z-1073993/manifest.json)
also removed a clock callback and produced 20,214 changed rows; it is a weaker
control than the clock-preserving run. The [first successful matrix](
20261116T111658Z-1052903/manifest.json) has the same exact trace digest.
An initial debug trace [was explicitly terminated](
20261116T111500Z-970763/manifest.json) after successful compilation and replaced
by bounded release-mode runs; its exit -15 is retained as an interrupted attempt.
An initial standalone lint check identified three fixture-only style issues,
corrected before the committed strict checks. An initial setup attempt used the
frozen-copy path in the original commit; the runner now reads its actual original
path and validates the frozen-copy provenance.

The runner's existing proof-service mode also passes its 2,352-row actual-original
comparison with strict Clippy in [the regression run](
../../maintenance/proof-service/20261116T111751Z-1088749/manifest.json).

## Scope and reproduction

```sh
python3 scripts/check_proof_service_parity.py --fixture executor \
  --revision c418e98 --revision f5b922f
python3 scripts/check_proof_service_parity.py --fixture executor \
  --revision c418e98 --revision f5b922f --negative-control-remove-expiry
```

The second command must fail comparison, after successful execution. Native
schemas, host signatures, IDs and clock are fixed; faults are test sentinels.
Public `ValidatedIntent` compatibility fields are deliberately mutated after base
construction to exercise malformed execution-core gates. This is not evidence
that structural validation proves cryptographic authentication.

The host is a bounded sentinel adapter. Compatibility sinks do not establish
real commit atomicity, receipts, rollback durability or recovery. Canonical
protected preparation/settlement APIs, cross-tenant/trade composition, arbitrary
fault combinations, randomized inputs and concurrent or physical failures need
separate qualification. Full F2/F8 and release readiness remain open. This change
adds development characterization only, with no production behavior, dependency
or API change; it makes no performance claim.
