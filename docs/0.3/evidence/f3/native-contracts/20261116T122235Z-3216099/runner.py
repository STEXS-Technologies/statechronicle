#!/usr/bin/env python3
"""Check external native API contracts against immutable committed source."""
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
import time


def sha(data):
    return hashlib.sha256(data).hexdigest()


def application_span(span):
    """Follow call-site expansion spans, never definition-site or child notes."""
    while span:
        if span['file_name'] == 'src/main.rs':
            return True
        span = (span.get('expansion') or {}).get('span')
    return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--revision', default='HEAD')
    parser.add_argument('--negative-control-valid-program', action='store_true',
                        help='Deliberately demand rejection of the valid program; must fail')
    args = parser.parse_args()
    root = Path(subprocess.check_output(['git', 'rev-parse', '--show-toplevel'], text=True).strip())
    revision = subprocess.check_output(['git', 'rev-parse', args.revision], cwd=root, text=True).strip()
    paths = ['Cargo.toml', 'Cargo.lock', 'crates', 'fuzz', 'ci/native-contracts']
    archive = subprocess.check_output(['git', 'archive', revision, '--', *paths], cwd=root)
    stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) + '-' + str(os.getpid())
    evidence = root / 'docs/0.3/evidence/f3/native-contracts' / stamp
    evidence.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, RUSTC_WRAPPER='', CARGO_BUILD_JOBS='8',
               CARGO_TARGET_DIR=str(root / 'target/native-contracts'))
    for key in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTDOCFLAGS']:
        env.pop(key, None)
    manifest = dict(candidate_commit=revision, archive_paths=paths, archive_sha256=sha(archive),
                    runner_sha256=sha(Path(__file__).read_bytes()), checks=[], status='running',
                    negative_control_valid_program=args.negative_control_valid_program,
                    environment={key: env[key] for key in ['RUSTC_WRAPPER', 'CARGO_BUILD_JOBS', 'CARGO_TARGET_DIR']},
                    rustc=subprocess.check_output(['rustc', '+1.100.0', '-vV'], text=True),
                    limitations=['Compiler diagnostics are checked for intended error code and source span; dynamic schema/byte checks run separately.',
                                 'No database, signing, durability, migration authorization or full release qualification is implied.'])
    (evidence / 'runner.py').write_bytes(Path(__file__).read_bytes())

    def save():
        (evidence / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

    def run(label, command, cwd, expected=0):
        start = time.monotonic()
        try:
            result = subprocess.run(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, timeout=180)
            data, code = result.stdout, result.returncode
        except subprocess.TimeoutExpired as error:
            data, code = error.stdout or b'', 'timeout'
        compressed = gzip.compress(data, mtime=0)
        artifact = label + '.log.gz'
        (evidence / artifact).write_bytes(compressed)
        manifest['checks'].append(dict(label=label, command=command, exit_code=code,
                                       expected_exit_code=expected, seconds=time.monotonic()-start,
                                       artifact=artifact, sha256=sha(compressed)))
        save()
        assert code == expected, (label, code, data.decode(errors='replace'))
        return data

    save()
    try:
        with tempfile.TemporaryDirectory(prefix='statechronicle-native-contracts-') as directory:
            directory = Path(directory)
            source = directory / 'source'
            source.mkdir()
            with tarfile.open(fileobj=io.BytesIO(archive)) as handle:
                handle.extractall(source, filter='data')
            hashes = {str(p.relative_to(source)): sha(p.read_bytes())
                      for p in sorted(source.rglob('*')) if p.is_file()}
            manifest['source_sha256'] = hashes
            consumer = directory / 'application'
            (consumer / 'src').mkdir(parents=True)
            (consumer / 'Cargo.toml').write_text('''[package]
name = "native-contract-consumer"
version = "0.0.0"
edition = "2024"
publish = false
[workspace]
[dependencies]
statechronicle = { path = "../source/crates/statechronicle" }
serde = { version = "1", features = ["derive"] }
ed25519-dalek = "3"
''')
            # Use the canonical example models, through the public umbrella only.
            domains = (source / 'crates/statechronicle/examples/common/domains.rs').read_bytes()
            (consumer / 'src/domains.rs').write_bytes(domains)
            positive = (source / 'ci/native-contracts/positive.rs').read_bytes()
            probe = consumer / 'src/main.rs'
            probe.write_bytes(positive)
            (consumer / 'Cargo.lock').write_bytes((source / 'Cargo.lock').read_bytes())
            json.loads(run('resolve', ['cargo', '+1.100.0', 'metadata', '--offline', '--format-version', '1'], consumer))
            lock = (consumer / 'Cargo.lock').read_bytes()
            import tomllib
            def packages(data):
                return {(p['name'], p['version'], p.get('source'))
                        for p in tomllib.loads(data.decode())['package'] if p.get('source')}
            assert not packages(lock) - packages((source / 'Cargo.lock').read_bytes())
            manifest.update(resolved_lock_sha256=sha(lock), domains_sha256=sha(domains),
                            positive_sha256=sha(positive), consumer_manifest_sha256=sha((consumer / 'Cargo.toml').read_bytes()),
                            registry_packages=sorted(packages(lock)))
            (evidence / 'consumer.lock').write_bytes(lock)
            cargo = ['cargo', '+1.100.0']
            common = ['--locked', '--offline']
            run('positive-check', cargo + ['check', *common], consumer)
            output = run('runtime-schema-domain-operation', cargo + ['run', '--quiet', *common], consumer)
            assert output.strip() == b'native contracts: typed domains, operation refusal and schema isolation verified'
            run('positive-clippy', cargo + ['clippy', *common, '--', '-D', 'warnings'], consumer)
            cases = json.loads((source / 'ci/native-contracts/cases.json').read_text())
            assert len(cases) == 7 and len({case['name'] for case in cases}) == len(cases)
            manifest['negative_probes'] = []
            for case in cases:
                if args.negative_control_valid_program:
                    case = dict(case, with_text=case['replace'])
                text = positive.decode()
                assert text.count(case['replace']) == 1, case['name']
                changed = text.replace(case['replace'], case['with_text'])
                probe.write_text(changed)
                raw = run(case['name'], cargo + ['check', *common, '--message-format=json'], consumer, 101)
                messages = []
                for line in raw.splitlines():
                    try:
                        message = json.loads(line)
                    except (ValueError, UnicodeDecodeError):
                        continue
                    if message.get('reason') == 'compiler-message' and message['message']['level'] == 'error':
                        messages.append(message['message'])
                assert messages and all(message.get('code', {}).get('code') == case['error_code'] for message in messages), case['name']
                assert any(case['diagnostic_contains'] in message.get('rendered', '')
                           and any(span['is_primary'] and application_span(span)
                                   for span in message['spans']) for message in messages), case['name']
                manifest['negative_probes'].append(dict(name=case['name'], error_code=case['error_code'],
                    source_sha256=sha(changed.encode()), diagnostic_contains=case['diagnostic_contains']))
                # Restore the exact positive application after each deliberate
                # refusal. This prevents unrelated dependency/build failure
                # from masquerading as an accepted negative probe.
                probe.write_bytes(positive)
                run(case['name'] + '-restored', cargo + ['check', *common], consumer)
                assert (consumer / 'Cargo.lock').read_bytes() == lock
                print(case['name'], case['error_code'], 'verified', flush=True)
            assert all(sha((source / path).read_bytes()) == value for path, value in hashes.items())
            assert sha((consumer / 'src/domains.rs').read_bytes()) == sha(domains)
            assert probe.read_bytes() == positive
            manifest['status'] = 'passed'
    except BaseException as error:
        manifest['status'] = 'failed_or_interrupted'
        manifest['failure'] = repr(error)
        raise
    finally:
        manifest['artifacts_sha256'] = {p.name: sha(p.read_bytes()) for p in evidence.iterdir() if p.name != 'manifest.json'}
        save()
    print(evidence.relative_to(root), flush=True)


if __name__ == '__main__':
    main()
