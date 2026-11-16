# Single-source command operation declarations

Candidate `0a302a8cb861edc937066a58c91e63600dae4ac7`, control
`48ff7ac6006b1c81d319dc150828c8859660cd71`. `command_operations!` generates an
ordinary discriminator match and fixed lazy operation array. The existing
manual `StateProfile::operation` plus `TypedProfile::new` API remains available.
No new trait, associated bound, runtime registration mutation, adapter layer,
dependency or wire representation is added. Names/schema remain validated by
the existing registry; decoded command-operation binding remains mandatory.

The actual Configuration and Equipment example removes duplicate operation
literals: seven names occur once each instead of twice. It shrinks from 250
lines/8,682 bytes to 241 lines/8,356 bytes. Those are example source counts, not
production-code reductions. Runtime string materialization still occurs once
in a fixed lazy array; there is no new heap vector. The public macro and its
facade re-export add API and source: +18 production syntax lines, +190 tokens,
zero parsed branch nodes and +1 public syntax item. The parser does not expand
macros, so those counts miss the exported macro itself and generated consumer
code. This is a justified authoring convenience, not a workspace shrink claim.

The external overlay compiles the migrated example through the public facade
and compares nine native commands with frozen original Configuration/Equipment
models: operation order, canonical envelope bytes, unborn-state evaluation and
wrong-operation refusal. Unknown domain/schema and empty/reserved operation
names fail through the existing registry. Compile-fail doctests reject omitted
variants, guards and duplicate patterns. Authors must use explicit variants
with ignored payloads; wildcard fallbacks and payload-specific patterns can
weaken their own policy. No macro can infer appropriate authority permissions.

Stable and MSRV each pass 865 finite workspace tests and 13 doctests. Strict
stable Clippy and stable/MSRV rustdoc pass. The MSRV archived external consumer,
wire, portable and bare-metal matrix passes (linked in the manifest). A further
strict MSRV Clippy run exposes issues in unchanged qualification/production code;
its failed log is retained, with the follow-up corrections in `d20a5cc`.
No clean MSRV Clippy result is claimed for this exact feature commit.

The focused `expansion-probe.rs` compares a manual and generated discriminator
under the same compiler/options; LLVM emits `manual` as an alias of `generated`.
The probe uses a local owned-string Operation stub for the macro's constructor
contract, not an actual storage adapter. This establishes identical optimized
mapping code for this model, not a workflow speedup or final performance gate.
To reproduce, copy the probe to `target/operation-registration/expansion-probe.rs`
and execute the exact manifest command from the repository root. Its relative
module path deliberately reads the actual production macro implementation.

Source hashes, logs and metrics are retained. Candidate checks used the clean
production worktree at this commit; external compatibility independently
archives it. Broader signed execution/replay/proof composition, remaining
F1–F7 work, full fuzz/property/crash/coverage/performance/package campaigns,
security advisory disposition and release qualification remain incomplete.
