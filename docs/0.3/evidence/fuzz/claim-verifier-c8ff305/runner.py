#!/usr/bin/env python3
"""Instrumented canonical claim fuzz campaign on immutable archived source."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--candidate', default='HEAD')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--seconds', type=int, default=60)
    parser.add_argument('--runs', type=int, default=1000000)
    args = parser.parse_args()
    if args.seconds < 1 or args.runs < 100:
        parser.error('positive time and at least 100 runs required')
    root = Path.cwd()
    revision = subprocess.check_output(['git', 'rev-parse', args.candidate + '^{commit}'], text=True).strip()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    paths = ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'clippy.toml', 'crates', 'fuzz']
    archive = subprocess.check_output(['git', 'archive', revision, '--', *paths])
    sha = lambda data: hashlib.sha256(data).hexdigest()
    nightly = subprocess.check_output(['rustc', '+nightly', '-vV'], text=True)
    host = next(line.removeprefix('host: ') for line in nightly.splitlines() if line.startswith('host: '))
    env = dict(os.environ, RUSTC_WRAPPER='', CARGO_BUILD_JOBS='8', CARGO_NET_OFFLINE='true',
               CARGO_TARGET_DIR=str(root / 'target/claim-fuzz-campaign'))
    manifest = {'candidate_commit': revision, 'archive_paths': paths, 'archive_sha256': sha(archive),
                'nightly': nightly, 'status': 'running', 'checks': [], 'campaigns': [],
                'environment': {k: env[k] for k in ['RUSTC_WRAPPER', 'CARGO_BUILD_JOBS', 'CARGO_NET_OFFLINE', 'CARGO_TARGET_DIR']}}
    (output / 'runner.py').write_bytes(Path(__file__).read_bytes())
    def save():
        (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    def run(label, command, source, expected=0, diagnostic=None):
        start = time.monotonic()
        result = subprocess.run(command, cwd=source, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        data = gzip.compress(result.stdout, mtime=0)
        name = label + '.log.gz'
        (output / name).write_bytes(data)
        record = {'label': label, 'command': command, 'exit_code': result.returncode,
                  'expected_exit_code': expected, 'seconds': time.monotonic() - start,
                  'artifact': name, 'sha256': sha(data), 'expected_diagnostic': diagnostic}
        manifest['checks'].append(record)
        save()
        if result.returncode != expected or (diagnostic and diagnostic.encode() not in result.stdout):
            raise RuntimeError(label + ': unexpected exit/diagnostic')
        print(label, result.returncode, flush=True)
        return result.stdout.decode(errors='replace')
    save()
    try:
        with tempfile.TemporaryDirectory(prefix='statechronicle-claim-fuzz-') as temp:
            source = Path(temp) / 'source'
            source.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
                tar.extractall(source, filter='data')
            def inputs():
                return {str(p.relative_to(source)): sha(p.read_bytes()) for p in source.rglob('*') if p.is_file()}
            original_inputs = inputs()
            manifest['source_sha256'] = original_inputs
            seeds = source / 'fuzz/seed-corpus/claim-verifier'
            seed_hashes = {p.name: sha(p.read_bytes()) for p in seeds.iterdir()}
            if len(seed_hashes) != 18:
                raise RuntimeError('incomplete seed corpus')
            manifest['seed_sha256'] = seed_hashes
            build = ['cargo', '+nightly', 'fuzz', 'build', 'fuzz_claim_verifier', '--target', host]
            run('positive-build', build, source)
            binary = Path(env['CARGO_TARGET_DIR']) / host / 'release/fuzz_claim_verifier'
            manifest['binary_sha256'] = sha(binary.read_bytes())
            for seed in [7331, 42]:
                corpus = Path(temp) / ('corpus-' + str(seed))
                shutil.copytree(seeds, corpus)
                artifacts = Path(temp) / ('artifacts-' + str(seed))
                artifacts.mkdir()
                command = [str(binary), str(corpus), '-seed=' + str(seed), '-runs=' + str(args.runs),
                           '-max_total_time=' + str(args.seconds), '-max_len=64', '-artifact_prefix=' + str(artifacts) + '/']
                log = run('positive-' + str(seed), command, source)
                counts = re.findall(r'Done (\d+) runs in (\d+) second', log)
                if len(counts) != 1 or int(counts[0][0]) <= 100 or 'inline 8-bit counters' not in log:
                    raise RuntimeError('missing instrumented extended completion')
                name = 'corpus-final-' + str(seed) + '.tar.gz'
                with tarfile.open(output / name, 'w:gz') as tar:
                    tar.add(corpus, arcname='corpus')
                manifest['campaigns'].append({'seed': seed, 'runs_budget': args.runs, 'seconds_budget': args.seconds,
                    'completed_runs': int(counts[0][0]), 'reported_seconds': int(counts[0][1]), 'final_corpus': name,
                    'final_corpus_sha256': sha((output / name).read_bytes()), 'crash_artifacts': len(list(artifacts.iterdir()))})
                if list(artifacts.iterdir()):
                    raise RuntimeError('unexpected positive crash artifact')
                save()
            if inputs() != original_inputs:
                raise RuntimeError('positive source/lock drift')
            manifest['positive_source_unchanged'] = True
            # Compile and execute two actual insecure-source controls, never the live checkout.
            claim = source / 'crates/statechronicle-proof/src/claim.rs'
            original = claim.read_text()
            controls = [
                ('head', 'ensure_canonical(self.reader, expected.tenant(), &signed).await?;',
                 'let _ = &signed;', 13, 'claim.current.head_advance'),
                ('pin', 'if &proof.commit().commit_id != pinned_commit {',
                 'if false && &proof.commit().commit_id != pinned_commit {', 7, 'claim.historical.pin'),
            ]
            for label, before, after, choice, diagnostic in controls:
                if original.count(before) != 1:
                    raise RuntimeError('negative mutation anchor changed')
                claim.write_text(original.replace(before, after))
                manifest[label + '_mutated_source_sha256'] = sha(claim.read_bytes())
                run('negative-' + label + '-build', build, source)
                probe = Path(temp) / ('probe-' + label)
                probe.write_bytes(bytes([choice]))
                run('negative-' + label, [str(binary), str(probe), '-runs=1', '-seed=7331'], source,
                    expected=77, diagnostic=diagnostic)
                claim.write_text(original)
                if inputs() != original_inputs:
                    raise RuntimeError('negative restoration drift')
            manifest['source_restored'] = True
            manifest['status'] = 'passed'
    except BaseException as error:
        manifest['status'] = 'failed_or_interrupted'
        manifest['failure'] = repr(error)
        raise
    finally:
        manifest['artifacts_sha256'] = {p.name: sha(p.read_bytes()) for p in output.iterdir() if p.is_file() and p.name != 'manifest.json'}
        save()


if __name__ == '__main__':
    main()
