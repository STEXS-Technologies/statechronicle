# Read-only supplied-claim verifier qualification

Exact candidate `888d0a67b6951527c1b0b9f457ed0b8ca93687ee` passes stable and
Rust 1.85.0 qualification: 884 finite workspace tests and 16 doctests per
compiler, strict workspace/all-target/all-feature Clippy including fuzz
compilation, strict rustdoc, formatting and the native-domain example.
Committed production and candidate overlays remained unchanged during these runs.

`proof::claim::ClaimVerifier` requires only the new object-safe `CommitReader`:
immutable tenant-scoped commit lookup and, for current claims, a linearizable
full canonical-head tuple. A history-only implementation can omit the head
method; its default `None` refuses current claims. No write method, sequence
lookup, proof index, projection index or snapshot adapter is required. The
compile-fail doctest prevents treating the verifier as a commit writer.

Existing `CommitStore` adapters automatically implement the read capability,
forwarding their existing boxed futures directly. Method names differ from the
legacy trait to avoid ambiguity when both traits are imported. Protected
`ProofService` methods retain their public signatures and delegate to the same
private expected-claim verification owner. Commit-result normalization and
canonical-head comparison each have one owner. Compatibility lookup and head
helpers return the shared future directly. No cryptographic, wire, dependency,
feature or portable contract change is intended or observed by the retained gates.

Five reader-only public-consumer tests verify frozen original-version ownership
and absence proofs, supplied-independent expected claims and signer keys,
current/historical interpretation, final head-read order, stale/missing/failing
heads, wrong pin, resource/key substitution, signer refusal and unavailable
commit reads. A history-only adapter implements no head method. The separate
legacy bridge test compares verified results/error text and head-read counts
across four expectations and four head modes, including a head advance during
lookup. Historical verification performs no head read. Existing adversarial
proof suites remain intact. These comparisons share the verification core;
they are not claimed as an independent full original-version execution model.

The [external compatibility/wire and portable/bare-metal MSRV run](../../compat/20261116T093457Z-1188197/manifest.json)
passes, including the new consumer that implements only `CommitReader` against
the public umbrella crate. Frozen original fixtures/proof bytes remain unchanged.
The native standalone application also passes its isolated locked/offline
metadata, four tests, CLI output, strict Clippy and application formatting on
[stable](../../tutorials/minimal-native-domain-20261116T093457Z-1188192/manifest.json)
and [MSRV](../../tutorials/minimal-native-domain-20261116T093457Z-1188193/manifest.json)
against this exact library candidate.

Production syntax grows by 106 lines, 715 tokens and 10 public syntax items,
with no added branch nodes; tracked source bytes grow by 5,523. Totals are
16,540 syntax lines, 121,280 tokens, 1,985 branch nodes and 1,445 public syntax
items. The prior production checkpoint is `886bc56` (unchanged library source
through `3b5b007`). This is the cost of an explicit read capability and a consumer
surface that removes unrelated adapters, not a source-shrink or performance
claim. Parser/public-count limitations are retained in both metrics files.

Two initial focused Clippy failures from shadowed test bindings, followed by
corrected preflight logs, are retained. Production lints were not suppressed,
and existing fixture policy was not changed. Actual commands, compiler versions,
source/lock/runner/artifact hashes and all full compressed logs are in the
manifest. To reproduce, check out the exact candidate and invoke `qualify.py`
from the repository root with installed stable/MSRV toolchains and cached Cargo
dependencies; run `scripts/refactor_compat.py --lane candidate --portable` under
MSRV for the external matrix and `scripts/check_tutorials.py` for each compiler.

Read capabilities do not sandbox arbitrary host code; read implementations must
not persist or dispatch. Caller-selected verifying keys still require independent
tenant/time/revocation policy. A current result is pinned to the final head read
and does not remain fresh indefinitely. Direct boxed-future forwarding is a
source fact, not proof of whole-path allocation or latency equivalence; no new
performance campaign was run. Durable custom-ledger integration, coherent proof
serving, full native durable/replay/proof tutorial, broader footprint reduction,
fuzz/coverage, physical crash, packaging/security and final release gates remain
open. This is a concrete F4 prerequisite, not completed end-to-end qualification.
