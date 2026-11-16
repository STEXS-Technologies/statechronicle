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


The following operation-authoring checkpoint adds a public declarative macro
instead of a new runtime abstraction. Seven example operation names now appear
once rather than in both mapping and static table. It adds 190 production syntax
tokens; the subsequent MSRV correction adds 28. At `f8e953d` totals are 16,143
syntax lines, 118,417 tokens and 1,951 parsed branch nodes. Parser counts do not
expand macros. This is 299 tokens below the pre-guard-extraction checkpoint,
but still above the original release; the broader footprint target remains open.
No source-reduction credit is assigned to the macro or MSRV tooling corrections.


Projection rebuild addressing now has one private `projection_address` owner
shared by continuity validation and latest-row selection. It preserves the
existing tenant/resource order for resource-keyed state and adds the subject
for subject-held state, using the typed `ResourceState::subject` contract.
Mutable owner values never enter the key. Event uniqueness is tenant-scoped.
This is a correctness fix for two retained failing valid-history regressions,
not a claim of equivalent acceptance or a source shrink: independent holders
and coincident cross-tenant IDs were previously refused. Existing digest,
schema, version, continuity and same-tenant duplicate checks remain. The public
API, dependencies, canonical bytes and cryptography are unchanged. Full
verified-source/checkpoint composition remains a separate boundary.

The verified-history composition adds a private-construction owned source to
prevent unverified event slices and unbound numeric checkpoints from being
advertised as authenticated recovery. It reuses commit signature policy,
chain continuity, event-root and replay-root implementations. Existing
`validate_committed_events` becomes a documented public event-local integrity
helper so durable writes and verified histories share the same schema/type/
version/digest/custom-domain owner. This adds API/source to enforce a concrete
trust boundary; it is not counted as a deletion or footprint reduction. Legacy
rebuild validation remains intentionally narrower for compatibility. Full
history verification retains accumulated updates and recomputes roots; no
constant-memory, throughput improvement or final release claim is made.


Protected preparation now has a read-only `Preparer` composition with named
`PreparationPorts` and `PreparationConfig`. Compatibility `Executor` retains
its existing builder and write-capable ports for legacy operations, and stores
the same preparation configuration type. Both surfaces borrow the same private
evaluation core; ingress, profile selection, reduction, event formation and
batch-group validation have no second implementation. Legacy marker lookup and
claim receive an explicit private enum capability, absent in preparation.
The new public declarations prevent requiring unrelated marker/transaction
adapters in preparation consumers; they do not grant durable authority. The
module extraction is given no deletion credit. Generic host read/callback
purity remains a deployment contract; no sandbox is implied. Full durable/read
composition and fresh tutorials remain open.


Supplied-claim verification now has one minimum `CommitReader` capability and
`ClaimVerifier`, without requiring commit writes, proof generation, projections
or snapshots. Existing `CommitStore` adapters receive a direct-future forwarding
implementation. New names avoid method ambiguity with legacy traits. One
private generic verification owner serves both the read-only verifier and
compatibility `ProofService`; one canonical-head comparison and synchronous
lookup-result interpretation serve existing callers. Canonical verification
still requires the final linearizable full head tuple. This adds a real
capability boundary and consumer API, not source shrink. Boxed port futures
are forwarded unchanged by the legacy bridge; no performance equivalence or
allocation-free claim follows for the complete verifier. Full proof-serving
composition and durable external integration remain separate work.

Compatibility state-proof orchestration now makes ordinary derived-key `verify`
a thin delegate to existing `verify_with_key`. A 2,352-row external matrix
matches actual original-release errors and tenant/commit/head read order exactly.
The canonical derived-key wrapper deliberately retains its direct orchestration:
a prototype nested delegation repeatedly added stale-head refusal cost. Lookup,
head comparison and bundle cryptography still share their existing owners.
This is a measured retained duplication, not a second canonicality invariant.
The retained extraction removes four syntax lines, 28 tokens and one parsed
branch node, with no new public API/dependency. It does not establish the broader
hotspot target or a speedup. See [the source, parity and paired measurements](
evidence/maintenance/proof-service/README.md) for every sample and limitation.


The executor's shared core now carries scope/profile/version facts in one private
`PreparedProfile` instead of loose locals spanning every stage. It borrows the
immutable intent and owns the observed projection; no extra projection clone or
marker capability is introduced. Four focused methods own scope loading, authority,
reduction and event construction. The outer core keeps ingress and compatibility
marker policy, including final vector allocation after the claim. Its body moves
from 1,791 to 367 tokens, while total production grows 374 tokens and one parsed
branch. This buys an explicit checked boundary, not source reduction. Actual-original
23,544-row parity and exact-source qualification pass. [Measurements](
evidence/f2/prepared-profile-a52d53f/README.md) retain small positive median timing
changes amid shared-host noise, larger binary size and identical full-process
allocation totals for three bounded workloads. No full F2/F6/F8 pass is claimed.


