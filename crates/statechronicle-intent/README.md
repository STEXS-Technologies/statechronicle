# statechronicle-intent

## What it is

The intent parsing and validation stage: it turns a raw client payload (JSON
or bytes) into a validated, canonical `ValidatedIntent` before that intent
enters the execution pipeline. Follows the stage-separated `Raw -> Validated`
convention.

## Protocol sections owned

| § | Title | Normative summary |
|---|---|---|
| §11 | Intent Model | The fields and requirements of a valid intent document |
| §11.1 | Intent Fields | `schema`, tenant, intent_id, operation, actor, resource, state_type, expected_version, inputs, created_at, expires_at, nonce |
| §11.2 | Intent Requirements | Structural validity, idempotency key, size limits, optional detached signature |

## Key types

- `raw::RawIntent`: the unvalidated document as submitted by a client.
- `validated::ValidatedIntent`: the canonical, validated form the executor
  consumes, carrying its idempotency key and optional detached signature.
- `validated::IdempotencyKey`.
- `parse::parse_intent`: schema + size check over the raw payload.
- `validate::validate`: structural validation into `ValidatedIntent`.
- `error`: intent processing error type.

## How it's used

A client submits a raw payload; `parse_intent` then `validate` produce a
`ValidatedIntent`. The composition root (or the e2e harness) signs the
canonical intent body and binds an optional authority proof before passing it
to the executor.

```rust
use statechronicle_intent::{parse::parse_intent, validate::validate};

let raw = /* client payload bytes */;
let parsed = parse_intent(&raw)?;
let validated = validate(&parsed)?;
```

## Dependencies

`statechronicle-core`, `statechronicle-domain`, `serde_json`. Dev-only:
`proptest`.

## Tests

`tests/`: `pipeline.rs` (parse -> validate end to end), `property.rs`, plus
inline unit tests for idempotency keys, size limits, and validation rules.

## Where it fits

The first stage of the canonical pipeline (`submit -> parse+validate ->
execute -> commit -> proof`). It feeds `statechronicle-executor`. The umbrella
crate re-exports `ValidatedIntent` and `IdempotencyKey`.
