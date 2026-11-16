"""Matched hosted typed transition measurements; never edits frozen fixtures."""
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import tarfile
import time

root = Path.cwd()
control = subprocess.check_output(['git', 'rev-parse', '63b9a47'], text=True).strip()
candidate = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
if subprocess.check_output(['git', 'status', '--porcelain', '--', 'crates', 'Cargo.toml', 'Cargo.lock']):
    raise RuntimeError('Commit production changes before measuring')
stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) + '-' + str(os.getpid())
work = root / 'target' / 'typed-bench' / stamp
out = root / 'docs/0.3/evidence/f3/typed-performance' / stamp
out.mkdir(parents=True)
work.mkdir(parents=True)
hash_bytes = lambda data: hashlib.sha256(data).hexdigest()
fixture = (root / 'ci/performance/typed_evaluation.rs').read_bytes()
env = dict(os.environ, RUSTC_WRAPPER='', CARGO_BUILD_JOBS='8')
for key in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS']:
    env.pop(key, None)
manifest = {'control_commit': control, 'candidate_commit': candidate,
            'runner_sha256': hash_bytes(Path(__file__).read_bytes()), 'fixture_sha256': hash_bytes(fixture),
            'toolchain': {tool: subprocess.check_output([tool, '-Vv' if tool == 'rustc' else '-V'], text=True, env=env) for tool in ['rustc', 'cargo']},
            'platform': platform.platform(), 'cpu': min(os.sched_getaffinity(0)),
            'environment': {'RUSTC_WRAPPER': '', 'CARGO_BUILD_JOBS': '8', 'RUSTFLAGS': 'unset', 'CARGO_ENCODED_RUSTFLAGS': 'unset'},
            'status': 'running', 'checks': [], 'artifacts_sha256': {},
            'limitations': ['Shared host; seven alternating rounds, no discarded samples.',
                            'Tail values are percentiles of 31 sample averages, not individual request latency.',
                            'Only fixed configuration create/update/malformed-command profiles; no universal speedup claim.',
                            'Same resolved dependencies and compiler; control profile source matches frozen original.',
                            'Control-subtracted whole-process heap totals include setup/profiler effects; raw totals retained.',
                            'Native Configuration reducer retains its own validation call; adapter does not bypass it.']}

def save():
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

def artifact(name, data):
    (out / name).write_bytes(data)
    manifest['artifacts_sha256'][name] = hash_bytes(data)
    return name

