# StateChronicle 0.3 signature compatibility baseline

`baseline.json` identifies the annotated `v0.3.0` tag, source tree, lockfile,
toolchain, published registry archive checksums, and workspace dependency
edges. The source archive digest is for `git archive --format=tar v0.3.0`.
The dependency graph includes the unpublished fuzz package.

The paired BCS and JSON files in each crate's `tests/fixtures/wire-v0.3.0/`
directory were captured from the pinned release in a detached checkout. The
capture added serialization to the original tests, using the release's
fixtures, signing keys, dependency lockfile, and library code. The branch's
tests compare the full serialized bytes against those files.

| Fixture | Released scenario |
| --- | --- |
| Signed intent | `statechronicle-intent/tests/pipeline.rs`, empty-input signed intent round trip |
| Signed tenant commit and state membership proof | `statechronicle/tests/e2e.rs`, mint/transfer/lock and inclusion |
| State non-membership proof | `statechronicle/tests/e2e.rs`, absence against the same tenant commit |
| Signed global checkpoint | `statechronicle-commit/tests/ordering_checkpoint_fork.rs`, two tenant roots |
| Cross-tenant trade proof | `statechronicle/tests/trade_history_proof.rs`, settled trade with a remote value leg |
| Cross-tenant settlement witnesses | `statechronicle/tests/trade_history_proof.rs`, authenticated asset and payment history |

The settlement snapshot serializes the two ordered `(commit, events)` pairs
passed to the borrowed witness API. The v0.3 API has no standalone wire
envelope for those witnesses.

Run the compatibility checks with the release toolchain:

```sh
cargo test --locked \
  -p statechronicle-intent --test pipeline \
  -p statechronicle-commit --test ordering_checkpoint_fork \
  -p statechronicle --test e2e --test trade_history_proof
```

The release tag also pins the original consumer-style test sources for strict
Ed25519 signing and verification, current and historical claims, protected
single and batch mutation, exact recovery, and portable verification. These
scenarios cover signature preimages, commit identity, root continuity, outbox
effects, and lost-acknowledgement recovery. The original known-answer signature
is in `crates/statechronicle-core/src/signature.rs`.

To inventory the tagged signature boundary across the whole source tree:

```sh
git grep -nE 'Signature|Signed<|VerifyingKey|CommitSigner|SignedCommitVerifier|sign_commit|verify_|SignedMutation|KeyRegistry|[Cc]heckpoint|CommitRef' \
  v0.3.0 -- 'crates/**/*.rs' 'fuzz/**/*.rs'
```

Performance and allocation qualification and independent consumer compilation
are open baseline work.
