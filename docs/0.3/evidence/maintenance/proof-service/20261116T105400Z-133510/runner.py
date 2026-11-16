#!/usr/bin/env python3
"""Compare exact legacy proof errors and read traces across committed revisions."""
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

from refactor_compat import packages


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--revision', action='append', required=True)
    args = cli.parse_args()
    if len(args.revision) < 2:
        cli.error('supply at least two revisions')
    root = Path(__file__).resolve().parents[1]
    def git(*args):
        return subprocess.check_output(['git', *args], cwd=root)
    def sha(data):
        return hashlib.sha256(data).hexdigest()
    revisions = [git('rev-parse', rev).decode().strip() for rev in args.revision]
    stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) + f'-{os.getpid()}'
    run = root / 'target/proof-service-parity' / stamp
    evidence = root / 'docs/0.3/evidence/maintenance/proof-service' / stamp
    evidence.mkdir(parents=True)
    env = dict(os.environ, RUSTC_WRAPPER='', CARGO_BUILD_JOBS='8', CARGO_TARGET_DIR=str(root/'target/proof-service-parity/build'))
    fixture = root/'ci/characterization/proof_service.rs'
    wire = root/'ci/wire-0.2/history.json'
    manifest = {'revisions': revisions, 'fixture_sha256':sha(fixture.read_bytes()), 'wire_sha256':sha(wire.read_bytes()),
                'rustc':subprocess.check_output(['rustc','-vV'],cwd=root,text=True), 'checks':[], 'lanes':[], 'status':'running',
                'environment':{k:env[k] for k in ['RUSTC_WRAPPER','CARGO_BUILD_JOBS','CARGO_TARGET_DIR']},
                'limitations':['Four compatibility state-proof methods; new expected-claim and absence/trade verification are separate.',
                               'Deterministic matrix over frozen original wire; not exhaustive input/property/concurrency qualification.',
                               'A dependency-patched original comparator must be identified separately from unmodified release execution.']}
    (evidence/'proof_service.rs').write_bytes(fixture.read_bytes())
    (evidence/'runner.py').write_bytes(Path(__file__).read_bytes())
    def save():
        (evidence/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    def check(label, command, cwd):
        start=time.monotonic()
        result=subprocess.run(command,cwd=cwd,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
        artifact=label+'.log.gz';data=gzip.compress(result.stdout,mtime=0)
        (evidence/artifact).write_bytes(data)
        manifest['checks'].append({'label':label,'command':command,'cwd':str(cwd),'exit_code':result.returncode,'seconds':time.monotonic()-start,'artifact':artifact,'sha256':sha(data)})
        save();print(label,result.returncode,flush=True)
        if result.returncode: raise RuntimeError(label+' failed')
        return result.stdout
    save()
    try:
        traces=[]
        for index, revision in enumerate(revisions):
            source=run/str(index)/'source';source.mkdir(parents=True)
            data=git('archive',revision,'--','Cargo.toml','Cargo.lock','rust-toolchain.toml','clippy.toml','crates','fuzz')
            with tarfile.open(fileobj=io.BytesIO(data)) as archive: archive.extractall(source,filter='data')
            consumer=source.parent/'consumer';(consumer/'src').mkdir(parents=True)
            (consumer/'src/main.rs').write_bytes(fixture.read_bytes())
            dependencies=tomllib.loads((source/'Cargo.toml').read_text())['workspace']['dependencies']
            ed=dependencies['ed25519-dalek']['version']
            (consumer/'Cargo.toml').write_text('[package]\nname="proof-service-characterization"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\nstatechronicle={path="../source/crates/statechronicle"}\nasync-trait="0.1"\nserde_json="1"\ntokio={version="1",features=["rt"]}\ned25519-dalek="'+ed+'"\n')
            (consumer/'Cargo.lock').write_bytes((source/'Cargo.lock').read_bytes())
            check(f'{index}-resolve',['cargo','metadata','--offline','--format-version','1'],consumer)
            assert not packages(consumer/'Cargo.lock')-packages(source/'Cargo.lock'), 'third-party dependency drift'
            lock=(consumer/'Cargo.lock').read_bytes();(evidence/f'{index}.lock').write_bytes(lock)
            check(f'{index}-build',['cargo','build','--locked','--offline'],consumer)
            binary=Path(env['CARGO_TARGET_DIR'])/'debug/proof-service-characterization'
            raw=check(f'{index}-trace',[str(binary),str(wire)],consumer)
            trace=json.loads(raw);assert len(trace)==2352
            traces.append(trace)
            manifest['lanes'].append({'revision':revision,'archive_sha256':sha(data),'source_sha256':{str(p.relative_to(source)):sha(p.read_bytes()) for p in sorted(source.rglob('*')) if p.is_file()},'consumer_lock_sha256':sha(lock),'trace_sha256':sha(raw),'rows':len(trace),'binary_sha256':sha(binary.read_bytes())})
            assert (consumer/'Cargo.lock').read_bytes()==lock
            save()
        assert all(trace==traces[0] for trace in traces[1:]), 'error or port-order trace mismatch'
        manifest['status']='passed'
    except BaseException as error:
        manifest['status']='failed_or_interrupted';manifest['failure']=repr(error);raise
    finally:
        manifest['artifacts_sha256']={p.name:sha(p.read_bytes()) for p in sorted(evidence.iterdir()) if p.name!='manifest.json'}
        save()
    print(evidence.relative_to(root),flush=True)


if __name__=='__main__': main()
