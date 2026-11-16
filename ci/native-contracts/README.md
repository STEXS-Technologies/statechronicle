# Native consumer type and value boundaries

From a committed checkout, run:

```sh
python3 scripts/check_native_contracts.py
```

The runner archives the selected revision and builds an application outside the
repository. Its only dependencies are the public umbrella, Serde and the signing
key type used by proof APIs. It copies the canonical example's two native domains;
no private engine imports or workspace test harness are used. It resolves once
from the root lock, refuses new registry packages, then uses locked/offline Cargo
commands and checks lock/source stability. Evidence includes exact source,
compiler, diagnostics, commands, exit codes and hashes under
`docs/0.3/evidence/f3/native-contracts/`.

`positive.rs` must check, run and pass strict Clippy. Each `cases.json` probe
changes exactly one expression in that otherwise valid consumer. Cargo must
return 101, every compiler error must have the intended code, and an intended
message must have a primary span in the application source, directly or through
its macro call-site expansion chain. The original valid
application is restored and checked after every refusal. Unrelated compile,
dependency or infrastructure errors cannot count as a successful probe.

| Misuse | Required compiler refusal |
| --- | --- |
| Equipment command passed to configuration adapter | `E0308` |
| Configuration reduction returned as equipment state | `E0308` |
| Owned prepared mutation extracted from borrowed batch leg | `E0508` |
| Historical claim requested without a commit pin | `E0061` |
| Historical replay source requested without a history pin | `E0061` |
| New command variant omitted from explicit operation mapping | `E0004` |
| Native reducer returns an event instead of its declared state | `E0053` |

Schema version numbers, wire bytes and external domain metadata are runtime
values. The positive application also proves zero-schema registration/encoding
refusal, version-one/two command and state isolation, cross-domain state refusal,
wrong-operation refusal and valid execution of both domains. The compiler cannot
prove that two types with the same Rust representation use the same domain or
that a chosen migration is authorized. The historical-pin probes prove required
API inputs, not the trustworthiness of their values. Signed history, explicit
migration, old-version vectors and signer authorization remain separate checks.

The runner's negative control deliberately requires the valid program to be
rejected, and **must exit nonzero**:

```sh
python3 scripts/check_native_contracts.py --negative-control-valid-program
```

This demonstrates that acceptance is not silently treated as a successful
compile-fail case. It changes the generated external probe only; source archives,
fixtures and production code remain intact. Neither command creates a PR,
triggers a GitHub workflow, changes dependencies or publishes a package.
