# Authorized schema migration and interrupted historical replay

Test source: `7489a5fd03c168c30f2ff173d9d6aea0d26990a4`. No production library,
API, dependency or lockfile changes occur in this checkpoint.

## Independent original history

The new `original_configuration_history.json` corpus was produced by the actual
original release `c418e98b91720188158c531cd5c2c4b8ab71cbcf` using its unchanged
library source and lockfile. [Producer provenance](../migration-original-producer/manifest.json)
retains the exact producer, archive/source hashes, compiler, command, real exit
and output hash. Existing original wire/consumer fixtures are unchanged.

The corpus has four schema-one configuration commits/events/intents, their
original BCS byte arrays, revision four with `enabled = false`, and the signed
final root. Candidate tests compare every original byte array, enforce the
corpus hash and independently fixed historical tip/root, verify signed history
from genesis and reexecute all old transitions through the schema-one adapter.
The independently trusted fixture key and historical pin are selected by the
host test, not accepted from a provider's substituted history. Altered events,
missing genesis, invalid signatures and a newly signed fork are refused.

## Separate migration authority

A read-only `Preparer` prepares a schema-two migration to a **new resource**.
The host authorizer's grant is fixed from the verified old source before client
input is considered. It binds the migration service principal/session, tenant,
target, operation, unborn target version, native command, source resource,
verified-history digest and old state digest. Those source references are
covered by the intent signature. Ordinary operators have a valid fixture key
scoped to the operation but lack the separate migration grant.

Eight request cases refuse without ledger or symbolic transaction effects:
ordinary actor, missing source, changed history/state digest, changed command,
changed target, revoked grant and bad signature. Revocation is checked again
before commit and before recovery lookup. The accepted migration preserves the
old schema-one resource and creates schema two with the verified old value and
revision. Both schema decoders and transition verifiers retain their original
meaning; relabeling old bytes as schema two is refused.

The historical records are restored through the advanced persistence API only
after source verification, under a separate exact-intent administrative restore
policy. This fixture restore does not claim to authenticate an original player's
current session. The migration uses protected preparation and `SignedMutation`.
A modeled lost ACK returns unknown; a fresh handle recovers the exact original
signed migration, and retry replays without new effects. The migration event ID
uses a distinct injected namespace so a restored producer counter cannot reuse
old event identities.

This grants migration of an explicitly pinned **historical snapshot**. It does
not assert that the source remains a live current head during migration. A
policy requiring live source freshness must bind and atomically enforce that
additional read dependency; these tests do not provide that contract.

## Interrupted rebuild

The old four-event source passes 18 interruption schedules. The five-event
source, including the authorized migration, passes 26. For every chunk width
from one through the source length, every checkpoint save is interrupted either
before persistence or after persistence with a lost acknowledgement. Fresh
handles share the retained checkpoint/rows, resume without skipping a projection,
produce the exact signed final root and permit an idempotent completed restart.
The sink uses monotonic idempotent upserts, including retries after partial
chunk writes. Old/new verified sources with the same job name have different
checkpoint namespaces. Both native schema states survive the completed rebuild.

These are in-memory handles and injected failures, not process or disk crashes.
Signed historical verification establishes integrity under a host signer/pin
policy; it does not reenact historical domain authorization or establish current
canonical freshness.

## Retained qualification and controls

[Runtime manifest](manifest.json) pins the source archive, commands and logs.
All three new tests pass, including all 44 checkpoint schedules. Five isolated
controls fail selected runtime tests with real Cargo exit 101 and checked
assertion markers:

- Ignoring the migration grant admits a revoked request.
- Ignoring expected source inputs admits omitted source binding.
- Discarding rebuilt rows violates the final root assertion.
- Reusing a job checkpoint across sources violates source namespace isolation.
- Reusing a historical event ID is refused by signed-history construction.

Each deliberately incorrect source change is retained as a patch; the checkout
is never mutated by the runner. Compiler errors do not count as runtime controls.
The last control reproduces a fixture counter-reset mistake caught during early
integration, and demonstrates why applications own unique event ID generation.
It does not qualify global duplicate-ID enforcement by this memory backend.

[Immutable-source root campaign](../qualification-7489a5f/manifest.json) passes
896 finite tests, 16 doctests, formatting, strict Clippy and strict rustdoc.
Source hashes are checked after all commands. Run the retained runner from the
pinned checkout in a fresh evidence directory; it refuses to overwrite a run.

This advances roadmap Scenario E for the retained configuration domain and
historical fixture. Generic migrations, independently implemented database
backends, final coverage/fuzz/performance/package gates and complete release
qualification remain separate requirements.
