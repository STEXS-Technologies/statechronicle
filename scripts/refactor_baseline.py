#!/usr/bin/env python3
"""Run local checks against the original release, retaining exact evidence.

Never fetches, pushes, dispatches workflows, or regenerates reference fixtures.
Tracked-source byte counts are NOT production-only LOC/complexity metrics.
"""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import time

BASELINE = 'c418e98b91720188158c531cd5c2c4b8ab71cbcf'


def output(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd).decode().strip()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--release-checks', action='store_true',
                        help='also run original release script (includes instrumented fuzz)')
    parser.add_argument('--fuzz-seconds', type=int, default=0,
                        help='run only the original parallel fuzz campaign for this many seconds per target')
    args = parser.parse_args()
    if args.fuzz_seconds < 0 or (args.release_checks and args.fuzz_seconds):
        parser.error('Choose one nonnegative fuzz duration or release checks')
    root = Path(__file__).resolve().parents[1]
    resolved = output('git', 'rev-parse', 'v0.2.0^{commit}', cwd=root)
    if resolved != BASELINE:
        raise SystemExit('Original release tag does not match pinned baseline')
    archive = subprocess.check_output(['git', 'archive', BASELINE], cwd=root)
    stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())
    run = root / 'target' / 'refactor-baseline' / f'{stamp}-{os.getpid()}'
    source = run / 'source'
    source.mkdir(parents=True)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        tar.extractall(source, filter='data')
    # git diff --check in the original release script needs a repository.
    subprocess.run(['git', 'init', '-q', str(source)], check=True)
    subprocess.run(['git', 'add', '.'], cwd=source, check=True)
    evidence = root / 'docs' / '0.3' / 'evidence' / 'baseline' / run.name
    evidence.mkdir(parents=True)
    files = sorted(p for p in source.rglob('*') if p.is_file() and '.git' not in p.parts)
    hashes = {str(p.relative_to(source)): digest(p.read_bytes()) for p in files}
    rust = {name: (source / name).stat().st_size for name in hashes
            if name.startswith('crates/') and '/src/' in name and name.endswith('.rs')}
    manifest = {
        'baseline_commit': BASELINE,
        'candidate_commit': output('git', 'rev-parse', 'HEAD', cwd=root),
        'archive_sha256': digest(archive),
        'runner_sha256': digest(Path(__file__).read_bytes()),
        'toolchain': {'rustc': output('rustc', '-Vv'), 'cargo': output('cargo', '-V')},
        'environment': {'RUSTC_WRAPPER': os.environ.get('RUSTC_WRAPPER', ''),
                        'CARGO_BUILD_JOBS': os.environ.get('CARGO_BUILD_JOBS', '')},
        'platform': platform.platform(),
        'cpu': platform.processor(),
        'source_sha256': hashes,
        'tracked_rust_source': {'files': len(rust), 'bytes': sum(rust.values()), 'by_file': rust},
        'limitations': [
            'Source counts include tests and comments; not production-only metrics.',
            'No performance, coverage, API inventory or independent consumer qualification implied.',
            'Uses installed stable toolchain and original frozen lockfile.',
            'No original goldens are regenerated; no candidate behavior is tested here.',
        ],
        'checks': [], 'status': 'running',
    }
    path = evidence / 'manifest.json'

    def save():
        path.write_text(json.dumps(manifest, indent=2) + '\n')

    save()
    commands = [
        ['cargo', 'fmt', '--all', '--', '--check'],
        ['cargo', 'test', '--workspace', '--exclude', 'statechronicle-fuzz',
         '--all-targets', '--all-features', '--locked'],
    ]
    if args.release_checks:
        commands = [['bash', 'scripts/run_release_checks.sh']]
    environment = os.environ.copy()
    if args.fuzz_seconds:
        commands = [['bash', 'scripts/run_fuzz_parallel.sh']]
        environment['STATECHRONICLE_FUZZ_SECONDS'] = str(args.fuzz_seconds)
        environment['STATECHRONICLE_FUZZ_LOGS'] = str(source / 'fuzz-logs')
        manifest['fuzz_seconds_per_target'] = args.fuzz_seconds
        manifest['nightly_toolchain'] = output('rustc', '+nightly', '-Vv')
    try:
        for index, command in enumerate(commands):
            log = run / f'check-{index}.log'
            record = {'command': command, 'cwd': str(source), 'status': 'running'}
            manifest['checks'].append(record)
            save()
            print('Running:', ' '.join(command), flush=True)
            start = time.monotonic()
            try:
                with log.open('wb') as handle:
                    result = subprocess.run(command, cwd=source, stdout=handle,
                                            stderr=subprocess.STDOUT, env=environment)
                record.update(exit_code=result.returncode,
                              status='passed' if result.returncode == 0 else 'failed')
            finally:
                compressed = gzip.compress(log.read_bytes(), mtime=0)
                artifact = evidence / f'check-{index}.log.gz'
                artifact.write_bytes(compressed)
                record.update(seconds=time.monotonic() - start,
                              artifact=artifact.name, sha256=digest(compressed))
                if record['status'] == 'running':
                    record['status'] = 'interrupted'
                save()
            print(f'{record["status"]}: {artifact}', flush=True)
        manifest['status'] = ('passed' if all(c['exit_code'] == 0 for c in manifest['checks'])
                              else 'failed')
    except BaseException:
        manifest['status'] = 'interrupted'
        raise
    finally:
        changed = [name for name, expected in hashes.items()
                   if not (source / name).exists() or digest((source / name).read_bytes()) != expected]
        manifest['modified_original_files'] = changed
        if changed:
            manifest['status'] = 'failed'
        if (source / 'fuzz-logs').exists():
            artifact = evidence / 'fuzz-campaign.tar.gz'
            with tarfile.open(artifact, 'w:gz') as tar:
                tar.add(source / 'fuzz-logs', arcname='logs')
                if (source / 'fuzz/corpus').exists():
                    tar.add(source / 'fuzz/corpus', arcname='corpus')
            manifest['fuzz_artifact'] = {'path': artifact.name,
                                         'sha256': digest(artifact.read_bytes())}
        save()
    raise SystemExit(0 if manifest['status'] == 'passed' else 1)


if __name__ == '__main__':
    main()
