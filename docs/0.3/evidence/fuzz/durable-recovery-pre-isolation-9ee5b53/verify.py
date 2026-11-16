#!/usr/bin/env python3
"""Verify retained claim campaign source, logs, corpora and runtime controls."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import tarfile

out = Path(__file__).resolve().parent
manifest = json.loads((out / 'manifest.json').read_text())
sha = lambda data: hashlib.sha256(data).hexdigest()

def require(condition, message):
    if not condition:
        raise ValueError(message)

require(manifest['status'] == 'passed' and manifest['positive_source_unchanged'] and manifest['source_restored'], 'campaign status/guards')
archive = subprocess.check_output(['git', 'archive', manifest['candidate_commit'], '--', *manifest['archive_paths']])
require(sha(archive) == manifest['archive_sha256'], 'archive hash')
with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
    files = {p.name: tar.extractfile(p).read() for p in tar.getmembers() if p.isfile()}
require({name: sha(data) for name, data in files.items()} == manifest['source_sha256'], 'source map')
runner = subprocess.check_output(['git', 'show', manifest['candidate_commit'] + ':scripts/check_fuzz_campaign.py'])
require(runner == (out / 'runner.py').read_bytes(), 'retained runner differs from candidate')
for name, digest in manifest['artifacts_sha256'].items():
    require(sha((out / name).read_bytes()) == digest, 'artifact ' + name)
durable = manifest['target'] == 'fuzz_durable_recovery'
seed_prefix = 'fuzz/seed-corpus/' + ('durable-recovery/' if durable else 'claim-verifier/')
require({name.removeprefix(seed_prefix): sha(data) for name, data in files.items() if name.startswith(seed_prefix)} == manifest['seed_sha256'], 'seed provenance')
logs = {}
for check in manifest['checks']:
    require(check['exit_code'] == check['expected_exit_code'], 'exit ' + check['label'])
    raw = (out / check['artifact']).read_bytes()
    require(sha(raw) == check['sha256'], 'check log hash')
    log = gzip.decompress(raw).decode(errors='replace')
    if check['expected_diagnostic']:
        require(check['expected_diagnostic'] in log and manifest['target'] + '.rs' in log, 'control runtime assertion')
    logs[check['label']] = log
controls = ['ack','signature'] if durable else ['head','pin']
require(set(logs) == {'positive-build','positive-7331','positive-42'} | {name for c in controls for name in ['negative-'+c, 'negative-'+c+'-build']}, 'check completeness')
require(sorted(c['seed'] for c in manifest['campaigns']) == [42,7331], 'independent seeds')
for campaign in manifest['campaigns']:
    log = logs['positive-' + str(campaign['seed'])]
    counts = re.findall(r'Done (\d+) runs in (\d+) second', log)
    require(counts == [(str(campaign['completed_runs']),str(campaign['reported_seconds']))], 'observed completion')
    require(campaign['completed_runs'] > 100 and campaign['crash_artifacts'] == 0, 'extended outcome')
    require('INFO: Seed: ' + str(campaign['seed']) in log and 'inline 8-bit counters' in log, 'seed/instrumentation')
    require(sha((out / campaign['final_corpus']).read_bytes()) == campaign['final_corpus_sha256'], 'corpus hash')
    with tarfile.open(out / campaign['final_corpus'], 'r:gz') as tar:
        members = [m for m in tar.getmembers() if m.isfile()]
        require(len(members) >= (20 if durable else 18) and all(m.size <= 64 for m in members), 'retained corpus scope')
source = files['crates/statechronicle-commit/src/durable.rs' if durable else 'crates/statechronicle-proof/src/claim.rs'].decode()
mutations = [
    ('ack','TransactionPhase::Commit | TransactionPhase::Rollback','TransactionPhase::Rollback'),
    ('signature','verifier\n        .verify(&record.commit)\n        .map_err(DurableError::Corrupt)?;','let _ = verifier;')
] if durable else [
    ('head','ensure_canonical(self.reader, expected.tenant(), &signed).await?;','let _ = &signed;'),
    ('pin','if &proof.commit().commit_id != pinned_commit {','if false && &proof.commit().commit_id != pinned_commit {')]
for name,before,after in mutations:
    require(source.count(before) == 1 and sha(source.replace(before,after).encode()) == manifest[name+'_mutated_source_sha256'], 'negative source '+name)
qualification = json.loads((out / 'stable-qualification.json').read_text())
require(qualification['status'] == 'passed' and qualification['candidate_commit'] == manifest['candidate_commit'] and qualification['source_matches_campaign'], 'stable qualification')
require({c['label'] for c in qualification['checks']} == {'stable-fmt','stable-clippy','dependency-policy'}, 'stable check completeness')
for check in qualification['checks']:
    require(check['exit_code'] == 0 and sha((out / check['artifact']).read_bytes()) == check['sha256'], 'stable result/hash')
print('Verified two instrumented seeded campaigns, fixed/final corpora, immutable source, stable checks, restoration and two intended runtime failures for ' + manifest['target'] + '.')
