# Protected durable commit and recovery fuzz checkpoint

Candidate: `a3b57a206eb44ece9ae47ba96e58475deb507c7e`.
The retained immutable-source campaign and stable qualification both pass.

| Seed | Completed executions | Reported seconds | Crash artifacts |
|---|---:|---:|---:|
| 7331 | 7,890 | 61 | 0 |
| 42 | 7,246 | 61 | 0 |

Each independent campaign starts from the 20 binary seed inputs, uses nightly
ASAN instrumentation, and stops at a 60-second or 1,000,000-run limit with a
64-byte input bound. The observed total is 15,136 executions, not two million.
Raw logs, final corpora, compiler version, commands, source hashes and budgets
are recorded in `manifest.json`. Stable formatting, strict fuzz-bin Clippy and
cargo-deny pass in `stable-qualification.json`.

## Exercised contract

The target prepares and signs a protected single mutation, reauthorizes at
commit and recovery, and compares the returned signed record, exact identity,
events, projections and outbox against independently prepared expectations.
Eight injected transaction boundaries, optional rollback failure, successful
commit and lost acknowledgement cover absence, unknown outcome, fresh-handle
recovery, exact retry and replay without duplicate effects. Lookup failure and
conflicting payloads refuse. Every input also exercises six corrupt-record
classes: digest, events, projections, outbox payload, signature and intent.
Redundant fake-storage copies remain internally consistent so they cannot mask
the receipt verifier's signature check.

Both insecure-source controls compile and exit 77 at their intended runtime
assertion: removing commit-phase unknown classification fails
`durable.ack.unknown`; removing recovered commit signature verification fails
`durable.recovery.corrupt_record:4`. Positive and control binaries have distinct
hashes. All positive source/lock inputs remain unchanged, and deliberate
mutations are restored exactly.

## Reproduction and evidence verification

Run from the repository root:

```sh
python3 scripts/check_fuzz_campaign.py --candidate a3b57a206eb44ece9ae47ba96e58475deb507c7e --target fuzz_durable_recovery --output <new-directory>
python3 docs/0.3/evidence/fuzz/durable-recovery-current/verify.py
```

`runner.py` retains the exact campaign driver. `verify.py` checks the original
Git archive, complete source/seed maps, artifact hashes, observed completion,
control diagnostics and mutation hashes, stable qualification, fresh positive
compilation and distinct binaries. It verifies retained evidence without
rerunning the campaign.

## Earlier attempts and runner qualification

The sibling evidence directories retain unsuccessful or superseded work:

- `durable-recovery-initial-4107da6`: positive execution fails when the conflict
  probe exceeds the nonce bound. The later probe flips a byte within the bound.
- `durable-recovery-masked-control-9f83384`: positive campaigns pass, but the
  signature bypass incorrectly survives because redundant fake storage rejects
  the inconsistent fixture first. This is a failed control, not qualification.
- `durable-recovery-pre-isolation-9ee5b53`: recorded checks pass, but builds used
  a shared target directory. Archived mtimes can allow a cached insecure binary
  to contaminate later archived builds. The current campaign supersedes it with
  a fresh per-archive target directory and observed positive compilation.
- `fresh-failure-artifact-control`: the isolated runner revisits `4107da6`,
  observes the expected positive failure and retains its 64-byte crash input in
  `positive-7331-artifacts.tar.gz`, proving failure artifacts survive cleanup.
- `shared-runner-claim-regression`: two short read-side campaigns and both
  controls pass through the shared driver, but predate build isolation and do
  not establish a fresh current-candidate claim campaign.

## Limits and remaining work

This is bounded logical testing with a synchronous in-memory atomic fake and an
explicit test authorization policy. Reopening shares memory; it does not test
process restart, physical storage durability, power loss, concurrent adapters,
arbitrary multi-mutation histories, settlement or migration. No production
permissive policy is introduced. This advances canonical durable API fuzz
coverage; full exact-candidate release qualification remains open.
