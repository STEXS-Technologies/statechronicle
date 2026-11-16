#!/usr/bin/env python3
"""Exercise frozen external consumer against original and candidate revisions."""
import argparse
import gzip
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time
import tomllib

from refactor_baseline import BASELINE, digest, output


def packages(path):
    return {(p['name'], p['version'], p.get('source'), p.get('checksum'))
            for p in tomllib.loads(path.read_text())['package'] if p.get('source')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lane', choices=['baseline', 'candidate', 'both'], default='both')
    parser.add_argument('--portable', action='store_true', help='also qualify external no-std consumer')
    parser.add_argument('--produce-wire', action='store_true',
                        help='produce initial wire corpus using original release only')
    args = parser.parse_args()
    if args.produce_wire and args.lane == 'candidate':
        parser.error('Wire production requires the baseline lane')
    lanes = [('baseline', BASELINE)] if args.lane == 'baseline' else []
    root = Path(__file__).resolve().parents[1]
    fixture = root / 'ci' / 'compat-0.2'
    wire = root / 'ci' / 'wire-0.2' / 'history.json'
    if args.produce_wire and wire.exists():
        raise SystemExit('Refusing to overwrite frozen original wire corpus')
    provenance = json.loads((fixture / 'provenance.json').read_text())
    if provenance['baseline_commit'] != BASELINE:
        raise SystemExit('Frozen consumer provenance revision mismatch')
    for name, item in provenance['files'].items():
        original = subprocess.check_output(['git', 'show', f"{BASELINE}:{item['original_path']}"], cwd=root)
        if digest(original) != item['sha256'] or (fixture / name).read_bytes() != original:
            raise SystemExit(f'Frozen consumer changed: {name}')
    if output('git', 'rev-parse', 'v0.2.0^{commit}', cwd=root) != BASELINE:
        raise SystemExit('Original release tag mismatch')
    dirty = output('git', 'status', '--porcelain', '--', 'crates', 'Cargo.toml',
                   'Cargo.lock', cwd=root)
    if dirty:
        raise SystemExit('Commit production changes before candidate qualification')
    candidate = output('git', 'rev-parse', 'HEAD', cwd=root)
    if args.lane == 'both':
        lanes.append(('baseline', BASELINE))
    if args.lane in {'both', 'candidate'}:
        lanes.append(('candidate', candidate))
    stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) + f'-{os.getpid()}'
    run = root / 'target' / 'refactor-compat' / stamp
    evidence = root / 'docs' / '0.3' / 'evidence' / 'compat' / stamp
    evidence.mkdir(parents=True)
    manifest = {
        'baseline_commit': BASELINE, 'candidate_commit': candidate,
        'runner_sha256': digest(Path(__file__).read_bytes()),
        'fixture_sha256': {str(p.relative_to(fixture)): digest(p.read_bytes())
                           for p in sorted(fixture.rglob('*')) if p.is_file()},
        'toolchain': {'rustc': output('rustc', '-Vv'), 'cargo': output('cargo', '-V')},
        'environment': {'RUSTC_WRAPPER': os.environ.get('RUSTC_WRAPPER', ''),
                        'CARGO_BUILD_JOBS': os.environ.get('CARGO_BUILD_JOBS', '')},
        'limitations': ['Frozen released tests, not an independent semantic model.',
                        'No performance, fuzz, bare-metal or exhaustive API qualification.'],
        'checks': [], 'status': 'running',
    }
    path = evidence / 'manifest.json'

    def save():
        path.write_text(json.dumps(manifest, indent=2) + '\n')

    def check(command, cwd, label):
        record = {'label': label, 'command': command, 'status': 'running'}
        manifest['checks'].append(record)
        save()
        log = run / f'{label}.log'
        print('Running:', label, flush=True)
        start = time.monotonic()
        try:
            with log.open('wb') as handle:
                result = subprocess.run(command, cwd=cwd, stdout=handle,
                                        stderr=subprocess.STDOUT)
            record.update(exit_code=result.returncode,
                          status='passed' if result.returncode == 0 else 'failed')
        finally:
            data = gzip.compress(log.read_bytes(), mtime=0)
            artifact = evidence / f'{label}.log.gz'
            artifact.write_bytes(data)
            record.update(seconds=time.monotonic() - start, artifact=artifact.name,
                          sha256=digest(data))
            if record['status'] == 'running':
                record['status'] = 'interrupted'
            save()
        print(f'{label}: {record["status"]}', flush=True)
        return record['exit_code'] == 0

    save()
    try:
        for label, revision in lanes:
            source = run / label / 'source'
            source.mkdir(parents=True)
            archive = subprocess.check_output(['git', 'archive', revision], cwd=root)
            with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
                tar.extractall(source, filter='data')
            consumer_root = run / label / 'consumer'
            shutil.copytree(fixture, consumer_root)
            consumer = consumer_root / 'crates' / 'consumer'
            cargo = consumer / 'Cargo.toml'
            cargo.write_text(cargo.read_text().replace('../../../../crates/statechronicle',
                                                       str(source / 'crates/statechronicle')))
            shutil.copyfile(source / 'Cargo.lock', consumer / 'Cargo.lock')
            if not check(['cargo', 'metadata', '--offline', '--format-version', '1',
                          '--all-features', '--filter-platform',
                          output('rustc', '-vV').split('host: ')[1].splitlines()[0]], consumer, f'{label}-resolve'):
                continue
            added = packages(consumer / 'Cargo.lock') - packages(source / 'Cargo.lock')
            if added:
                raise RuntimeError(f'Unapproved third-party dependency drift: {sorted(added)}')
            lock = (consumer / 'Cargo.lock').read_bytes()
            (evidence / f'{label}.lock').write_bytes(lock)
            manifest[f'{label}_lock_sha256'] = digest(lock)
            if args.produce_wire and label == 'baseline':
                destination = evidence / 'history.json'
                if not check(['cargo', 'run', '--example', 'wire_producer', '--locked',
                              '--offline', '--', str(destination)], consumer, 'wire-producer'):
                    continue
                wire.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(destination, wire)
                origin = {'producer_commit': BASELINE,
                          'producer_sha256': digest((consumer / 'examples/wire_producer.rs').read_bytes()),
                          'history_sha256': digest(wire.read_bytes()),
                          'evidence_manifest': str(path.relative_to(root))}
                (wire.parent / 'provenance.json').write_text(json.dumps(origin, indent=2) + '\n')
            origin = json.loads((wire.parent / 'provenance.json').read_text())
            if digest(wire.read_bytes()) != origin['history_sha256']:
                raise RuntimeError('Original wire corpus hash mismatch')
            (consumer_root / 'wire').mkdir()
            shutil.copyfile(wire, consumer_root / 'wire' / 'history.json')
            manifest['wire_sha256'] = origin['history_sha256']
            check(['cargo', 'test', '--all-targets', '--all-features', '--locked',
                   '--offline'], consumer, f'{label}-tests')
            if args.portable:
                portable = consumer_root / 'crates' / 'portable'
                cargo = portable / 'Cargo.toml'
                cargo.write_text(cargo.read_text().replace('../../../../crates/',
                                                           str(source / 'crates') + '/'))
                shutil.copyfile(source / 'Cargo.lock', portable / 'Cargo.lock')
                if not check(['cargo', 'metadata', '--offline', '--format-version', '1',
                              '--filter-platform', 'riscv32imafc-unknown-none-elf'],
                             portable, f'{label}-portable-resolve'):
                    continue
                if packages(portable / 'Cargo.lock') - packages(source / 'Cargo.lock'):
                    raise RuntimeError('Portable consumer dependency drift')
                portable_lock = (portable / 'Cargo.lock').read_bytes()
                (evidence / f'{label}-portable.lock').write_bytes(portable_lock)
                manifest[f'{label}_portable_lock_sha256'] = digest(portable_lock)
                check(['cargo', 'test', '--locked', '--offline'], portable,
                      f'{label}-portable-tests')
                check(['cargo', 'check', '--target', 'riscv32imafc-unknown-none-elf',
                       '--locked', '--offline'], portable, f'{label}-bare-metal')
        manifest['status'] = ('passed' if len(manifest['checks']) == (5 if args.portable else 2) * len(lanes) + int(args.produce_wire) and
                              all(c['status'] == 'passed' for c in manifest['checks'])
                              else 'failed')
    except (KeyboardInterrupt, SystemExit):
        manifest['status'] = 'interrupted'
        raise
    except Exception as error:
        manifest.update(status='failed', failure=str(error))
        raise
    finally:
        save()
    raise SystemExit(0 if manifest['status'] == 'passed' else 1)


if __name__ == '__main__':
    main()
