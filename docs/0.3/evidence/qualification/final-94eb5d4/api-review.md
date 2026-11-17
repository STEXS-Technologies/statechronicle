# External consumer API review

Reviewer: Codex, implementing the external `statechronicle-final-consumer`
benchmark and exercising the separately built native-domain/file-ledger tutorial
applications. This is an agent review, not a human maintainer sign-off or a
promise about arbitrary downstream source compatibility.

The current thirteen compiler catalogs are byte-for-byte equivalent after JSON
normalization to the earlier reviewed `64c5350` catalogs. Against original 0.2,
there are 86 added, zero removed and 22 changed named export records. Export
records include aliases and nested declarations and are not independent concept
counts. `verify_api.py` regenerates each normalized catalog from retained raw
rustdoc data, checks actual Git archive/input hashes and compiler identity, and
rejects removed exports. `review_api.py` aligns nested methods/impls; the
structural differences remain documented lifetime elision, inherited default
policy/evaluation methods and dependency-induced blanket implementations.

The external benchmark imports protocol APIs only through the umbrella crate.
Its copied consumer-owned memory adapter implements public ports, with immutable
fixture hashes. Both earlier coherent 0.3 and current lanes compile the same
consumer. It constructs a protected prepared mutation, binds/signs it, commits,
recovers/replays exact identity, refuses a changed payload and tests denial and
unknown acknowledgement. It also implements the minimum `CommitReader`, supplies
independent owner/resource/key/pin expectations, and verifies original 0.2
current/historical present/absent proofs. Eight-thread exact retries preserve
one complete result. This is a bounded usability/contract check, not a physical
backend or performance result until measurement finishes.

| Consumer decision | Reviewed outcome |
|---|---|
| Is preparing/signing success? | Both remain uncommitted; fields are private and neither is a receipt |
| What proves accepted write/replay? | Checked original `CommittedMutation`/`CommittedBatch` from the ledger authority |
| What does lost ACK mean? | Explicit `Unknown`; recover the same identity and original payload |
| Is absence a receipt? | `NotFound`/`InProgress` are distinct recovery variants |
| Can a batch partially succeed? | Complete retained membership; unsupported batch ports refuse by default |
| Does a trusted signature imply current state? | No; current verification additionally checks authoritative full head tuple |
| Who chooses tenant/key/owner/pin? | Independent consumer expectations/policy; not supplied proof data |
| Can legacy sinks be treated as durable? | Documentation labels them advanced/planning compatibility; canonical examples use receipts |
| Who owns physical atomicity/authentication? | Consumer adapter and injected host policy; library does not infer them |

The native tutorial covers custom `StateProfile`, typed operation registration
and schema binding. The durable tutorial covers a public file-ledger adapter,
reopen, exact replay, unknown acknowledgement, original historical evidence,
current proofs and selected SIGKILL boundaries. Canonical single-tenant batches
and settlement are documented separately; cross-tenant coordination is advanced
explicit composition. Migration documents the approved Rust 1.100 and Ed25519
Dalek 3 dependency-type decision. No undocumented production permissive policy,
new protocol enum variant or replacement of the original frozen wire fixtures
was introduced by final qualification.

No P0/P1 API issue was found in this bounded consumer review. Automated compiler
and behavior evidence supplements this judgment; it does not establish safety
for malicious adapters, unavailable signer policy or all downstream macros.

## Consumer concepts and setup accounting

Counting convention: a concept is a separate decision the application must make,
not a Rust export or alias. The reviewed canonical journey has ten such groups:
(1) native state/command semantics, (2) static operation/schema/codec identity,
(3) profile addressing/economic rules, (4) signed ingress and principal,
(5) authorization/key/time policy, (6) read-side preparation inputs,
(7) exact signed mutation identity and signer policy, (8) atomic ledger adapter,
(9) receipt/unknown recovery, and (10) independent current/historical proof
expectations. These are distinct trust or semantic decisions; the claim is not
that all ten can safely become one universal interface. The public decision table
and standalone examples identify each decision without requiring private modules.

For the minimal standalone consumer, eight setup groups are observable in
`domains.rs`, `host.rs` and `main.rs`: define native models/rules; bind the static
operation/schema/codec mapping; construct typed adapters; register profiles;
provide state/tenant readers; configure key/principal/authorization/time;
construct `Preparer`; encode/sign/validate the request before preparation.
The durable tutorial adds four groups: implement the atomic ledger contract;
bind/sign a prepared mutation under signer policy; interpret receipt/lookup and
exact-identity recovery; supply independently trusted proof-reader expectations.
These counts describe this consumer's setup granularity, not an API-wide theorem
or a measured human onboarding time. A real database requires further backend
configuration and qualification.

The frozen manual Configuration/Equipment authoring path repeats seven operation
literals twice (discriminator plus registry); the current declaration uses seven
once. `encode_command` replaces three manually assembled envelope fields plus a
separate operation derivation with one checked public call. Native state/command
models, codecs and explicit schema decisions remain consumer-owned. The frozen
external overlay compares nine representative commands and the exact old envelope
bytes, so this reduction is supported by compilation/behavior evidence rather
than by export counts. No numerical reduction in all ten trust concepts is
claimed, and necessary ledger/trust decisions are retained explicitly.


`consumer-setup-metrics.json` measures that same pure native planning consumer:
main example syntax lines fall from 24 to 13 (physical 26 to 15), while its
shared model/codec/registry file falls from 241 to 232 syntax lines (physical
250 to 241). Combined handwritten syntax tokens fall from 1,958 to 1,778.
Command construction changes from three explicit steps (canonicalize, assemble
metadata envelope, derive operation) to one checked `encode_command` call.
The pinned parser and Git source hashes make these counts reproducible. This
is a bounded native-authoring improvement; adding real authentication, ledger
atomicity and independently expected proof decisions is not advertised as less
setup than the original planning-only example. Macro-expanded code and human
onboarding time are not part of these handwritten-source counts.
