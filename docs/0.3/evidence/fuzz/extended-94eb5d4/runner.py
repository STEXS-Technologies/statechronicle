"""One-hour seeded ASAN campaign for every target on an immutable Git archive."""
import gzip, hashlib, io, json, os
from pathlib import Path
import re, shutil, subprocess, tarfile, tempfile, time, tomllib

root = Path.cwd()
revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
out = root / 'docs/0.3/evidence/fuzz' / ('extended-' + revision[:7])
out.mkdir(exist_ok=False)
sha = lambda b: hashlib.sha256(b).hexdigest()
paths = ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'clippy.toml', 'crates', 'fuzz']
archive = subprocess.check_output(['git', 'archive', revision, '--', *paths])
manifest = dict(candidate_commit=revision, archive_paths=paths, archive_sha256=sha(archive),
                status='running', seconds_per_target=3600, checks=[], campaigns=[],
                nightly=subprocess.check_output(['rustc', '+nightly', '-vV'], text=True),
                limitations=['One deterministic seed per target; finite CPU/time/input coverage.',
                             'In-memory fixtures do not prove physical backend durability.',
                             'Targets run concurrently on a shared host; no performance claim.'])
(out / 'runner.py').write_bytes(Path(__file__).read_bytes())
def save():
    (out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
save()
try:
    with tempfile.TemporaryDirectory(prefix='statechronicle-final-fuzz-') as temporary:
        work = Path(temporary) / 'source'
        work.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(work, filter='data')
        def inputs():
            return {str(p.relative_to(work)): sha(p.read_bytes()) for p in work.rglob('*') if p.is_file()}
        manifest['source_sha256'] = inputs()
        target_dir = Path(temporary) / 'target'
        env = dict(os.environ, RUSTC_WRAPPER='', CARGO_BUILD_JOBS='8', CARGO_TARGET_DIR=str(target_dir), CARGO_NET_OFFLINE='true')
        manifest['environment'] = {k: env[k] for k in ['RUSTC_WRAPPER', 'CARGO_BUILD_JOBS', 'CARGO_TARGET_DIR', 'CARGO_NET_OFFLINE']}
        manifest['build_isolation'] = 'fresh per-archive target directory'
        host = re.search(r'^host: (.+)$', manifest['nightly'], re.M)[1]
        command = ['cargo', '+nightly', 'fuzz', 'build', '--target', host]
        result = subprocess.run(command, cwd=work, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        data = gzip.compress(result.stdout, mtime=0)
        (out / 'build.log.gz').write_bytes(data)
        manifest['checks'].append(dict(command=command, exit_code=result.returncode, artifact='build.log.gz', sha256=sha(data)))
        save()
        if result.returncode:
            raise RuntimeError('instrumented build failed')
        declared = sorted(b['name'] for b in tomllib.loads((work / 'fuzz/Cargo.toml').read_text())['bin'])
        listed = sorted(subprocess.check_output(['cargo', '+nightly', 'fuzz', 'list'], cwd=work, env=env, text=True).split())
        if declared != listed or not declared:
            raise RuntimeError('manifest/list mismatch')
        manifest['targets'] = declared
        jobs = []
        for index, name in enumerate(declared):
            corpus = Path(temporary) / 'corpus' / name
            corpus.mkdir(parents=True)
            fixed = {'fuzz_claim_verifier': 'claim-verifier', 'fuzz_durable_recovery': 'durable-recovery'}.get(name)
            if fixed:
                shutil.copytree(work / 'fuzz/seed-corpus' / fixed, corpus, dirs_exist_ok=True)
            initial = {p.name: sha(p.read_bytes()) for p in corpus.iterdir()}
            artifacts = Path(temporary) / 'artifacts' / name
            artifacts.mkdir(parents=True)
            binary = target_dir / host / 'release' / name
            command = [str(binary), str(corpus), '-seed=' + str(7331 + index), '-max_total_time=3600', '-artifact_prefix=' + str(artifacts) + '/']
            stream = (out / (name + '.log')).open('wb')
            process = subprocess.Popen(command, cwd=work, env=env, stdout=stream, stderr=subprocess.STDOUT)
            record = dict(target=name, pid=process.pid, command=command, seed=7331+index, binary_sha256=sha(binary.read_bytes()), initial_corpus_sha256=initial, status='running')
            manifest['campaigns'].append(record)
            jobs.append((process, stream, record, corpus, artifacts))
        save()
        print('Started', len(jobs), 'targets for 3600 seconds each; evidence:', out, flush=True)
        # Poll all processes so an early failure is recorded immediately.
        pending = list(jobs)
        while pending:
            for job in list(pending):
                process, stream, record, corpus, artifacts = job
                code = process.poll()
                if code is None:
                    continue
                stream.close()
                log = out / (record['target'] + '.log')
                raw = log.read_bytes()
                compressed = gzip.compress(raw, mtime=0)
                name = log.name + '.gz'
                (out / name).write_bytes(compressed)
                log.unlink()
                counts = re.findall(rb'Done (\d+) runs in (\d+) second', raw)
                record.update(exit_code=code, artifact=name, sha256=sha(compressed), crash_artifacts=len(list(artifacts.iterdir())))
                if len(counts) == 1:
                    record.update(completed_runs=int(counts[0][0]), reported_seconds=int(counts[0][1]))
                record['status'] = 'passed' if code == 0 and len(counts) == 1 and int(counts[0][1]) >= 3600 and not list(artifacts.iterdir()) and b'inline 8-bit counters' in raw else 'failed'
                for kind, directory in [('corpus', corpus), ('artifacts', artifacts)]:
                    name = record['target'] + '-' + kind + '.tar.gz'
                    with tarfile.open(out / name, 'w:gz') as tar:
                        tar.add(directory, arcname=kind)
                    record[kind + '_archive'] = name
                    record[kind + '_sha256'] = sha((out / name).read_bytes())
                pending.remove(job)
                save()
                print(record['target'], record['status'], code, flush=True)
            if pending:
                time.sleep(5)
        manifest['source_unchanged'] = inputs() == manifest['source_sha256']
        manifest['status'] = 'passed' if manifest['source_unchanged'] and all(c['status'] == 'passed' for c in manifest['campaigns']) else 'failed'
except BaseException as error:
    manifest.update(status='failed_or_interrupted', error=repr(error))
    for process, stream, *_ in locals().get('jobs', []):
        if process.poll() is None:
            process.terminate()
        process.wait()
        stream.close()
    raise
finally:
    save()
print(manifest['status'], flush=True)
if manifest['status'] != 'passed':
    raise SystemExit(1)
