# Native domains with authenticated preparation

This standalone application defines configuration and equipment models, signs one
command for each, and prepares both as one authenticated batch through the public
`statechronicle` dependency. It returns native typed states and uncommitted plans.
It has no intent store, transaction manager, ledger writer or dispatch adapter.

The example uses a fixed development key, clock, principal/session, and empty
state reader. These are visible fixtures, not production authentication, durable
storage or key management. The host must authenticate a session independently;
copying a `credential_id` string is not authentication. Replace these fixtures
before accepting external requests. Nothing here proves physical equipment state.

## Run it

From the repository root, using Rust 1.85 or newer:

```sh
cargo run --manifest-path tutorials/minimal-native-domain/Cargo.toml --locked
cargo test --manifest-path tutorials/minimal-native-domain/Cargo.toml --locked
```

This crate has its own `[workspace]` and committed lockfile. It depends directly
on the umbrella `statechronicle` crate plus Serde, Tokio, Chrono, async-trait and
Ed25519 for application models, runtime, time and host signing. It imports no
workspace test helpers or underlying StateChronicle implementation crates.
The path dependency intentionally targets this refactoring checkout, which still
uses the existing workspace version. A released registry dependency must wait
for the separately qualified and authorized release.

Expected output:

```text
Prepared configuration: ConfigurationState { revision: 1, enabled: true }
Prepared equipment: EquipmentState { mode: Idle, cycles: 0 }
Uncommitted: no intent claim, ledger write, outbox dispatch or durable receipt.
```

`expected-output.txt` is the executable output contract used by the isolated
qualification runner. For an offline build with populated Cargo caches, append
`--offline` to the Cargo commands. If your environment installs a broken compiler
wrapper, clear it with `RUSTC_WRAPPER=`; the application itself needs no wrapper.

## Follow the ownership

1. In `src/domains.rs`, ordinary Rust structs/enums implement `StateProfile`.
   Validation, deterministic reduction and resulting-state invariants belong to
   the application. Each domain fixes its identity, schema version and byte bound.
   `command_operations!` derives the command discriminator and operation registry
   from one exhaustive mapping. There is no separate JSON schema or hand-written
   portable codec in this hosted tutorial.
2. `TypedProfile` encodes commands and states as canonical BCS payloads and binds
   each operation to its native command. The static registry selects the domain
   and schema. Keeping a schema number while changing its meaning is unsafe;
   register a new version and an explicit migration instead.
3. In `src/host.rs`, `Application::request` takes the encoded operation and inputs,
   binds tenant, actor, resource, expected version and original intent identity,
   then signs the canonical intent body. It never obtains operation authority
   from an untrusted command string.
4. The composition names `PreparationPorts` and `PreparationConfig` once. The
   metadata registry scopes the operator key to the two creation operations;
   the verifier independently checks the actual signature under the trusted key.
   `OperatorPolicy` denies every session/operation/resource combination except
   the explicitly declared operator creation routes. Unknown credentials,
   different actors and later lifecycle operations refuse.
5. `main` passes the independently selected development principal, policy and
   key registry to `Preparer::prepare_player_batch`. The result owns both accepted
   requests and their complete events. Failure returns no successful subset.
   Decode the resulting states through the same typed domain adapters.

This application deliberately uses `with_optional_authority()` for delegated
TrustGrant bindings: its restricted host principal/key/operation policy owns
permission for the two creation routes. It still requires detached signatures,
key metadata and explicit principal authorization through protected preparation.
It does not install an allow-all authorizer or disable protected ingress. If your
policy requires delegated grants, retain the default mandatory authority behavior
and supply the corresponding independently trusted evaluators and bindings.

The tests cover both domains, deterministic transition recomputation, native
state/domain substitution, schema and command-operation substitution, missing
signatures, signed-body modification, key revocation, default-deny host policy,
and refusal of a batch containing a forbidden leg. Domain `verify_transition`
checks reduction and commitments; it is not signer/history trust or a current proof.

## Continue to durable state

Preparation reads can become stale. Event counters may advance on a failed attempt.
This development counter resets on restart, and the request helper deliberately
uses fixed original IDs, so it is unsuitable for durable event/intent identity
generation. Repeated preparation is not an idempotent commit retry or receipt.

To commit, retain the original intent identity/digest, construct a signed plan and
use the authoritative ledger boundary to atomically bind claims, events, signed
commit, canonical head, projections and outbox. Recheck authorization and
concurrency there. Resolve an unknown acknowledgement by authoritative lookup
before retrying. See [signed mutation](../../docs/0.3/signed-mutation.md).
A current proof additionally needs an independent expected claim, trusted signer
policy and authoritative canonical head; a historical proof needs an independently
selected pin. An uncommitted event alone provides neither guarantee.

The separate durable-custom-ledger tutorial, migration/operator walkthroughs and
full current/historical journey remain roadmap work. This native tutorial does
not claim to complete those gates.

## Reproduce the isolated checks

With Python 3.12 or newer, both Rust toolchains installed and Cargo dependencies
cached, after committing the source, run from the repository root:

```sh
python3 scripts/check_tutorials.py --toolchain stable
python3 scripts/check_tutorials.py --toolchain 1.85.0
```

The runner extracts the committed library and application, copies the application
to a separate temporary directory outside the repository, and changes only its
umbrella path dependency to point at the sibling archive. It runs locked/offline
metadata, all-target tests, the CLI output contract, strict Clippy and application-package formatting;
checks that no registry packages were added relative to the library lockfile; and
retains exact compiler/source/command/hash evidence in `docs/0.3/evidence/tutorials/`.
It does not dispatch GitHub workflows or create a pull request.