def check(label, command, cwd):
    start = time.monotonic()
    run = subprocess.run(command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = artifact(label + '.log.gz', gzip.compress(run.stdout, mtime=0))
    manifest['checks'].append({'label': label, 'command': command, 'cwd': str(cwd), 'exit_code': run.returncode,
                               'seconds': time.monotonic() - start, 'artifact': log})
    save()
    print(label, run.returncode, flush=True)
    if run.returncode:
        raise RuntimeError(label + ' failed')
    return run.stdout

save()
try:
    source_locks = []
    resolved_locks = []
    binaries = {}
    for lane, revision in [('control', control), ('candidate', candidate)]:
        source = work / lane / 'source'
        source.mkdir(parents=True)
        data = subprocess.check_output(['git', 'archive', revision])
        with tarfile.open(fileobj=io.BytesIO(data)) as archive:
            archive.extractall(source, filter='data')
        source_locks.append((source / 'Cargo.lock').read_bytes())
        if lane == 'control':
            original = subprocess.check_output(['git', 'show', 'c418e98b:crates/statechronicle-profiles/src/custom.rs'])
            assert original == (source / 'crates/statechronicle-profiles/src/custom.rs').read_bytes()
            manifest['original_profile_sha256'] = hash_bytes(original)
        consumer_root = work / lane / 'consumer'
        shutil.copytree(root / 'ci/compat-0.2', consumer_root)
        consumer = consumer_root / 'crates/consumer'
        cargo = consumer / 'Cargo.toml'
        cargo.write_text(cargo.read_text().replace('../../../../crates/statechronicle', str(source / 'crates/statechronicle')))
        shutil.copyfile(source / 'Cargo.lock', consumer / 'Cargo.lock')
        # Only the combined call is substituted in the control: both modes use
        # legacy check/apply there. Production source and frozen fixtures stay intact.
        code = fixture
        if lane == 'control':
            old = b'rules.evaluate(current, operation, inputs).map_err(|error| error.to_string())'
            assert code.count(old) == 1
            code = code.replace(old, b'legacy(rules, current, operation, inputs)')
        artifact(lane + '-benchmark.rs', code)
        (consumer / 'examples/typed_evaluation.rs').write_bytes(code)
        check(lane + '-resolve', ['cargo', 'metadata', '--offline', '--format-version', '1', '--all-features'], consumer)
        lock = (consumer / 'Cargo.lock').read_bytes()
        resolved_locks.append(lock)
        artifact(lane + '.lock', lock)
        check(lane + '-build', ['cargo', 'build', '--example', 'typed_evaluation', '--release', '--locked', '--offline'], consumer)
        binary = consumer / 'target/release/examples/typed_evaluation'
        binaries[lane] = binary
        manifest[lane + '_binary_sha256'] = hash_bytes(binary.read_bytes())
    assert source_locks[0] == source_locks[1], 'Source lockfiles differ'
    assert resolved_locks[0] == resolved_locks[1], 'Resolved consumer lockfiles differ'
    manifest['resolved_lock_sha256'] = hash_bytes(resolved_locks[0])
    lanes = [('control', 'legacy'), ('candidate', 'legacy'), ('candidate', 'combined')]
    results = {lane + '-' + mode: [] for lane, mode in lanes}
    for pair in range(7):
        order = lanes if pair % 2 == 0 else list(reversed(lanes))
        for lane, mode in order:
            label = lane + '-' + mode
            data = check(f'pair-{pair}-{label}', ['taskset', '-c', str(manifest['cpu']), str(binaries[lane]), mode.ljust(32, '_')], root)
            results[label].append({row['workload']: row for row in json.loads(data)['workloads']})
    summary = []
    for baseline, selected in [('control-legacy', 'candidate-legacy'), ('control-legacy', 'candidate-combined'), ('candidate-legacy', 'candidate-combined')]:
        for name in results[baseline][0]:
            before = [row[name] for row in results[baseline]]
            after = [row[name] for row in results[selected]]
            deltas = [(c['median_ns'] / b['median_ns'] - 1) * 100 for b, c in zip(before, after)]
            tails = [(c['p95_ns'] / b['p95_ns'] - 1) * 100 for b, c in zip(before, after)]
            summary.append({'baseline': baseline, 'candidate': selected, 'workload': name,
                            'baseline_median_ns': [row['median_ns'] for row in before], 'candidate_median_ns': [row['median_ns'] for row in after],
                            'paired_percent_deltas': deltas, 'median_paired_percent_delta': statistics.median(deltas),
                            'median_paired_absolute_ns_delta': statistics.median([c['median_ns'] - b['median_ns'] for b,c in zip(before,after)]),
                            'paired_p95_percent_deltas': tails, 'median_paired_p95_percent_delta': statistics.median(tails)})
    artifact('summary.json', (json.dumps(summary, indent=2) + '\n').encode())
    allocations = {}
    for lane, mode in lanes:
        key = lane + '-' + mode
        allocations[key] = {}
        for name in ['hosted_create', 'hosted_update', 'hosted_malformed']:
            totals = {}
            for variant in ['control', mode]:
                data = check(f'alloc-{key}-{name}-{variant}', ['taskset', '-c', str(manifest['cpu']), 'valgrind', '--tool=memcheck', '--leak-check=no', '--error-exitcode=97', str(binaries[lane]), variant.ljust(32, '_'), name.ljust(32, '_')], root)
                match = re.search(rb'total heap usage: ([\d,]+) allocs, ([\d,]+) frees, ([\d,]+) bytes allocated', data)
                assert match, 'Missing allocation totals'
                totals[variant] = {'allocations': int(match[1].replace(b',',b'')), 'bytes': int(match[3].replace(b',',b''))}
            allocations[key][name] = {'raw': totals, 'control_subtracted_per_operation': {metric: (totals[mode][metric] - totals['control'][metric]) / 1000 for metric in ['allocations', 'bytes']}}
    artifact('allocations.json', (json.dumps(allocations, indent=2) + '\n').encode())
    manifest['status'] = 'passed'
finally:
    if manifest['status'] == 'running':
        manifest['status'] = 'failed_or_interrupted'
    save()
