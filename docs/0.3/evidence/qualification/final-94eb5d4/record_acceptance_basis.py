"""Attach clause-specific manual review bases; terminal acceptance is separate."""
import json
from pathlib import Path
out=Path(__file__).resolve().parent
notes={
'F0':[
'Actual original peeled tag commit/archive/lock/toolchain/system evidence is independently checked by verify_baseline.py; annotated tag object is not confused with the commit.',
'Thirteen same-compiler rustdoc catalogs, real old implementers/exhaustive matches, examples and separate no-std consumer cover promised API; no custom feature matrix exists.',
'Original producer/provenance and immutable wire corpus supply old BCS/signature/root/history bytes. New receipt semantics are candidate overlays because original 0.2 had no canonical receipt type.',
'Frozen hosted/portable consumers plus separately identified new canonical overlays exercise all requested use cases; legacy imports remain original and independently compiled.',
'Pinned syntax/complexity metrics, semantic owner review, six cold builds, complete workload matrix, dependency graph and portable archive costs report both whole-project and per-file data.',
'Original 794-test success and original dependency-policy failure are independently retained, with original 25-target ASAN campaign. Current approved dependency policy is a separate passing gate.',
'Original acceptance probes reproduce supplied-key/freshness, marker-only durable success and tenant/cache problems; finding-disposition.md maps fixes and retained legacy limits.'
],
'F1':[
'ExpectedClaim carries independently selected address/key/owner/kind; current and original-proof external tests reject substitution and failed reads.',
'Current mode performs a final full-head authoritative comparison; historical mode requires a pin. Results document the observed verification point and do not promise later immutability.',
'Tenant/commit scoped lookup and leg-tenant checks reject the reproduced substitution; genuine same-ID/different-tenant proofs and independent signer keys remain supported.',
'Canonical protected mutation returns only verified retained ledger receipts; planning markers/empty vectors cannot qualify success. Original compatibility marker behavior stays labeled.',
'Committed/replayed, refusal/conflict, unknown and corrupt readback are distinct checked outcomes; dispatch authority comes from the receipt/outbox contract.',
'Before-fix original probes and retained chronological regressions precede fixes; old helper semantics remain explicitly documented where preserved.'
],
'F2':[
'Private shared evaluation owns checked identity/state/command/candidate/event facts; protected preparation types preserve signer, timestamp and authorization provenance.',
'Ordered gate tables and 348621 exact original/current characterization cases preserve event/error/callback order, including string diagnostic equality.',
'Single/batch/read-only executor forwards use one evaluation owner where equivalent; settlement/economic and whole-group atomicity remain explicit specialized policies.',
'Preparer receives only read capabilities and makes no durable intent/ledger claim; legacy Executor semantics remain characterized separately.',
'Protected signed plans bind the complete prepared event set, tenant/scope/profile/schema/identity before persistence, with substitution and reauthorization tests.',
'Model, stale/invalid, policy unavailable, unknown and corrupt conditions retain typed/actionable causes; unknown storage never becomes safe fresh retry.',
'Focused private pipeline owners retain earlier differential controls; no generic framework or new published crate was added.'
],
'F3':[
'Standalone native and file-ledger consumers compile against the public umbrella; concept/setup accounting names application decisions and imports without equating aliases with concepts.',
'TypedProfile combined evaluation shares decoded state/command while preserving validate/apply/output validation; default forwarding supports old StateProfile implementations.',
'Ordinary native enums plus static command_operations registration are compared with frozen manual assembly; seven literals replace fourteen occurrences without runtime registry mutation.',
'External command overlays compare exact old operations/envelopes and schema checks; schema migration requires separately authorized versioned policy.',
'Portable runtime has no new service/runtime/serializer dependency, bare-metal compilation succeeds, and all allocation probes must independently meet the hard zero-allocation condition.',
'Native-contract compiler diagnostics reject mixed types, omitted mappings, owned batch-leg misuse and omitted pins; dynamic schema/operation numbers are runtime-validated where Rust cannot prove them.',
'All eight built-in profiles are reviewed; equivalent field guards/forwarding overrides share owners, while different error ordering and reducer visibility remain separate.'
],
'F4':[
'Consumer guide/root README identify prepare, signed atomic commit, current/historical claims and verified rebuild as separate intent paths without requiring exact provisional namespaces.',
'LedgerTransaction alone owns canonical atomic effects; protected coordinator checks identity, complete events, commit, head, projections and outbox under one supported transaction.',
'Private-field CommittedMutation/CommittedBatch bind original signed records and complete membership, with first/replay disposition and exact readback validation.',
'ClaimVerifier needs only CommitReader and historical-only readers need no live head; external read-only consumer compiles without write ports.',
'TransactionManager and sink shortcuts are explicitly advanced symbolic planning/compatibility; canonical durable examples use protected receipts.',
'File tutorial provides one composition root with explicit principal/auth/key/signer/ledger policy and selected read/dispatch capabilities; API review counts required setup groups.',
'Compiler structural diff and migration/release matrix document default method additions, unchanged original imports and approved Rust/Dalek dependency-type boundary.'
],
'F5':[
'Unpublished conformance factory supports independent handles/failures/reopen/raw inspection and remains a dev tool, not a mandatory runtime dependency.',
'Deterministic single/batch conformance vectors and separately scoped file subprocess/SIGKILL tutorial cover logical and selected process boundaries.',
'Full durable and conformance tests inject failures at each named stage, rollback and ambiguous ACK, inspect independent authority, and refuse unsupported partial cross-tenant effects.',
'2/8/32 contender tests and mixed seeded schedules inspect exact same/conflicting identity results, CAS winners and tenant separation without timing as correctness oracle.',
'Corrupt/missing/reordered/replaced/duplicate records and reopened replay are checked by history/receipt/conformance tests; failed attempts expose no accepted partial canonical authority.',
'Verified history binds chain/event/root/signature/trust/pin; original upgrade and checkpoint-interruption schedules recover expected roots, with additional signed-history cost checks.',
'Frozen original producer and bytes remain unchanged; current verifier consumes original migration history and separately versioned new schema fixtures.',
'Broken partial/replay/ACK fakes, corrected deadlock barrier and OS-lock bypass controls fail at runtime; ineffective first control remains failed.'
],
'F6':[
'Maintenance inventory names decode, projection/schema, freshness, key/commit lookup, routing, error/string/digest and facade owners and their semantic differences.',
'Per-extraction source/parity/control evidence and owner tables record both callers, refusal order, portability/allocation limits and deliberate non-equivalence.',
'Actual deleted duplicate guards/string shells/evaluator/read wrappers are paired with original/current trace and targeted negative controls; moves alone are not counted as simplification.',
'Hotspot path/body metrics and complete-project growth are reported; private owner extraction retains persist/settlement/atomicity obligations without new generic frameworks.',
'API structural review and four retained duplicate forwarding/error groups document compatibility shells with no duplicated authority implementation.',
'Measured allocations/latency/build costs and source review cover decoding, strings, BTree coverage checks, hashes/signatures and forwarding; no trust check is removed to meet a cost target.',
'Thirteen-crate acyclic internal normal graph is preserved, unused production SHA-2 moved to dev, portable coupling remains minimal; matched RISC-V archive costs are measured.',
'Existing-path net shrink and added-owner growth are separated; every new trust owner and retained duplicate is justified with concrete consumer/invariant benefit.',
'Sparse-Merkle representation/algorithm is preserved with original known-answer vectors; no stylistic file split is claimed as simplification.'
],
'F7':[
'Root README leads through native domain, protected durable mutation and independently expected proof; inventory/trade appears as specialized usage.',
'Two complete standalone locked applications are copied/fresh-built by the release driver, including four native and twelve file tests and execution/lint/format checks.',
'Consumer guide decision table separates all requested profile/registry, plan/commit, proof mode, transaction, outbox/publisher and projection/history concepts.',
'Durable outcome/operator docs name every persistence/failure boundary and require same-identity lookup/recovery after unknown ACK.',
'Migration/release matrix map old imports/methods, frozen old BCS, explicit schema evolution and the approved Rust/Dalek type boundary; versions/tags/publication remain separate.',
'Tutorial host policy is default-deny for unrecognized authority and examples bind tenant/principal/key/head/expectations; fixed development keys/policies are explicitly test/tutorial scope.',
'Fresh unpacked thirteen-package tests/docs/lint plus standalone consumer locks, separate portable and bare-metal gates demonstrate clean declared-dependency builds.',
'Current audit links supersede dated pending evidence without rewriting historical ADRs; root/TODO/changelog/migration reflect current branch and unpublised version scope.',
'Operator recovery guide distinguishes exact retry, lookup, reconciliation, stop/repair and verified restore with explicit authority/policy limits.'
],
'F8':[
'Full runtime/test/build/lock scopes are byte-identical across all tested candidate commits and current tree; only affected checker gates are rerun after two checker-only fixes.',
'Exact-source stable/MSRV 1.100 local driver passes all required tests/lint/docs/audit/deny/source/layer/portable gates; 955 named test executions include standalone tutorials.',
'All 27 ASAN targets complete full recorded budgets plus fixed seeded claim/durable campaigns and runtime bypass controls; original 25-target evidence is separately retained.',
'Actual-original/current compatibility, 348621 strict semantic cases, original-produced history/reopen/migration and current signed receipts/history tests preserve intended old/new contracts.',
'Four-lane 22-path paired matrix, all-path individual samples/88 full allocations plus 88 matched setup controls, heavy larger sample sets, canonical 13-path and complete 1/32/256 signed-history campaigns cover every named workload class.',
'Whole-project source growth, specific deletions, repeated build/RSS/size data and every measured trigger have explicit invariant/consumer-cost disposition; no universal shrink/speedup is claimed.',
'Thirteen actual crate archives pass fresh unpacked tests/docs/lint in dependency order; changelog and compatibility matrix describe the unpublished candidate.',
'Codex implemented and compiled external native/ledger/read-only consumers and reviewed outcome/trust/implicit-policy decisions; review is explicitly agent-authored, not human signoff.',
'Separate authorization is required for a version bump, tag or registry publication; current goal authorizes qualification, commit and normal branch push only.'
],
'7.1':[
'Original producer hashes and frozen byte comparisons preserve BCS indices/envelopes and append-only custom variants.',
'Original independent signature payloads/digests/key/root/inclusion/absence vectors verify in candidate and packaged consumers.',
'Original event/commit/proof records decode with original meanings; new claim/history authority is an additional API, not reinterpretation.',
'Frozen imports/implementers/exhaustive matches compile; migration names intentional toolchain/dependency-type decisions.',
'Original schema history plus schema-one/two rejection and separately authorized migration/checkpoint tests preserve coexistence.',
'Frozen hosted and no-std portable consumers execute original/current transition comparisons independently.',
'Approved Rust 1.100 decision is documented; no new default feature coupling; supported no-alloc bare-metal consumer compiles.'
],
'7.2':[
'Protected preparation binds signed ingress/principal/tenant/resource/operation before constructing canonical signed plans; signer and policy refusal precede ledger access.',
'Ed25519 verification and separately selected signer policy both run; digest equality is never promoted into authorization.',
'Preparer lacks write/dispatch capability and refusal/cancellation tests inspect unchanged effects; signing alone creates no receipt.',
'Ledger transaction and retained readback bind the exact whole effect set under one supported atomic authority.',
'Same identity/payload recovers original receipt; altered payload/group/member/event scope refuses without new canonical effects.',
'Lost ACK/readback remain explicit unknown until same-identity lookup; control mutation that returns false success is detected.',
'Current claims reject stale full heads while historical claims require explicit independent pins and preserve original evidence.',
'Independent address/key/subject/kind/signer expectations reject relabeling and cross-tenant/commit substitution.',
'Unsupported atomic scope/cross-tenant topology refuses before partial persistence; separate orchestration cannot fabricate canonical success.',
'Original/current verified-source rebuild, 44 interrupted migration schedules and ACK-loss resume assertions agree with expected canonical roots.',
'Intentionally partial/replay/ACK/deadlock/lock-bypass implementations produce intended runtime failures, not unrelated compiler failures.'
],
'7.3':[
'Consumer guide and standalone apps show each requested native/mutation/proof/adapter intent explicitly.',
'Ordinary examples use public umbrella APIs with selected public adapter/runtime dependencies; no private module import is required.',
'Native models are defined once; static mapping removes seven duplicate operation literals and encode_command removes manual envelope assembly.',
'Short diagram/table separates pure StateProfile, addressing/economic ProfileRules and hosted TypedProfile adapter responsibilities.',
'Private verified success types and distinct rejected/conflict/unknown/corrupt/recovery variants have explicit action semantics.',
'Legacy sinks/TransactionManager remain labeled planning/compatibility/advanced and are absent from the recommended durable path.',
'API review names the specific misuse prevented by protected plans, complete receipts/batches, read-only claims and verified history; no alias count stands in for concepts.',
'Standalone apps use independent Cargo manifests/locks and fresh copied builds; unpacked crate consumers have no hidden workspace source dependency.'
],
'7.4':[
'Ownership map names native evaluation, preparation, commit validation, ledger, receipt/batch, claim and verified-history authorities with non-equivalent checks retained.',
'Decode instrumentation/parity demonstrates shared native work; different output validation and economic addressing checks remain justified.',
'Exact original/current syntax reports plus physical/body/per-function approximation measure all requested source dimensions with explicit macro limits.',
'Per-extraction removals/string guards/default forwards and original-path negative token/branch accounting substantiate bounded deletions; moves and generated branches are not relabeled as savings.',
'Thirteen published crates and internal normal graph unchanged; no new portable runtime/allocator/service edge; remaining hosted allocations have explicit safety reasons.',
'Six fresh cold builds and tiny/full/portable artifacts report overlapping time range, accepted 4MiB RSS growth and consumer-specific binary effects.',
'Changed current docs pass file-link checks and old evidence/ADR chronology is retained with current-audit pointers.'
],
'Scenario-A':[
'Configuration native state/command declarations occur once in the standalone model.',
'Static command_operations mapping, schema constants/codecs and typed registry bind the domain.',
'Signed ingress/authenticated preparation produces private protected events without persistence.',
'Canonical tutorial/reference ledger writes complete signed effects atomically under exact identity.',
'Exact retry returns retained original receipt without reduction or new outbox effects.',
'Changed payload same identity refuses and independent raw inspection preserves previous authority.',
'Current proof uses independently expected configuration address/owner/key/head through public claim APIs.'
],
'Scenario-B':[
'Original frozen absence followed by signed creation supplies independent historical/current evidence.',
'Current absence rejects the old valid absent root after head advance.',
'Expected absent address derives an independent key and rejects proof of another resource.',
'Expected tenant/resource/subject/key and stored signed reference checks refuse substitution.',
'Historical mode preserves original pre-creation absence under selected pin and trusted signer.'
],
'Scenario-C':[
'Protected signed mutation binds immutable idempotency and exact payload/event identity.',
'Failure injection occurs after authority commit and before caller ACK, including reopened adapters.',
'Caller receives Unknown with no fabricated committed receipt or safe fresh-retry classification.',
'Fresh handle lookup verifies exact retained identity/digest/original effects.',
'Original committed replay is returned while raw head/outbox inspection proves one accepted effect set.',
'Precommit failures expose no accepted effects; only supported exact-identity retry is allowed after authoritative resolution.'
],
'Scenario-D':[
'Settlement contention/conformance schedules exercise competing supported requests.',
'CAS/identity/whole-group authority checks allow at most the supported winner; losers receive refusal/conflict with no accepted partial transfer.',
'Protected settlement/group readback binds every event/projection/root under one supported scope.',
'Unsupported multi-tenant authority refuses before partial writes; independent signed legs do not imply distributed atomicity.',
'External Penelope coordination is optional and cannot override canonical state results; any new operation needs its own authorization.'
],
'Scenario-E':[
'Actual original commit producer provides unchanged four-commit configuration history and wire bytes.',
'Current verifier/rebuilder consumes old history in bounded chunks across every retained checkpoint interruption schedule.',
'Resumed verified history reproduces signed canonical roots with explicit source identity/pin.',
'Schema-one/two registry tests reject reinterpretation of old bytes as a new schema.',
'Separately bound host migration grant authorizes new version/resource policy while original source remains historically verifiable.'
],
'DoD':[
'Root/consumer guide/tutorials present one typed-domain to protected receipt and explicit proof-mode journey.',
'Parity-backed evaluator/decode/guard/string/read-owner deletions reduce duplicated true control paths; macro/file moves do not stand in for simplification.',
'Original byte/crypto/history/identity/transaction/portable gates plus adversarial controls preserve documented guarantees and deliberate fixes.',
'Public dev conformance contract/vectors and independent inspection support custom backend qualification within explicit logical/process limits.',
'Actual-original cross-version schemas/signatures/roots/proofs/import obligations and new separately versioned history/migration all pass.',
'Full frozen candidate gates include stable/MSRV/property/all-target fuzz/coverage/crash/concurrency/performance/package plus independent artifact verification.',
'Source growth, target-local reductions, compiler/API concept counts, retained duplicates and repeated consumer-specific costs are reported honestly with ownership benefits.',
'Finding disposition closes known canonical trust defects and distinguishes migrated legacy limits; no known P0/P1 or unnamed deferred P2 implementation defect is waived.'
]}
p=out/'requirement-inventory.json';j=json.loads(p.read_text())
for group,values in notes.items():
 rows=[r for r in j['requirements'] if r['id'].rsplit('-',1)[0]==group];assert len(rows)==len(values),(group,len(rows),len(values))
 for row,note in zip(rows,values):row['acceptance_basis']=note
assert all('acceptance_basis' in r for r in j['requirements'])
p.write_text(json.dumps(j,indent=2)+'\n');print('Recorded clause-specific manual acceptance bases for',len(j['requirements']),'requirements; terminal statuses remain unchanged.')
