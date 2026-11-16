# Protected settlement preparation checkpoint

Production source: `f6cd75d7688fef7815acb8acafd8edb66d37c9ab`.

The canonical preparation surface previously rejected complete declared value
legs through ordinary batches, while ordinary single and batch routes could
accept an incomplete declared bundle. The explicit protected settlement route
now authenticates every member through the shared preparation pipeline,
validates the complete group with the legacy settlement invariant owner, and
returns the existing `PreparedBatch` for `SignedBatch`. Ordinary resource routes
refuse bundle declarations. Native custom-domain policies retain their existing
routing ownership. No new wire structure or result type is introduced.

## Retained checks

- [Before-fix regressions](regression-before/manifest.json): both intended tests
  fail with Cargo exit 101 against `c2010ea`, using valid actor-scoped fixture key
  records. The earlier key-scope setup error is separately retained and excluded
  from product conclusions.
- [Positive and bypass controls](controls-f6cd75d/manifest.json): all five tests
  pass; independently removing settlement validation and bundle routing in
  isolated source copies makes the relevant assertion fail with exit 101.
- [Immutable-source qualification](qualification-f6cd75d/manifest.json): 889 finite
  tests, 16 doctests, formatting, strict workspace Clippy and strict rustdoc pass.
- [Original settlement trace](../../f2/executor-order/20261116T132742Z-1746381/manifest.json):
  all 10,128 rows match the actual original release.
- [Original executor trace](../../f2/executor-order/20261116T132831Z-1791947/manifest.json):
  all 23,544 rows match the actual original release.
- [Frozen external consumer](../../compat/20261116T132808Z-1771431/manifest.json):
  legacy and additive consumer, portable tests and bare-metal check pass.

The five integration tests cover a complete asset/payment group, missing and
mismatched payments, invalid bundle cardinality, ordinary-route refusals,
read-only `Preparer` composition and a valid two-asset bundle. Both preparation
surfaces leave symbolic transaction logs unchanged. The all-allow test authorizer
and fixed signing key are fixtures, not deployment policy.

## Cost and remaining qualification

[Parser metrics](metrics-f6cd75d.json) record 121,096 production lexical tokens,
1,951 branch nodes, 16,523 syntax lines and 1,457 public syntax items. Relative
to the preceding built-in cleanup this adds 400 tokens, three branch nodes,
60 syntax lines and five public syntax items (including private-scope syntax).
The public addition is two forwarding/owner methods; existing types are reused.
The explicit settlement path makes a bounded flat event clone to reuse the
existing slice validator. Ordinary batch preparation avoids that clone.

This checkpoint qualifies preparation and selected compatibility behavior. It
still needs contested canonical ownership/payment commits, authoritative loser
recovery, schema-migration authorization, and the remaining exact-candidate
release/performance gates. It does not qualify physical storage durability or
cross-tenant settlement. The intentional bundle-routing tightening is documented
in the migration guide; unchanged compatibility traces do not cover every
possible bundle declaration.
