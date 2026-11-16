"""Alternate exact measured binaries; run after refactor_compat.py --measure."""
import gzip
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
import sys
import time

source = Path(sys.argv[1]).resolve()
prior = json.loads(source.read_text())
assert prior['status'] == 'passed'
root = Path.cwd()
output = source.parent / 'alternating'
output.mkdir(exist_ok=False)
manifest = {'measurement_manifest': str(source.relative_to(root)),
            'measurement_manifest_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
            'runner_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            'candidate_commit': prior['candidate_commit'],
            'baseline_commit': prior['baseline_commit'], 'toolchain': prior['toolchain'],
            'cpu': prior['measurement_cpu'], 'status': 'running', 'runs': [],
            'limitations': ['Shared host, seven alternating pairs; not an isolated final release qualification.',
                            'Same compiler, original/candidate lockfiles differ; not attribution to one extraction.',
                            'Tail values describe 31 sample averages, not individual-request latency.',
                            'Legacy workloads omit new owned claims and verified durable record readback.']}
binaries = {}
for lane in ['baseline', 'candidate']:
    binaries[lane] = Path(next(c['command'][-1] for c in prior['checks'] if c['label'] == lane + '-latency-0'))
    assert hashlib.sha256(binaries[lane].read_bytes()).hexdigest() == prior[lane + '_binary_sha256']
    manifest[lane + '_binary_sha256'] = prior[lane + '_binary_sha256']
path = output / 'manifest.json'

def save():
    path.write_text(json.dumps(manifest, indent=2) + '\n')

results = {lane: [] for lane in binaries}
save()
try:
    for pair in range(7):
        for lane in (['baseline', 'candidate'] if pair % 2 == 0 else ['candidate', 'baseline']):
            command = ['taskset', '-c', str(manifest['cpu']), str(binaries[lane])]
            start = time.monotonic()
            run = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            data = gzip.compress(run.stdout, mtime=0)
            log = output / f'pair-{pair}-{lane}.log.gz'
            log.write_bytes(data)
            manifest['runs'].append({'pair': pair, 'lane': lane, 'command': command,
                                     'exit_code': run.returncode, 'seconds': time.monotonic() - start,
                                     'artifact': log.name, 'sha256': hashlib.sha256(data).hexdigest()})
            save()
            assert run.returncode == 0
            results[lane].append({w['workload']: w for w in json.loads(run.stdout)['workloads']})
            print(f'pair {pair} {lane}: passed', flush=True)
    summary = []
    for name in results['baseline'][0]:
        base = [r[name] for r in results['baseline']]
        candidate = [r[name] for r in results['candidate']]
        median_deltas = [(c['median_ns'] / b['median_ns'] - 1) * 100 for b, c in zip(base, candidate)]
        p95_deltas = [(c['p95_ns'] / b['p95_ns'] - 1) * 100 for b, c in zip(base, candidate)]
        summary.append({'workload': name,
                        'baseline_median_ns': [b['median_ns'] for b in base],
                        'candidate_median_ns': [c['median_ns'] for c in candidate],
                        'paired_median_percent_deltas': median_deltas,
                        'median_paired_percent_delta': statistics.median(median_deltas),
                        'median_paired_absolute_ns_delta': statistics.median([c['median_ns'] - b['median_ns'] for b, c in zip(base, candidate)]),
                        'paired_p95_percent_deltas': p95_deltas,
                        'median_paired_p95_percent_delta': statistics.median(p95_deltas)})
    report = output / 'summary.json'
    report.write_text(json.dumps(summary, indent=2) + '\n')
    manifest['summary_sha256'] = hashlib.sha256(report.read_bytes()).hexdigest()
    manifest['status'] = 'passed'
finally:
    if manifest['status'] == 'running':
        manifest['status'] = 'failed_or_interrupted'
    save()
