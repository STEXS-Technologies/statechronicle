# Later-head retention qualification

Exact candidate `9de9fc2ce6f40c5cc3edc7b89244da193f0f8541` passes all ten checks: stable and Rust 1.85.0 each run 12 integration functions, zero doctest examples, strict Clippy, strict rustdoc and formatting. All three faulty retention modes fail the intended `lookup.original` oracle.

The reference writes two independently signed commits to different resources in one tenant. The expected raw view retains both histories and finalizations and only the later head. Exact original receipts survive head advance and removal of the first commit's pending outbox rows; an exact retry after head advance returns its original commit without changing raw effects.

No production crate, root manifest/lockfile or fuzz tree changed from `1db60a8`; matching Git objects are recorded. No new production throughput, workspace-test or release-gate result is claimed. Same-key projection replacement, batch retention/rebuild, interrupted/corrupt replay and physical backend recovery remain open.

`qualify.py` archives committed HEAD into an isolated source tree and runs both compiler lanes with isolated targets. Run from repository root at the candidate, with installed toolchains and cached dependencies. Machine-specific paths in the manifest describe the actual run. Compressed logs, hashes, compiler versions, source hashes and scope are in `manifest.json`. The preflight and AST-only graph update logs are also retained.
