#!/usr/bin/env python3
"""Archive and qualify the committed dev-only mixed-history conformance suite."""
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import time

root = Path(__file__).resolve().parents[1]
def output(*args):
    return subprocess.check_output(args, cwd=root, text=True).strip()
def sha(data):
    return hashlib.sha256(data).hexdigest()
revision = output('git', 'rev-parse', 'HEAD')
stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())
evidence = root / 'docs/0.3/evidence/conformance' / ('mixed-history-' + revision[:7])
evidence.mkdir(parents=True, exist_ok=False)
run = root / 'target/mixed-history' / stamp
source = run / 'source'
source.mkdir(parents=True)
paths = ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'clippy.toml', 'crates', 'fuzz', 'ci/conformance']
archive = subprocess.check_output(['git', 'archive', revision, '--', *paths], cwd=root)
with tarfile.open(fileobj=io.BytesIO(archive)) as handle:
    handle.extractall(source, filter='data')
assert not output('git', 'diff', revision, '--', *paths)
manifest = {
    'candidate_commit': revision,
    'baseline_commit': 'd829c7c8e3f62e03b465c3afa1143c0f505bab65',
    'archive_paths': paths, 'archive_sha256': sha(archive),
    'rustc': output('rustc', '-vV'), 'cargo': output('cargo', '-V'),
    'environment': {'RUSTC_WRAPPER': '', 'CARGO_BUILD_JOBS': '8', 'CARGO_TARGET_DIR': str(root / 'ci/conformance/target')},
    'scope': 'Archived committed conformance source, 32 single mutations, two tenants, native asset and subject-held balance history; all old conformance tests retained.',
    'limitations': ['Logical in-memory reference and faulty variants, not a database/power-loss qualification.', 'Fresh connection handles do not restart the process.', 'No mixed single/batch history, randomized concurrency or live proof-serving claim.', 'No production Rust/manifests/locks or frozen reference data changed; hosted workspace tests were not repeated for this dev-only change.'],
    'source_sha256': {str(p.relative_to(source)): sha(p.read_bytes()) for p in sorted(source.rglob('*')) if p.is_file()},
    'checks': [], 'status': 'running',
}
(evidence/'runner.py').write_bytes(Path(__file__).read_bytes())
for name in ['history-preflight.log', 'history-preflight-fixed.log', 'history-clippy-preflight.log', 'history-tests-preflight.log', 'history-clippy-fixed.log']:
    (evidence/(name+'.gz')).write_bytes(gzip.compress((root/'target'/name).read_bytes(), mtime=0))
manifest['preflight_note'] = 'Uncommitted development attempts, not exact-candidate qualification: initial compile failed because TenantId is not Ord; initial strict Clippy failed on nested if; corrected preflights passed. Exact source identity applies to the archived checks below.'
def save():
    (evidence/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
def check(label, args, extra=None):
    env = dict(os.environ, **manifest['environment'], **(extra or {}))
    start=time.monotonic()
    run=subprocess.run(args, cwd=source, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    data=gzip.compress(run.stdout,mtime=0)
    artifact=label+'.log.gz'
    (evidence/artifact).write_bytes(data)
    manifest['checks'].append({'label':label, 'command':args, 'cwd':'archived source', 'extra_environment':extra or {}, 'exit_code':run.returncode, 'seconds':time.monotonic()-start, 'artifact':artifact, 'sha256':sha(data)})
    save()
    print(label, run.returncode, flush=True)
    if run.returncode:
        raise RuntimeError(label+' failed')
save()
try:
    cargo=['cargo', '--manifest-path', 'ci/conformance/Cargo.toml']
    check('fmt',['cargo','fmt','--manifest-path','ci/conformance/Cargo.toml','--package','statechronicle-conformance','--','--check'])
    check('all-target-tests',['cargo','test','--manifest-path','ci/conformance/Cargo.toml','--all-targets','--all-features','--locked','--offline','--','--nocapture'])
    check('doctests',['cargo','test','--manifest-path','ci/conformance/Cargo.toml','--doc','--all-features','--locked','--offline'])
    check('strict-clippy',['cargo','clippy','--manifest-path','ci/conformance/Cargo.toml','--all-targets','--all-features','--locked','--offline','--','-D','warnings'])
    check('strict-rustdoc',['cargo','doc','--manifest-path','ci/conformance/Cargo.toml','--all-features','--no-deps','--locked','--offline'], {'RUSTDOCFLAGS':'-D warnings'})
    assert all(sha((source/p).read_bytes()) == h for p,h in manifest['source_sha256'].items())
    assert not output('git', 'diff', revision, '--', *paths)
    manifest['status']='passed'
except BaseException as error:
    manifest['status']='failed_or_interrupted'
    manifest['failure']=repr(error)
    raise
finally:
    manifest['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in sorted(evidence.iterdir()) if p.name!='manifest.json'}
    save()
print(evidence.relative_to(root), flush=True)
