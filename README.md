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

## Quick start

Requires **Rust 1.88+**. Create a project and add the 0.3 source from `main`:

```sh
cargo new first-proof
cd first-proof
cargo add statechronicle --git https://github.com/STEXS-Technologies/statechronicle --branch main
```

**An old ownership proof stops working when the owner changes.**
Replace `src/main.rs` with this complete program:

```rust
use statechronicle::portable::integrity::hash;
use statechronicle::{StateAccumulator, StateKey, StateUpdate};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sword = StateKey::for_resource("my-game", "sword-1");
    let mut inventory = StateAccumulator::empty();

    // Record Alice as the owner and get a proof of this snapshot.
    let alice = StateUpdate::new(sword, hash(b"alice"));
    let before = inventory.insert_batch(&[alice])?;
    let proof = inventory.prove_inclusion(&sword).ok_or("missing sword")?;
    assert!(StateAccumulator::verify_inclusion(&before, &proof));
    println!("Alice's proof: valid");

    // Change the owner. The old proof no longer matches the new snapshot.
    let bob = StateUpdate::new(sword, hash(b"bob"));
    let after = inventory.insert_batch(&[bob])?;
    assert!(!StateAccumulator::verify_inclusion(&after, &proof));
    println!("After owner change: Alice's old proof rejected");
    Ok(())
}
```

Run it with `cargo run`:

```text
Alice's proof: valid
After owner change: Alice's old proof rejected
```

Each snapshot has a root: a fingerprint of the inventory. A proof can be checked
against that root without sharing the whole inventory. Alice's proof still
matches the earlier snapshot, but cannot establish ownership in the updated one.

This small example updates an in-memory tree directly. In an application, use
the authenticated mutation APIs to enforce transfer permissions, and obtain the
root from a trusted signed commit. The tutorials add those pieces:

- [Define your own data and authenticate requests](tutorials/minimal-native-domain/README.md).
- [Save signed changes, recover a lost reply and check proofs](tutorials/durable-custom-ledger/README.md).

The second tutorial uses a temporary file store and requires Rust 1.89+.

This checkout prepares version **0.3.0**. After publication, you can use
`cargo add statechronicle@0.3.0` instead of the Git dependency above.
For applications upgrading from 0.2, see the [migration guide](docs/0.3/migration.md).
See [Rust support](docs/0.3/rust-support.md) for compiler details.

[Documentation](docs/README.md) · [Consumer guide](docs/0.3/consumer-guide.md) ·
[Changelog](CHANGELOG.md) · [Security](SECURITY.md)

Licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE).
