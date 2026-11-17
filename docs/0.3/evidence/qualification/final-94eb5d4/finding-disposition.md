# Known finding and residual-scope disposition

This is the final interpretation of the chronological `docs/0.3/findings.md`
log, not an independent security certification. Earlier “open” statements remain
as dated evidence. The named tests below are indexed in `test-evidence-index.json`
and run in the full local driver; final aggregate acceptance additionally requires
all terminal verification records. No known P0/P1 issue is accepted as an open
canonical-path defect. A future vulnerability remains possible.

| Observed hazard / review concern | Current disposition and concrete evidence |
|---|---|
| Supplied absence key accepted without independent requested key | Canonical `ExpectedClaim` binds address/key and expected kind; original compatibility helper retains its documented narrower contract. `reader_only_consumer_verifies_present_and_absent_original_claims` and expected-claim adversarial tests reject substitution. |
| Genuine historical proof mistaken for current state | Current verification performs a final authoritative full-head comparison; historical verification requires an independent pin. `current_head_is_required_and_rechecked_while_historical_pin_stays_explicit`, head-bypass fuzz rejection and stale-head controls cover the distinction. The result describes the observed verification point, not future immutability. |
| Proof/commit/signature/key/tenant relabeling | Exact reference/stored commit binding and independent signer/key expectations fail closed; claim integration tests plus pin-bypass negative control check these boundaries. |
| Planning intent marker bypasses durable sink and returns empty events | Canonical mutation uses protected signed plans and authoritative ledger receipts, never the marker as proof. `active_reservations_and_absent_markers_never_become_receipts` and protected-plan tests reject this shortcut. Legacy execution retains its explicitly labeled planning behavior; migration is required for durable callers. |
| ACK loss or failed readback presented as safe failure/success | Explicit unknown and same-identity lookup/recovery; `lost_ack_is_unknown_until_authoritative_lookup_and_exact_retry`, failed-readback tests and ACK-bypass fuzz control. No new identity is authorized by uncertainty. |
| Corrupt/replaced stored effects or partial batch membership accepted as replay | Full original event/commit/outbox membership and scope are independently checked; `corrupted_or_partial_ledger_records_and_bare_replay_claims_fail_closed`, `partial_corrupt_or_unavailable_readback_never_issues_a_receipt`, group-substitution and advanced-preflight tests. |
| Tenant-substituted trade leg or commit-ID cache collision | Proof tenant must match its leg; service lookup/key cache uses tenant + commit ID. Same-ID/different-tenant positives and substitution/key-swap negatives run in the driver. The legacy global-map signature cannot represent coincident IDs and is documented accordingly. |
| Duplicate trade witnesses conceal missing coverage | Unique summary tenant/asset and proof leg/resource checks reject duplicate coverage; before-fix regression and final trade tests remain retained. The hosted allocation cost requires the corrected workload-matched controls and explicit final acceptance in `performance-review.md`. |
| Unsigned trade summary/operation metadata advertised as settlement facts | Legacy state proof is explicitly narrower. The separate expected settlement-history API verifies signed event membership and caller expectations; `requested_settlement_facts_require_authenticated_event_history` rejects summary-only authority. No cross-tenant atomicity is inferred from independent signed legs. |
| Shared evaluator removes required validation or changes refusal order | Typed evaluation instrumentation, strict original/current characterization (348,621 cases), compile contracts and decoder/validation controls preserve original supported contracts; semantically different authority/economic checks remain separate owners. |
| Checkpoint/rebuild selects unverified or reinterpreted history | Verified-history construction checks genesis/chain/event/root/signature/trust/pin; resumable source uses immutable verified identity. Original-produced migration/reopen tests include 44 interruption schedules and separately authorized schema migration. New signed-history performance uses the same checked API, not a validation bypass. |
| Projection rebuild merges holders or rejects equal event-ID text across tenants | Full projection address includes subject-held identity; event deduplication is tenant-scoped. `rebuild_keeps_independent_holders_and_their_own_continuity`, `rebuild_event_identity_is_scoped_by_tenant` and holder/duplicate refusal tests run in the final driver. Source/root encodings remain unchanged; projection adapters must retain holder identity. The legacy reconstruction allocation increment is explicitly accepted for this corrected identity contract. |
| Backend appears correct because the harness has no useful oracle | Independent authority inspection and deliberately broken partial/replay/ACK/deadlock implementations are detected. The ineffective first deadlock mutation remains a failed attempt; its corrected fresh runtime control detects the timeout. OS-lock removal also produces the intended runtime rejection. |

## Remaining limits and ownership

These are explicit contract/deployment limits, not silent P2 defects deferred by
this branch: physical power-loss behavior and real database linearizability,
trustworthy key/revocation/time policy, production authorization, external service
availability, outbox delivery/reconciliation, and independent expected claim/pin
selection remain consumer responsibilities. The adapter contract and operator
recovery guide describe the required backend qualification before deployment.
The file tutorial proves selected process-kill/reopen boundaries, not every
filesystem, power-loss mode or interleaving. Future database adapters require their
own conformance and crash qualification; no delivery date or generic backend
certification is implied.

Source growth, four deliberate duplicate shells, measured build RSS/binary growth,
and performance/allocation deltas are individually reviewed in maintenance/API/
performance records. They are accepted design costs with bounded evidence, not
unresolved correctness findings. No additional known P2 implementation defect is
being moved to an unnamed future schedule in this qualification. New findings
must be triaged on their own evidence; “gap-free” is not a release claim.
