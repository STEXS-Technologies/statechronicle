# statechronicle-index

## What it is

The trade read-side vertical slice: a pure, deterministic builder that projects
committed batches into trade records, plus an async `TradeService` that ingests
batches incrementally, rebuilds from a raw stream, serves ordered trade history,
and assembles/verifies portable trade proofs. The builder is pure and free of
clocks, RNG, and `HashMap`; the service composes the read-side ports and holds no
logic.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §16 (trade ext.) | Trade Proof Read | Assembles the per-tenant trade proof from settled-asset state proofs |
| §18.3 (trade ext.) | Trade Read Projection | Accumulates trade records: sides, value legs, event order, status |

## Key types

- `build::{apply, IngestBatch, batch_trade_ids}`: the pure builder and the
  per-batch ingestion unit (events + declaring settle intents + signing commit).
- `build::{TradeRecord, TradeProof, TRADE_PROOF_SCHEMA}`: the accumulated
  read-side record and the portable proof it feeds (defined in
  `statechronicle-domain`, consumed here).
- `service::{TradeService, TradePorts}`: the async composition layer over the
  trade index / event store / proof index / commit store ports, with the
  `ingest_batch` / `rebuild` / `get_history` / `get_proof` service methods.
- `history::reconstruct_history`: the pure ordered-history reconstruction.
- `error::{IndexError, TradeServiceError}`.

## How it's used

Each committed trade execution result is ingested as one `IngestBatch` per tenant
commit. `ingest_batch` seeds the records already present for the trade ids a
batch touches, applies the batch, and upserts the result, so incremental
ingestion merges into (rather than clobbers) existing records; `rebuild` replays
a raw batch stream into a fresh index. `get_history` reconstructs the ordered
trade events, and `get_proof` assembles and internally verifies a portable trade
proof, fetching each settled asset's state proof at the commit that settled it
(per-asset commit tracking).

```rust
let batch = IngestBatch { events, settle_intents, commit };
service.ingest_batch(&batch).await?;
let proof = service.get_proof(&trade_id).await?.unwrap();
assert!(verify_trade_proof(&proof, &commits_by_tenant).is_ok());
```

## Dependencies

`statechronicle-core`, `statechronicle-domain`, `statechronicle-accumulator`,
`statechronicle-commit`, `statechronicle-profiles`, `statechronicle-ports`,
`statechronicle-proof`. Dev-only: `proptest`, `tokio`, `bcs`.

## Tests

`crates/statechronicle/tests/trade_history_proof.rs` covers the end-to-end
history + proof slice, incremental-vs-rebuild index equality, and the per-asset
commit fetch; inline unit tests in `build.rs` cover the pure projection and the
determinism proptest. Fuzz target: `fuzz_trade_index_build`.

## Where it fits

The read side of the trade completion pipeline (`execute -> commit -> index ->
proof -> verify`). It projects committed trade executions into the records that
`get_history` serves and `get_proof`/`verify_trade_proof` prove. The umbrella
crate re-exports `TradeService`, `TradePorts`, `IngestBatch`, and the trade
read-side domain types.
