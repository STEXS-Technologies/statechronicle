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
| Fungible/stack positive-availability helper | profile quantity invariant | Shared private registry guard; frozen original 14,364-case public validation trace preserves exact amount fields and refusal precedence |
| Asset/entitlement/marketplace single-from helper | profile transition selection | Shared private registry adapter to require_from; same original trace covers lifecycle and malformed-state refusals |
| Newtype Display/FromStr/serde implementations | domain identifier types | Intentional type separation; macros may reduce text but must not erase validation differences or wire names |
| Replay/index canonical-history checks | canonical history verifier | Compare missing/reordered/rewritten event and checkpoint semantics before extracting |

Baseline syntax counts include all production conditional alternatives but
exclude exact test-only items, comments and attribute syntax. The counts are
not a semantic duplicate score or compiled executable complexity. At the initial inventory checkpoint no production
code had been deleted. Each extraction must identify the deleted path, equivalence evidence and measured
source/performance delta. Protected hashing, signatures, canonical encoding and
SMT algorithms remain separate from orchestration cleanup.

The independent claim route shares tenant/resource comparison with the existing
`bundle::check_claim` through `check_claim_identity`, and all five compatibility
verification wrappers share `ProofService::load_commit`. Old gate order is
preserved. New expected-claim verification owns expectation/kind checks and
creates a private-field result only after signed-envelope verification and the
requested temporal check. The final head read is required for an observed-current
result; historical verification has an independent pin and no freshness claim.

The event Merkle root now has one production owner in the existing accumulator
crate (`event_tree::event_merkle_root`). `commit::roots::event_root` preserves
its existing return/error contract as a thin delegate; the independently
requested historical settlement verifier uses that same root implementation.
The former commit-local leaf helper remains only under `cfg(test)` as an
independent oracle. No new published crate or dependency is needed.

The new settlement expectation structs make event/commit/subject/payment pins
explicit; the private verified result prevents unsigned trade summary fields
from being promoted to authenticated facts. This deliberately adds validation
and public input types, rather than claiming an orchestration shrink. It does
not merge current SMT claims, signed event history and backend atomicity into
one authority contract. See `settlement-history.md` for the limits.

After moving the event-root owner, `statechronicle-commit` uses SHA-256 directly
only in its independent test oracle. Its direct `sha2` dependency moves to dev
scope. The accumulator still requires SHA-256, so this removes a production
manifest edge without claiming a reduction in transitive compiled packages.

The profile guard extraction shares only identical validation rules. Registry
`require_positive_available` checks zero before shortage and retains both original
amount values. Stack debit/consume/reserve share their ordered existence,
state-quantity, input-quantity and availability gates; balance debit/reserve/spend
share the corresponding balance gates. Each operation retains its literal name
in errors. Asset, entitlement, listing and escrow status-only checks delegate to
one private `single_from` adapter around `require_from`.

Domain differences remain explicit: stack credit accepts zero while balance
credit requires positive input. Transfer recipient, burn authorization and
conversion unit checks remain in their original positions. No reduction,
execution policy, authorization, canonical encoding or portable implementation
is changed. The original-release oracle in
`evidence/maintenance/profile-guards/` compares exact structured errors and
Display text, including competing failures; it characterizes pure validation,
not complete durable or reduction semantics. This extraction adds no public
API, dependency or crate. Whole-roadmap footprint and qualification targets
remain open.

Exact `e8500d6` versus `89e2494` metrics remove 72 syntax lines, 517 tokens and
18 branch nodes across the six affected files: -5.49%, -5.56% and -7.17%
respectively. These are whole-file hotspot counts, not a claim that the broader
10–20% reduction target is met. Whole-workspace counts are 16,122 syntax lines,
118,199 tokens and 1,950 branch nodes, still above the original release. Raw
tracked source bytes decrease 4,126 (including comments/test source); three new
crate-private helper declarations increase the parser's public syntax count
without changing exported API. See
[evidence](evidence/maintenance/profile-guards/candidate-e8500d6/README.md)
for exact metrics, compatibility checks and inconclusive shared-host timings.
