#!/usr/bin/env python3
"""Qualify committed standalone tutorials outside the repository (local only)."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import time
import tomllib


def sha(data):
    return hashlib.sha256(data).hexdigest()


def registry_packages(lock):
    return {(item['name'], item['version'], item.get('source'), item.get('checksum'))
            for item in tomllib.loads(lock.read_text())['package'] if item.get('source')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--toolchain', default='stable')
    parser.add_argument('--tutorial', default='minimal-native-domain',
                        help='directory under tutorials/; no path separators')
    args = parser.parse_args()
    if not args.tutorial or '/' in args.tutorial or '\\' in args.tutorial or args.tutorial in {'.', '..'}:
        parser.error('tutorial must be one directory name')
    root = Path(__file__).resolve().parents[1]
    tutorial = root / 'tutorials' / args.tutorial
    if not (tutorial / 'Cargo.toml').is_file():
        parser.error('tutorial manifest not found')
    guarded = ['crates', 'fuzz', 'Cargo.toml', 'Cargo.lock', 'tutorials', 'scripts/check_tutorials.py']
    if subprocess.check_output(['git', 'status', '--porcelain', '--', *guarded], cwd=root):
        raise SystemExit('Commit source, tutorial and runner changes before qualification')
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
    stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) + '-' + str(os.getpid())
    evidence = root / 'docs/0.3/evidence/tutorials' / (args.tutorial + '-' + stamp)
    evidence.mkdir(parents=True)
    tracked = subprocess.check_output(['git', 'ls-files', '--', *guarded], cwd=root, text=True).splitlines()
    manifest = {'candidate_commit': revision, 'tutorial': args.tutorial,
                'runner_sha256': sha(Path(__file__).read_bytes()),
                'source_sha256': {name: sha((root / name).read_bytes()) for name in tracked},
                'checks': [], 'status': 'running',
                'limitations': ['Local tutorial qualification, not complete release qualification.',
                                'Development keys, clock, principal and empty reads are fixture composition.',
                                'Preparation and transition checks are not durable receipts or trusted history proofs.']}
    path = evidence / 'manifest.json'

    def save():
        path.write_text(json.dumps(manifest, indent=2) + '\n')

    overrides = {'RUSTC_WRAPPER': '', 'CARGO_BUILD_JOBS': '8',
                 'CARGO_TARGET_DIR': str(root / 'target/tutorials' / args.tutorial / args.toolchain)}
    env = dict(os.environ, **overrides)
    manifest['environment'] = overrides
    manifest['toolchain'] = subprocess.check_output(['rustc', '+' + args.toolchain, '-Vv'], text=True)
    save()
    try:
        with tempfile.TemporaryDirectory(prefix='statechronicle-tutorial-') as temporary:
            sandbox = Path(temporary)
            source = sandbox / 'source'
            source.mkdir()
            archive = subprocess.check_output(['git', 'archive', revision, 'Cargo.toml', 'Cargo.lock',
                                                'crates', 'fuzz', 'tutorials'], cwd=root)
            tarfile.open(fileobj=io.BytesIO(archive)).extractall(source, filter='data')
            application = sandbox / 'application'
            shutil.copytree(source / 'tutorials' / args.tutorial, application)
            cargo = application / 'Cargo.toml'
            original = cargo.read_text()
            expected = 'path = "../../crates/statechronicle"'
            if original.count(expected) != 1:
                raise RuntimeError('Expected exactly one public path dependency')
            cargo.write_text(original.replace(expected, 'path = "../source/crates/statechronicle"'))
            manifest['isolated_application_manifest_sha256'] = sha(cargo.read_bytes())
            original_lock = (application / 'Cargo.lock').read_bytes()
            manifest['tutorial_lock_sha256'] = sha(original_lock)
            manifest['isolation'] = {'application': str(application), 'source_archive': str(source),
                                     'only_manifest_edit': 'Relocate the umbrella dependency to sibling archived source'}
            checks = [
                ('metadata', ['metadata', '--format-version', '1', '--locked', '--offline']),
                ('tests', ['test', '--all-targets', '--all-features', '--locked', '--offline']),
                ('run', ['run', '--quiet', '--locked', '--offline']),
                ('clippy', ['clippy', '--all-targets', '--all-features', '--locked', '--offline', '--', '-D', 'warnings']),
                ('format', ['fmt', '--all', '--', '--check']),
            ]
            for label, command_args in checks:
                command = ['cargo', '+' + args.toolchain, *command_args]
                record = {'label': label, 'command': command, 'status': 'running'}
                manifest['checks'].append(record)
                save()
                start = time.monotonic()
                result = subprocess.run(command, cwd=application, env=env, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT)
                log = evidence / (label + '.log.gz')
                log.write_bytes(gzip.compress(result.stdout, mtime=0))
                record.update({'exit_code': result.returncode, 'seconds': time.monotonic() - start,
                               'log': log.name, 'log_sha256': sha(log.read_bytes()),
                               'status': 'passed' if result.returncode == 0 else 'failed'})
                save()
                print(args.toolchain, label, record['status'], flush=True)
                if result.returncode:
                    raise RuntimeError(label + ' failed; retained log: ' + str(log))
                if (application / 'Cargo.lock').read_bytes() != original_lock:
                    raise RuntimeError('Tutorial lockfile changed in isolated application')
                if label == 'metadata':
                    added = registry_packages(application / 'Cargo.lock') - registry_packages(source / 'Cargo.lock')
                    if added:
                        raise RuntimeError('Tutorial adds registry packages absent from source lockfile: ' + repr(sorted(added)))
                if label == 'run':
                    expected_output = (application / 'expected-output.txt').read_bytes()
                    if result.stdout != expected_output:
                        raise RuntimeError('Tutorial output differs from expected-output.txt')
            # Compare archived production files before cleanup; checks may not alter sources.
            for name in tracked:
                if name.startswith(('crates/', 'fuzz/')) or name in {'Cargo.toml', 'Cargo.lock'}:
                    if sha((source / name).read_bytes()) != manifest['source_sha256'][name]:
                        raise RuntimeError('Archived source changed: ' + name)
        if subprocess.check_output(['git', 'diff', revision, '--', *guarded], cwd=root):
            raise RuntimeError('Source changed during qualification')
        manifest['status'] = 'passed'
    except BaseException as error:
        manifest['status'] = 'failed'
        manifest['error'] = str(error)
        raise
    finally:
        save()
        print('Evidence:', path.relative_to(root), flush=True)


if __name__ == '__main__':
    main()
