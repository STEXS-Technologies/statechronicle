#!/usr/bin/env python3
"""Record a fresh-target facade consumer build, dependency graph and binary."""
import argparse
import gzip
import io
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import time

from refactor_baseline import BASELINE, digest, output
from refactor_compat import packages


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--revision', default=BASELINE)
    args = cli.parse_args()
    root = Path(__file__).resolve().parents[1]
    revision = output('git', 'rev-parse', args.revision, cwd=root)
    stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) + f'-{os.getpid()}'
    run = root / 'target/refactor-costs' / stamp
    source = run / 'source'
    source.mkdir(parents=True)
    archive = subprocess.check_output(['git', 'archive', revision], cwd=root)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        tar.extractall(source, filter='data')
    consumer = run / 'consumer'
    shutil.copytree(root / 'ci/cost-consumer', consumer)
    cargo = consumer / 'Cargo.toml'
    cargo.write_text(cargo.read_text().replace('../../crates/statechronicle',
                                               str(source / 'crates/statechronicle')))
    shutil.copyfile(source / 'Cargo.lock', consumer / 'Cargo.lock')
    evidence = root / 'docs/0.3/evidence/costs' / stamp
    evidence.mkdir(parents=True)
    affinity = sorted(os.sched_getaffinity(0))[:8]
    manifest = {
        'revision': revision, 'archive_sha256': digest(archive),
        'runner_sha256': digest(Path(__file__).read_bytes()),
        'consumer_sha256': {str(p.relative_to(root / 'ci/cost-consumer')): digest(p.read_bytes())
                            for p in sorted((root / 'ci/cost-consumer').rglob('*')) if p.is_file()},
        'toolchain': output('rustc', '-Vv'), 'platform': platform.platform(),
        'cpu_affinity': affinity, 'cpu_model': next((line.split(':', 1)[1].strip()
            for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')), ''),
        'environment': {'RUSTC_WRAPPER': os.environ.get('RUSTC_WRAPPER', ''),
                        'CARGO_BUILD_JOBS': os.environ.get('CARGO_BUILD_JOBS', '')},
        'limitations': ['Cold target, warm registry/filesystem caches; shared host, one build sample.',
                        'Binary exercises amount construction and resource-key hashing only.',
                        'Not latency/allocation/performance qualification of all protocol workflows.'],
        'checks': [], 'status': 'running',
    }
    path = evidence / 'manifest.json'

    def save():
        path.write_text(json.dumps(manifest, indent=2) + '\n')

    def check(label, command):
        record = {'label': label, 'command': command, 'status': 'running'}
        manifest['checks'].append(record)
        save()
        print('Running:', label, flush=True)
        log = run / (label + '.log')
        start = time.monotonic()
        with log.open('wb') as handle:
            result = subprocess.run(command, cwd=consumer, stdout=handle, stderr=subprocess.STDOUT,
                                    preexec_fn=lambda: os.sched_setaffinity(0, affinity))
        data = gzip.compress(log.read_bytes(), mtime=0)
        artifact = evidence / (label + '.log.gz')
        artifact.write_bytes(data)
        record.update(exit_code=result.returncode, seconds=time.monotonic() - start,
                      artifact=artifact.name, sha256=digest(data),
                      status='passed' if result.returncode == 0 else 'failed')
        save()
        if result.returncode:
            raise RuntimeError(f'{label} failed; see {artifact}')
        return log

    save()
    try:
        check('dependency-graph', ['cargo', 'metadata', '--offline', '--format-version', '1',
                                   '--filter-platform', output('rustc', '-vV').split('host: ')[1].splitlines()[0]])
        if packages(consumer / 'Cargo.lock') - packages(source / 'Cargo.lock'):
            raise RuntimeError('Third-party dependency drift')
        lock = (consumer / 'Cargo.lock').read_bytes()
        (evidence / 'consumer.lock').write_bytes(lock)
        manifest['lock_sha256'] = digest(lock)
        log = check('cold-release-build', ['/usr/bin/time', '-v', 'cargo', 'build', '--release',
                                          '--locked', '--offline', '--message-format=json'])
        artifacts = [json.loads(line) for line in log.read_text().splitlines() if line.startswith('{')]
        compiled = {a['package_id'] for a in artifacts if a.get('reason') == 'compiler-artifact' and not a['fresh']}
        manifest['compiled_packages'] = len(compiled)
        binary = consumer / 'target/release/statechronicle-cost-consumer'
        manifest['binary'] = {'bytes': binary.stat().st_size, 'sha256': digest(binary.read_bytes())}
        check('consumer-run', [str(binary)])
        manifest['status'] = 'passed'
    except (KeyboardInterrupt, SystemExit):
        manifest['status'] = 'interrupted'
        raise
    except Exception as error:
        manifest.update(status='failed', failure=str(error))
        raise
    finally:
        save()


if __name__ == '__main__':
    main()
