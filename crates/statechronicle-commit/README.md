# statechronicle-commit

## What it is

Commit formation: groups ordered events into durable, signed commits. It owns
deterministic ordering, event/state root computation, Ed25519 commit signing,
tenant checkpoint commits, and fork/failure semantics.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §13 | Commit Model | The structure and requirements of a signed commit |
| §13.1 | Commit Fields | Commit ID, scope, event Merkle root, previous/next state roots, signature |
| §13.2 | Commit Requirements | What makes a commit valid and verifiable |
| §13.3 | Batch Semantics | How events group into a batch within one commit |
| §13.4 | Tenant Checkpoint Commits | Optional global checkpoints over tenant roots |
| §14 | State Root Model | The previous/next state root contract |
| §19 | Commit Authority | Commit keys and the signed chain |
| §31 | Forks and Recovery | Fork detection and append-only fork evidence |

## Key types

- `batch::CommitBatch`: an ordered, validated group of events.
- `builder::CommitBuilder`: assembles a `Commit` from a batch, previous root,
  and references.
- `roots::{compute_state_root, state_root_updates}`: pure state-root functions.
- `sign::{sign_commit, verify_commit}`: Ed25519 commit signing/verification.
- `ordering`: deterministic event ordering.
- `persist`: commit persistence orchestration.
- `checkpoint`: tenant checkpoint commits.
- `fork`: fork detection, chain-continuity checks, and `ForkEvidence`.
- `error::CommitError`.

## How it's used

After the executor emits events, the composition root builds a batch, runs them
through `CommitBuilder`, and signs the resulting commit body. The signed commit
is the anchor every proof verifies against.

```rust
use statechronicle_commit::{batch::CommitBatch, builder::CommitBuilder, sign::sign_commit};
// Build batch, form commit with builder, sign the body...
let signed = sign_commit(&commit, &key, &key_id)?;
```

## Dependencies

`statechronicle-core`, `statechronicle-domain`, `chrono`. Dev-only: `proptest`.

## Tests

`tests/`: `commit_pipeline.rs`, `ordering_checkpoint_fork.rs`, `property.rs`,
plus inline unit tests for root determinism, signing, ordering, checkpoints,
and fork detection.

## Where it fits

The third stage of the canonical pipeline, after `statechronicle-executor`.
`statechronicle-proof` verifies against the signed commits this crate produces.
