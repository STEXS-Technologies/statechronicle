#!/usr/bin/env python3
"""Recheck retained compiler evidence against Git archives and raw rustdoc JSON."""
import gzip
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parent


def require(condition, message):
    if not condition:
        raise ValueError(message)


def main():
    catalogs = []
    compilers = []
    for lane in ['compiler-c418e98', 'compiler-64c5350']:
        directory = ROOT / lane
        manifest = json.loads((directory / 'manifest.json').read_text())
        require(manifest['status'] == 'passed' and manifest['source_unchanged'], lane + ': status')
        archive = subprocess.check_output(['git', 'archive', manifest['revision']])
        require(hashlib.sha256(archive).hexdigest() == manifest['archive_sha256'], lane + ': archive')
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            inputs = {p.name: hashlib.sha256(tar.extractfile(p).read()).hexdigest()
                      for p in tar.getmembers() if p.isfile() and 'target' not in Path(p.name).parts}
        require(inputs == manifest['source_inputs'], lane + ': input hashes')
        spec = importlib.util.spec_from_file_location('retained_inventory', directory / 'rustdoc_api_inventory.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        packages = {}
        for record in manifest['packages']:
            require(record['exit'] == 0, record['package'] + ': rustdoc exit')
            data = {}
            for suffix in ['rustdoc.json.gz', 'api.json.gz']:
                artifact = record[suffix]
                raw = gzip.decompress((directory / artifact['file']).read_bytes())
                require(hashlib.sha256(raw).hexdigest() == artifact['uncompressed_sha256'], artifact['file'] + ': hash')
                data[suffix] = json.loads(raw)
            normalized = module.inventory(data['rustdoc.json.gz'])
            require(normalized == data['api.json.gz'], record['package'] + ': regeneration')
            require(len(normalized['exports']) == record['exports'], record['package'] + ': count')
            packages[record['package']] = normalized['exports']
        require(len(packages) == 13, lane + ': package count')
        catalogs.append(packages)
        compilers.append(manifest['compiler'])
    require(compilers[0] == compilers[1], 'compiler mismatch')
    before, after = catalogs
    require(before.keys() == after.keys(), 'package mismatch')
    comparison = {}
    for package in before:
        a, b = before[package], after[package]
        comparison[package] = {'removed': sorted(a.keys() - b.keys()),
                               'added': sorted(b.keys() - a.keys()),
                               'changed': sorted(k for k in a.keys() & b.keys() if a[k] != b[k])}
    require(comparison == json.loads((ROOT / 'comparison.json').read_text()), 'comparison mismatch')
    print('Verified 26 raw/normalized compiler catalog pairs, immutable source inputs, compiler identity and export comparison.')


if __name__ == '__main__':
    main()
