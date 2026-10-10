# statechronicle-portable

Allocation-free `no_std` primitives for bounded consumers of StateChronicle.
No allocator, storage adapter, clock, transport or runtime is required.

`transition::execute` uses the portable `StateProfile` contract and consumer
codecs to validate and reduce native states. It checks immutable domain/schema,
expected versions, command/operation binding and state invariants, re-encodes
inputs to check canonicality, and writes the output into a caller-owned buffer.
Separate caller-owned scratch storage bounds canonicalization. Consumer codecs
and reducers must also be allocation-free for the complete call to be so.

`codec` supplies BCS primitive readers/writers over borrowed input and
caller-owned output buffers. Consumers implement `CanonicalCodec` for their
native state and command types, preserving field order and enforcing collection
and recursion limits. It is not a general Serde serializer.

`integrity` computes protocol-compatible resource keys and custom-state hashes,
verifies strict Ed25519 signatures over caller-supplied canonical bytes, and
checks sparse-Merkle inclusion/absence paths. Proof paths are borrowed,
strictly ascending and bounded to 256 siblings. Verification iterates 256
levels with constant scratch storage instead of allocating a tree or hash table.

These primitives do not supply signer trust, freshness, profile interpretation
or durable execution policy. Consumers retain those responsibilities.

Interoperability tests compare the standard encoder, hashes, signatures and
accumulator proofs. Bare-metal check:

```sh
cargo check -p statechronicle-portable --target riscv32imafc-unknown-none-elf
```