Four compatibility batch/settlement completion handlers now share the private
`BatchCompletion` enum and lazy begin/success/refusal methods. A handle and sink
cannot coexist, removing unreachable sink-error rollback branches. Coordinators
retain results, policy validators, group flattening and marker semantics. Two
private helpers share tenant partitioning and settle selection. Actual-original
10,128-row errors/effects/warning parity and exact retained root/external checks
pass. Production loses 375 tokens and 27 parsed branches; affected files shrink
6.05% in tokens, with no full hotspot/flat-footprint gate claim. [Evidence](
evidence/f2/batch-completion/README.md) retains a rejected generic result wrapper,
two matched retained campaigns, equal allocation totals and an open refusing-sink
p95 concern. Canonical ledger authority remains separate.


All eight built-in profiles now inherit the existing `ProfileRules` execution-policy
default rather than duplicating its exact forwarding body. Reducer required-input
strings and typed amount/boolean reads use the existing registry helpers. Distinct
missing-state, float-error and string-field contracts remain explicit. No new
helper, public API, dependency or crate is introduced. The combined change removes
68 syntax lines, 555 tokens and 10 parsed branches; whole production is 120,696
tokens and 1,948 branches, still above the original release. [Original-release
296,604-case reduction parity, 23,544-row executor parity, exact-source checks and
all retained measurement attempts](evidence/maintenance/builtin-profiles/README.md)
include a 0.133% binary increase and short-regime portable/wrong-key timing
concerns that diminish in longer measurements. These are explicit limits, not a
full performance or footprint-gate pass.

Durable preflight's complete outbox validator was followed by the same per-record
payload/key/size validation in an immutable scope-binding loop. Source `52e8e6f`
deletes that second call, retaining the first pass and its refusal precedence.
The [original/control/candidate 5,688-case oracle and measured cost](evidence/maintenance/outbox-preflight/README.md)
retain adversarial callback traces, a 552-row digest-bypass control, exact-source
qualification, six matched preflight workloads and allocations. Large payload
medians improve 30–47%; three allocations per tested one-row preflight are
removed. This adds no abstraction or public concept and deletes six tokens/one
parsed branch. It is not a whole-workspace footprint or full performance pass.
Recovered untrusted records still require independent validation on lookup.

The same review found an order-label error in two earlier built-in paired timing
campaigns: rotation and reversal canceled lane alternation. Their raw results
are retained with an [observed-order audit](evidence/maintenance/builtin-profiles/order-audit.json);
separate explicit-order repeat campaigns do alternate. The full final matched
performance gate remains open and must verify actual command order.

The index now uses the existing profile owner for the presence-only value-leg
predicate (source `a315713`). Original/control/candidate contract checks cover
512 input combinations; a compiled AND-for-OR negative control fails. This
removes 37 production tokens without changing public concepts. Index pair
attribution and profile net-zero validation remain separate owners with different
contracts. [Evidence](evidence/maintenance/index-value-declaration/README.md).

## Current string-trait owner and retained exact matches

At `d6aa9f4`, nine domain string types share one private macro for six conversion,
formatting and serde shells. Type declarations, attributes, inherent methods and
validators retain 45 exact matching hashes. Status's borrowed helper is unchanged;
the template emits its identical constructor expression directly. Nonce's base64
codec and unchecked identity serde derives retain their distinct contracts.
[Evidence](evidence/maintenance/string-traits/README.md) includes 10,305 original/
control/candidate cases, a 5,608-difference compiled negative control, 897 finite
tests, 16 doctests and the frozen external consumer.

Production syntax drops 2,129 tokens and 267 lines; the affected domain-file set
falls 5,955→3,826 tokens including the template. Whole-workspace totals are
118,924 tokens and 16,251 syntax lines. Macro expansion is excluded: the nine
removed parsed try branches still exist in generated code. The additional
private module centralizes maintenance; it adds no public macro/crate/dependency.
The matched release consumer grows 720 binary bytes and 16 text bytes. This
bounded cost is accepted for one trait owner; no speed/allocation gain is claimed.

Four exact function groups remain, with explicit ownership:

| Group | Retained boundary and owner |
| --- | --- |
| `From<StateChronicleError>` in executor/intent error modules | Distinct public destination enums own their classification. `SizeLimitExceeded` retains its fields; canonicalization/digest/signature errors wrap through each enum's domain error. Sharing would require an extra cross-crate generic mapper/trait for two small translations. |
| `prepare_player` on Executor/Preparer | Separate capability/composition surfaces; both forward directly to the same private Evaluation implementation. |
| `prepare_player_batch` on Executor/Preparer | Same shared Evaluation batch owner; no second ingress or reduction implementation. |
| `prepare_player_settlement` on Executor/Preparer | Same shared Evaluation settlement owner; both retain the explicit coordinated route. |

These are bounded compatibility/capability shells, not duplicate implementations
of preparation. Exact token matching does not resolve `Self` to its destination
type and does not expand macros; semantic duplication and the broader source/cost
budget still require the full roadmap audit.
