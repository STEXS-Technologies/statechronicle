# Independently requested historical settlement facts

`statechronicle::proof::settlement::verify_settlement_history` authenticates
requested asset transfers and optional fungible payment pairs using full signed
commit event batches. It does not use `TradeProof.summary`, `TradeHistory`'s
unsigned ordering/status, or `ResourceStateProof.latest_event` as authority.
Those existing wire types and legacy verifier semantics remain unchanged.

The caller supplies `ExpectedSettlement` independently of the witness provider:

- A trade ID and a nonempty set of asset settlements, each naming tenant,
  resource, event ID, intent ID, full canonical commit-body digest, previous
  owner and recipient.
- For every payment to assert, both debit and credit event pins, payer,
  recipient, exact `Amount` and denomination unit. The two events must share
  tenant, resource, intent and commit and constitute that intent's exact
  two-event group. Their amounts must be equal and opposite, positive, and
  match the independently expected amount. A newly created recipient must
  have version zero and the execution policy's exact canonical unborn state;
  existing recipients retain their expected subject and unit.

Obtain these pins and business expectations from the application's trusted
request/receipt or an independent historical anchor. Computing an expectation
from the untrusted summary merely repeats its assertion. A textual commit ID
alone is insufficient; the digest pins the complete canonical signed body.
Tenant scope remains explicit even when commit/event identifiers coincide.

Supply one `SettlementWitness` per referenced commit. Each contains a borrowed
signed commit and its **complete ordered event batch**, including any events
outside the requested facts. The verifier checks the tenant schema, independent
historical key policy, strict Ed25519 signature, exact count, state hashes,
checked version increments, unique event identities and the event Merkle root.
Asset events must be `trade.settle`, leave the named `trade_held` trade and
previous owner, and produce active state owned by the expected recipient with
no remaining trade hold. Missing, duplicate, substituted or unrequested commit
witnesses fail closed. Witness order is immaterial; event order is authenticated.

The key resolver receives tenant and signed commit so the application can
independently check key ID, executor, historical authorization/revocation policy
and time. Returning a key is a trust decision; this verifier always additionally
checks the cryptographic signature. No trust is inferred from digest equality.

Only successful verification constructs `VerifiedSettlement`. Its immutable
`claim()` returns exactly the independently expected and authenticated facts;
there is no deserialization or public receipt constructor. Public expectation
structs are inputs, not verified facts. The current root integration test and
candidate-only external `trade_coverage` consumer exercise this route through
the umbrella facade with the real executor, signatures and roots, including
payments to unborn recipients and assets settled in distinct commits.

## Exact limits and interpretation

At most 256 requested events, 256 witnesses and 256 events per witness are
accepted. Canonical event bytes are bounded at 64 KiB each and 4 MiB aggregate
across all witnesses; each commit body is bounded at 1 MiB. Full event batches
are used because v0 has no event-path transport DTO. No new wire format,
cryptographic algorithm, dependency, default feature or portable allocation
requirement is introduced. Commit formation and this verifier use the same
`accumulator::event_tree::event_merkle_root`; the existing commit API delegates
and preserves empty-input/error semantics and event bytes.

This verifies **historical signed event facts under independently pinned
commits**. It does not prove canonical chain membership, latest ownership,
complete trade lifecycle, that no other assets/payments exist, physical truth,
or cross-tenant atomicity. An empty payment list makes no payment assertion.
Use the independent current-state claim verifier separately for current
ownership and the backend's transaction contract for actual atomic persistence.

Payment events do not contain a trade ID or the full value declaration from a
settle intent. The caller explicitly requests those exact payments alongside
the asset facts. Matching signed payment deltas does not authenticate an
unsigned declaration or prove an intrinsic cross-tenant trade linkage. Do not
present this receipt as a global atomic settlement certificate.

This API verifies caller-supplied witnesses. Applications must fetch those
witnesses from trusted sources and choose the historical signer policy and
commit pins independently. Use [verified history](verified-rebuild.md) for
chain authentication and [proof claims](proof-claims.md) for current-state
checks. See the [qualification summary](qualification.md) for tested scope and
costs; historical settlement facts alone establish neither current ownership
nor cross-tenant atomicity.
