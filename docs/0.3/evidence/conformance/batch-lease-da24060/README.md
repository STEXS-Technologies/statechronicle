# Batch lease takeover conformance

Exact archived `da2406008c0e08c87f96166c11a82d32b99ee8e1` qualifies the isolated
unpublished conformance package on Rust 1.85.0 and 1.100.0. Each compiler passes
11 integration functions, strict Clippy, strict rustdoc and formatting. The
doctest command also succeeds with zero examples.
`qualify.py` archives HEAD and uses isolated targets per compiler. Reproduce in
a disposable checkout pinned to this commit; it writes only under `target/`.
The manifest records source/archive/log hashes, commands, compilers and exits.

No production Rust, root manifest/lock or conformance manifest/lock changes.
Git object equality against `d70ce92` proves this, including the entire crates
tree. No runtime API or dependency changes. No production allocation or performance
delta is claimed.
Only reusable dev checks, reference faults and their instructions change.

Whole-group expiry checks every token, stale commit/rollback/drop, independent
pending lookup, complete publication and exact reopened group recovery. A shared
expiry model passes this schedule. The expiry callback must tolerate siblings
already expired by one shared-clock advance; it cannot forge replacement tokens.

A separate optional configuration hook arranges independently expirable member
leases before ordinary claims. Each of the two members is replaced in turn while
the other remains live. The new claim reply must match independent pending
lookup, old effects/retained rows stay invisible, stale cleanup preserves the
successor, and partial acquisition remains pending until rollback. Shared-only
models explicitly reject this unsupported setup as `adapter.operation`.

The tests expose an important negative-control boundary: a backend fencing only
the first member passes whole-group takeover but fails isolated member 1. The
source asserts that exact member index. Five fault behaviors produce named
errors in retained `--nocapture` logs:

| Fault | Required observed oracle |
| --- | --- |
| Accept stale whole-group writer | `batch.lease.stale_commit` |
| Check only first member | `batch.lease.stale_member_commit`, `Some(1)` |
| Return wrong successor token | `batch.lease.claim_token` |
| Erase successor in stale cleanup | `batch.lease.successor` |
| Reuse stale attempt token | `batch.lease.token` |

The initial prototype failed compilation on a missing Intent import and borrowed
error details. Successful intermediate prototypes also remain unqualified: the
first combined schedule did not distinguish shared from isolated deadlines.
Only the archived final checks establish this checkpoint. No prior tests or
negative assertions were removed to obtain a pass.

These are bounded logical overlapping-transaction schedules with two-member
signed-commit genesis fixtures and independent raw observations. They do not
qualify actual database time/locking, process termination, power loss, larger
groups, every interleaving, later-head retention/rebuild or corrupt replay.
Those F5 requirements and full F8 release qualification remain open.
