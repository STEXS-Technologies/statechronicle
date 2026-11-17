# StateChronicle

Controlled state changes, signed history and verifiable state for Rust. Start
with your own typed state and commands; add authenticated mutations, durable
commits, proof verification or replay as needed. Built-in profiles cover
inventory, currency, access, marketplaces and escrow.

Mutations prepare a plan before committing it. Durable success requires a
checked receipt; uncertain writes must be resolved through recovery before retry.

## Get started

This branch contains the upcoming 0.3 API. Workspace package versions remain
`0.2.0`. Use Rust 1.100 or newer; repository builds use the toolchain pinned in
`rust-toolchain.toml`.

```sh
cargo new my-app
cd my-app
cargo add statechronicle --git https://github.com/STEXS-Technologies/statechronicle --branch refactor/coherence
```

Follow the [native-domain tutorial](tutorials/minimal-native-domain/README.md) for
complete configuration and equipment models with authenticated preparation.
The [durable-ledger tutorial](tutorials/durable-custom-ledger/README.md) adds
signed commits, recovery and proof verification using a bounded file adapter.

To try the included example first:

```sh
git clone --branch refactor/coherence https://github.com/STEXS-Technologies/statechronicle.git
cd statechronicle
cargo run --manifest-path tutorials/minimal-native-domain/Cargo.toml --locked
```

For the published API, use `cargo add statechronicle@0.2.0` and its
[versioned API reference](https://docs.rs/statechronicle/0.2.0/statechronicle/).
The tutorials use the unreleased branch and fixed development credentials.

## Choose what you need

| Need | Start here |
| --- | --- |
| Define typed state and legal commands | [Native domains](tutorials/minimal-native-domain/README.md) · `StateProfile` and `TypedProfile` |
| Authenticate, prepare and commit mutations | [Durable ledger](tutorials/durable-custom-ledger/README.md) · `statechronicle::mutation` |
| Recover an uncertain write or retry a batch | [Recovery and batches](docs/0.3/durable-batches.md) · `mutation::recover` and `SignedBatch` |
| Verify current state or a pinned historical claim | [Proof claims](docs/0.3/proof-claims.md) · `proof::claim::ClaimVerifier` |
| Verify signed history and resume projection rebuilds | [Verified rebuild](docs/0.3/verified-rebuild.md) · `index::rebuild::verified` |
| Run bounded transitions without allocation | [Portable core](crates/statechronicle-portable/README.md) · `statechronicle::portable` |

Applications supply authentication, authorization, key management and storage
adapters. StateChronicle ships no database or server; durable guarantees depend
on an adapter satisfying the [ledger contract](docs/0.3/adapter-contract.md).
The file tutorial demonstrates selected recovery cases, not production database
qualification. The `StateProfile` contract and portable core support `no_std`;
the facade and full engine require `std`.

[Consumer guide](docs/0.3/consumer-guide.md) ·
[Architecture](docs/ARCHITECTURE.md) · [Migration](docs/0.3/migration.md) ·
[Completed qualification](docs/0.3/evidence/qualification/final-94eb5d4/README.md) ·
[Changelog](CHANGELOG.md) · [Security](SECURITY.md)

Licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE).
