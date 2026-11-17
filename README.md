# StateChronicle

StateChronicle is a **Rust library for tracking changes to application data,
keeping a signed history, and checking that the resulting state matches that
history**.

Use it when knowing the current value is not enough: you also need to know how
it changed, whether the change followed your rules, and whether someone has
altered the recorded history.

## What does it do?

Imagine a game where Alice transfers a sword to Bob. Your application needs to
check that Alice owns it, that she is allowed to transfer it, and that the sword
is available. After the transfer, it needs a record of what happened.

StateChronicle provides the logic to check the request, calculate the new state,
and produce events and a signed record for your application to store. Later,
you can rebuild ownership from those events or verify an ownership proof against
a trusted signed record. Changing recorded data makes its cryptographic checks
fail.

You can use the built-in item and balance rules, or define your own data types
and allowed changes.

## What is it useful for?

- **Game inventories and currencies:** track item ownership, transfers and balances,
  with a history you can replay after restoring your database.
- **Licenses and access rights:** record grants, suspensions and revocations, and
  check who held a right at a particular point in the history.
- **Configuration and equipment records:** define permitted updates and keep a
  verifiable history of revisions or recorded status changes.

It fits applications where changes must follow explicit rules and the history
needs to be checked by another part of the system. Signed records establish
integrity under keys you trust; your application still decides who may act and
whether an input reflects something that happened in the real world.

## How does it fit into my application?

StateChronicle runs inside your Rust application. You provide login and access
policies, signing keys, and a storage adapter for your database. The library
provides change validation, event and signed-record construction, replay, and
proof verification.

For reliable writes and retries, your storage adapter must implement the
[ledger contract](docs/0.3/adapter-contract.md). Start with the
[durable-ledger tutorial](tutorials/durable-custom-ledger/README.md) to see a
complete example using a small file store.

## Get started

Requires **Rust 1.88+**. This development branch contains the upcoming 0.3 API;
its package versions still read `0.2.0`.

```sh
cargo add statechronicle --git https://github.com/STEXS-Technologies/statechronicle --branch refactor/coherence
```

The [first tutorial](tutorials/minimal-native-domain/README.md) defines simple
configuration and equipment models and checks requests to change them. It
prepares changes without saving them. The
[second tutorial](tutorials/durable-custom-ledger/README.md) adds storage, signed
records, recovery and proof checks; its file store requires Rust 1.89+.
Both use development keys that must be replaced in a real application.

For the published 0.2 API, use `cargo add statechronicle@0.2.0` and its
[API reference](https://docs.rs/statechronicle/0.2.0/statechronicle/).
See [Rust support](docs/0.3/rust-support.md) for compiler details.

[Documentation](docs/README.md) · [Consumer guide](docs/0.3/consumer-guide.md) ·
[Changelog](CHANGELOG.md) · [Security](SECURITY.md)

Licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE).
