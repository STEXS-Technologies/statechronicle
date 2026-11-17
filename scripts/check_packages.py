#!/usr/bin/env python3
"""Build and test all publishable crate archives outside the source checkout (no publish)."""
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import time
import tomllib


def sha(data):
    return hashlib.sha256(data).hexdigest()


def registry_packages(path):
    return {(p['name'], p['version'], p.get('source'), p.get('checksum'))
            for p in tomllib.loads(path.read_text())['package'] if p.get('source')}


def main():
    root = Path(__file__).resolve().parents[1]
    guarded = ['Cargo.toml', 'Cargo.lock', 'clippy.toml', 'crates', 'fuzz', 'scripts/check_packages.py']
    if subprocess.check_output(['git', 'status', '--porcelain', '--', *guarded], cwd=root):
        raise SystemExit('Commit library/package-checker changes before qualification')
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
    fixture_root = root / 'crates/statechronicle/tests/fixtures'
    fixtures = json.loads((fixture_root / 'provenance.json').read_text())['files']
    for name, record in fixtures.items():
        packaged = (fixture_root / name).read_bytes()
        original = (root / record['source_path']).read_bytes()
        pinned = subprocess.check_output(['git', 'show', record['source_commit'] + ':' + record['source_path']], cwd=root)
        if packaged != original or packaged != pinned or sha(packaged) != record['sha256']:
            raise SystemExit('Packaged reference differs from pinned original: ' + name)
    stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) + '-' + str(os.getpid())
    evidence = root / 'docs/0.3/evidence/packages' / stamp
    evidence.mkdir(parents=True)
    tracked = subprocess.check_output(['git', 'ls-files', '--', *guarded], cwd=root, text=True).splitlines()
    manifest = {'candidate_commit': revision, 'source_sha256': {p: sha((root / p).read_bytes()) for p in tracked},
                'toolchain': subprocess.check_output(['rustc', '-Vv'], text=True),
                'fixtures': fixtures, 'checks': [], 'archives': {}, 'status': 'running',
                'limitations': ['Local archives with internal dependencies patched only to other unpacked archives.',
                                'No crates.io publication or fresh registry-service qualification.',
                                'Not complete release, benchmark, coverage or crash qualification.']}
    path = evidence / 'manifest.json'

    def save():
        path.write_text(json.dumps(manifest, indent=2) + '\n')

    overrides = {'RUSTC_WRAPPER': '', 'CARGO_BUILD_JOBS': '8'}
    env = dict(os.environ, **overrides)
    manifest['environment'] = overrides

    def check(label, args, cwd):
        command = ['cargo', *args]
        start = time.monotonic()
        result = subprocess.run(command, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        log = evidence / (label + '.log.gz')
        log.write_bytes(gzip.compress(result.stdout, mtime=0))
        manifest['checks'].append({'label': label, 'command': command, 'exit_code': result.returncode,
                                   'seconds': time.monotonic() - start, 'log': log.name,
                                   'log_sha256': sha(log.read_bytes())})
        save()
        print(label, result.returncode, flush=True)
        if result.returncode:
            raise RuntimeError(label + ' failed; retained ' + str(log))
        return result.stdout

    save()
    try:
        with tempfile.TemporaryDirectory(prefix='statechronicle-packages-') as temporary:
            sandbox = Path(temporary)
            # Archive mtimes can predate cached compilation of another revision.
            # Qualify the actual unpacked bytes with a fresh build directory.
            overrides['CARGO_TARGET_DIR'] = str(sandbox / 'target')
            env.update(overrides)
            manifest['build_isolation'] = 'fresh per-archive target directory'
            save()
            source = sandbox / 'source'
            source.mkdir()
            archive = subprocess.check_output(['git', 'archive', revision, 'Cargo.toml', 'Cargo.lock',
                                               'rust-toolchain.toml', 'clippy.toml', 'crates', 'fuzz',
                                               'LICENSE-APACHE', 'LICENSE-MIT'], cwd=root)
            tarfile.open(fileobj=io.BytesIO(archive)).extractall(source, filter='data')
            metadata = json.loads(check('metadata', ['metadata', '--no-deps', '--format-version', '1',
                                                     '--locked', '--offline'], source))
            packages = [p for p in metadata['packages'] if p['id'] in metadata['workspace_members'] and p['publish'] != []]
            assert len(packages) == 13, 'Expected the existing thirteen publishable library crates'
            check('package', ['package', '--workspace', '--exclude', 'statechronicle-fuzz',
                              '--locked', '--offline', '--no-verify'], source)
            unpacked = sandbox / 'unpacked'
            unpacked.mkdir()
            members = []
            patches = []
            for package in sorted(packages, key=lambda p: p['name']):
                name = package['name'] + '-' + package['version']
                crate = Path(overrides['CARGO_TARGET_DIR']) / 'package' / (name + '.crate')
                data = crate.read_bytes()
                # Keep actual archives, not merely lists or source checkout copies.
                (evidence / crate.name).write_bytes(data)
                manifest['archives'][crate.name] = sha(data)
                tarfile.open(fileobj=io.BytesIO(data)).extractall(unpacked, filter='data')
                members.append(name)
                patches.append(json.dumps(package['name']) + ' = { path = ' + json.dumps(name) + ' }')
            (unpacked / 'Cargo.toml').write_text('[workspace]\nresolver = "3"\nmembers = ' + json.dumps(members)
                                                + '\n\n[patch.crates-io]\n' + '\n'.join(patches) + '\n')
            # Preserve the existing lint policy (not Clippy's different defaults).
            (unpacked / 'clippy.toml').write_bytes((source / 'clippy.toml').read_bytes())
            manifest['clippy_configuration_sha256'] = sha((unpacked / 'clippy.toml').read_bytes())
            manifest['isolated_workspace_manifest'] = (unpacked / 'Cargo.toml').read_text()
            manifest['isolation'] = {'source': str(source), 'unpacked': str(unpacked),
                                     'dependency_rule': 'All StateChronicle dependencies resolve to unpacked .crate content; no source paths'}
            # Resolve the unpacked paths from the candidate's locked graph.
            # generate-lockfile would select newer cached registry versions,
            # making qualification depend on unrelated local cache updates.
            (unpacked / 'Cargo.lock').write_bytes((source / 'Cargo.lock').read_bytes())
            check('resolve', ['metadata', '--format-version', '1', '--offline'], unpacked)
            lock = (unpacked / 'Cargo.lock').read_bytes()
            (evidence / 'unpacked.lock').write_bytes(lock)
            manifest['unpacked_lock_sha256'] = sha(lock)
            added = registry_packages(unpacked / 'Cargo.lock') - registry_packages(source / 'Cargo.lock')
            if added:
                raise RuntimeError('Unpacked graph adds registry packages: ' + repr(sorted(added)))
            resolved = json.loads(check('unpacked-metadata', ['metadata', '--format-version', '1', '--locked', '--offline'], unpacked))
            for package in resolved['packages']:
                if package['name'].startswith('statechronicle'):
                    Path(package['manifest_path']).resolve().relative_to(unpacked.resolve())
            check('tests', ['test', '--workspace', '--all-targets', '--all-features', '--locked', '--offline'], unpacked)
            check('doctests', ['test', '--workspace', '--doc', '--all-features', '--locked', '--offline'], unpacked)
            check('clippy', ['clippy', '--workspace', '--all-targets', '--all-features', '--locked', '--offline', '--', '-D', 'warnings'], unpacked)
            if (unpacked / 'Cargo.lock').read_bytes() != lock:
                raise RuntimeError('Unpacked lock changed during checks')
            for name in tracked:
                if name.startswith(('crates/', 'fuzz/')) or name in {'Cargo.toml', 'Cargo.lock', 'clippy.toml'}:
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
