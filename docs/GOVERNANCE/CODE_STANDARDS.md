# Code Standards

These standards apply to **every crate and lane** in this workspace. They are binding
for all implementation work, including delegated implementation.

## 1. `mod.rs` / `lib.rs` are declaration shells only

- `mod.rs` / `lib.rs` contain **module declarations and re-exports only** — no logic.
- `lib.rs` is a declaration + re-export surface: `pub mod ...;` and `pub use ...;`
- Every module's real code lives in its own named file (`canonicalize.rs`, `digest.rs`,
  `signature.rs`, ...), never inside `mod.rs`.

## 2. Imports at the top of the file — no inline paths

- All `use` statements are grouped at the **top of the file**, before any code.
- Never write fully-qualified paths inline in code bodies
  (`crate::foo::Bar::new(...)` in the middle of a function).
- If a path is needed, `use` it at the top first, then reference the short name.

## 3. No string matching — always newtypes

- **Never** match, compare, or branch on raw `&str`/`String` values for domain
  concepts (identifiers, kinds, operations, statuses, digests).
- Every domain concept is a **validated newtype** or enum:
  `ContentDigest`, `TenantId`, `ResourceId`, `SubjectId`, `OperationName`, `StateType`,
  `EvaluationResult`, ...
- Newtypes are validated at construction and expose typed accessors, never raw string
  fields that invite string comparisons elsewhere.
- Enum variants are exhaustive; newtype validation rejects invalid values at the
  boundary.

## 4. Testing — five layers, required

Every implemented unit of logic requires:

| Layer | Where | Tooling |
|---|---|---|
| **Unit tests** | inline `#[cfg(test)] mod tests` in the module file | cargo test |
| **Integration tests** | `crates/<crate>/tests/*.rs` exercising the public API | cargo nextest |
| **Property tests** | proptest strategies asserting invariants (roundtrip, determinism, root-is-function-of-set, verify∘prove = identity) | proptest |
| **Fuzz targets** | `fuzz/fuzz_targets/*.rs` (cargo-fuzz) for parse/canonicalize/verify hot paths | cargo-fuzz (+nightly) |
| **e2e** | full-flow tests with real signatures / real adapters where applicable | nextest (e2e) |

- No production code without a unit test. No public API surface without an integration
  test. No canonicalization/parsing/verification path without property + fuzz coverage.
- Conformance vectors (language-agnostic JSON) for canonicalization, event replay, and
  proof verification (trustgrant convention).

## 5. Strict lint discipline (inherited from workspace lints)

- No `unwrap`, `expect`, `panic`, `unreachable`, `todo!`, `unimplemented!` in
  non-test code.
- No indexing/slicing (`indexing_slicing` denied) — use `.get()`.
- No `arithmetic_side_effects` — use checked/wrapping ops explicitly.
- `#![deny(unsafe_code)]` in every crate.
- Errors are typed (`thiserror`) with `# Errors` doc sections on fallible public APIs.

## 6. Canonical forms (ADR-004)

- All hashed/signed objects use **BCS canonical serialization**; JSON is the HTTP API
  logical view only.
- Signed objects use an explicit envelope body: `Obj { body, signature }` — signatures
  cover the BCS bytes of `body`, never the signature field.
- Digests are `sha256:<lowercase-hex>` via the `ContentDigest` newtype.

## 7. Ports

- Port traits live in `statechronicle-ports`; **no implementations inside**.
- Port signatures reference statechronicle-domain types only — dependency-free by
  construction. Adapters (incl. trustgrant, storage) live in consumer composition roots
  (stexs), never in statechronicle crates.
