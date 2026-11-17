#!/usr/bin/env python3
"""Collect immutable-revision compiler API evidence, without running GitHub CI."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib

from rustdoc_api_inventory import inventory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('revision')
    parser.add_argument('output', type=Path)
    parser.add_argument('--toolchain', default='nightly')
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    revision = subprocess.check_output(['git', 'rev-parse', args.revision + '^{commit}'], text=True).strip()
    archive = subprocess.check_output(['git', 'archive', revision])
    compiler = subprocess.check_output(['rustc', '+' + args.toolchain, '-vV'], text=True)
    manifest = {'revision': revision, 'archive_sha256': hashlib.sha256(archive).hexdigest(),
                'compiler': compiler, 'status': 'running', 'packages': []}
    def save():
        (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    save()
    for script in ['collect_rustdoc_api.py', 'rustdoc_api_inventory.py']:
        (output / script).write_bytes(Path(__file__).with_name(script).read_bytes())
    try:
        with tempfile.TemporaryDirectory(prefix='statechronicle-api-') as temp:
            root = Path(temp)
            with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
                tar.extractall(root, filter='data')
            workspace = tomllib.loads((root / 'Cargo.toml').read_text())
            def inputs():
                return {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
                        for p in root.rglob('*') if p.is_file()
                        and 'target' not in p.relative_to(root).parts}
            before = inputs()
            env = dict(os.environ, RUSTC_WRAPPER='', CARGO_BUILD_JOBS='8',
                       CARGO_TARGET_DIR=str(root / 'target'))
            for member in workspace['workspace']['members']:
                package = tomllib.loads((root / member / 'Cargo.toml').read_text())['package']['name']
                if package == 'statechronicle-fuzz':
                    continue
                command = ['cargo', '+' + args.toolchain, 'rustdoc', '-p', package, '--lib',
                           '--all-features', '--locked', '--offline', '--', '-Z',
                           'unstable-options', '--output-format', 'json', '--document-hidden-items']
                log = output / (package + '.log.gz')
                result = subprocess.run(command, cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                log.write_bytes(gzip.compress(result.stdout, mtime=0))
                record = {'package': package, 'command': command, 'exit': result.returncode}
                manifest['packages'].append(record)
                save()
                if result.returncode:
                    raise RuntimeError(f'{package} rustdoc failed: {result.returncode}')
                raw = (root / 'target/doc' / (package.replace('-', '_') + '.json')).read_bytes()
                normalized = json.dumps(inventory(json.loads(raw)), sort_keys=True, indent=2).encode() + b'\n'
                for suffix, data in [('rustdoc.json.gz', raw), ('api.json.gz', normalized)]:
                    name = package + '.' + suffix
                    (output / name).write_bytes(gzip.compress(data, mtime=0))
                    record[suffix] = {'file': name, 'uncompressed_sha256': hashlib.sha256(data).hexdigest()}
                record['exports'] = len(json.loads(normalized)['exports'])
                print(package, record['exports'], flush=True)
                save()
            after = inputs()
            manifest['source_inputs'] = before
            manifest['source_unchanged'] = before == after
            if before != after:
                raise RuntimeError('archive inputs changed during collection')
            manifest['status'] = 'passed'
    except Exception as error:
        manifest['status'] = 'failed'
        manifest['error'] = str(error)
        raise
    finally:
        save()


if __name__ == '__main__':
    main()
