# Refactor ownership and duplication inventory

Baseline: `c418e98b91720188158c531cd5c2c4b8ab71cbcf`. This is the initial
source-grounded inventory; exact function-token matches are in `metrics.json`.

| Repeated flow | Authoritative owner | Refactor constraint / measurement |
| --- | --- | --- |
| TypedProfile check/apply command and current-state decoding | hosted native profile adapter | Preserve canonical decode, identity/schema/operation binding, validate-state and refusal order; instrument decode/validate/reduce counts before changing it |
| Executor single/player/batch preparation | executor gates plus profile policy | Preserve authentication/delegation ordering and domain-specific companion mutations; pure preparation must not claim intents |
| Service load commit / verify proof wrappers | proof envelope verifier + explicit trusted context | Preserve legacy error precedence; new expected-claim path requires independent scope and current/historical mode |
| Commit-only trade lookup maps | tenant-scoped commit read authority | Reproduced cross-tenant cache reuse; tuple scope is required, not a cosmetic helper |
| Legacy intent replay versus ledger replay | ledger transaction / receipt lookup | Not equivalent; do not consolidate legacy marker into canonical durability authority |
| Symbolic transaction manager versus ledger transaction | separate planning and durable authorities | Intentionally distinct; sharing a name or wrapper must not imply shared atomicity |
| Fungible/stack positive-availability helper | profile quantity invariant | Exact duplicate tokens; compare decimal/quantity/error contracts before sharing |
| Asset/entitlement/marketplace single-from helper | profile transition selection | Exact duplicate tokens; verify each authority contract and refusal precedence |
| Newtype Display/FromStr/serde implementations | domain identifier types | Intentional type separation; macros may reduce text but must not erase validation differences or wire names |
| Replay/index canonical-history checks | canonical history verifier | Compare missing/reordered/rewritten event and checkpoint semantics before extracting |

Baseline syntax counts include all production conditional alternatives but
exclude exact test-only items, comments and attribute syntax. The counts are
not a semantic duplicate score or compiled executable complexity. No production
code has been deleted and no reduction percentage is claimed yet. Each future
extraction must identify the deleted path, equivalence evidence and measured
source/performance delta. Protected hashing, signatures, canonical encoding and
SMT algorithms remain separate from orchestration cleanup.

The independent claim route shares tenant/resource comparison with the existing
`bundle::check_claim` through `check_claim_identity`, and all five compatibility
verification wrappers share `ProofService::load_commit`. Old gate order is
preserved. New expected-claim verification owns expectation/kind checks and
creates a private-field result only after signed-envelope verification and the
requested temporal check. The final head read is required for an observed-current
result; historical verification has an independent pin and no freshness claim.
