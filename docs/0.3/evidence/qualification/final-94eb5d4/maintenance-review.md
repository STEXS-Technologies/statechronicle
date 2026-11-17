# Final source and ownership review

Candidate `0799afe` and the original `v0.2.0` are measured with the same pinned
Python 3.13/tree-sitter versions and production-syntax policy. Raw reports are
`source-metrics.json` and `baseline-source-metrics.json`.

| Metric | Original 0.2 | Candidate | Delta |
|---|---:|---:|---:|
| syntax_lines | 14,137 | 16,251 | +2,114 |
| lexical_tokens | 105,588 | 118,924 | +13,336 |
| branch_nodes | 1,676 | 1,941 | +265 |
| public_syntax_items | 1,221 | 1,458 | +237 |

The workspace grows; there is no overall source-size reduction claim. The
additional surface owns protected read-only preparation, signed exact-plan
binding, retained original receipt/recovery, complete atomic batch membership,
expected current/historical claims and verified resumable history. Those
contracts are additional functionality and explicit trust checks rather than
replacement names for legacy success. Their runtime and build costs require
separate measurements, not an inference from syntax counts.

The maintained ownership boundaries are:

| Invariant | Owner | Reason retained paths remain |
|---|---|---|
| Native decode/validate/reduce | `TypedProfile` evaluation | Legacy profile defaults support existing implementations; native path shares decoded values |
| Authentication and preparation | Executor preparation owner | Legacy execution/sink APIs retain their existing effects and error order |
| Event root/projections/outbox preflight | Commit validation owners | Readback validates potentially untrusted stored copies independently |
| Atomic canonical effects | `LedgerTransaction` | Legacy `TransactionManager` cannot enlist independent stores and is not upgraded by aliasing |
| Exact durable identity/receipt | Commit durable single/batch owners | Batch membership is a complete group contract, not repeated single commits |
| Expected claim trust/freshness | `ClaimVerifier` | Historical pin and current live head are intentionally distinct |
| Verified history/rebuild | Verified history owner | Derived projections cannot select canonical history |
| Value-leg declaration | Existing profile predicate | Economic amount/pair attribution remains a separate validator |
| Validated string trait shells | Private domain macro | Each constructor retains its own validation and wire name |

Per-extraction evidence in `evidence/maintenance/` and `evidence/f2/` records
actual deletions and equivalence controls. The string-trait extraction removes
2,129 production syntax tokens; profile guards remove 517 tokens/18 branch
nodes across their six files; the value-leg declaration removes 37 tokens.
These are bounded hotspot results, not additive percentages or macro-expanded
runtime savings. Exact function-token duplicates still include distinct amount
and identity semantics: token equality alone does not authorize merging types.

There remain thirteen publishable crates, with no new runtime storage service,
permissive production authorizer or cross-tenant atomicity promise. The approved
dependency/toolchain refresh is documented separately in `dependency-refresh.md`;
this qualification changes neither library dependencies nor default features.
A final build-cost/performance acceptance record is required before closure.

## Remaining exact duplicate function groups

The final parser reports exactly four groups of at least 30 tokens. All are
retained deliberately:

| Group | Size per function | Retention decision |
|---|---:|---|
| `From<StateChronicleError>` for executor and intent errors | 77 tokens | Different public error types own their conversion contract. Sharing the match would require a generic error-construction interface or a new cross-crate conversion abstraction; it would not remove validation work. Both retain direct tested size-limit remapping and domain wrapping. |
| `Executor::prepare_player` / `Preparer::prepare_player` | 58 tokens | Compatibility and minimal-read-only entry points forward to the same evaluation owner; no duplicated gate/reduction implementation. |
| `Executor::prepare_player_batch` / `Preparer::prepare_player_batch` | 57 tokens | Same bounded forwarding-shell justification; complete-group semantics remain in one owner. |
| `Executor::prepare_player_settlement` / `Preparer::prepare_player_settlement` | 57 tokens | Same compatibility shell; protected settlement validation remains in the shared evaluator. |

The paired public forwards preserve the old executor integration while allowing
new consumers to inject only read preparation ports. Removing one would remove
a supported entry point; a macro would exchange three short transparent forwards
for another syntax abstraction without deleting any authoritative execution path.


## Source growth accounting

`source-growth-accounting.json` separates unchanged-path files from files added
since original 0.2. Added files contain 17,007 production syntax tokens and 379
parsed branch nodes; original-path files have a net decrease of 3,671 tokens,
446 syntax lines and 114 parsed branch nodes. The total therefore grows 13,336
tokens and 265 parsed branch nodes. New files include both functionality and
extracted owners, and existing files include moves and safety changes, so these
are accounting partitions rather than an equivalence-only speedup or a precise
cost per new feature. Macro expansion can preserve multiple runtime branches
behind one parsed syntax node. The retained per-extraction parity evidence is
what establishes whether a particular deletion preserves behavior.

## Physical and function-body complexity supplement

`complexity-metrics.json` and `measure_complexity.py` add physical file lines, exact-test-item-excluded physical lines, function-body token lines and per-function approximate cyclomatic complexity. These supplement the existing syntax-token/branch counts. They do not expand macros or count generated executable paths.

| Metric | Original | Candidate | Delta |
|---|---:|---:|---:|
| physical_file_lines | 35,781 | 39,169 | +3,388 |
| physical_lines_excluding_exact_test_items | 23,478 | 26,304 | +2,826 |
| function_body_token_lines | 8,802 | 10,133 | +1,331 |
| function_count | 702 | 775 | +73 |
| approximate_cyclomatic_sum | 2,438 | 2,804 | +366 |

The per-function estimate is 1 plus explicit if/while/for/try decisions, short-circuit operators and match alternatives beyond the first. It is an auditable syntax approximation, not a control-flow graph or machine-code complexity proof. Body lines include declarations/braces; raw physical lines include comments/blanks/test blocks, while the separately labeled filtered count removes exact test-attributed items.

| Existing hotspot path | Original body lines / approximate complexity | Candidate body lines / approximate complexity |
|---|---:|---:|
| `crates/statechronicle-executor/src/pipeline.rs` | 671 / 170 | 403 / 98 |
| `crates/statechronicle-profiles/src/atomicity.rs` | 583 / 132 | 581 / 132 |
| `crates/statechronicle-profiles/src/transition.rs` | 674 / 277 | 646 / 266 |
| `crates/statechronicle-index/src/build.rs` | 327 / 71 | 327 / 68 |
| `crates/statechronicle-commit/src/persist.rs` | 465 / 134 | 509 / 150 |

Path-local decreases do not prove deletion: private evaluator/claim/receipt/history owners are also present in the complete candidate totals. The complete project grows under this approximation, consistent with the separately justified new trust capabilities; the parity-backed duplicate removals remain the evidence of simplification. Macro consolidation may hide generated branches and is never counted as runtime branch elimination.
