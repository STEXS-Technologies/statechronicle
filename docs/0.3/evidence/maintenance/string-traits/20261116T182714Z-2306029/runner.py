#!/usr/bin/env python3
"""Compare frozen external string-newtype conversion, formatting and wire traces."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import time
import tomllib

import sys
sys.path.insert(0, str(Path.cwd() / "scripts"))
from refactor_compat import packages


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--revision', action='append', required=True)
    cli.add_argument('--negative-control-unchecked-deserialization', action='store_true')
    args = cli.parse_args()
    require(len(args.revision) >= 2, 'requires at least two revisions')
    root = Path.cwd()
    revisions = [subprocess.check_output(['git', 'rev-parse', ref], cwd=root, text=True).strip() for ref in args.revision]
    stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) + '-' + str(os.getpid())
    out = root / 'docs/0.3/evidence/maintenance/string-traits' / stamp
    out.mkdir(parents=True)
    run = root / 'target/string-traits-parity' / stamp
    fixture = (root / 'ci/characterization/string_traits.rs').read_bytes()
    def sha(data):
        return hashlib.sha256(data).hexdigest()
    env = dict(os.environ, RUSTC_WRAPPER='', CARGO_BUILD_JOBS='8', CARGO_TARGET_DIR=str(run / 'build'))
    manifest = {'revisions': revisions, 'fixture_sha256': sha(fixture), 'negative_control': args.negative_control_unchecked_deserialization, 'python_optimization': __import__('sys').flags.optimize, 'rustc': subprocess.check_output(['rustc', '-vV'], text=True), 'environment': {key: env[key] for key in ['RUSTC_WRAPPER', 'CARGO_BUILD_JOBS', 'CARGO_TARGET_DIR']}, 'checks': [], 'lanes': [], 'status': 'running'}
    (out / 'fixture.rs').write_bytes(fixture)
    (out / 'runner.py').write_bytes(Path(__file__).read_bytes())
    def save():
        (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    def check(label, command, cwd):
        start = time.monotonic()
        result = subprocess.run(command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=600)
        artifact = label + '.log.gz'
        # Cargo's progress diagnostics (including cache-lock waits) belong to
        # stderr, not the metadata JSON consumed by the caller. Retain both in
        # the log without feeding diagnostics into the machine-readable output.
        data = gzip.compress(result.stdout + result.stderr, mtime=0)
        (out / artifact).write_bytes(data)
        manifest['checks'].append({'label': label, 'command': command, 'exit_code': result.returncode, 'seconds': time.monotonic()-start, 'artifact': artifact, 'sha256': sha(data)})
        save()
        print(label, result.returncode, flush=True)
        require(result.returncode == 0, label + ' failed')
        return result.stdout
    names = ['StateId', 'IntentId', 'EventId', 'CommitId', 'SnapshotId', 'ProfileId', 'Operation', 'KeyId', 'Status']
    expected = {f'{name}:{kind}:{index}' for name in names for kind, count in [('string', 1128), ('json', 10), ('bcs', 7)] for index in range(count)}
    traces = []
    save()
    try:
        for index, revision in enumerate(revisions):
            source = run / str(index) / 'source'
            source.mkdir(parents=True)
            archive = subprocess.check_output(['git', 'archive', revision, '--', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'clippy.toml', 'crates', 'fuzz'], cwd=root)
            with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
                tar.extractall(source, filter='data')
            original_hashes = {str(p.relative_to(source)): sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()}
            if args.negative_control_unchecked_deserialization and index == len(revisions)-1:
                owner = source / 'crates/statechronicle-domain/src/validated_string.rs'
                text = owner.read_text()
                needle = 'Self::new(value).map_err(::serde::de::Error::custom)'
                require(text.count(needle) == 1, 'negative control anchor drift')
                owner.write_text(text.replace(needle, 'Ok(Self(value))'))
                (out / 'negative-control.json').write_text(json.dumps({'revision': revision, 'path': str(owner.relative_to(source)), 'original_sha256': sha(text.encode()), 'mutated_sha256': sha(owner.read_bytes()), 'removed': needle, 'replacement': 'Ok(Self(value))'}, indent=2) + '\n')
            source_hashes = {str(p.relative_to(source)): sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()}
            consumer = source.parent / 'consumer'
            (consumer / 'src').mkdir(parents=True)
            (consumer / 'src/main.rs').write_bytes(fixture)
            deps = tomllib.loads((source / 'Cargo.toml').read_text())['workspace']['dependencies']
            version = lambda value: value['version'] if isinstance(value, dict) else value
            (consumer / 'Cargo.toml').write_text('[package]\nname="string-traits-characterization"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\nstatechronicle={path="../source/crates/statechronicle"}\nserde_json="' + version(deps['serde_json']) + '"\nbcs="' + version(deps['bcs']) + '"\n')
            (consumer / 'Cargo.lock').write_bytes((source / 'Cargo.lock').read_bytes())
            metadata = json.loads(check(f'{index}-resolve', ['cargo', 'metadata', '--offline', '--format-version', '1'], consumer))
            require(not packages(consumer / 'Cargo.lock') - packages(source / 'Cargo.lock'), 'third-party dependency drift')
            lock = (consumer / 'Cargo.lock').read_bytes()
            (out / f'{index}.lock').write_bytes(lock)
            check(f'{index}-build', ['cargo', 'build', '--release', '--locked', '--offline'], consumer)
            check(f'{index}-clippy', ['cargo', 'clippy', '--locked', '--offline', '--', '-D', 'warnings'], consumer)
            binary = Path(env['CARGO_TARGET_DIR']) / 'release/string-traits-characterization'
            frozen = source.parent / 'binary'
            frozen.write_bytes(binary.read_bytes())
            frozen.chmod(0o755)
            raw = check(f'{index}-trace', [str(frozen)], consumer)
            trace = json.loads(raw)
            require(len(trace) == len(expected) and {row['case'] for row in trace} == expected, 'incomplete or duplicate corpus')
            traces.append(trace)
            section = check(f'{index}-sections', ['size', '-A', str(frozen)], consumer).decode()
            text_size = next(int(line.split()[1]) for line in section.splitlines() if line.startswith('.text '))
            by_id = {p['id']: p for p in metadata['packages']}
            features = sorted((by_id[n['id']]['name'], by_id[n['id']]['version'], sorted(n['features'])) for n in metadata['resolve']['nodes'] if by_id[n['id']]['source'] is not None)
            manifest['lanes'].append({'revision': revision, 'archive_sha256': sha(archive), 'original_source_sha256': original_hashes, 'source_sha256': source_hashes, 'lock_sha256': sha(lock), 'registry_features': features, 'rows': len(trace), 'trace_sha256': sha(raw), 'binary_sha256': sha(frozen.read_bytes()), 'binary_bytes': frozen.stat().st_size, 'text_bytes': text_size})
            require((consumer / 'Cargo.lock').read_bytes() == lock, 'consumer lock drift')
            require(all(sha((source/name).read_bytes()) == digest for name, digest in source_hashes.items()), 'archived source drift')
            save()
        differences = [{'lane': index, 'case': actual['case'], 'expected': expected_row, 'actual': actual} for index, trace in enumerate(traces[1:], 1) for expected_row, actual in zip(traces[0], trace) if actual != expected_row]
        manifest['different_rows'] = len(differences)
        if differences:
            (out / 'differences.json.gz').write_bytes(gzip.compress(json.dumps(differences, separators=(',', ':')).encode(), mtime=0))
        require(not differences, 'conversion/format/wire trace mismatch')
        manifest['status'] = 'passed'
    except BaseException as error:
        manifest['status'] = 'failed_or_interrupted'
        manifest['failure'] = repr(error)
        raise
    finally:
        manifest['artifacts_sha256'] = {p.name: sha(p.read_bytes()) for p in sorted(out.iterdir()) if p.name != 'manifest.json'}
        save()
    print(out.relative_to(root), flush=True)


if __name__ == '__main__':
    main()
