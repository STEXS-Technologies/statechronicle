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

## Run your first example

You need Rust and Git installed. These examples use this development branch's
upcoming 0.3 API; its package versions still read `0.2.0`.

**1. Define data and check a change.** This example creates configuration and
equipment records using your own Rust types. It checks the requests and shows
the proposed results, without saving anything. No database or service is needed.

```sh
git clone --branch refactor/coherence https://github.com/STEXS-Technologies/statechronicle.git
cd statechronicle
rustup toolchain install 1.88.0 --profile minimal
cargo +1.88.0 run --manifest-path tutorials/minimal-native-domain/Cargo.toml --locked
```

Expected output:

```text
Prepared configuration: ConfigurationState { revision: 1, enabled: true }
Prepared equipment: EquipmentState { mode: Idle, cycles: 0 }
Uncommitted: no intent claim, ledger write, outbox dispatch or durable receipt.
```

See the [data types and rules](tutorials/minimal-native-domain/src/domains.rs),
[application code](tutorials/minimal-native-domain/src/main.rs) and
[step-by-step explanation](tutorials/minimal-native-domain/README.md).

**2. Save changes and recover a lost reply.** From the same checkout, run:

```sh
rustup toolchain install 1.89.0 --profile minimal
cargo +1.89.0 run --manifest-path tutorials/durable-custom-ledger/Cargo.toml --locked
```

This example saves signed changes to a temporary file, simulates losing the
save confirmation, then reopens the file to find out what happened. Retrying
the same request returns the original result without saving it twice. It also
updates the configuration and checks proofs of its old and new state.

The output includes:

```text
Commit acknowledgement: unknown; retain original identity and signed plan.
Recovery after reopen: original committed mutation verified.
Exact retry: replay; one original event, commit and outbox row.
```

See the [application code](tutorials/durable-custom-ledger/src/main.rs) and
[walkthrough](tutorials/durable-custom-ledger/README.md). Its file store needs
Rust 1.89+; the library itself supports Rust 1.88+. Both examples use development
keys that must be replaced in a real application.

## Add it to your project

For the development API used above:

```sh
cargo add statechronicle --git https://github.com/STEXS-Technologies/statechronicle --branch refactor/coherence
```

For the published 0.2 API, use `cargo add statechronicle@0.2.0` and its
[API reference](https://docs.rs/statechronicle/0.2.0/statechronicle/).
See [Rust support](docs/0.3/rust-support.md) for compiler details.

[Documentation](docs/README.md) · [Consumer guide](docs/0.3/consumer-guide.md) ·
[Changelog](CHANGELOG.md) · [Security](SECURITY.md)

Licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE).
